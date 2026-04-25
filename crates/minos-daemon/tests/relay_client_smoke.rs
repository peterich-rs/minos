//! Integration smoke-tests for `minos_daemon::relay_client::RelayClient`.
//!
//! Each test boots a real in-process backend (axum + sqlx over a temp-file
//! SQLite DB, copied from `crates/minos-backend/tests/e2e.rs`'s harness) on
//! `127.0.0.1:0`, spawns a `RelayClient` targeting it, and drives the
//! flow end-to-end. The assertions freeze the contract Phase F will wire
//! into `DaemonHandle`:
//!
//! 1. `connect_becomes_connected` — link transitions
//!    `Connecting{0}` → `Connected` within a bounded window.
//! 2. `ping_local_rpc_returns_ok_true` — round-trips a `LocalRpc::Ping`
//!    and gets back `{"ok": true}` with full correlation handling.
//! 3. `request_pairing_token_returns_qr_with_mac_name` — issues
//!    `RequestPairingToken`, wraps into `RelayQrPayload`, and cross-checks
//!    the backend URL and mac display name.
//!
//! The harness lives inline here (rather than a shared crate) so the
//! daemon's test tree does not take a production dep on the backend; the
//! dev-dep is scoped to this file.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use minos_backend::{
    http::{router, BackendPublicConfig, BackendState},
    ingest::translate::ThreadTranslators,
    pairing::PairingService,
    session::SessionRegistry,
    store,
};
use minos_daemon::config::RelayConfig;
use minos_daemon::relay_client::{PersistenceCtx, RelayClient};
use minos_domain::{DeviceId, MinosError, RelayLinkState};
use minos_protocol::envelope::LocalRpcMethod;
use pretty_assertions::assert_eq;
use sqlx::SqlitePool;
use tempfile::{NamedTempFile, TempDir};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// Wall-clock ceiling for each test's primary await. Copied from the
/// relay's own e2e wrapper — plenty of slack for a shared-runner CI.
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

/// Token TTL fed into the relay state; tests exercise the ISSUANCE path,
/// not expiry, so a generous value is fine.
const TOKEN_TTL: Duration = Duration::from_mins(5);

/// In-process backend harness. Holds the axum serve task and the temp-file
/// SQLite pool. Drop aborts the task so parallel tests don't leak tokio
/// resources (matches the pattern used in `minos-backend/tests/e2e.rs`).
struct Relay {
    addr: SocketAddr,
    _pool: SqlitePool,
    _db_file: NamedTempFile,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Boot a fresh backend on `127.0.0.1:0` backed by a tempfile DB. Mirrors
/// `minos-backend/tests/e2e.rs::spawn_relay_with_token_ttl`.
///
/// The listener is bound BEFORE the `BackendState` is constructed so the
/// `BackendPublicConfig.public_url` carries the real `ws://HOST:PORT/devices`
/// address — the QR-assembly path in `RequestPairingQr` echoes that URL back
/// in the payload, and the smoke test asserts it matches what the client dialed.
async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let public_url = format!("ws://{addr}/devices");

    let state = BackendState {
        registry: Arc::new(SessionRegistry::new()),
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl: TOKEN_TTL,
        translators: ThreadTranslators::new(),
        public_cfg: Arc::new(BackendPublicConfig {
            public_url,
            cf_access_client_id: None,
            cf_access_client_secret: None,
        }),
        version: "daemon-smoke-test",
    };
    let app = router(state);

    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        _pool: pool,
        _db_file: tmp,
        task,
    })
}

/// `ws://HOST:PORT/devices` URL for the running relay. Matches the shape
/// that `minos_daemon::config::BACKEND_URL` would carry in production.
fn relay_url(relay: &Relay) -> String {
    format!("ws://{}/devices", relay.addr)
}

/// Default empty-CF config — the in-process backend is not behind CF Access.
fn test_config() -> RelayConfig {
    RelayConfig::new(String::new(), String::new())
}

/// Fresh `PersistenceCtx` backed by a throwaway tempdir.
///
/// The returned `TempDir` must stay in scope for the duration of the test
/// — the dispatch task writes `local-state.json` underneath it on every
/// `EventKind::Paired` / `Unpaired`. Dropping early would race those
/// writes against the directory teardown.
fn test_persistence() -> (PersistenceCtx, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let ctx = PersistenceCtx {
        peer_store: Arc::new(StdMutex::new(None)),
        local_state_path: tmp.path().join("local-state.json"),
        last_error: Arc::new(StdMutex::new(None::<MinosError>)),
    };
    (ctx, tmp)
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn connect_becomes_connected() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let backend_url = relay_url(&relay);
    let (persistence, _tmp) = test_persistence();

    let (client, mut link_rx, _peer_rx) = RelayClient::spawn(
        test_config(),
        DeviceId::new(),
        None,
        None,
        "Fan's Mac".to_string(),
        backend_url,
        None,
        persistence,
    );

    // Initial state is `Disconnected`; wait for `Connected` within the
    // step timeout. The intermediate `Connecting { attempt: 0 }` is
    // deliberately not asserted — it's a transient the watch may coalesce.
    timeout(STEP_TIMEOUT, async {
        loop {
            if matches!(*link_rx.borrow_and_update(), RelayLinkState::Connected) {
                return;
            }
            // `changed()` returns `Err` only once every sender drops; the
            // client holds one, so a bare `.await` and unwrap is safe
            // for the bounded timeout.
            link_rx
                .changed()
                .await
                .expect("link sender must stay alive for the test's duration");
        }
    })
    .await
    .expect("relay link did not reach Connected within timeout");

    client.stop().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ping_local_rpc_returns_ok_true() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let backend_url = relay_url(&relay);
    let (persistence, _tmp) = test_persistence();

    let (client, mut link_rx, _peer_rx) = RelayClient::spawn(
        test_config(),
        DeviceId::new(),
        None,
        None,
        "Fan's Mac".to_string(),
        backend_url,
        None,
        persistence,
    );

    // Wait until the link is up so the Ping isn't racing the handshake.
    timeout(STEP_TIMEOUT, async {
        loop {
            if matches!(*link_rx.borrow_and_update(), RelayLinkState::Connected) {
                return;
            }
            link_rx.changed().await.expect("link sender alive");
        }
    })
    .await
    .expect("relay link did not reach Connected within timeout");

    let result = timeout(
        STEP_TIMEOUT,
        client.send_local_rpc(LocalRpcMethod::Ping, serde_json::json!({})),
    )
    .await
    .expect("ping did not complete within timeout")?;

    assert_eq!(result, serde_json::json!({"ok": true}));

    client.stop().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn request_pairing_token_returns_qr_with_mac_name() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let backend_url = relay_url(&relay);
    let (persistence, _tmp) = test_persistence();

    let mac_name = "Fan's MacBook Pro".to_string();
    let (client, mut link_rx, _peer_rx) = RelayClient::spawn(
        test_config(),
        DeviceId::new(),
        None,
        None,
        mac_name.clone(),
        backend_url.clone(),
        None,
        persistence,
    );

    timeout(STEP_TIMEOUT, async {
        loop {
            if matches!(*link_rx.borrow_and_update(), RelayLinkState::Connected) {
                return;
            }
            link_rx.changed().await.expect("link sender alive");
        }
    })
    .await
    .expect("relay link did not reach Connected within timeout");

    let qr = timeout(STEP_TIMEOUT, client.request_pairing_token())
        .await
        .expect("request_pairing_token did not complete within timeout")?;

    // QR payload v2: backend assembles the full payload (ADR 0014). v=1 was
    // the legacy host-assembled shape; the new flow returns v=2.
    assert_eq!(qr.v, 2);
    assert_eq!(qr.backend_url, backend_url);
    assert_eq!(qr.mac_display_name, mac_name);
    assert!(
        !qr.token.as_str().is_empty(),
        "expected non-empty pairing token, got {:?}",
        qr.token
    );

    client.stop().await;
    Ok(())
}
