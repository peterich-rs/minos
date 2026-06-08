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
//! 2. `request_pairing_token_returns_qr_with_mac_name` — issues
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
    http::{router, BackendState},
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_daemon::config::RelayConfig;
use minos_daemon::relay_client::{PersistenceCtx, RelayClient};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, MinosError, RelayLinkState};
use pretty_assertions::assert_eq;
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
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
    pool: SqlitePool,
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
async fn spawn_relay() -> anyhow::Result<Relay> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let mut state = BackendState::new(
        Arc::new(SessionRegistry::new()),
        Arc::new(PairingService::new(pool.clone())),
        pool.clone(),
        TOKEN_TTL,
        "daemon-smoke-test-jwt-secret-32b".to_string(),
        None,
        "daemon-smoke-test-instance".to_string(),
    );
    state.version = "daemon-smoke-test";
    let app = router(state);

    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        pool,
        _db_file: tmp,
        task,
    })
}

/// `ws://HOST:PORT/devices` URL for the running relay. Matches the shape
/// that `minos_daemon::config::BACKEND_URL` would carry in production.
fn relay_url(relay: &Relay) -> String {
    format!("ws://{}/devices", relay.addr)
}

/// Default config for the in-process backend.
fn test_config() -> RelayConfig {
    RelayConfig::new(String::new())
}

/// Fresh in-memory `PersistenceCtx` for relay-client tests.
fn test_persistence() -> PersistenceCtx {
    PersistenceCtx {
        peer_store: Arc::new(StdMutex::new(None)),
        peers_store: Arc::new(StdMutex::new(Vec::new())),
        last_error: Arc::new(StdMutex::new(None::<MinosError>)),
        // No Reconciliator wired: these smoke tests exercise the
        // relay-client transport, not Phase D reconciliation.
        reconciliator: None,
    }
}

async fn register_formal_host(
    pool: &SqlitePool,
    host_id: DeviceId,
) -> anyhow::Result<DeviceSecret> {
    store::devices::insert_device(pool, host_id, "Fan's Mac", DeviceRole::AgentHost, 0).await?;
    let account = store::accounts::create(pool, "relay-smoke@example.com", "phc").await?;
    let mobile_id = DeviceId::new();
    store::devices::insert_device(pool, mobile_id, "iPhone", DeviceRole::MobileClient, 0).await?;
    store::devices::set_account_id(pool, &mobile_id, &account.account_id).await?;

    let pairing = PairingService::new(pool.clone());
    let (code, _) = pairing.request_code(host_id, TOKEN_TTL).await?;
    pairing
        .confirm_pairing_code(
            &code,
            &account.account_id,
            mobile_id,
            Some("relay-smoke-confirm"),
        )
        .await
        .map_err(|error| anyhow::anyhow!("confirm formal pairing code failed: {error:?}"))?;
    let redeemed = pairing
        .redeem_host_installation(&code, host_id, Some("relay-smoke-redeem"))
        .await
        .map_err(|error| anyhow::anyhow!("redeem formal host token failed: {error:?}"))?;
    let secret = DeviceSecret(redeemed.token);

    // `/v1/me/peers` is still the legacy host snapshot route and checks
    // X-Device-Secret. Mirror the formal token into the legacy hash slot
    // until that route is retired from the daemon refresh path.
    let hash = hash_secret(&secret)?;
    store::devices::upsert_secret_hash(pool, host_id, &hash).await?;
    Ok(secret)
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn connect_becomes_connected() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let backend_url = relay_url(&relay);
    let persistence = test_persistence();
    let host_id = DeviceId::new();
    let host_secret = register_formal_host(&relay.pool, host_id).await?;

    let (client, mut link_rx, _peer_rx) = RelayClient::spawn(
        test_config(),
        host_id,
        None,
        Some(host_secret),
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
async fn request_pairing_token_returns_qr_with_mac_name() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;
    let backend_url = relay_url(&relay);
    let persistence = test_persistence();

    let mac_name = "Fan's MacBook Pro".to_string();
    let host_id = DeviceId::new();
    let (client, _link_rx, _peer_rx) = RelayClient::spawn(
        test_config(),
        host_id,
        None,
        None,
        mac_name.clone(),
        backend_url.clone(),
        None,
        persistence,
    );

    let qr = timeout(STEP_TIMEOUT, client.request_pairing_token())
        .await
        .expect("request_pairing_token did not complete within timeout")?;

    // QR payload v2: backend assembles the full payload (ADR 0014). v=1 was
    // the legacy host-assembled shape; the new flow returns v=2.
    assert_eq!(qr.v, 2);
    assert_eq!(qr.host_display_name, mac_name);
    assert!(qr.expires_at_ms > 0, "expected epoch-ms expiry");
    assert!(
        !qr.pairing_token.as_str().is_empty(),
        "expected non-empty pairing token, got {:?}",
        qr.pairing_token
    );

    client.stop().await;
    Ok(())
}
