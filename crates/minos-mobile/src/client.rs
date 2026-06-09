//! Topic-based realtime mobile client.
//!
//! Uses [`RealtimeSession`] for the WebSocket connection (ClientFrame/
//! ServerFrame protocol) and REST HTTP for all outbound operations
//! (agent sessions, social, projects, etc.) via [`crate::http`].
//!
//! Responsibilities:
//!
//! - Parse a scanned QR v2 payload (`PairingQrPayload` from
//!   `minos_protocol::messages`) and persist its fields into the
//!   [`MobilePairingStore`].
//! - Maintain a single WebSocket via [`RealtimeSession`]; expose
//!   `ConnectionState` via a `watch::Receiver` and live `UiEventFrame`
//!   over a `broadcast::Sender`.
//!
//! For FFI use, [`MobileClient::new_with_in_memory_store`] avoids exposing
//! the `Arc<dyn MobilePairingStore>` trait object across the frb boundary
//! (real Keychain persistence lives on the Dart side; see plan D5).

use std::sync::Arc;
use std::time::{Duration, Instant};

use http::{Method, Request};
use minos_domain::{ConnectionState, DeviceId, MinosError};
use minos_protocol::{
    AddAgentToGroupRequest, AddGroupMemberRequest, AgentSummary, ApprovalDecisionRequest,
    AuthSummary, ChatMessageSummary, ConversationAgentMembersResponse, ConversationMembersResponse,
    ConversationReadResponse, ConversationResponse, ConversationsResponse,
    CreateFriendRequestRequest, CreateGroupConversationRequest, EnsureDirectConversationRequest,
    FriendRequestSummary, FriendRequestsResponse, FriendsResponse, GetThreadLastSeqParams,
    GetThreadLastSeqResponse, HostSummary, ListAgentsResponse, ListChatMessagesResponse,
    ListClisResponse, ListHostSkillsResponse, ListThreadsParams, ListThreadsResponse,
    MyProfileResponse, PairingQrPayload, ReadThreadParams, ReadThreadResponse, RefreshResponse,
    RegisterAgentRequest, RemoveAgentFromGroupRequest, SendChatMessageRequest, SetMinosIdRequest,
    UserSummary, WriteHostSkillConfigResponse,
};
use minos_ui_protocol::UiEventMessage;
use openwire::websocket::WebSocket;
use openwire::{Client as OpenwireClient, RequestBody, WireError, WireErrorKind};
use openwire_core::websocket::{HandshakeFailure, WebSocketEngineError, WebSocketError};
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use uuid::Uuid;

use crate::auth::{AuthSession, AuthStateFrame};
use crate::openwire_trace::OpenwireTraceFactory;
use crate::realtime::{RealtimeSession, SubscriptionManager};
use crate::store::{InMemoryPairingStore, MobilePairingStore, PersistedPairingState};
use crate::ReconnectController;

const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WS_PING_INTERVAL: Duration = Duration::from_secs(30);
const WS_PONG_TIMEOUT: Duration = Duration::from_secs(45);

macro_rules! auth_http_call {
    ($self:expr, |$http:ident, $access:ident| $call:expr) => {{
        let $access = $self.access_token_or_unauthorized().await?;
        let $http = $self.http_client_no_secret()?;
        let result = $call.await;
        $self.finish_authenticated_http_call(result).await
    }};
}

/// One live UI event pushed from backend fan-out. Mobile layers consume
/// these via [`MobileClient::ui_events_stream`] (broadcast receiver).
#[derive(Debug, Clone)]
pub struct UiEventFrame {
    pub thread_id: String,
    pub seq: u64,
    pub ui: UiEventMessage,
    pub ts_ms: i64,
}

/// One live social-chat message pushed from backend fan-out.
#[derive(Debug, Clone)]
pub struct SocialEventFrame {
    pub conversation_id: String,
    pub message: ChatMessageSummary,
}

/// Topic-based realtime mobile client. One instance per iPhone process.
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
    social_events_tx: broadcast::Sender<SocialEventFrame>,
    device_id: DeviceId,
    self_name: String,
    /// Live RealtimeSession task handle. Aborted in `connect` before a
    /// fresh one is spawned.
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Tracks subscribed topics and durable seq cursors across reconnects.
    subscription_mgr: Arc<SubscriptionManager>,
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
        let (social_events_tx, _) = broadcast::channel(256);
        let (auth_state_tx, auth_state_rx) = watch::channel(AuthStateFrame::Unauthenticated);
        Self {
            store,
            state_tx,
            state_rx,
            ui_events_tx,
            social_events_tx,
            device_id,
            self_name,
            tasks: Arc::new(Mutex::new(Vec::new())),
            subscription_mgr: SubscriptionManager::new(),
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
    /// malformed device credentials fall back to a fresh device id; any
    /// persisted auth is still restored into the in-memory pairing store.
    /// Restored auth tokens also seed the live `auth_session` and emit
    /// `AuthStateFrame::Authenticated` so a cold-start resume sees the same
    /// state as a fresh login.
    ///
    /// Device id resolution priority: prefer the persisted id (so the JWT
    /// `did` claim still matches after relaunch), fall back to a freshly
    /// minted one only if no persisted id is present.
    #[must_use]
    pub fn new_with_persisted_state(self_name: String, state: PersistedPairingState) -> Self {
        let device = restored_device_id_only(&state);
        let device_id = device.unwrap_or_default();
        let auth_persisted = restored_auth(&state);
        let store = Arc::new(InMemoryPairingStore::from_parts(
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
        let (social_events_tx, _) = broadcast::channel(256);
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
            social_events_tx,
            device_id,
            self_name,
            tasks: Arc::new(Mutex::new(Vec::new())),
            subscription_mgr: SubscriptionManager::new(),
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

    /// Subscribe to live social-chat events from backend fan-out.
    #[must_use]
    pub fn social_events_stream(&self) -> broadcast::Receiver<SocialEventFrame> {
        self.social_events_tx.subscribe()
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
        let _ = self.store.load_device().await?;
        let auth = self.store.load_auth().await?;

        Ok(PersistedPairingState {
            // Bearer tokens are bound to this id, so a register/login ->
            // cold-launch -> pair flow must not silently rotate it.
            device_id: Some(self.device_id.to_string()),
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
        self.resume_persisted_session_inner(crate::build_config::BACKEND_URL)
            .await
    }

    /// Test/dev override: resume against an explicit `backend_url`. Used by
    /// integration tests; production callers go through
    /// [`Self::resume_persisted_session`].
    #[doc(hidden)]
    pub async fn resume_persisted_session_at(&self, backend_url: &str) -> Result<(), MinosError> {
        self.resume_persisted_session_inner(backend_url).await
    }

    async fn resume_persisted_session_inner(&self, backend_url: &str) -> Result<(), MinosError> {
        if matches!(self.current_state(), ConnectionState::Connected) {
            return Ok(());
        }

        let Some(device_id) = self.store.load_device().await? else {
            return Err(MinosError::StoreCorrupt {
                path: "persisted_pairing_state.device".into(),
                message: "missing device_id for resume".into(),
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

        let _ = self
            .state_tx
            .send(ConnectionState::Reconnecting { attempt: 1 });

        // Cold-start resume can rehydrate an already-expired access token from
        // keychain. Mirror the reconnect loop's preflight refresh check so we
        // do not burn the first WS upgrade on a known-stale bearer.
        if self
            .auth_session
            .read()
            .await
            .as_ref()
            .is_some_and(access_token_needs_refresh)
        {
            self.refresh_session().await?;
        }

        let access = self
            .auth_session
            .read()
            .await
            .as_ref()
            .map(|s| s.access_token.clone());

        let result = self.connect(backend_url, access.as_deref()).await;

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

    /// Test/dev override: drive the same flow as `pair_with_qr_json` but
    /// against an explicit `backend_url` instead of the compile-time
    /// `build_config::BACKEND_URL`. Used by integration tests and the
    /// `fake-peer` dev binary; production code paths must use
    /// [`Self::pair_with_qr_json`] so the URL stays compile-time pinned.
    #[doc(hidden)]
    pub async fn pair_with_qr_json_at(
        &self,
        qr_json: String,
        backend_url: &str,
    ) -> Result<(), MinosError> {
        self.pair_with_qr_json_inner(qr_json, backend_url).await
    }

    /// Scan a QR v2 payload (raw JSON). Calls `POST /v1/pairing/confirm`
    /// over HTTP at the compile-time `BACKEND_URL`, records the paired
    /// Mac as the active forward target, opens the authenticated
    /// WebSocket, and transitions [`ConnectionState`] through
    /// `Pairing → Connected`. Bearer-only post ADR-0020.
    ///
    /// The QR carries only `host_display_name`, `pairing_token`, and the
    /// expiry; backend routing stays in the mobile crate's
    /// [`crate::build_config`]. Older QR builds may still carry extra
    /// fields in the JSON — they are silently ignored by `serde` and never
    /// enter durable storage.
    ///
    /// Errors:
    /// - `StoreCorrupt { path: "qr_payload", .. }` when the JSON doesn't
    ///   parse.
    /// - `PairingQrVersionUnsupported` when `qr.v != 2`.
    /// - `ConnectFailed` / `Disconnected` on WS or RPC round-trip failures.
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<(), MinosError> {
        self.pair_with_qr_json_inner(qr_json, crate::build_config::BACKEND_URL)
            .await
    }

    /// Shared implementation for `pair_with_qr_json` (production) and
    /// `pair_with_qr_json_at` (test/dev).
    async fn pair_with_qr_json_inner(
        &self,
        qr_json: String,
        backend_url: &str,
    ) -> Result<(), MinosError> {
        let qr: PairingQrPayload =
            serde_json::from_str(&qr_json).map_err(|e| MinosError::StoreCorrupt {
                path: "qr_payload".into(),
                message: e.to_string(),
            })?;
        if qr.v != 2 {
            return Err(MinosError::PairingQrVersionUnsupported { version: qr.v });
        }
        let _ = self.state_tx.send(ConnectionState::Pairing);

        // Formal account pairing is bearer-gated for mobile clients.
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

        // Step 1: confirm the formal pairing code over HTTP. The backend
        // records the account-host link and returns the host installation id.
        let http = crate::http::MobileHttpClient::new(
            backend_url,
            self.device_id,
            self.self_name.clone(),
        )?;
        let pair_resp = match http.pair_confirm(&qr.pairing_token, &access).await {
            Ok(resp) => resp,
            Err(error @ MinosError::Unauthorized { .. }) => {
                self.clear_auth_session_and_disconnect().await;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let host_id = Uuid::parse_str(&pair_resp.host_installation_id)
            .map(minos_domain::DeviceId)
            .map_err(|error| MinosError::BackendInternal {
                message: format!("pair_confirm returned invalid host_installation_id: {error}"),
            })?;

        // Persist the device id (rebound across runs) and remember the
        // newly-paired Mac as the active forward target. Bearer-only post
        // ADR-0020 — no DeviceSecret is minted to the iOS rail.
        self.store.save_device(&self.device_id).await?;
        self.store.save_active_host(&host_id).await?;

        // Step 2: open the WS bearer-authenticated. The reconnect loop
        // re-uses the same auth_session.
        self.connect(backend_url, Some(&access)).await?;

        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(())
    }

    /// Tear down a specific paired Mac. The path-bound `host` is the Mac
    /// to forget. Idempotent. Clears the active-mac slot only when it
    /// matches the deleted Mac so a concurrent `set_active_host` to a
    /// different Mac is preserved.
    pub async fn forget_host(&self, host: DeviceId) -> Result<(), MinosError> {
        let backend_url = crate::build_config::BACKEND_URL;
        let access = self.access_token_or_unauthorized().await?;

        // Best-effort delete on the backend. Failure here must not block
        // local cleanup — the user re-pairs to recover.
        if let Ok(http) =
            crate::http::MobileHttpClient::new(backend_url, self.device_id, self.self_name.clone())
        {
            let _ = http.delete_pair(&access, host).await;
        }

        self.store.clear_active_if(&host).await?;

        // If we forgot the *active* mac and the WS is currently up, the
        // backend will emit Unpaired and the recv loop will tear the WS
        // down via the existing path. Don't preemptively shut it down
        // here — multiple Macs may still be paired.
        Ok(())
    }

    // ─────────────────────────── history rpcs ────────────────────────────

    /// Request a page of thread summaries from the backend. Bearer-only
    /// post ADR-0020.
    pub async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.list_threads(&access, req))
    }

    /// Read a window of translated UI events from one thread.
    pub async fn read_thread(
        &self,
        req: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        auth_http_call!(self, |http, access| http.read_thread(&access, req))
    }

    /// Host-only helper (mobile rarely uses this; included for parity).
    pub async fn get_thread_last_seq(
        &self,
        req: GetThreadLastSeqParams,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.get_thread_last_seq(&access, &req.thread_id)
        })
    }

    /// List every Mac paired to the caller's account.
    pub async fn list_paired_hosts(&self) -> Result<Vec<HostSummary>, MinosError> {
        let resp = auth_http_call!(self, |http, access| http.list_paired_hosts(&access))?;
        Ok(resp.hosts)
    }

    pub async fn my_profile(&self) -> Result<MyProfileResponse, MinosError> {
        auth_http_call!(self, |http, access| http.my_profile(&access))
    }

    pub async fn set_minos_id(&self, minos_id: String) -> Result<MyProfileResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.set_minos_id(&access, SetMinosIdRequest { minos_id })
        })
    }

    pub async fn search_users(&self, minos_id: String) -> Result<Vec<UserSummary>, MinosError> {
        let resp = auth_http_call!(self, |http, access| {
            http.search_users(&access, &minos_id)
        })?;
        Ok(resp.users)
    }

    pub async fn friends(&self) -> Result<FriendsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.friends(&access))
    }

    pub async fn register_agent(
        &self,
        name: String,
        description: String,
        runtime_agent: String,
        model: String,
    ) -> Result<AgentSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.register_agent(
                &access,
                RegisterAgentRequest {
                    name,
                    description,
                    runtime_agent,
                    model,
                },
            )
        })
    }

    pub async fn list_agents(&self) -> Result<ListAgentsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.list_agents(&access))
    }

    pub async fn create_friend_request(
        &self,
        target_minos_id: String,
    ) -> Result<FriendRequestSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.create_friend_request(&access, CreateFriendRequestRequest { target_minos_id })
        })
    }

    pub async fn friend_requests(&self) -> Result<FriendRequestsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.friend_requests(&access))
    }

    pub async fn accept_friend_request(
        &self,
        request_id: String,
    ) -> Result<FriendRequestSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.accept_friend_request(&access, &request_id)
        })
    }

    pub async fn reject_friend_request(
        &self,
        request_id: String,
    ) -> Result<FriendRequestSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.reject_friend_request(&access, &request_id)
        })
    }

    pub async fn conversations(&self) -> Result<ConversationsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.conversations(&access))
    }

    pub async fn ensure_direct_conversation(
        &self,
        friend_account_id: String,
    ) -> Result<ConversationResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.ensure_direct_conversation(
                &access,
                EnsureDirectConversationRequest { friend_account_id },
            )
        })
    }

    pub async fn create_group_conversation(
        &self,
        title: String,
        member_account_ids: Vec<String>,
    ) -> Result<ConversationResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.create_group_conversation(
                &access,
                CreateGroupConversationRequest {
                    title,
                    member_account_ids,
                },
            )
        })
    }

    pub async fn add_group_member(
        &self,
        conversation_id: String,
        member_account_id: String,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.add_group_member(
                &access,
                &conversation_id,
                AddGroupMemberRequest { member_account_id },
            )
        })
    }

    pub async fn conversation_members(
        &self,
        conversation_id: String,
    ) -> Result<ConversationMembersResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.conversation_members(&access, &conversation_id)
        })
    }

    pub async fn list_conversation_agents(
        &self,
        conversation_id: String,
    ) -> Result<ConversationAgentMembersResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.list_conversation_agents(&access, &conversation_id)
        })
    }

    pub async fn add_agent_to_conversation(
        &self,
        conversation_id: String,
        agent_id: String,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.add_agent_to_group(
                &access,
                &conversation_id,
                AddAgentToGroupRequest { agent_id },
            )
        })
    }

    pub async fn remove_agent_from_conversation(
        &self,
        conversation_id: String,
        agent_id: String,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.remove_agent_from_group(
                &access,
                &conversation_id,
                RemoveAgentFromGroupRequest { agent_id },
            )
        })
    }

    pub async fn mark_conversation_read(
        &self,
        conversation_id: String,
    ) -> Result<ConversationReadResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.mark_conversation_read(&access, &conversation_id)
        })
    }

    pub async fn list_chat_messages(
        &self,
        conversation_id: String,
        before_ts_ms: Option<i64>,
        limit: u32,
    ) -> Result<ListChatMessagesResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.list_chat_messages(&access, &conversation_id, before_ts_ms, limit)
        })
    }

    pub async fn send_chat_message(
        &self,
        conversation_id: String,
        text: String,
        reply_to_message_id: Option<String>,
    ) -> Result<minos_protocol::ChatMessageSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.send_chat_message(
                &access,
                &conversation_id,
                SendChatMessageRequest {
                    text,
                    reply_to_message_id,
                },
            )
        })
    }

    pub async fn recall_chat_message(
        &self,
        conversation_id: String,
        message_id: String,
    ) -> Result<minos_protocol::ChatMessageSummary, MinosError> {
        auth_http_call!(self, |http, access| {
            http.recall_chat_message(&access, &conversation_id, &message_id)
        })
    }

    /// Override the active forward target. Persisted so cold-launch
    /// restores the same target.
    pub async fn set_active_host(&self, host: DeviceId) -> Result<(), MinosError> {
        self.store.save_active_host(&host).await
    }

    /// Read the current active Mac id, or `None` if none has been paired
    /// yet (or every paired Mac has been forgotten).
    pub async fn active_host(&self) -> Result<Option<DeviceId>, MinosError> {
        self.store.load_active_host().await
    }

    /// Resolve the active Mac id for outbound forward-RPCs, or surface
    /// `NotConnected` (no specific "no active mac" variant — the caller
    /// experience is identical: the WS exists but has no routable target).
    async fn require_active_host(&self) -> Result<DeviceId, MinosError> {
        self.store
            .load_active_host()
            .await?
            .ok_or(MinosError::NotConnected)
    }

    /// Pluck the live access token out of `auth_session`, or surface
    /// `Unauthorized` if no session is in place. Used by every
    /// account-aware HTTP call. The reconnect loop keeps the token fresh
    /// even while the websocket stays connected; callers only need the
    /// latest cached bearer here.
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

    async fn finish_authenticated_http_call<T>(
        &self,
        result: Result<T, MinosError>,
    ) -> Result<T, MinosError> {
        match result {
            Err(error @ MinosError::Unauthorized { .. }) => {
                self.clear_auth_session_and_disconnect().await;
                Err(error)
            }
            other => other,
        }
    }

    // ─────────────────────────── agent dispatch ────────────────────────────

    /// Detect the CLI agents available on the paired runtime.
    pub async fn list_clis(&self) -> Result<ListClisResponse, MinosError> {
        let host_installation_id = self.require_active_host().await?.to_string();
        auth_http_call!(self, |http, access| {
            http.list_clis_http(
                &access,
                minos_protocol::ListHostClisRequest {
                    host_installation_id: host_installation_id.clone(),
                },
            )
        })
    }

    /// Scan the host-side skills exposed by the selected runtime.
    pub async fn list_host_skills(
        &self,
        host_device_id: Option<String>,
        force_reload: bool,
    ) -> Result<ListHostSkillsResponse, MinosError> {
        let host_installation_id = if let Some(host_device_id) = host_device_id {
            if Uuid::parse_str(&host_device_id).is_err() {
                return Err(MinosError::RpcCallFailed {
                    method: "minos_list_host_skills".into(),
                    message: format!("invalid host_device_id: {host_device_id}"),
                });
            }
            host_device_id
        } else {
            self.require_active_host().await?.to_string()
        };
        auth_http_call!(self, |http, access| {
            http.list_host_skills_http(
                &access,
                minos_protocol::ListHostSkillsCommandRequest {
                    host_installation_id: host_installation_id.clone(),
                    workspace: String::new(),
                    force_reload,
                },
            )
        })
    }

    /// Enable or disable one host-side skill by path.
    pub async fn write_host_skill_config(
        &self,
        host_device_id: Option<String>,
        path: String,
        enabled: bool,
    ) -> Result<WriteHostSkillConfigResponse, MinosError> {
        let host_installation_id = if let Some(host_device_id) = host_device_id {
            if Uuid::parse_str(&host_device_id).is_err() {
                return Err(MinosError::RpcCallFailed {
                    method: "minos_write_host_skill_config".into(),
                    message: format!("invalid host_device_id: {host_device_id}"),
                });
            }
            host_device_id
        } else {
            self.require_active_host().await?.to_string()
        };
        auth_http_call!(self, |http, access| {
            http.write_host_skill_config_http(
                &access,
                minos_protocol::WriteHostSkillConfigCommandRequest {
                    host_installation_id: host_installation_id.clone(),
                    workspace: String::new(),
                    path: path.clone(),
                    enabled,
                },
            )
        })
    }

    /// Send a user message into an existing agent session via REST.
    pub async fn send_user_message(
        &self,
        session_id: String,
        text: String,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.send_agent_input(&access, &session_id, &text)
        })?;
        Ok(())
    }

    /// Submit a user approval decision back to the backend relay.
    pub async fn send_approval_decision(
        &self,
        request_id: String,
        thread_id: String,
        decision: serde_json::Value,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.submit_approval_decision(
                &access,
                ApprovalDecisionRequest {
                    request_id,
                    thread_id,
                    decision,
                },
            )
        })
    }

    /// Pause an in-flight turn on the named thread via REST.
    pub async fn interrupt_thread(&self, thread_id: String) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.stop_agent_session(&access, &thread_id)
        })
    }

    /// Permanently close the named thread via REST. Idempotent.
    pub async fn close_thread(&self, thread_id: String) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| {
            http.stop_agent_session(&access, &thread_id)
        })
    }

    // ─────────────────────────── project rpcs ──────────────────────────────

    /// Create a project in the account-scoped backend store.
    pub async fn create_project(
        &self,
        req: minos_protocol::CreateProjectRequest,
    ) -> Result<minos_protocol::CreateProjectResponse, MinosError> {
        auth_http_call!(self, |http, access| http.create_project(&access, req))
    }

    /// List account-scoped projects from the backend.
    pub async fn list_projects(&self) -> Result<minos_protocol::ListProjectsResponse, MinosError> {
        auth_http_call!(self, |http, access| http.list_projects(&access))
    }

    /// Update a project's name in the backend.
    pub async fn update_project(
        &self,
        req: minos_protocol::UpdateProjectRequest,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| http.update_project(&access, req))
    }

    /// Delete a project from the backend.
    pub async fn delete_project(
        &self,
        req: minos_protocol::DeleteProjectRequest,
    ) -> Result<(), MinosError> {
        auth_http_call!(self, |http, access| http.delete_project(&access, req))
    }

    /// List backend-known threads within a project.
    pub async fn list_project_threads(
        &self,
        req: minos_protocol::ListProjectThreadsParams,
    ) -> Result<minos_protocol::ListProjectThreadsResponse, MinosError> {
        auth_http_call!(self, |http, access| {
            http.list_project_threads(&access, req)
        })
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
        let http = self.http_client_no_secret()?;
        let resp = http.register(&email, &password).await?;
        // Persist the device id pre-pair so cold-launch resumes against
        // the same JWT-bound id.
        self.store.save_device(&self.device_id).await?;
        let summary = self.adopt_auth_response(resp).await;
        self.ensure_reconnect_loop().await;
        Ok(summary)
    }

    /// Log into an existing account on the backend. Same shape as
    /// `register` modulo the create-vs-find behaviour on the server. Spec
    /// §5.4.
    pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
        let http = self.http_client_no_secret()?;
        let resp = http.login(&email, &password).await?;
        self.store.save_device(&self.device_id).await?;
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
        let http = self.http_client_no_secret()?;
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

    /// Log out of the current session. Wipes local auth state and drops the
    /// WS; the daemon's per-thread close happens via `close_thread` (Phase C
    /// rewrite — the legacy `stop_agent` RPC is gone). Spec §5.4 / §8.3.
    pub async fn logout(&self) -> Result<(), MinosError> {
        // Pre-Phase-C this called `stop_agent` to halt the active session.
        // Post-Phase-C the daemon owns multiple threads; logout no longer
        // closes them implicitly. The Mac side reaps idle threads via the
        // manager's reaper (C19) once the iOS client disconnects.

        let session = self.auth_session.read().await.clone();
        if let Some(s) = session {
            // Best-effort logout. If the network is down or the bearer is
            // already invalid we still wipe local state.
            if let Ok(http) = self.http_client_no_secret() {
                let _ = http.logout(&s.access_token, &s.refresh_token).await;
            }
        }
        self.clear_auth_session_and_disconnect().await;
        Ok(())
    }

    /// Build an HTTP client without requiring a paired device-secret.
    /// Used by the auth surface — `register` / `login` happen before the
    /// device is paired. The backend URL comes from compile-time
    /// `build_config`. Sync because the helper no longer awaits any store
    /// load.
    fn http_client_no_secret(&self) -> Result<crate::http::MobileHttpClient, MinosError> {
        crate::http::MobileHttpClient::new(
            crate::build_config::BACKEND_URL,
            self.device_id,
            self.self_name.clone(),
        )
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
            social_events_tx: self.social_events_tx.clone(),
            subscription_mgr: self.subscription_mgr.clone(),
            tasks: self.tasks.clone(),
            device_id: self.device_id,
            self_name: self.self_name.clone(),
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
    /// background. Starts a short grace window before pausing reconnects
    /// so brief app switches do not force an immediate reconnect on
    /// return. Spec §6.3 / §8.3. Same runtime-handling shape as
    /// `notify_foregrounded`.
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

    // ─────────────────────────── internals ────────────────────────────

    fn build_websocket_client() -> Result<OpenwireClient, MinosError> {
        let tls_connector =
            crate::tls::build_mobile_tls_connector().map_err(|e| MinosError::BackendInternal {
                message: format!("build mobile websocket TLS connector: {e}"),
            })?;

        OpenwireClient::builder()
            .tls_connector(tls_connector)
            .event_listener_factory(OpenwireTraceFactory::new("mobile_ws"))
            .build()
            .map_err(|e| MinosError::BackendInternal {
                message: format!("build websocket client: {e}"),
            })
    }

    fn build_websocket_url(base_url: &str, ticket: &str) -> String {
        let ws_url = websocket_base(base_url);
        format!("{ws_url}/ws/client?ticket={ticket}")
    }

    async fn open_backend_websocket(url: &str) -> Result<WebSocket, WebSocketError> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(url)
            .body(RequestBody::empty())
            .map_err(|error| WebSocketError::Io(WireError::invalid_request(error.to_string())))?;
        let client = Self::build_websocket_client().map_err(|error| {
            WebSocketError::Io(WireError::new(WireErrorKind::Internal, error.to_string()))
        })?;

        client
            .new_websocket(request)
            .handshake_timeout(WS_HANDSHAKE_TIMEOUT)
            .ping_interval(WS_PING_INTERVAL)
            .pong_timeout(WS_PONG_TIMEOUT)
            .execute()
            .await
    }

    async fn connect(&self, url: &str, access_token: Option<&str>) -> Result<(), MinosError> {
        let bearer_present = access_token.is_some();
        tracing::info!(
            target: "minos_mobile::client",
            url,
            device_id = %self.device_id,
            bearer_present,
            "mobile: opening backend WebSocket"
        );
        let ws_url = self
            .fetch_ticket_and_build_ws_url(url, access_token)
            .await?;
        let websocket = Self::open_backend_websocket(&ws_url)
            .await
            .map_err(|error| {
                connect_error_to_minos(&ws_url, error, &self.device_id, bearer_present)
            })?;

        let account_id = self
            .auth_session
            .read()
            .await
            .as_ref()
            .map(|s| s.account.account_id.clone())
            .unwrap_or_default();

        let (_frame_tx, frame_rx) = tokio::sync::mpsc::channel(1);
        let session_handle = tokio::spawn(RealtimeSession::run(
            websocket,
            account_id,
            self.subscription_mgr.clone(),
            self.ui_events_tx.clone(),
            self.social_events_tx.clone(),
            self.state_tx.clone(),
            frame_rx,
        ));

        let mut tasks = self.tasks.lock().await;
        for h in tasks.drain(..) {
            h.abort();
        }
        tasks.push(session_handle);
        Ok(())
    }

    async fn fetch_ticket_and_build_ws_url(
        &self,
        base_url: &str,
        access_token: Option<&str>,
    ) -> Result<String, MinosError> {
        let access = access_token.ok_or(MinosError::NotConnected)?;
        let http =
            crate::http::MobileHttpClient::new(base_url, self.device_id, self.self_name.clone())?;
        let ticket_resp = http
            .fetch_ws_ticket(access, &self.device_id.to_string())
            .await?;
        Ok(websocket_url_from_ticket_response(
            base_url,
            &ticket_resp.ticket,
            ticket_resp.gateway_url,
        ))
    }

    async fn shutdown_outbound(&self) {
        let mut tasks = self.tasks.lock().await;
        for h in tasks.drain(..) {
            h.abort();
        }
    }
}

fn websocket_base(base_url: &str) -> String {
    let Ok(url) = url::Url::parse(base_url) else {
        return base_url.trim_end_matches('/').to_string();
    };
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => other,
    };
    let Some(host) = url.host_str() else {
        return base_url.trim_end_matches('/').to_string();
    };
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{scheme}://{host}{port}")
}

/// Map an OpenWire websocket failure into a typed `MinosError`, picking
/// the variant the localized UI hint should reflect and stuffing the raw
/// classification into the `message` field so the iOS log panel surfaces
/// the actual cause instead of just `e.to_string()`.
fn connect_error_to_minos(
    url: &str,
    err: WebSocketError,
    device_id: &DeviceId,
    bearer_present: bool,
) -> MinosError {
    let detail = describe_ws_error(&err);
    // `error_variant` is the OpenWire websocket failure class that fired
    // (e.g. `handshake` when the upgrade got a non-101 HTTP response,
    // `connect`/`tls` for transport failures). NOT the request scheme —
    // the URL is always `ws(s)://` here, regardless of variant.
    tracing::warn!(
        target: "minos_mobile::client",
        url,
        device_id = %device_id,
        bearer_present,
        error_variant = detail.kind,
        http_status = ?detail.http_status,
        message = %detail.message,
        "mobile: WebSocket connect failed"
    );

    if matches!(detail.http_status, Some(401)) && bearer_present {
        return MinosError::Unauthorized {
            reason: detail.message,
        };
    }

    if matches!(detail.http_status, Some(302 | 401 | 403)) {
        return MinosError::Unauthorized {
            reason: detail.message,
        };
    }

    MinosError::ConnectFailed {
        url: url.to_string(),
        message: detail.message,
    }
}

/// Structured view of an OpenWire websocket error, kept private so the mapping
/// in `connect_error_to_minos` doesn't have to keep a parallel `match`.
struct WsErrorDetail {
    kind: &'static str,
    message: String,
    /// Set when the handshake or underlying wire error surfaced an HTTP
    /// status.
    http_status: Option<u16>,
}

fn describe_ws_error(err: &WebSocketError) -> WsErrorDetail {
    match err {
        WebSocketError::Handshake { status, reason } => {
            let reason_detail = match reason {
                HandshakeFailure::SubprotocolMismatch { offered, returned } => {
                    format!("subprotocol mismatch returned={returned} offered={offered:?}")
                }
                HandshakeFailure::UnsupportedExtension(extension) => {
                    format!("unsupported extension: {extension}")
                }
                HandshakeFailure::Other(message) => message.clone(),
                other => format!("{other:?}"),
            };
            let status_detail = status.map(|value| format!(" {value}")).unwrap_or_default();
            WsErrorDetail {
                kind: "handshake",
                message: format!("handshake{status_detail}: {reason_detail}"),
                http_status: status.map(|value| value.as_u16()),
            }
        }
        WebSocketError::Engine(engine_err) => describe_engine_error(engine_err),
        WebSocketError::Io(wire_err) => describe_wire_error(wire_err),
        WebSocketError::ClosedByPeer { code, reason } => WsErrorDetail {
            kind: "closed_by_peer",
            message: format!("closed by peer: {code} {reason}"),
            http_status: None,
        },
        WebSocketError::Timeout(kind) => WsErrorDetail {
            kind: "timeout",
            message: format!("websocket timeout: {kind:?}"),
            http_status: None,
        },
        WebSocketError::LocalCancelled => WsErrorDetail {
            kind: "canceled",
            message: "local cancellation".into(),
            http_status: None,
        },
    }
}

fn describe_engine_error(err: &WebSocketEngineError) -> WsErrorDetail {
    match err {
        WebSocketEngineError::Io(wire_err) => describe_wire_error(wire_err),
        other => WsErrorDetail {
            kind: "engine",
            message: format!("engine: {other}"),
            http_status: None,
        },
    }
}

fn describe_wire_error(err: &WireError) -> WsErrorDetail {
    WsErrorDetail {
        kind: describe_wire_error_kind(err.kind()),
        message: err.to_string(),
        http_status: err.response_status().map(|value| value.as_u16()),
    }
}

fn describe_wire_error_kind(kind: WireErrorKind) -> &'static str {
    match kind {
        WireErrorKind::InvalidRequest => "invalid_request",
        WireErrorKind::Timeout => "timeout",
        WireErrorKind::Canceled => "canceled",
        WireErrorKind::Dns => "dns",
        WireErrorKind::Connect => "connect",
        WireErrorKind::Tls => "tls",
        WireErrorKind::Protocol => "protocol",
        WireErrorKind::Redirect => "redirect",
        WireErrorKind::Body => "body",
        WireErrorKind::Interceptor => "interceptor",
        WireErrorKind::Internal => "internal",
    }
}

fn access_token_needs_refresh(session: &AuthSession) -> bool {
    refresh_check_delay(session).is_zero()
}

fn refresh_check_delay(session: &AuthSession) -> Duration {
    session
        .access_expires_at
        .saturating_duration_since(Instant::now() + Duration::from_secs(120))
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

/// Restore the persisted device id. The JWT's `did` claim binds the
/// bearer to this id, so we MUST keep using the same value across the
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
    social_events_tx: broadcast::Sender<SocialEventFrame>,
    subscription_mgr: Arc<SubscriptionManager>,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    device_id: DeviceId,
    self_name: String,
}

/// Reconnect loop owned by [`MobileClient::ensure_reconnect_loop`].
///
/// Spec §6.3:
/// - Sleeps `reconnect.next_delay()` between attempts.
/// - Honours `reconnect.is_paused()` after the background grace window.
/// - Refreshes the access token if its expiry is within 2 minutes, even while
///   the websocket stays connected.
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

        // Idle until we have a session. The backend URL comes from
        // compile-time `build_config`.
        let backend_url = crate::build_config::BACKEND_URL;

        // Stale-access pre-emptive refresh: if we're within 2 minutes of
        // expiry, rotate first. The refresh updates auth_session in
        // place; a refresh failure transitions us out of Authenticated
        // (publishes RefreshFailed) and the loop bails on the next
        // iteration via the auth_session check below.
        let needs_refresh = {
            let guard = ctx.auth_session.read().await;
            guard.as_ref().is_some_and(access_token_needs_refresh)
        };
        if needs_refresh && !refresh_inline(&ctx, backend_url).await {
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
            // No session yet — idle and check again.
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let _ = ctx
            .state_tx
            .send(ConnectionState::Reconnecting { attempt: 1 });

        match connect_with_handles(&ctx, backend_url, Some(&access)).await {
            Ok(()) => {
                // Subscribe BEFORE publishing `Connected` so the
                // RealtimeSession can't fire `Disconnected` between the
                // send and the subscribe and leave us hanging.
                // The borrow_and_update() right after subscribe handles the
                // case where Disconnected lands inside the very-narrow
                // window between subscribe and Connected publishing.
                let mut state_rx = ctx.state_tx.subscribe();
                let _ = ctx.state_tx.send(ConnectionState::Connected);
                ctx.reconnect.record_success().await;
                loop {
                    if matches!(*state_rx.borrow_and_update(), ConnectionState::Disconnected) {
                        break;
                    }
                    tokio::select! {
                        changed = state_rx.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                        () = tokio::time::sleep(connected_refresh_delay(&ctx).await) => {
                            let needs_refresh = {
                                let guard = ctx.auth_session.read().await;
                                guard.as_ref().is_some_and(access_token_needs_refresh)
                            };
                            if needs_refresh && !refresh_inline(&ctx, backend_url).await {
                                return;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(?e, "mobile: reconnect attempt failed");
                let _ = ctx.state_tx.send(ConnectionState::Disconnected);
                if matches!(e, MinosError::Unauthorized { .. }) {
                    clear_auth_session_and_disconnect_ctx(&ctx).await;
                    return;
                }
                ctx.reconnect.record_failure().await;
            }
        }

        let delay = ctx.reconnect.next_delay().await;
        tokio::time::sleep(delay).await;
    }
}

async fn connected_refresh_delay(ctx: &ReconnectContext) -> Duration {
    let guard = ctx.auth_session.read().await;
    let Some(session) = guard.as_ref() else {
        return Duration::from_secs(1);
    };
    refresh_check_delay(session)
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
    let http =
        match crate::http::MobileHttpClient::new(backend_url, ctx.device_id, ctx.self_name.clone())
        {
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

async fn clear_auth_session_and_disconnect_ctx(ctx: &ReconnectContext) {
    *ctx.auth_session.write().await = None;
    let _ = ctx.store.clear_auth().await;
    let _ = ctx.auth_state_tx.send(AuthStateFrame::Unauthenticated);
    let mut tasks = ctx.tasks.lock().await;
    for handle in tasks.drain(..) {
        handle.abort();
    }
    let _ = ctx.state_tx.send(ConnectionState::Disconnected);
}

async fn fetch_ticket_and_build_ws_url_ctx(
    ctx: &ReconnectContext,
    base_url: &str,
    access_token: Option<&str>,
) -> Result<String, MinosError> {
    let access = access_token.ok_or(MinosError::NotConnected)?;
    let http = crate::http::MobileHttpClient::new(base_url, ctx.device_id, ctx.self_name.clone())
        .map_err(|e| MinosError::BackendInternal {
        message: format!("build http client for ws-ticket: {e}"),
    })?;
    let ticket_resp = http
        .fetch_ws_ticket(access, &ctx.device_id.to_string())
        .await?;
    Ok(websocket_url_from_ticket_response(
        base_url,
        &ticket_resp.ticket,
        ticket_resp.gateway_url,
    ))
}

fn websocket_url_from_ticket_response(
    base_url: &str,
    ticket: &str,
    gateway_url: Option<String>,
) -> String {
    if let Some(gateway_url) = gateway_url {
        // The backend returns a relative gateway URL like
        // "/ws/client?ticket=...". `base_url` may still be the legacy
        // `wss://host/devices` compile-time value, so always normalize it to
        // scheme/host/port before joining.
        if gateway_url.starts_with('/') {
            format!("{}{}", websocket_base(base_url), gateway_url)
        } else {
            gateway_url
        }
    } else {
        MobileClient::build_websocket_url(base_url, ticket)
    }
}

/// Standalone connect helper that takes the same handle bundle as the
/// reconnect loop. Mirrors [`MobileClient::connect`] but doesn't borrow
/// `&self` so we can call it from a task that doesn't hold a reference
/// to the originating client.
async fn connect_with_handles(
    ctx: &ReconnectContext,
    url: &str,
    access_token: Option<&str>,
) -> Result<(), MinosError> {
    let bearer_present = access_token.is_some();
    let ws_url = fetch_ticket_and_build_ws_url_ctx(ctx, url, access_token).await?;
    let websocket = MobileClient::open_backend_websocket(&ws_url)
        .await
        .map_err(|error| connect_error_to_minos(&ws_url, error, &ctx.device_id, bearer_present))?;

    let account_id = ctx
        .auth_session
        .read()
        .await
        .as_ref()
        .map(|s| s.account.account_id.clone())
        .unwrap_or_default();

    let (_frame_tx, frame_rx) = tokio::sync::mpsc::channel(1);
    let session_handle = tokio::spawn(RealtimeSession::run(
        websocket,
        account_id,
        ctx.subscription_mgr.clone(),
        ctx.ui_events_tx.clone(),
        ctx.social_events_tx.clone(),
        ctx.state_tx.clone(),
        frame_rx,
    ));

    let mut tasks = ctx.tasks.lock().await;
    for h in tasks.drain(..) {
        h.abort();
    }
    tasks.push(session_handle);
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
            device_id: Some(DeviceId::new().to_string()),
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

    #[test]
    fn persisted_pairing_state_exports_device_id_before_pairing() {
        let persisted = PersistedPairingState {
            device_id: Some(DeviceId::new().to_string()),
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

    #[test]
    fn access_token_needs_refresh_is_true_for_expired_session() {
        let session = AuthSession {
            access_token: "access".into(),
            access_expires_at_ms: 0,
            access_expires_at: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            refresh_token: "refresh".into(),
            account: AuthSummary {
                account_id: "acct-1".into(),
                email: "a@b.com".into(),
            },
        };

        assert!(access_token_needs_refresh(&session));
    }

    #[test]
    fn access_token_needs_refresh_is_false_for_fresh_session() {
        let session = AuthSession {
            access_token: "access".into(),
            access_expires_at_ms: 0,
            access_expires_at: Instant::now() + Duration::from_secs(600),
            refresh_token: "refresh".into(),
            account: AuthSummary {
                account_id: "acct-1".into(),
                email: "a@b.com".into(),
            },
        };

        assert!(!access_token_needs_refresh(&session));
    }

    #[test]
    fn relative_gateway_url_ignores_legacy_devices_path() {
        let url = websocket_url_from_ticket_response(
            "wss://example.com/devices",
            "ticket-abc",
            Some("/ws/client?ticket=ticket-abc".into()),
        );

        assert_eq!(url, "wss://example.com/ws/client?ticket=ticket-abc");
    }

    #[test]
    fn missing_gateway_url_fallback_ignores_legacy_devices_path() {
        let url =
            websocket_url_from_ticket_response("wss://example.com/devices", "ticket-abc", None);

        assert_eq!(url, "wss://example.com/ws/client?ticket=ticket-abc");
    }

    #[test]
    fn absolute_gateway_url_is_preserved() {
        let url = websocket_url_from_ticket_response(
            "wss://example.com/devices",
            "ticket-abc",
            Some("wss://edge.example/ws/client?ticket=edge-ticket".into()),
        );

        assert_eq!(url, "wss://edge.example/ws/client?ticket=edge-ticket");
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
    async fn list_threads_without_persisted_state_errors_unauthorized() {
        // ADR-0020 dropped the device-secret rail; list_threads is
        // bearer-only. With no auth_session it surfaces Unauthorized
        // (not StoreCorrupt — the device-secret is no longer required).
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
            matches!(err, MinosError::Unauthorized { .. }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn forbidden_handshake_maps_to_unauthorized_with_status_in_reason() {
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WebSocketError::Handshake {
                status: Some(http::StatusCode::FORBIDDEN),
                reason: HandshakeFailure::UnexpectedStatus,
            },
            &DeviceId::new(),
            true,
        );

        match err {
            MinosError::Unauthorized { reason } => {
                assert!(
                    reason.contains("403"),
                    "Unauthorized reason should embed the status code: {reason}"
                );
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn ws_401_with_bearer_maps_to_unauthorized() {
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WebSocketError::Handshake {
                status: Some(http::StatusCode::UNAUTHORIZED),
                reason: HandshakeFailure::UnexpectedStatus,
            },
            &DeviceId::new(),
            true,
        );

        match err {
            MinosError::Unauthorized { reason } => {
                assert!(
                    reason.contains("401"),
                    "expected unauthorized reason to include status: {reason}"
                );
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn non_cf_http_status_maps_to_connect_failed_with_status_detail() {
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WebSocketError::Handshake {
                status: Some(http::StatusCode::BAD_GATEWAY),
                reason: HandshakeFailure::UnexpectedStatus,
            },
            &DeviceId::new(),
            true,
        );

        match err {
            MinosError::ConnectFailed { url, message } => {
                assert_eq!(url, "wss://example.com/devices");
                assert!(
                    message.contains("502"),
                    "expected status in message: {message}"
                );
                assert!(
                    message.contains("UnexpectedStatus"),
                    "expected handshake detail in message: {message}"
                );
            }
            other => panic!("expected ConnectFailed, got {other:?}"),
        }
    }

    #[test]
    fn io_error_maps_to_connect_failed_with_kind_in_message() {
        let err = connect_error_to_minos(
            "wss://example.com/devices",
            WebSocketError::Io(WireError::tcp_connect(
                "io ConnectionRefused",
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "nope"),
            )),
            &DeviceId::new(),
            false,
        );

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
    async fn send_user_message_requires_authentication() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client
            .send_user_message("thr_1".into(), "ping".into())
            .await;
        assert!(matches!(res, Err(MinosError::Unauthorized { .. })));
    }

    #[tokio::test]
    async fn close_thread_requires_authentication() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client.close_thread("thr".into()).await;
        assert!(matches!(res, Err(MinosError::Unauthorized { .. })));
    }

    #[tokio::test]
    async fn send_approval_decision_requires_authentication() {
        let client = MobileClient::new_with_in_memory_store("iPhone".into());
        let res = client
            .send_approval_decision(
                "req-1".into(),
                "thr-1".into(),
                serde_json::json!({ "decision": "accept" }),
            )
            .await;
        assert!(matches!(res, Err(MinosError::Unauthorized { .. })));
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
}
