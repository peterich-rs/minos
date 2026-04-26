//! Envelope-aware mobile client.
//!
//! Plan 05 replaces the jsonrpsee-backed client with a bespoke envelope
//! WebSocket loop (spec §6) so the mobile side can consume
//! `EventKind::UiEventMessage` frames live; pairing and history reads now
//! ride the backend's HTTP `/v1/*` control plane via [`crate::http`].
//!
//! Responsibilities:
//!
//! - Parse a scanned QR v2 payload (`PairingQrPayload` from
//!   `minos_protocol::messages`) and persist its fields into the
//!   [`MobilePairingStore`]: backend URL, CF Access tokens (if any),
//!   eventually the `DeviceSecret` minted by the backend on successful
//!   pair.
//! - Maintain a single outbound WebSocket; expose `ConnectionState` via a
//!   `watch::Receiver` and live `UiEventFrame` over a `broadcast::Sender`.
//!
//! For FFI use, [`MobileClient::new_with_in_memory_store`] avoids exposing
//! the `Arc<dyn MobilePairingStore>` trait object across the frb boundary
//! (real Keychain persistence lives on the Dart side; see plan D5).

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{AgentName, ConnectionState, DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    AuthSummary, Envelope, EventKind, GetThreadLastSeqParams, GetThreadLastSeqResponse,
    ListThreadsParams, ListThreadsResponse, PairingQrPayload, ReadThreadParams, ReadThreadResponse,
    RefreshResponse, SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
};
use minos_ui_protocol::UiEventMessage;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex, RwLock};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::auth::{AuthSession, AuthStateFrame};
use crate::rpc::{drain_pending, forward_rpc, RpcReply};
use crate::store::{InMemoryPairingStore, MobilePairingStore, PersistedPairingState};
use crate::ReconnectController;

/// One live UI event pushed from backend fan-out. Mobile layers consume
/// these via [`MobileClient::ui_events_stream`] (broadcast receiver).
#[derive(Debug, Clone)]
pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

/// Envelope-speaking mobile client. One instance per iPhone process.
///
/// Several fields are `Arc<Mutex<...>>` rather than plain `Mutex<...>` so
/// the reconnect loop spawned by [`MobileClient::ensure_reconnect_loop`]
/// can hold its own clone without needing `Arc<Self>`. The opaque-handle
/// pattern frb uses (the wrapper holds a plain `MobileClient`, not an
/// `Arc`) makes `Arc<Self>` infeasible.
pub struct MobileClient {
    store: Arc<dyn MobilePairingStore>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    outbox: Arc<Mutex<Option<mpsc::Sender<Envelope>>>>,
    device_id: DeviceId,
    self_name: String,
    /// Live send + recv task handles for the current WebSocket. Aborted
    /// in `connect` / `connect_with_handles` before a fresh pair is
    /// pushed, so the Vec stays bounded at exactly two entries per live
    /// connection rather than growing across reconnects.
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Outstanding forward-RPC oneshots, keyed by the JSON-RPC id we
    /// allocated. The recv-loop drains this on every disconnect via
    /// [`crate::rpc::drain_pending`].
    pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>>,
    /// Monotonic id allocator for outbound forward-RPCs. Process-local;
    /// re-issued from 1 on each new MobileClient (a fresh client = a
    /// fresh paired/auth session in practice).
    next_id: Arc<AtomicU64>,
    /// Watch channel publishing the latest [`AuthStateFrame`] to UI /
    /// reconnect-loop subscribers.
    auth_state_tx: watch::Sender<AuthStateFrame>,
    auth_state_rx: watch::Receiver<AuthStateFrame>,
    /// Live auth tuple. `Some` between login/refresh and logout/refresh-
    /// failure. The reconnect loop and the bearer-stamping helpers read
    /// it; only the auth public methods write.
    auth_session: Arc<RwLock<Option<AuthSession>>>,
    /// Backoff state machine consulted by the reconnect loop.
    reconnect: Arc<ReconnectController>,
    /// Live reconnect-loop join handle. Aborted on Unauthenticated /
    /// RefreshFailed so we don't keep poking the backend after logout.
    reconnect_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl MobileClient {
    #[must_use]
    pub fn new(store: Arc<dyn MobilePairingStore>, self_name: String) -> Self {
        Self::new_with_device_id(store, self_name, DeviceId::new())
    }

    #[must_use]
    fn new_with_device_id(
        store: Arc<dyn MobilePairingStore>,
        self_name: String,
        device_id: DeviceId,
    ) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (ui_events_tx, _) = broadcast::channel(256);
        let (auth_state_tx, auth_state_rx) = watch::channel(AuthStateFrame::Unauthenticated);
        Self {
            store,
            state_tx,
            state_rx,
            ui_events_tx,
            outbox: Arc::new(Mutex::new(None)),
            device_id,
            self_name,
            tasks: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(DashMap::new()),
            next_id: Arc::new(AtomicU64::new(1)),
            auth_state_tx,
            auth_state_rx,
            auth_session: Arc::new(RwLock::new(None)),
            reconnect: Arc::new(ReconnectController::new()),
            reconnect_handle: Mutex::new(None),
        }
    }

    /// FFI-friendly constructor. The Dart side owns real persistence via
    /// `flutter_secure_storage` (plan D5); this default is the in-memory
    /// backing so the FFI surface never leaks `Arc<dyn MobilePairingStore>`.
    #[must_use]
    pub fn new_with_in_memory_store(self_name: String) -> Self {
        Self::new(Arc::new(InMemoryPairingStore::new()), self_name)
    }

    /// Rehydrate a client from a previously persisted snapshot. Missing or
    /// malformed device credentials fall back to a fresh device id; any valid
    /// backend URL / CF Access headers / persisted auth are still restored
    /// into the in-memory pairing store. Restored auth tokens also seed the
    /// live `auth_session` and emit `AuthStateFrame::Authenticated` so a
    /// cold-start resume sees the same state as a fresh login.
    ///
    /// Device id resolution priority: prefer the persisted id (so the JWT
    /// `did` claim still matches after relaunch), fall back to a freshly
    /// minted one only if no persisted id is present.
    #[must_use]
    pub fn new_with_persisted_state(self_name: String, state: PersistedPairingState) -> Self {
        let device = restored_device(&state);
        let device_id = device
            .as_ref()
            .map(|(id, _)| *id)
            .or_else(|| restored_device_id_only(&state))
            .unwrap_or_default();
        let auth_persisted = restored_auth(&state);
        let store = Arc::new(InMemoryPairingStore::from_parts(
            state.backend_url.clone(),
            restored_cf_access(&state),
            device,
            auth_persisted.clone(),
        ));

        // Pre-build the live AuthSession so we can seed the RwLock at
        // construction time — there's no async runtime guarantee at this
        // call site (Dart calls this during first-run isolate spawn,
        // tests call from #[tokio::test]; only the latter has a runtime).
        let live_auth = auth_persisted.map(|a| {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let remaining_ms = u64::try_from((a.access_expires_at_ms - now_ms).max(0)).unwrap_or(0);
            AuthSession {
                access_token: a.access_token,
                access_expires_at_ms: a.access_expires_at_ms,
                access_expires_at: Instant::now() + Duration::from_millis(remaining_ms),
                refresh_token: a.refresh_token,
                account: AuthSummary {
                    account_id: a.account_id,
                    email: a.account_email,
                },
            }
        });

        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (ui_events_tx, _) = broadcast::channel(256);
        let initial_auth_frame = match &live_auth {
            Some(s) => AuthStateFrame::Authenticated {
                account: s.account.clone(),
            },
            None => AuthStateFrame::Unauthenticated,
        };
        let (auth_state_tx, auth_state_rx) = watch::channel(initial_auth_frame);
        Self {
            store,
            state_tx,
            state_rx,
            ui_events_tx,
            outbox: Arc::new(Mutex::new(None)),
            device_id,
            self_name,
            tasks: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(DashMap::new()),
            next_id: Arc::new(AtomicU64::new(1)),
            auth_state_tx,
            auth_state_rx,
            auth_session: Arc::new(RwLock::new(live_auth)),
            reconnect: Arc::new(ReconnectController::new()),
            reconnect_handle: Mutex::new(None),
        }
    }

    /// Current connection state snapshot. Cheap and synchronous.
    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    /// Subscribe to connection-state transitions. First read on the
    /// receiver returns the current cached value.
    #[must_use]
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Subscribe to live UI events from backend fan-out. Creates a fresh
    /// broadcast receiver so each subscriber gets its own lag window.
    #[must_use]
    pub fn ui_events_stream(&self) -> broadcast::Receiver<UiEventFrame> {
        self.ui_events_tx.subscribe()
    }

    /// Return the device id the client registered with. Stable for the
    /// lifetime of the process (re-generated on restart until persisted).
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Export the current pairing snapshot so Dart can mirror it into secure
    /// storage after pairing succeeds.
    pub async fn persisted_pairing_state(&self) -> Result<PersistedPairingState, MinosError> {
        let backend_url = self.store.load_backend_url().await?;
        let cf_access = self.store.load_cf_access().await?;
        let device = self.store.load_device().await?;
        let auth = self.store.load_auth().await?;

        Ok(PersistedPairingState {
            backend_url,
            device_id: device.as_ref().map(|(id, _)| id.to_string()),
            device_secret: device
                .as_ref()
                .map(|(_, secret)| secret.as_str().to_string()),
            cf_access_client_id: cf_access.as_ref().map(|(id, _)| id.clone()),
            cf_access_client_secret: cf_access.as_ref().map(|(_, secret)| secret.clone()),
            access_token: auth.as_ref().map(|a| a.access_token.clone()),
            access_expires_at_ms: auth.as_ref().map(|a| a.access_expires_at_ms),
            refresh_token: auth.as_ref().map(|a| a.refresh_token.clone()),
            account_id: auth.as_ref().map(|a| a.account_id.clone()),
            account_email: auth.as_ref().map(|a| a.account_email.clone()),
        })
    }

    /// Reconnect using the durable pairing snapshot already loaded into the
    /// backing store. This is the cold-start resume path used by Dart after it
    /// reconstructs the client from secure storage.
    pub async fn resume_persisted_session(&self) -> Result<(), MinosError> {
        if matches!(self.current_state(), ConnectionState::Connected) {
            return Ok(());
        }

        let Some(backend_url) = self.store.load_backend_url().await? else {
            return Err(MinosError::StoreCorrupt {
                path: "persisted_pairing_state.backend_url".into(),
                message: "missing backend_url for resume".into(),
            });
        };
        let Some((device_id, device_secret)) = self.store.load_device().await? else {
            return Err(MinosError::StoreCorrupt {
                path: "persisted_pairing_state.device".into(),
                message: "missing device_id/device_secret for resume".into(),
            });
        };
        if device_id != self.device_id {
            return Err(MinosError::StoreCorrupt {
                path: "persisted_pairing_state.device_id".into(),
                message: format!(
                    "stored device_id {device_id} does not match client device_id {}",
                    self.device_id
                ),
            });
        }

        let cf_access = self.store.load_cf_access().await?;
        let _ = self
            .state_tx
            .send(ConnectionState::Reconnecting { attempt: 1 });

        let access = self
            .auth_session
            .read()
            .await
            .as_ref()
            .map(|s| s.access_token.clone());

        let result = self
            .connect(
                &backend_url,
                device_secret.as_str(),
                cf_access,
                access.as_deref(),
            )
            .await;

        match result {
            Ok(()) => {
                let _ = self.state_tx.send(ConnectionState::Connected);
                // If the persisted snapshot was authenticated, fire up
                // the reconnect loop so subsequent drops are handled
                // automatically.
                if access.is_some() {
                    self.ensure_reconnect_loop().await;
                }
                Ok(())
            }
            Err(err) => {
                let _ = self.state_tx.send(ConnectionState::Disconnected);
                Err(err)
            }
        }
    }

    // ─────────────────────────── pairing flow ────────────────────────────

    /// Scan a QR v2 payload (raw JSON). Persists `backend_url` + CF tokens
    /// to the store, calls `POST /v1/pairing/consume` over HTTP, persists
    /// the returned `DeviceSecret`, opens the authenticated WebSocket, and
    /// transitions [`ConnectionState`] through `Pairing → Connected`.
    ///
    /// Errors:
    /// - `StoreCorrupt { path: "qr_payload", .. }` when the JSON doesn't
    ///   parse.
    /// - `PairingQrVersionUnsupported` when `qr.v != 2`.
    /// - `ConnectFailed` / `Disconnected` on WS or RPC round-trip failures.
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        let qr: PairingQrPayload =
            serde_json::from_str(&qr_json).map_err(|e| MinosError::StoreCorrupt {
                path: "qr_payload".into(),
                message: e.to_string(),
            })?;
        if qr.v != 2 {
            return Err(MinosError::PairingQrVersionUnsupported { version: qr.v });
        }
        self.store.save_backend_url(&qr.backend_url).await?;
        let cf = match (
            qr.cf_access_client_id.clone(),
            qr.cf_access_client_secret.clone(),
        ) {
            (Some(id), Some(sec)) => {
                self.store.save_cf_access(&id, &sec).await?;
                Some((id, sec))
            }
            _ => None,
        };

        let _ = self.state_tx.send(ConnectionState::Pairing);

        // Phase 2 made `/v1/pairing/consume` bearer-gated for ios-client.
        // Caller must already be authenticated (register/login set
        // auth_session). Surface the missing-bearer case as Unauthorized
        // rather than a raw HTTP 401, since UI hint is the same.
        let access = {
            let guard = self.auth_session.read().await;
            guard
                .as_ref()
                .map(|s| s.access_token.clone())
                .ok_or_else(|| MinosError::Unauthorized {
                    reason: "pair_with_qr_json requires login".into(),
                })?
        };

        // Step 1: redeem the pairing token over HTTP. The backend records
        // both device-secret hashes and pushes Event::Paired to the Mac
        // before returning, so by the time we get the response the Mac is
        // already updated.
        let http = crate::http::MobileHttpClient::new(&qr.backend_url, self.device_id, cf.clone())?;
        let pair_resp = http
            .pair_consume(
                minos_protocol::PairConsumeRequest {
                    token: minos_domain::PairingToken(qr.pairing_token),
                    device_name: self.self_name.clone(),
                },
                &access,
            )
            .await?;

        let device_secret = pair_resp.your_device_secret.clone();
        self.store
            .save_device(&self.device_id, &device_secret)
            .await?;

        // Step 2: now open the WS with the freshly-issued secret. From here
        // on every connect carries `X-Device-Secret` and `Authorization:
        // Bearer` (the latter required by Phase 2 for ios-client uploads).
        self.connect(&qr.backend_url, device_secret.as_str(), cf, Some(&access))
            .await?;

        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(())
    }

    /// Forget the current pairing. Clears secure storage, drops the
    /// socket, and emits `Disconnected`. Idempotent.
    pub async fn forget_peer(&self) -> Result<(), MinosError> {
        let backend_url = self.store.load_backend_url().await?;
        let device = self.store.load_device().await?;
        let cf = self.store.load_cf_access().await?;

        // Best-effort: ask the backend to tear down its side too. Failure
        // here must not block the local-state cleanup below — the user
        // re-pairs to recover.
        if let (Some(url), Some((_, secret))) = (backend_url.as_deref(), device.as_ref()) {
            let http = crate::http::MobileHttpClient::new(url, self.device_id, cf)?;
            let _ = http.forget_pairing(secret).await;
        }

        self.store.clear_all().await?;
        self.shutdown_outbound().await;
        let _ = self.state_tx.send(ConnectionState::Disconnected);
        Ok(())
    }

    // ─────────────────────────── history rpcs ────────────────────────────

    /// Request a page of thread summaries from the backend. Phase 2 made
    /// the route bearer-gated; callers must already have logged in.
    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let (backend_url, secret, cf) = self.http_creds().await?;
        let access = self.access_token_or_unauthorized().await?;
        let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
        http.list_threads(&secret, &access, req).await
    }

    /// Read a window of translated UI events from one thread.
    pub async fn read_thread(
        &self,
        req: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        let (backend_url, secret, cf) = self.http_creds().await?;
        let access = self.access_token_or_unauthorized().await?;
        let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
        http.read_thread(&secret, &access, req).await
    }

    /// Host-only helper (mobile rarely uses this; included for parity).
    pub async fn get_thread_last_seq(
        &self,
        req: GetThreadLastSeqParams,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let (backend_url, secret, cf) = self.http_creds().await?;
        let access = self.access_token_or_unauthorized().await?;
        let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
        http.get_thread_last_seq(&secret, &access, &req.thread_id)
            .await
    }

    /// Pluck the live access token out of `auth_session`, or surface
    /// `Unauthorized` if no session is in place. Used by every
    /// account-aware HTTP call. The reconnect loop is responsible for
    /// triggering a refresh before the token expires; callers do not
    /// retry on 401 here (Phase 6 Task 6.4 layers retry on top).
    async fn access_token_or_unauthorized(&self) -> Result<String, MinosError> {
        self.auth_session
            .read()
            .await
            .as_ref()
            .map(|s| s.access_token.clone())
            .ok_or_else(|| MinosError::Unauthorized {
                reason: "no active session".into(),
            })
    }

    // ─────────────────────────── agent dispatch ────────────────────────────

    /// Start a fresh agent session and deliver `prompt` as the first user
    /// message. Spec §6.2 / plan 08a Task 5.4.
    ///
    /// Composes two forward-RPCs:
    ///   1. `minos_start_agent` → `StartAgentResponse { session_id, cwd }`
    ///   2. `minos_send_user_message` against that session, carrying the
    ///      caller-supplied prompt.
    ///
    /// `StartAgentResponse.session_id` IS the daemon's `thread_id` (per the
    /// doc comment on the protocol type). Per-call timeouts: 60s for the
    /// start, 10s for the prompt delivery. Returns the start response so
    /// callers can stash the session id; on send failure the caller knows
    /// the session is started but unaddressed.
    pub async fn start_agent(
        &self,
        agent: AgentName,
        prompt: String,
    ) -> Result<StartAgentResponse, MinosError> {
        let outbox = self
            .outbox
            .lock()
            .await
            .clone()
            .ok_or(MinosError::NotConnected)?;
        let req = StartAgentRequest { agent };
        let resp: StartAgentResponse = forward_rpc(
            &self.pending,
            &self.next_id,
            &outbox,
            "minos_start_agent",
            req,
            Duration::from_secs(60),
        )
        .await?;
        let send_req = SendUserMessageRequest {
            session_id: resp.session_id.clone(),
            text: prompt,
        };
        let _: () = forward_rpc(
            &self.pending,
            &self.next_id,
            &outbox,
            "minos_send_user_message",
            send_req,
            Duration::from_secs(10),
        )
        .await?;
        Ok(resp)
    }

    /// Send a user message into an existing agent session. Spec §6.2.
    pub async fn send_user_message(
        &self,
        session_id: String,
        text: String,
    ) -> Result<(), MinosError> {
        let outbox = self
            .outbox
            .lock()
            .await
            .clone()
            .ok_or(MinosError::NotConnected)?;
        let req = SendUserMessageRequest { session_id, text };
        let _: () = forward_rpc(
            &self.pending,
            &self.next_id,
            &outbox,
            "minos_send_user_message",
            req,
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Stop the agent currently running on the paired Mac. The daemon's
    /// `stop_agent` RPC takes `()` (no `StopAgentRequest` struct exists),
    /// so we serialise `serde_json::Value::Null` as the params payload.
    pub async fn stop_agent(&self) -> Result<(), MinosError> {
        let outbox = self
            .outbox
            .lock()
            .await
            .clone()
            .ok_or(MinosError::NotConnected)?;
        let _: () = forward_rpc(
            &self.pending,
            &self.next_id,
            &outbox,
            "minos_stop_agent",
            serde_json::Value::Null,
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Subscribe to auth-state transitions. The first read on the receiver
    /// returns the current cached frame. Spec §6.1.
    #[must_use]
    pub fn subscribe_auth_state(&self) -> watch::Receiver<AuthStateFrame> {
        self.auth_state_rx.clone()
    }

    // ─────────────────────────── auth surface ──────────────────────────────

    /// Register a new account on the backend. On success the bearer +
    /// refresh tokens are stored both in memory (via `auth_session`) and
    /// in the durable store. The auth-state watch transitions to
    /// `Authenticated` and the reconnect loop starts. Spec §5.4 / §6.1.
    pub async fn register(
        &self,
        email: String,
        password: String,
    ) -> Result<AuthSummary, MinosError> {
        let http = self.http_client_no_secret().await?;
        let resp = http.register(&email, &password).await?;
        let summary = self.adopt_auth_response(resp).await;
        self.ensure_reconnect_loop().await;
        Ok(summary)
    }

    /// Log into an existing account on the backend. Same shape as
    /// `register` modulo the create-vs-find behaviour on the server. Spec
    /// §5.4.
    pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
        let http = self.http_client_no_secret().await?;
        let resp = http.login(&email, &password).await?;
        let summary = self.adopt_auth_response(resp).await;
        self.ensure_reconnect_loop().await;
        Ok(summary)
    }

    /// Rotate the bearer + refresh tokens. The auth-state watch
    /// transitions to `Refreshing` for the duration of the call; on
    /// success it returns to `Authenticated` (with the same account
    /// summary), on failure the session is wiped and the watch publishes
    /// `RefreshFailed`. Spec §5.4 / §6.1.
    pub async fn refresh_session(&self) -> Result<(), MinosError> {
        let session = self.auth_session.read().await.clone().ok_or_else(|| {
            MinosError::AuthRefreshFailed {
                message: "no session".into(),
            }
        })?;
        let _ = self.auth_state_tx.send(AuthStateFrame::Refreshing);
        let http = self.http_client_no_secret().await?;
        match http.refresh(&session.refresh_token).await {
            Ok(r) => {
                self.adopt_refresh_response(session.account.clone(), r)
                    .await;
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = self.auth_state_tx.send(AuthStateFrame::RefreshFailed {
                    error: Arc::new(MinosError::AuthRefreshFailed {
                        message: msg.clone(),
                    }),
                });
                self.clear_auth_session_and_disconnect().await;
                Err(MinosError::AuthRefreshFailed { message: msg })
            }
        }
    }

    /// Log out of the current session. Best-effort `stop_agent` (2s
    /// timeout) so the daemon doesn't keep a session running for an
    /// account that's no longer holding the iOS client; then call the
    /// backend's `/v1/auth/logout` to revoke the named refresh token;
    /// then wipe the local auth state and drop the WS. Spec §5.4 / §8.3.
    pub async fn logout(&self) -> Result<(), MinosError> {
        // Best-effort agent stop so the Mac doesn't stay in a running
        // session if the user logs out mid-thread. Failures are silenced
        // — we still want to clear local state on logout.
        let _ = tokio::time::timeout(Duration::from_secs(2), self.stop_agent()).await;

        let session = self.auth_session.read().await.clone();
        if let Some(s) = session {
            // Best-effort logout. If the network is down or the bearer is
            // already invalid we still wipe local state.
            if let Ok(http) = self.http_client_no_secret().await {
                let _ = http.logout(&s.access_token, &s.refresh_token).await;
            }
        }
        self.clear_auth_session_and_disconnect().await;
        Ok(())
    }

    /// Build an HTTP client without requiring a paired device-secret.
    /// Used by the auth surface — `register` / `login` happen before the
    /// device is paired. Backend URL falls back to a localhost stub so
    /// tests that haven't seeded the store can still call into the auth
    /// surface (real builds always have the URL set from a prior QR
    /// scan or a hard-coded default).
    async fn http_client_no_secret(&self) -> Result<crate::http::MobileHttpClient, MinosError> {
        let backend_url =
            self.store
                .load_backend_url()
                .await?
                .ok_or_else(|| MinosError::StoreCorrupt {
                    path: "backend_url".into(),
                    message: "missing backend_url for auth call".into(),
                })?;
        let cf = self.store.load_cf_access().await?;
        crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)
    }

    /// Apply a fresh `AuthResponse` onto the live + durable stores and
    /// emit the `Authenticated` frame. Returns the account summary so
    /// callers can hand it back to Dart.
    async fn adopt_auth_response(&self, resp: minos_protocol::AuthResponse) -> AuthSummary {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let exp_ms = now_ms + (resp.expires_in * 1000);
        let session = AuthSession {
            access_token: resp.access_token.clone(),
            access_expires_at_ms: exp_ms,
            access_expires_at: Instant::now()
                + Duration::from_secs(u64::try_from(resp.expires_in.max(0)).unwrap_or(0)),
            refresh_token: resp.refresh_token.clone(),
            account: resp.account.clone(),
        };
        let _ = self
            .store
            .save_auth(
                resp.access_token.clone(),
                exp_ms,
                resp.refresh_token.clone(),
                resp.account.account_id.clone(),
                resp.account.email.clone(),
            )
            .await;
        *self.auth_session.write().await = Some(session);
        let _ = self.auth_state_tx.send(AuthStateFrame::Authenticated {
            account: resp.account.clone(),
        });
        resp.account
    }

    /// Bundle the handles the reconnect loop needs into one cheap-to-clone
    /// struct so the spawned task can hold them without a `Weak<Self>`.
    fn reconnect_context(&self) -> ReconnectContext {
        ReconnectContext {
            reconnect: self.reconnect.clone(),
            store: self.store.clone(),
            auth_session: self.auth_session.clone(),
            auth_state_tx: self.auth_state_tx.clone(),
            state_tx: self.state_tx.clone(),
            ui_events_tx: self.ui_events_tx.clone(),
            pending: self.pending.clone(),
            outbox: self.outbox.clone(),
            tasks: self.tasks.clone(),
            device_id: self.device_id,
        }
    }

    /// Spawn the reconnect loop as a background task. Idempotent: a
    /// running loop short-circuits the call. Aborted on Unauthenticated
    /// / RefreshFailed by `clear_auth_session_and_disconnect`. Spec §6.3,
    /// plan 08a Task 6.2.
    async fn ensure_reconnect_loop(&self) {
        let mut guard = self.reconnect_handle.lock().await;
        if let Some(h) = guard.as_ref() {
            if !h.is_finished() {
                return;
            }
        }
        let ctx = self.reconnect_context();
        let handle = tokio::spawn(reconnect_loop(ctx));
        *guard = Some(handle);
    }

    /// Apply a fresh `RefreshResponse` onto the live session in place,
    /// preserving the bound account. Emits `Authenticated` again so
    /// observers see a state transition (Refreshing → Authenticated).
    async fn adopt_refresh_response(&self, account: AuthSummary, r: RefreshResponse) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let exp_ms = now_ms + (r.expires_in * 1000);
        let secs = u64::try_from(r.expires_in.max(0)).unwrap_or(0);
        {
            let mut guard = self.auth_session.write().await;
            if let Some(s) = guard.as_mut() {
                s.access_token.clone_from(&r.access_token);
                s.access_expires_at_ms = exp_ms;
                s.access_expires_at = Instant::now() + Duration::from_secs(secs);
                s.refresh_token.clone_from(&r.refresh_token);
            }
        }
        let _ = self
            .store
            .save_auth(
                r.access_token,
                exp_ms,
                r.refresh_token,
                account.account_id.clone(),
                account.email.clone(),
            )
            .await;
        let _ = self
            .auth_state_tx
            .send(AuthStateFrame::Authenticated { account });
    }

    /// Wipe the live + durable auth state, abort any reconnect loop, and
    /// drop the active WS. Used by logout and refresh-failure.
    async fn clear_auth_session_and_disconnect(&self) {
        *self.auth_session.write().await = None;
        let _ = self.store.clear_auth().await;
        if let Some(h) = self.reconnect_handle.lock().await.take() {
            h.abort();
        }
        let _ = self.auth_state_tx.send(AuthStateFrame::Unauthenticated);
        self.shutdown_outbound().await;
        let _ = self.state_tx.send(ConnectionState::Disconnected);
    }

    // ─────────────────────────── lifecycle hooks ───────────────────────────

    /// Notify the reconnect controller that the iOS app moved to the
    /// foreground. Resets backoff and clears the paused flag so the loop
    /// reconnects immediately. Spec §6.3 / §8.3.
    ///
    /// Sync wrapper so Dart's `WidgetsBindingObserver` (main isolate) can
    /// call without an awaitable; the actual mutation is async-safe. If
    /// no Tokio runtime is bound to the calling thread (e.g. an early
    /// lifecycle hook fires before the FFI side has spun one up) we log
    /// at debug and return rather than panicking.
    pub fn notify_foregrounded(&self) {
        let r = self.reconnect.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(async move {
                    r.notify_foregrounded().await;
                });
            }
            Err(_) => {
                tracing::debug!("notify_foregrounded called outside Tokio runtime");
            }
        }
    }

    /// Notify the reconnect controller that the iOS app moved to the
    /// background. Sets paused so the loop's next wakeup exits. Spec
    /// §6.3 / §8.3. Same runtime-handling shape as `notify_foregrounded`.
    pub fn notify_backgrounded(&self) {
        let r = self.reconnect.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(async move {
                    r.notify_backgrounded().await;
                });
            }
            Err(_) => {
                tracing::debug!("notify_backgrounded called outside Tokio runtime");
            }
        }
    }

    /// Resolve `(backend_url, device_secret, cf_access)` from the persisted
    /// pairing store. Used by every HTTP-backed thread query.
    async fn http_creds(
        &self,
    ) -> Result<(String, DeviceSecret, Option<(String, String)>), MinosError> {
        let backend_url =
            self.store
                .load_backend_url()
                .await?
                .ok_or_else(|| MinosError::StoreCorrupt {
                    path: "backend_url".into(),
                    message: "missing backend_url".into(),
                })?;
        let (_, secret) =
            self.store
                .load_device()
                .await?
                .ok_or_else(|| MinosError::StoreCorrupt {
                    path: "device".into(),
                    message: "missing device secret".into(),
                })?;
        let cf = self.store.load_cf_access().await?;
        Ok((backend_url, secret, cf))
    }

    // ─────────────────────────── internals ────────────────────────────

    // The body is mostly header-stamping boilerplate that mirrors
    // `connect_with_handles`; deduplicating across them is deferred to
    // I2 (post-Wave-2 cleanup), so allow the line count for now.
    #[allow(clippy::too_many_lines)]
    async fn connect(
        &self,
        url: &str,
        device_secret: &str,
        cf_access: Option<(String, String)>,
        access_token: Option<&str>,
    ) -> Result<(), MinosError> {
        tracing::info!(
            target: "minos_mobile::client",
            url,
            device_id = %self.device_id,
            cf_access_present = cf_access.is_some(),
            bearer_present = access_token.is_some(),
            "mobile: opening backend WebSocket"
        );
        let mut req = url
            .into_client_request()
            .map_err(|e| MinosError::ConnectFailed {
                url: url.to_string(),
                message: format!("invalid backend URL: {e}"),
            })?;
        let headers = req.headers_mut();
        headers.insert(
            "X-Device-Id",
            self.device_id
                .to_string()
                .parse()
                .map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "device_id is not a valid header value".into(),
                })?,
        );
        headers.insert(
            "X-Device-Role",
            "ios-client".parse().expect("static header value is valid"),
        );
        headers.insert(
            "X-Device-Secret",
            device_secret
                .parse()
                .map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "device_secret is not a valid header value".into(),
                })?,
        );
        if let Some(tok) = access_token {
            headers.insert(
                "Authorization",
                format!("Bearer {tok}")
                    .parse()
                    .map_err(|_| MinosError::ConnectFailed {
                        url: url.to_string(),
                        message: "access_token is not a valid header value".into(),
                    })?,
            );
        }
        if let Some((id, sec)) = cf_access {
            headers.insert(
                "CF-Access-Client-Id",
                id.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_access_client_id is not a valid header value".into(),
                })?,
            );
            headers.insert(
                "CF-Access-Client-Secret",
                sec.parse().map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "cf_access_client_secret is not a valid header value".into(),
                })?,
            );
        }

        let (ws, _resp) = connect_async(req)
            .await
            .map_err(|e| connect_error_to_minos(url, e))?;
        let (mut write, read) = ws.split();

        let (tx, mut rx) = mpsc::channel::<Envelope>(256);

        let send_handle = tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let text = match serde_json::to_string(&env) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(?e, "mobile: envelope serialise failed");
                        continue;
                    }
                };
                if let Err(e) = write.send(Message::Text(text.into())).await {
                    tracing::warn!(?e, "mobile: WS write failed; send loop exiting");
                    break;
                }
            }
        });

        let recv_handle = tokio::spawn(recv_loop(
            read,
            self.ui_events_tx.clone(),
            self.state_tx.clone(),
            self.pending.clone(),
        ));

        *self.outbox.lock().await = Some(tx);
        let mut tasks = self.tasks.lock().await;
        // Abort any handles from a prior connect/reconnect before pushing
        // the new pair — otherwise the Vec grows unboundedly across long
        // reconnect-heavy sessions (2 handles per attempt × N reconnects).
        for h in tasks.drain(..) {
            h.abort();
        }
        tasks.push(send_handle);
        tasks.push(recv_handle);
        Ok(())
    }

    async fn shutdown_outbound(&self) {
        let mut guard = self.outbox.lock().await;
        *guard = None; // drops the Sender; send task exits when channel closes
                       // Drain pending so any in-flight forward_rpc callers see
                       // RequestDropped instead of waiting until their per-call timeout.
        drain_pending(&self.pending);
    }
}

/// Inbound read loop. Decodes each text frame as `Envelope` and surfaces
/// `UiEventMessage` events to the broadcast channel; presence and
/// pairing-state events update the connection-state watch.
///
/// Always drains `pending` and publishes `Disconnected` on exit, regardless
/// of how the read loop terminated. The `Close`/error arms break out of the
/// loop, and a TCP reset that produces no Close frame falls through the
/// `while let Some(...)` and hits the post-loop drain. Without this,
/// in-flight `forward_rpc` callers would hang until per-call timeout.
async fn recv_loop<S>(
    mut read: S,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    state_tx: watch::Sender<ConnectionState>,
    pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>>,
) where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                let text: &str = t.as_ref();
                handle_text_frame(text, &ui_events_tx, &state_tx, &pending);
            }
            Ok(Message::Close(_)) => {
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?e, "mobile: WS read error; inbound loop exiting");
                break;
            }
        }
    }
    let _ = state_tx.send(ConnectionState::Disconnected);
    drain_pending(&pending);
}

fn handle_text_frame(
    text: &str,
    ui_events_tx: &broadcast::Sender<UiEventFrame>,
    state_tx: &watch::Sender<ConnectionState>,
    pending: &DashMap<u64, oneshot::Sender<RpcReply>>,
) {
    let env = match serde_json::from_str::<Envelope>(text) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(?e, text = %text, "mobile: inbound decode error");
            return;
        }
    };
    match env {
        Envelope::Event { event, .. } => match event {
            EventKind::UiEventMessage {
                thread_id,
                seq,
                ui,
                ts_ms,
            } => {
                let _ = ui_events_tx.send(UiEventFrame {
                    thread_id,
                    seq,
                    ui,
                    ts_ms,
                });
            }
            EventKind::Unpaired | EventKind::ServerShutdown => {
                let _ = state_tx.send(ConnectionState::Disconnected);
                drain_pending(pending);
            }
            _ => tracing::debug!(?event, "mobile: ignored event"),
        },
        Envelope::Forwarded { payload, .. } => {
            // Spec §6.2: JSON-RPC `{id, method, params/result/error}` rides
            // inside Envelope::Forwarded.payload; correlation id is the
            // inner `id`, not the envelope.
            let Some(id) = payload.get("id").and_then(serde_json::Value::as_u64) else {
                tracing::debug!(?payload, "mobile: Forwarded missing inner JSON-RPC id");
                return;
            };
            let Some((_, tx)) = pending.remove(&id) else {
                tracing::debug!(id, "mobile: Forwarded with no pending entry");
                return;
            };
            let reply = if let Some(result) = payload.get("result") {
                RpcReply::Ok(result.clone())
            } else if let Some(err) = payload.get("error") {
                let code = err
                    .get("code")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok())
                    .unwrap_or(-32000);
                let message = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                RpcReply::Err { code, message }
            } else {
                RpcReply::Err {
                    code: -32700,
                    message: "malformed jsonrpc reply".into(),
                }
            };
            let _ = tx.send(reply);
        }
        other => tracing::debug!(?other, "mobile: ignored inbound envelope"),
    }
}

/// Map a tungstenite handshake error into a typed `MinosError`, picking
/// the variant the localized UI hint should reflect and stuffing the raw
/// classification into the `message` field so the iOS log panel surfaces
/// the actual cause instead of just `e.to_string()`.
fn connect_error_to_minos(url: &str, err: WsError) -> MinosError {
    let detail = describe_ws_error(&err);
    tracing::warn!(
        target: "minos_mobile::client",
        url,
        kind = detail.kind,
        message = %detail.message,
        "mobile: WebSocket connect failed"
    );

    if matches!(detail.http_status, Some(302 | 401 | 403)) {
        return MinosError::CfAuthFailed {
            message: detail.message,
        };
    }

    MinosError::ConnectFailed {
        url: url.to_string(),
        message: detail.message,
    }
}

/// Structured view of a `tungstenite::Error`, kept private so the mapping
/// in `connect_error_to_minos` doesn't have to keep a parallel `match`.
struct WsErrorDetail {
    kind: &'static str,
    message: String,
    /// Set only when `err` is `WsError::Http(_)`; lets the caller treat
    /// CF-Access redirects specially without re-pattern-matching.
    http_status: Option<u16>,
}

fn describe_ws_error(err: &WsError) -> WsErrorDetail {
    match err {
        WsError::Io(io_err) => WsErrorDetail {
            kind: "io",
            message: format!("io {kind:?}: {io_err}", kind = io_err.kind()),
            http_status: None,
        },
        WsError::Tls(tls_err) => WsErrorDetail {
            kind: "tls",
            message: format!("tls: {tls_err}"),
            http_status: None,
        },
        WsError::Url(url_err) => WsErrorDetail {
            kind: "url",
            message: format!("url: {url_err}"),
            http_status: None,
        },
        WsError::Http(resp) => {
            let status = resp.status();
            let body = resp
                .body()
                .as_deref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let body_snippet = body
                .map(|s| format!(": {}", s.chars().take(160).collect::<String>()))
                .unwrap_or_default();
            WsErrorDetail {
                kind: "http",
                message: format!("http {status}{body_snippet}"),
                http_status: Some(status.as_u16()),
            }
        }
        WsError::HttpFormat(e) => WsErrorDetail {
            kind: "http_format",
            message: format!("http format: {e}"),
            http_status: None,
        },
        WsError::Protocol(e) => WsErrorDetail {
            kind: "protocol",
            message: format!("protocol: {e}"),
            http_status: None,
        },
        WsError::AttackAttempt => WsErrorDetail {
            kind: "attack_attempt",
            message: "tungstenite flagged the response as a potential attack".into(),
            http_status: None,
        },
        WsError::ConnectionClosed | WsError::AlreadyClosed => WsErrorDetail {
            kind: "closed",
            message: format!("{err}"),
            http_status: None,
        },
        other => WsErrorDetail {
            kind: "other",
            message: format!("{other}"),
            http_status: None,
        },
    }
}

fn restored_cf_access(state: &PersistedPairingState) -> Option<(String, String)> {
    match (
        state.cf_access_client_id.as_ref(),
        state.cf_access_client_secret.as_ref(),
    ) {
        (Some(id), Some(secret)) => Some((id.clone(), secret.clone())),
        _ => None,
    }
}

fn restored_auth(state: &PersistedPairingState) -> Option<crate::store::PersistedAuth> {
    match (
        state.access_token.as_ref(),
        state.access_expires_at_ms,
        state.refresh_token.as_ref(),
        state.account_id.as_ref(),
        state.account_email.as_ref(),
    ) {
        (Some(access), Some(exp), Some(refresh), Some(account_id), Some(email)) => {
            Some(crate::store::PersistedAuth {
                access_token: access.clone(),
                access_expires_at_ms: exp,
                refresh_token: refresh.clone(),
                account_id: account_id.clone(),
                account_email: email.clone(),
            })
        }
        _ => None,
    }
}

fn restored_device(state: &PersistedPairingState) -> Option<(DeviceId, DeviceSecret)> {
    let (Some(device_id), Some(device_secret)) =
        (state.device_id.as_deref(), state.device_secret.as_deref())
    else {
        return None;
    };

    match Uuid::parse_str(device_id) {
        Ok(uuid) => Some((DeviceId(uuid), DeviceSecret(device_secret.to_string()))),
        Err(e) => {
            tracing::warn!(
                error = %e,
                device_id,
                "mobile: ignoring malformed persisted device_id"
            );
            None
        }
    }
}

/// Restore just the device id when the device_secret hasn't been minted
/// yet (post-register, pre-pair). The JWT's `did` claim binds the bearer
/// to this id, so we MUST keep using the same value across the
/// register-then-pair flow.
fn restored_device_id_only(state: &PersistedPairingState) -> Option<DeviceId> {
    let raw = state.device_id.as_deref()?;
    match Uuid::parse_str(raw) {
        Ok(uuid) => Some(DeviceId(uuid)),
        Err(e) => {
            tracing::warn!(
                error = %e,
                device_id = raw,
                "mobile: ignoring malformed persisted device_id (id-only path)"
            );
            None
        }
    }
}

/// Cheap-to-clone bundle of the handles the reconnect loop needs. The
/// loop runs as `tokio::spawn(reconnect_loop(ctx))` and outlives the
/// originating call site; cloning the bundle costs only a handful of
/// `Arc::clone`s.
struct ReconnectContext {
    reconnect: Arc<ReconnectController>,
    store: Arc<dyn MobilePairingStore>,
    auth_session: Arc<RwLock<Option<AuthSession>>>,
    auth_state_tx: watch::Sender<AuthStateFrame>,
    state_tx: watch::Sender<ConnectionState>,
    ui_events_tx: broadcast::Sender<UiEventFrame>,
    pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>>,
    outbox: Arc<Mutex<Option<mpsc::Sender<Envelope>>>>,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    device_id: DeviceId,
}

/// Reconnect loop owned by [`MobileClient::ensure_reconnect_loop`].
///
/// Spec §6.3:
/// - Sleeps `reconnect.next_delay()` between attempts.
/// - Honours `reconnect.is_paused()` set by `notify_backgrounded`.
/// - Refreshes the access token if its expiry is within 2 minutes.
/// - Calls into [`connect_with_handles`] (mirrors `MobileClient::connect`
///   but keeps the loop free of `&self`).
/// - On success, records success and waits for the connection to drop;
///   on failure, records the failure and goes back to sleep.
async fn reconnect_loop(ctx: ReconnectContext) {
    loop {
        // Pause on background. We poll because the lifecycle hooks set
        // the flag asynchronously; checking once per loop iteration is
        // sufficient — the next foreground transition resets backoff.
        if ctx.reconnect.is_paused().await {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }

        // Bail early if we don't have everything we need to connect: a
        // backend URL, a device-secret (post-pair), and an active
        // session. The loop will idle here until pair_with_qr_json
        // completes.
        let backend_url_opt: Option<String> =
            ctx.store.load_backend_url().await.unwrap_or_default();
        let device_opt = ctx.store.load_device().await.unwrap_or_default();
        let (Some(backend_url), Some((_, device_secret))) = (backend_url_opt, device_opt) else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        // Stale-access pre-emptive refresh: if we're within 2 minutes of
        // expiry, rotate first. The refresh updates auth_session in
        // place; a refresh failure transitions us out of Authenticated
        // (publishes RefreshFailed) and the loop bails on the next
        // iteration via the auth_session check below.
        let needs_refresh = {
            let guard = ctx.auth_session.read().await;
            guard
                .as_ref()
                .is_some_and(|s| s.access_expires_at <= Instant::now() + Duration::from_secs(120))
        };
        if needs_refresh && !refresh_inline(&ctx, &backend_url).await {
            // refresh_inline returns false on failure; it has already
            // published RefreshFailed and cleared the auth state. Exit
            // the loop entirely.
            return;
        }

        // Snapshot the access token now that we may have refreshed.
        let access = ctx
            .auth_session
            .read()
            .await
            .as_ref()
            .map(|s| s.access_token.clone());
        let Some(access) = access else {
            // Auth was cleared mid-loop. Bail.
            return;
        };

        let cf = ctx.store.load_cf_access().await.ok().flatten();
        let _ = ctx
            .state_tx
            .send(ConnectionState::Reconnecting { attempt: 1 });

        match connect_with_handles(
            &ctx,
            &backend_url,
            device_secret.as_str(),
            cf,
            Some(&access),
        )
        .await
        {
            Ok(()) => {
                // Subscribe BEFORE publishing `Connected` so the recv_loop
                // can't fire `Disconnected` between the send and the
                // subscribe and leave us hanging on the next `changed()`.
                // The borrow_and_update() right after subscribe handles the
                // case where Disconnected lands inside the very-narrow
                // window between subscribe and Connected publishing.
                let mut state_rx = ctx.state_tx.subscribe();
                let _ = ctx.state_tx.send(ConnectionState::Connected);
                ctx.reconnect.record_success().await;
                loop {
                    if matches!(
                        *state_rx.borrow_and_update(),
                        ConnectionState::Disconnected
                    ) {
                        break;
                    }
                    if state_rx.changed().await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(?e, "mobile: reconnect attempt failed");
                let _ = ctx.state_tx.send(ConnectionState::Disconnected);
                ctx.reconnect.record_failure().await;
            }
        }

        let delay = ctx.reconnect.next_delay().await;
        tokio::time::sleep(delay).await;
    }
}

/// Inline-refresh path used by [`reconnect_loop`]. Returns `true` on
/// success (or when there's no session to refresh), `false` on failure
/// (publishes RefreshFailed and clears auth state). Spec §6.3.
async fn refresh_inline(ctx: &ReconnectContext, backend_url: &str) -> bool {
    // Hoist the session check above `Refreshing` so a no-op refresh
    // (no session) doesn't publish a `Refreshing → ?` transition with
    // no follow-up frame.
    let Some(session) = ctx.auth_session.read().await.clone() else {
        return true; // Nothing to refresh.
    };
    // Build the HTTP client BEFORE publishing Refreshing so a build
    // failure (effectively permanent under the current backend_url) is
    // surfaced as a refresh failure rather than leaving the auth state
    // machine stuck at `Refreshing` with no follow-up. Build failures
    // mean the next iteration would also fail, so treating them as a
    // hard refresh failure (clear auth, return false) is strictly
    // better than looping with an expired token.
    let cf_access = ctx.store.load_cf_access().await.ok().flatten();
    let http = match crate::http::MobileHttpClient::new(backend_url, ctx.device_id, cf_access) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(?e, "mobile: refresh aborted; could not build HTTP client");
            let _ = ctx.auth_state_tx.send(AuthStateFrame::RefreshFailed {
                error: Arc::new(MinosError::AuthRefreshFailed {
                    message: format!("build http client: {e}"),
                }),
            });
            *ctx.auth_session.write().await = None;
            let _ = ctx.store.clear_auth().await;
            return false;
        }
    };
    let _ = ctx.auth_state_tx.send(AuthStateFrame::Refreshing);
    match http.refresh(&session.refresh_token).await {
        Ok(r) => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let exp_ms = now_ms + r.expires_in * 1000;
            let secs = u64::try_from(r.expires_in.max(0)).unwrap_or(0);
            {
                let mut guard = ctx.auth_session.write().await;
                if let Some(s) = guard.as_mut() {
                    s.access_token.clone_from(&r.access_token);
                    s.access_expires_at_ms = exp_ms;
                    s.access_expires_at = Instant::now() + Duration::from_secs(secs);
                    s.refresh_token.clone_from(&r.refresh_token);
                }
            }
            let _ = ctx
                .store
                .save_auth(
                    r.access_token,
                    exp_ms,
                    r.refresh_token,
                    session.account.account_id.clone(),
                    session.account.email.clone(),
                )
                .await;
            let _ = ctx.auth_state_tx.send(AuthStateFrame::Authenticated {
                account: session.account,
            });
            true
        }
        Err(e) => {
            let _ = ctx.auth_state_tx.send(AuthStateFrame::RefreshFailed {
                error: Arc::new(MinosError::AuthRefreshFailed {
                    message: e.to_string(),
                }),
            });
            *ctx.auth_session.write().await = None;
            let _ = ctx.store.clear_auth().await;
            false
        }
    }
}

/// Standalone connect helper that takes the same handle bundle as the
/// reconnect loop. Mirrors [`MobileClient::connect`] but doesn't borrow
/// `&self` so we can call it from a task that doesn't hold a reference
/// to the originating client.
async fn connect_with_handles(
    ctx: &ReconnectContext,
    url: &str,
    device_secret: &str,
    cf_access: Option<(String, String)>,
    access_token: Option<&str>,
) -> Result<(), MinosError> {
    let mut req = url
        .into_client_request()
        .map_err(|e| MinosError::ConnectFailed {
            url: url.to_string(),
            message: format!("invalid backend URL: {e}"),
        })?;
    let headers = req.headers_mut();
    headers.insert(
        "X-Device-Id",
        ctx.device_id
            .to_string()
            .parse()
            .map_err(|_| MinosError::ConnectFailed {
                url: url.to_string(),
                message: "device_id is not a valid header value".into(),
            })?,
    );
    headers.insert(
        "X-Device-Role",
        "ios-client".parse().expect("static header value is valid"),
    );
    headers.insert(
        "X-Device-Secret",
        device_secret
            .parse()
            .map_err(|_| MinosError::ConnectFailed {
                url: url.to_string(),
                message: "device_secret is not a valid header value".into(),
            })?,
    );
    if let Some(tok) = access_token {
        headers.insert(
            "Authorization",
            format!("Bearer {tok}")
                .parse()
                .map_err(|_| MinosError::ConnectFailed {
                    url: url.to_string(),
                    message: "access_token is not a valid header value".into(),
                })?,
        );
    }
    if let Some((id, sec)) = cf_access {
        headers.insert(
            "CF-Access-Client-Id",
            id.parse().map_err(|_| MinosError::ConnectFailed {
                url: url.to_string(),
                message: "cf_access_client_id is not a valid header value".into(),
            })?,
        );
        headers.insert(
            "CF-Access-Client-Secret",
            sec.parse().map_err(|_| MinosError::ConnectFailed {
                url: url.to_string(),
                message: "cf_access_client_secret is not a valid header value".into(),
            })?,
        );
    }

    let (ws, _resp) = connect_async(req)
        .await
        .map_err(|e| connect_error_to_minos(url, e))?;
    let (mut write, read) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Envelope>(256);
    let send_handle = tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            let text = match serde_json::to_string(&env) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(?e, "mobile: envelope serialise failed");
                    continue;
                }
            };
            if let Err(e) = write.send(Message::Text(text.into())).await {
                tracing::warn!(?e, "mobile: WS write failed; send loop exiting");
                break;
            }
        }
    });
    let recv_handle = tokio::spawn(recv_loop(
        read,
        ctx.ui_events_tx.clone(),
        ctx.state_tx.clone(),
        ctx.pending.clone(),
    ));
    *ctx.outbox.lock().await = Some(tx);
    let mut tasks = ctx.tasks.lock().await;
    // Abort any handles from a prior connect/reconnect before pushing the
    // new pair — see the matching comment in `MobileClient::connect`.
    for h in tasks.drain(..) {
        h.abort();
    }
    tasks.push(send_handle);
    tasks.push(recv_handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_with_in_memory_store_starts_disconnected() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        assert_eq!(client.current_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn new_with_persisted_state_reuses_device_identity() {
        let persisted = PersistedPairingState {
            backend_url: Some("ws://127.0.0.1/devices".into()),
            device_id: Some(DeviceId::new().to_string()),
            device_secret: Some(DeviceSecret::generate().as_str().to_string()),
            cf_access_client_id: Some("cf-id".into()),
            cf_access_client_secret: Some("cf-secret".into()),
            access_token: Some("access".into()),
            access_expires_at_ms: Some(123_456),
            refresh_token: Some("refresh".into()),
            account_id: Some("acct-1".into()),
            account_email: Some("a@b.com".into()),
        };

        let client = MobileClient::new_with_persisted_state("test".into(), persisted.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let exported = rt.block_on(client.persisted_pairing_state()).unwrap();
        assert_eq!(exported, persisted);
    }

    #[tokio::test]
    async fn pair_with_qr_json_rejects_invalid_json_as_store_corrupt() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        let err = client
            .pair_with_qr_json("not json".into())
            .await
            .expect_err("invalid JSON must not parse into PairingQrPayload");
        assert!(
            matches!(&err, MinosError::StoreCorrupt { path, .. } if path == "qr_payload"),
            "expected StoreCorrupt {{ path: \"qr_payload\", .. }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn pair_with_qr_json_rejects_wrong_version() {
        let client = MobileClient::new_with_in_memory_store("test".into());
        let qr = serde_json::json!({
            "v": 1,
            "backend_url": "wss://x/devices",
            "host_display_name": "Mac",
            "pairing_token": "tok",
            "expires_at_ms": 1_i64,
        });
        let err = client
            .pair_with_qr_json(qr.to_string())
            .await
            .expect_err("v=1 must be rejected");
        assert!(
            matches!(err, MinosError::PairingQrVersionUnsupported { version: 1 }),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn list_threads_without_persisted_state_errors_store_corrupt() {
        // After Phase C, list_threads is HTTP-backed: with no persisted
        // backend_url / device-secret it surfaces StoreCorrupt rather than
        // the legacy "no live outbound channel" Disconnected error.
        let client = MobileClient::new_with_in_memory_store("test".into());
        let err = client
            .list_threads(ListThreadsParams {
                limit: 10,
                before_ts_ms: None,
                agent: None,
            })
            .await
            .expect_err("HTTP query with no creds must error");
        assert!(
            matches!(err, MinosError::StoreCorrupt { ref path, .. } if path == "backend_url"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn cf_access_http_rejection_maps_to_cf_auth_failed_with_status_in_message() {
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .status(403)
            .body(None::<Vec<u8>>)
            .unwrap();
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WsError::Http(Box::new(response)),
        );

        match err {
            MinosError::CfAuthFailed { message } => {
                assert!(
                    message.contains("403"),
                    "CfAuthFailed message should embed the status code: {message}"
                );
            }
            other => panic!("expected CfAuthFailed, got {other:?}"),
        }
    }

    #[test]
    fn non_cf_http_status_maps_to_connect_failed_with_status_detail() {
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .status(502)
            .body(Some(b"upstream timed out".to_vec()))
            .unwrap();
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WsError::Http(Box::new(response)),
        );

        match err {
            MinosError::ConnectFailed { url, message } => {
                assert_eq!(url, "wss://example.com/devices");
                assert!(
                    message.contains("502"),
                    "expected status in message: {message}"
                );
                assert!(
                    message.contains("upstream timed out"),
                    "expected body snippet in message: {message}"
                );
            }
            other => panic!("expected ConnectFailed, got {other:?}"),
        }
    }

    #[test]
    fn io_error_maps_to_connect_failed_with_kind_in_message() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "nope");
        let err = connect_error_to_minos("wss://example.com/devices", WsError::Io(io));

        match err {
            MinosError::ConnectFailed { message, .. } => {
                assert!(
                    message.contains("ConnectionRefused"),
                    "expected io kind in message: {message}"
                );
            }
            other => panic!("expected ConnectFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_text_frame_routes_forwarded_result_to_pending_oneshot() {
        let pending: DashMap<u64, oneshot::Sender<RpcReply>> = DashMap::new();
        let (tx, rx) = oneshot::channel::<RpcReply>();
        pending.insert(42, tx);
        let (ui_tx, _ui_rx) = broadcast::channel(8);
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Disconnected);

        let from = DeviceId::new();
        let env = Envelope::Forwarded {
            version: 1,
            from,
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 42,
                "result": {"session_id": "thr_1", "cwd": "/workdir"}
            }),
        };
        let text = serde_json::to_string(&env).unwrap();
        handle_text_frame(&text, &ui_tx, &state_tx, &pending);

        let reply = rx.await.unwrap();
        match reply {
            RpcReply::Ok(value) => {
                assert_eq!(value["session_id"], "thr_1");
                assert_eq!(value["cwd"], "/workdir");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        assert!(pending.is_empty(), "pending entry must be removed");
    }

    #[tokio::test]
    async fn handle_text_frame_routes_forwarded_error_to_pending_oneshot() {
        let pending: DashMap<u64, oneshot::Sender<RpcReply>> = DashMap::new();
        let (tx, rx) = oneshot::channel::<RpcReply>();
        pending.insert(7, tx);
        let (ui_tx, _ui_rx) = broadcast::channel(8);
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Disconnected);

        let env = Envelope::Forwarded {
            version: 1,
            from: DeviceId::new(),
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "error": {"code": -32002, "message": "agent already running"}
            }),
        };
        let text = serde_json::to_string(&env).unwrap();
        handle_text_frame(&text, &ui_tx, &state_tx, &pending);

        let reply = rx.await.unwrap();
        match reply {
            RpcReply::Err { code, message } => {
                assert_eq!(code, -32002);
                assert_eq!(message, "agent already running");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_text_frame_drops_forwarded_with_unknown_id() {
        // Pending starts empty: a Forwarded with id=99 must be a no-op.
        let pending: DashMap<u64, oneshot::Sender<RpcReply>> = DashMap::new();
        let (ui_tx, _ui_rx) = broadcast::channel(8);
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Disconnected);

        let env = Envelope::Forwarded {
            version: 1,
            from: DeviceId::new(),
            payload: serde_json::json!({"jsonrpc": "2.0", "id": 99, "result": {}}),
        };
        let text = serde_json::to_string(&env).unwrap();
        handle_text_frame(&text, &ui_tx, &state_tx, &pending);
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn start_agent_returns_not_connected_when_disconnected() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client.start_agent(AgentName::Codex, "ping".into()).await;
        assert!(matches!(res, Err(MinosError::NotConnected)));
    }

    #[tokio::test]
    async fn send_user_message_returns_not_connected_when_disconnected() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client
            .send_user_message("thr_1".into(), "ping".into())
            .await;
        assert!(matches!(res, Err(MinosError::NotConnected)));
    }

    #[tokio::test]
    async fn stop_agent_returns_not_connected_when_disconnected() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client.stop_agent().await;
        assert!(matches!(res, Err(MinosError::NotConnected)));
    }

    #[tokio::test]
    async fn subscribe_auth_state_emits_unauthenticated_initially() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let rx = client.subscribe_auth_state();
        let snapshot = rx.borrow().clone();
        assert!(
            matches!(snapshot, AuthStateFrame::Unauthenticated),
            "expected Unauthenticated, got {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn notify_foregrounded_and_backgrounded_roundtrip_through_reconnect() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        client.notify_backgrounded();
        // Spawn-then-poll because notify_* are sync wrappers around
        // tokio::spawn; let the spawned task land before checking.
        for _ in 0..40 {
            if client.reconnect.is_paused().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(client.reconnect.is_paused().await, "background must pause");

        client.notify_foregrounded();
        for _ in 0..40 {
            if !client.reconnect.is_paused().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !client.reconnect.is_paused().await,
            "foreground must un-pause"
        );
    }

    #[tokio::test]
    async fn logout_when_not_logged_in_is_a_noop_returning_ok() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        // No active session, no live WS — logout should still complete
        // cleanly (best-effort under the hood).
        let res = client.logout().await;
        assert!(res.is_ok(), "logout from unauthenticated must be Ok");
    }

    #[tokio::test]
    async fn handle_text_frame_unpaired_event_drains_pending() {
        let pending: DashMap<u64, oneshot::Sender<RpcReply>> = DashMap::new();
        let (tx, rx) = oneshot::channel::<RpcReply>();
        pending.insert(1, tx);
        let (ui_tx, _ui_rx) = broadcast::channel(8);
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Connected);

        let env = Envelope::Event {
            version: 1,
            event: EventKind::Unpaired,
        };
        let text = serde_json::to_string(&env).unwrap();
        handle_text_frame(&text, &ui_tx, &state_tx, &pending);

        let reply = rx.await.unwrap();
        match reply {
            RpcReply::Err { code, .. } => assert_eq!(code, crate::rpc::REQUEST_DROPPED_CODE),
            other => panic!("expected RequestDropped err, got {other:?}"),
        }
        assert!(pending.is_empty());
    }

    /// Regression for the "TCP reset without WS Close" path: when the read
    /// stream returns `None` immediately (no Close frame, no error) the
    /// recv loop must still drain `pending` and publish `Disconnected`.
    /// Otherwise an in-flight `forward_rpc` caller would hang until the
    /// per-call timeout fires.
    #[tokio::test]
    async fn recv_loop_drains_pending_on_stream_end_without_close() {
        let pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>> = Arc::new(DashMap::new());
        let (rpc_tx, rpc_rx) = oneshot::channel::<RpcReply>();
        pending.insert(1, rpc_tx);

        let (ui_tx, _ui_rx) = broadcast::channel(8);
        let (state_tx, mut state_rx) = watch::channel(ConnectionState::Connected);

        // An empty stream models a transport that closed without an
        // explicit WS Close frame — `next()` returns None on the first poll.
        let read = futures_util::stream::iter(
            Vec::<Result<Message, tokio_tungstenite::tungstenite::Error>>::new(),
        );
        recv_loop(read, ui_tx, state_tx, pending.clone()).await;

        // Pending must be drained.
        assert!(pending.is_empty(), "pending must be drained on stream end");
        let reply = rpc_rx.await.expect("oneshot must have been resolved");
        match reply {
            RpcReply::Err { code, .. } => assert_eq!(code, crate::rpc::REQUEST_DROPPED_CODE),
            other => panic!("expected RequestDropped err, got {other:?}"),
        }
        // And the connection-state watch transitioned to Disconnected.
        assert!(matches!(
            *state_rx.borrow_and_update(),
            ConnectionState::Disconnected
        ));
    }
}
