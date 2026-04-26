# HTTP Control-Plane Split (v1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **For this project specifically:** the user's standing preference is to execute plans **directly in the main conversation**, not via subagent dispatch. Write code in the main session, run `cargo xtask check-all` before each commit, then commit.

**Goal:** Move pairing and thread-history RPCs out of the multiplexed WebSocket envelope into versioned `/v1/*` HTTP routes. The WebSocket keeps only what genuinely needs to stream: `Forward`/`Forwarded` peer-RPC, `Ingest` from host, and `Event` push (presence + UI fan-out).

**Architecture:** The backend already runs an axum router with two routes (`/health`, `/devices`). We add a `/v1` nest with six new routes (3 pairing + 3 threads) that share the existing `BackendState` (registry, pairing service, store). Auth is unchanged: `X-Device-Id` / `X-Device-Role` / `X-Device-Secret` headers, validated by the same `classify` + `verify_secret` logic that today lives inside `ws_devices.rs`. We extract that logic into a shared `http::auth` module so both the `/devices` upgrade and the `/v1/*` handlers use it. Daemon and mobile clients add a `reqwest` HTTP client and switch their pairing + thread-history calls from `LocalRpc` envelopes to HTTP. After clients are migrated, we delete `Envelope::LocalRpc{,Response}`, `LocalRpcMethod`, `LocalRpcOutcome`, the dispatcher (`envelope/local_rpc.rs`), and the per-id pending-map machinery on both clients.

**Tech Stack:** Rust workspace (axum 0.7, sqlx-sqlite, tokio-tungstenite for WS, reqwest for new HTTP clients). No new framework. The wire envelope keeps `"v": 1` — only URLs gain versioning.

**Backwards-compatibility window:** Phases A–C add HTTP routes alongside the existing WS LocalRpc handlers. Phase D removes the WS handlers. During A–C the system stays usable on either transport; this lets us split the migration across commits without a flag-day. There is no production deployment yet, so we don't need feature flags or per-client compat shims.

**Versioning policy reaffirmed:**
- All public HTTP routes live under `/v1/*`. `/health` is unversioned (industry convention).
- The WS path stays `/devices` (unversioned). The envelope continues to carry `"v": 1`. URL versioning and envelope versioning are independent axes.
- Error JSON keeps the existing shape `{ "error": { "code": "snake_case", "message": "..." } }` — no `request_id`, no trace fields. Reuse the snake-case codes already defined in `crates/minos-backend/src/envelope/local_rpc.rs:23-35`.

**Out of scope (explicit):**
- WS path rename / envelope `v` bump / `kind` string rename — none.
- `Forward`/`Forwarded` peer-RPC tunnel — left untouched.
- Dead-code cleanup of `minos-transport::client::WsClient` and `minos-agent-runtime::ingest::Ingestor` — separate ticket.
- Observability work (request IDs, trace spans propagated to clients) — separate ticket.

---

## File Structure

**New files:**
- `crates/minos-backend/src/http/auth.rs` — shared header extraction + auth classifier extracted from `ws_devices.rs`.
- `crates/minos-backend/src/http/v1/mod.rs` — `Router` factory for the `/v1` nest.
- `crates/minos-backend/src/http/v1/pairing.rs` — `POST /v1/pairing/tokens`, `POST /v1/pairing/consume`, `DELETE /v1/pairing`.
- `crates/minos-backend/src/http/v1/threads.rs` — `GET /v1/threads`, `GET /v1/threads/{thread_id}/events`, `GET /v1/threads/{thread_id}/last_seq`.
- `crates/minos-daemon/src/relay_http.rs` — daemon-side reqwest client wrapping the new pairing endpoints.
- `crates/minos-mobile/src/http.rs` — mobile-side reqwest client wrapping pairing + thread endpoints.

**Modified files:**
- `crates/minos-backend/src/http/mod.rs` — add `pub mod v1; pub mod auth;`, nest the v1 router.
- `crates/minos-backend/src/http/ws_devices.rs` — replace inline header/classify helpers with `crate::http::auth` calls.
- `crates/minos-backend/src/envelope/mod.rs` — remove `LocalRpc` / `LocalRpcResponse` arms in Phase D.
- `crates/minos-backend/src/envelope/local_rpc.rs` — deleted in Phase D.
- `crates/minos-protocol/src/envelope.rs` — drop `LocalRpc{,Response}` / `LocalRpcMethod` / `LocalRpcOutcome` / `RpcError` in Phase D.
- `crates/minos-daemon/src/relay_client.rs` — replace `request_pairing_token` / `forget_peer` bodies; remove `send_local_rpc` + pending-map machinery in Phase D.
- `crates/minos-mobile/src/client.rs` — replace `pair_with_qr_json` / `forget_peer` / `list_threads` / `read_thread` / `get_thread_last_seq`; remove `local_rpc` helper + pending DashMap in Phase D.
- `Cargo.toml` (workspace) — add `reqwest` to `[workspace.dependencies]`.
- `crates/minos-daemon/Cargo.toml`, `crates/minos-mobile/Cargo.toml` — depend on `reqwest`.

**Test files:**
- `crates/minos-backend/tests/v1_pairing.rs` — integration tests for the three pairing routes.
- `crates/minos-backend/tests/v1_threads.rs` — integration tests for the three thread routes.
- Existing `crates/minos-backend/src/envelope/local_rpc.rs` test module is deleted in Phase D along with the file.
- Existing `crates/minos-protocol/tests/envelope_golden.rs` fixtures for `local_rpc*` are deleted in Phase D.

---

## Pre-flight: shared HTTP test scaffolding

Several tasks need a way to exercise an axum `Router` from a test. Add this once at the start of Phase A so the rest of the plan can reuse it.

**Tower's `ServiceExt::oneshot`** is the standard pattern. Add to the backend's `[dev-dependencies]`:

```toml
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

Helper function the integration tests will call (place at the top of each `tests/v1_*.rs` file — small enough to copy):

```rust
async fn send(
    app: &mut axum::Router,
    req: axum::http::Request<axum::body::Body>,
) -> (axum::http::StatusCode, serde_json::Value) {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, body)
}
```

Each integration test builds its own `BackendState` via `crate::test_support::backend_state()` (introduced in Task A1) and calls `http::router(state)` to get the live router.

---

## Phase A — Backend `/v1` scaffolding

After this phase the backend serves `POST /v1/pairing/tokens` (parallel to the WS `LocalRpcMethod::RequestPairingQr`). No clients are switched yet.

### Task A1: Extract shared header / classify helper into `http::auth`

**Why:** The header parsing (`extract_device_id`, `extract_device_role`, `extract_device_secret`, `extract_device_name`, `resolve_device_role`, `log_cf_access_presence`) and the `classify` decision live inside `ws_devices.rs`. Both the WS upgrade and every `/v1/*` handler need the same logic. Extracting it once is the only structural change of this phase.

**Files:**
- Create: `crates/minos-backend/src/http/auth.rs`
- Modify: `crates/minos-backend/src/http/mod.rs` (`pub mod auth;` + add `test_support` module)
- Modify: `crates/minos-backend/src/http/ws_devices.rs` (delegate to new module)

- [x] **Step 1: Write the failing test for `Authenticator::authenticate`**

Create `crates/minos-backend/tests/auth_helper.rs`:

```rust
use axum::http::{HeaderMap, HeaderValue};
use minos_backend::http::auth::{authenticate, AuthError, AuthOutcome};
use minos_backend::store::{devices::insert_device, test_support::memory_pool};
use minos_domain::{DeviceId, DeviceRole};

fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(*k, HeaderValue::from_str(v).unwrap());
    }
    h
}

#[tokio::test]
async fn first_connect_inserts_row_and_returns_authenticated() {
    let pool = memory_pool().await;
    let id = DeviceId::new();
    let headers = header_map(&[
        ("x-device-id", &id.to_string()),
        ("x-device-role", "agent-host"),
        ("x-device-name", "Mac"),
    ]);

    let outcome = authenticate(&pool, &headers).await.unwrap();
    assert!(matches!(outcome, AuthOutcome { device_id, role: DeviceRole::AgentHost, .. } if device_id == id));

    let row = minos_backend::store::devices::get_device(&pool, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.role, DeviceRole::AgentHost);
    assert_eq!(row.display_name, "Mac");
    assert!(row.secret_hash.is_none());
}

#[tokio::test]
async fn missing_device_id_returns_unauthorized() {
    let pool = memory_pool().await;
    let headers = HeaderMap::new();
    let err = authenticate(&pool, &headers).await.unwrap_err();
    assert!(matches!(err, AuthError::Unauthorized(_)));
}

#[tokio::test]
async fn role_mismatch_against_existing_row_is_unauthorized() {
    let pool = memory_pool().await;
    let id = DeviceId::new();
    insert_device(&pool, id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let headers = header_map(&[
        ("x-device-id", &id.to_string()),
        ("x-device-role", "ios-client"),
    ]);
    let err = authenticate(&pool, &headers).await.unwrap_err();
    assert!(matches!(err, AuthError::Unauthorized(_)));
}
```

- [x] **Step 2: Run — must fail**

```bash
cargo test -p minos-backend --test auth_helper
```

Expected: compilation error (`use minos_backend::http::auth::...` unresolved).

- [x] **Step 3: Implement `crates/minos-backend/src/http/auth.rs`**

```rust
//! Shared header extraction + auth classification for HTTP handlers.
//!
//! Both `GET /devices` (WS upgrade) and the `/v1/*` REST routes call
//! [`authenticate`] to resolve `(device_id, role)` from the
//! `X-Device-*` header bundle. First-connect devices are inserted into
//! the `devices` table with `secret_hash = NULL`; existing rows are
//! verified against the supplied secret if one is stored.

use axum::http::{HeaderMap, StatusCode};
use minos_domain::{DeviceId, DeviceRole};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;

use crate::pairing::secret::verify_secret;
use crate::store::{
    self,
    devices::{insert_device, DeviceRow},
};

pub const HDR_DEVICE_ID: &str = "x-device-id";
pub const HDR_DEVICE_ROLE: &str = "x-device-role";
pub const HDR_DEVICE_SECRET: &str = "x-device-secret";
pub const HDR_DEVICE_NAME: &str = "x-device-name";
pub const HDR_CF_ACCESS_ID: &str = "cf-access-client-id";
pub const HDR_CF_ACCESS_SECRET: &str = "cf-access-client-secret";

const DEFAULT_DISPLAY_NAME: &str = "unnamed";

/// Result of a successful classification.
#[derive(Debug, Clone)]
pub struct AuthOutcome {
    pub device_id: DeviceId,
    pub role: DeviceRole,
    /// `Some(secret)` if the request supplied `X-Device-Secret` AND the
    /// stored row had a hash that verified. `None` for first-connect or
    /// existing-but-no-hash rows. Used by handlers that need to decide
    /// whether to allow secret-less calls (e.g. `/v1/pairing/consume`).
    pub authenticated_with_secret: bool,
}

/// Auth-layer error kinds. Both variants carry an operator-facing
/// message; `Unauthorized` round-trips to HTTP 401 / WS pre-upgrade 401,
/// `Internal` round-trips to 500 / activation close 1011.
#[derive(Debug)]
pub enum AuthError {
    Unauthorized(String),
    Internal(String),
}

impl AuthError {
    pub fn into_response_tuple(self) -> (StatusCode, String) {
        match self {
            Self::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

/// Parse headers, look up the device row, classify, insert on first
/// connect, and return the resolved `(device_id, role)`. Side-effecting:
/// may insert into `devices`.
pub async fn authenticate(
    pool: &SqlitePool,
    headers: &HeaderMap,
) -> Result<AuthOutcome, AuthError> {
    let device_id = extract_device_id(headers)?;
    let requested_role = extract_device_role(headers)?;
    let device_secret = extract_device_secret(headers);
    let display_name = extract_device_name(headers).unwrap_or_else(|| DEFAULT_DISPLAY_NAME.into());
    log_cf_access_presence(headers);

    let existing = store::devices::get_device(pool, device_id)
        .await
        .map_err(|e| AuthError::Internal(e.to_string()))?;
    let role = resolve_device_role(existing.as_ref(), requested_role)?;

    let classification = classify(existing, device_secret.as_deref())?;
    let authenticated_with_secret = matches!(classification, Classification::Authenticated);

    if matches!(classification, Classification::FirstConnect) {
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(e) = insert_device(pool, device_id, &display_name, role, now).await {
            tracing::warn!(
                target: "minos_backend::http::auth",
                error = %e,
                device_id = %device_id,
                "first-connect insert_device failed (race?)",
            );
        }
    }

    Ok(AuthOutcome {
        device_id,
        role,
        authenticated_with_secret,
    })
}

/// Same as [`authenticate`] but also asserts the resolved role equals
/// `expected`. Used by handlers that are role-gated.
pub async fn authenticate_role(
    pool: &SqlitePool,
    headers: &HeaderMap,
    expected: DeviceRole,
) -> Result<AuthOutcome, AuthError> {
    let outcome = authenticate(pool, headers).await?;
    if outcome.role != expected {
        return Err(AuthError::Unauthorized(format!(
            "role required: {expected}, got {}",
            outcome.role
        )));
    }
    Ok(outcome)
}

#[derive(Debug)]
pub enum Classification {
    FirstConnect,
    UnpairedExisting,
    Authenticated,
}

pub fn classify(
    row: Option<DeviceRow>,
    provided_secret: Option<&str>,
) -> Result<Classification, AuthError> {
    match row {
        None => Ok(Classification::FirstConnect),
        Some(r) => match r.secret_hash {
            None => Ok(Classification::UnpairedExisting),
            Some(hash) => {
                let Some(secret) = provided_secret else {
                    return Err(AuthError::Unauthorized(
                        "X-Device-Secret required for authenticated device".into(),
                    ));
                };
                match verify_secret(secret, &hash) {
                    Ok(true) => Ok(Classification::Authenticated),
                    Ok(false) => Err(AuthError::Unauthorized(
                        "X-Device-Secret does not match stored hash".into(),
                    )),
                    Err(e) => Err(AuthError::Internal(e.to_string())),
                }
            }
        },
    }
}

pub fn extract_device_id(headers: &HeaderMap) -> Result<DeviceId, AuthError> {
    let raw = headers
        .get(HDR_DEVICE_ID)
        .ok_or_else(|| AuthError::Unauthorized("X-Device-Id required".into()))?;
    let s = raw
        .to_str()
        .map_err(|_| AuthError::Unauthorized("X-Device-Id not UTF-8".into()))?;
    Uuid::parse_str(s)
        .map(DeviceId)
        .map_err(|e| AuthError::Unauthorized(format!("X-Device-Id not a valid UUID: {e}")))
}

pub fn extract_device_role(headers: &HeaderMap) -> Result<Option<DeviceRole>, AuthError> {
    let Some(raw) = headers.get(HDR_DEVICE_ROLE) else {
        return Ok(None);
    };
    let s = raw
        .to_str()
        .map_err(|_| AuthError::Unauthorized("X-Device-Role not UTF-8".into()))?;
    DeviceRole::from_str(s)
        .map(Some)
        .map_err(|e| AuthError::Unauthorized(format!("X-Device-Role invalid: {e}")))
}

pub fn resolve_device_role(
    existing: Option<&DeviceRow>,
    requested_role: Option<DeviceRole>,
) -> Result<DeviceRole, AuthError> {
    match existing {
        Some(row) => {
            if let Some(role) = requested_role {
                if role != row.role {
                    return Err(AuthError::Unauthorized(format!(
                        "X-Device-Role mismatch for existing device: expected {}, got {}",
                        row.role, role
                    )));
                }
            }
            Ok(row.role)
        }
        None => Ok(requested_role.unwrap_or(DeviceRole::IosClient)),
    }
}

pub fn extract_device_secret(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_SECRET)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

pub fn extract_device_name(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HDR_DEVICE_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

pub fn log_cf_access_presence(headers: &HeaderMap) {
    let cf_id = headers.contains_key(HDR_CF_ACCESS_ID);
    let cf_sec = headers.contains_key(HDR_CF_ACCESS_SECRET);
    if cf_id || cf_sec {
        tracing::debug!(
            target: "minos_backend::http::auth",
            cf_access_client_id_present = cf_id,
            cf_access_client_secret_present = cf_sec,
            "CF-Access headers observed (edge-validated; backend does not re-check)",
        );
    }
}
```

- [x] **Step 4: Re-export from `http/mod.rs` and add `test_support`**

In `crates/minos-backend/src/http/mod.rs`, add `pub mod auth;` between the existing `pub mod health;` and `pub mod ws_devices;` lines. Then add a `test_support` factory exposed under `#[cfg(test)]` plus integration-test-friendly via `#[cfg(any(test, feature = "test-support"))]`:

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    use crate::store::test_support::memory_pool;

    pub async fn backend_state() -> BackendState {
        let pool = memory_pool().await;
        let registry = std::sync::Arc::new(SessionRegistry::new());
        let pairing = std::sync::Arc::new(PairingService::new(pool.clone()));
        BackendState::new(registry, pairing, pool, std::time::Duration::from_secs(300))
    }
}
```

In `crates/minos-backend/Cargo.toml` add a `test-support` feature gate:

```toml
[features]
test-support = []
```

- [x] **Step 5: Refactor `ws_devices.rs` to use `http::auth`**

Replace the inline `extract_device_id`, `extract_device_role`, `extract_device_secret`, `extract_device_name`, `resolve_device_role`, `classify`, `Classification`, `log_cf_access_presence`, and `HDR_*` constants in `crates/minos-backend/src/http/ws_devices.rs` with calls into `crate::http::auth`. The body of `pub async fn upgrade` keeps the same control flow but reads as:

```rust
pub async fn upgrade(
    State(state): State<BackendState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let outcome = crate::http::auth::authenticate(&state.store, &headers)
        .await
        .map_err(crate::http::auth::AuthError::into_response_tuple)?;

    let (handle, outbox_rx) = SessionHandle::new(outcome.device_id, outcome.role);

    let registry = Arc::clone(&state.registry);
    let pairing = Arc::clone(&state.pairing);
    let store = state.store.clone();
    let token_ttl = state.token_ttl;
    let translators = Arc::clone(&state.translators);
    let public_cfg = Arc::clone(&state.public_cfg);
    let device_id = outcome.device_id;
    let role = outcome.role;
    let device_secret = crate::http::auth::extract_device_secret(&headers);
    let requested_role = crate::http::auth::extract_device_role(&headers).ok().flatten();

    Ok(ws.on_upgrade(move |mut socket| async move {
        match revalidate_live_session_auth(
            &store,
            device_id,
            role,
            requested_role,
            device_secret.as_deref(),
        )
        .await
        {
            Ok(paired_with) => {
                *handle.paired_with.write().await = paired_with;
            }
            Err(ActivationAuthError::Unauthorized(message)) => {
                tracing::info!(target: "minos_backend::http", device_id = %device_id, reason = %message, "activation 4401");
                close_socket(&mut socket, CLOSE_CODE_AUTH_FAILURE, "auth_revoked").await;
                return;
            }
            Err(ActivationAuthError::Internal(message)) => {
                tracing::warn!(target: "minos_backend::http", device_id = %device_id, error = %message, "activation 1011");
                close_socket(&mut socket, CLOSE_CODE_INTERNAL_ERROR, "activation_revalidate_failed").await;
                return;
            }
        }

        activate_live_session(registry.as_ref(), &handle).await;

        if let Err(e) = run_session(socket, handle, outbox_rx, registry, pairing, store, token_ttl, translators, public_cfg).await {
            tracing::warn!(target: "minos_backend::http", error = %e, device_id = %device_id, "run_session error");
        }
    }))
}
```

`revalidate_live_session_auth` stays as-is in this file but is rewritten to call `crate::http::auth::classify` instead of the local `classify`. Delete the now-redundant private items.

The existing tests at the bottom of `ws_devices.rs` that exercise `extract_*` / `classify` (≈lines 525-700) **move** verbatim to `crates/minos-backend/src/http/auth.rs` — the only edits are import paths.

- [x] **Step 6: Run the new test + the relocated tests; confirm green**

```bash
cargo test -p minos-backend --test auth_helper
cargo test -p minos-backend --lib http::auth
cargo test -p minos-backend --lib http::ws_devices
```

Expected: all PASS. The relocated `extract_device_*` / `classify` / `revalidate_live_session_auth` tests still cover the same surface.

- [x] **Step 7: Workspace acceptance**

```bash
cargo xtask check-all
```

Expected: PASS (per the user's standing rule for this repo).

- [x] **Step 8: Commit**

```bash
git add crates/minos-backend/src/http/auth.rs \
        crates/minos-backend/src/http/mod.rs \
        crates/minos-backend/src/http/ws_devices.rs \
        crates/minos-backend/Cargo.toml \
        crates/minos-backend/tests/auth_helper.rs
git commit -m "refactor(backend): extract shared http::auth from ws_devices"
```

### Task A2: Wire `/v1` router nest (empty)

**Why:** Stand up the routing skeleton so handler tasks just add routes.

**Files:**
- Create: `crates/minos-backend/src/http/v1/mod.rs`
- Modify: `crates/minos-backend/src/http/mod.rs`

- [x] **Step 1: Write the failing test**

In `crates/minos-backend/tests/v1_routing.rs`:

```rust
use axum::http::{Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};

mod common;

#[tokio::test]
async fn unknown_v1_route_returns_404() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/no-such-route")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
```

Place the helper from the pre-flight section in `crates/minos-backend/tests/common.rs`:

```rust
#![allow(dead_code)]

pub async fn send(
    app: &mut axum::Router,
    req: axum::http::Request<axum::body::Body>,
) -> (axum::http::StatusCode, serde_json::Value) {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, body)
}
```

Also add to `crates/minos-backend/Cargo.toml`:

```toml
[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

- [x] **Step 2: Run — must fail**

```bash
cargo test -p minos-backend --test v1_routing
```

Expected: compile error or `404` reaches an empty router (status 404 may pass without `/v1` — that's fine, we still need the v1 module to exist for later tasks). If the test passes here, that's coincidence; the next steps still must run.

- [x] **Step 3: Create the v1 module skeleton**

`crates/minos-backend/src/http/v1/mod.rs`:

```rust
//! Versioned `/v1` HTTP routes.
//!
//! Resource layout:
//! - `POST   /v1/pairing/tokens`     — agent-host mints a pairing token (replaces WS RequestPairingQr)
//! - `POST   /v1/pairing/consume`    — ios-client redeems a pairing token (replaces WS Pair)
//! - `DELETE /v1/pairing`            — paired device tears down the pairing (replaces WS ForgetPeer)
//! - `GET    /v1/threads`            — paired device lists threads (replaces WS ListThreads)
//! - `GET    /v1/threads/{thread_id}/events`   — read window of UI events (replaces WS ReadThread)
//! - `GET    /v1/threads/{thread_id}/last_seq` — host helper (replaces WS GetThreadLastSeq)
//!
//! All routes share the auth model defined in [`crate::http::auth`].

use axum::Router;

use super::BackendState;

pub mod pairing;
pub mod threads;

pub fn router() -> Router<BackendState> {
    Router::new()
        .merge(pairing::router())
        .merge(threads::router())
}
```

Stub modules:

`crates/minos-backend/src/http/v1/pairing.rs`:

```rust
use axum::Router;
use super::super::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
}
```

`crates/minos-backend/src/http/v1/threads.rs`:

```rust
use axum::Router;
use super::super::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
}
```

In `crates/minos-backend/src/http/mod.rs`, change `pub fn router(state: BackendState) -> Router` body to:

```rust
pub fn router(state: BackendState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health::get))
        .route("/devices", axum::routing::get(ws_devices::upgrade))
        .nest("/v1", v1::router())
        .with_state(state)
}
```

And add `pub mod v1;` to `crates/minos-backend/src/http/mod.rs`.

- [x] **Step 4: Run — must pass**

```bash
cargo test -p minos-backend --test v1_routing
```

Expected: PASS.

- [x] **Step 5: Workspace acceptance**

```bash
cargo xtask check-all
```

- [x] **Step 6: Commit**

```bash
git add crates/minos-backend/src/http/v1/ \
        crates/minos-backend/src/http/mod.rs \
        crates/minos-backend/Cargo.toml \
        crates/minos-backend/tests/v1_routing.rs \
        crates/minos-backend/tests/common.rs
git commit -m "feat(backend): scaffold /v1 axum nest"
```

### Task A3: `POST /v1/pairing/tokens`

**Why:** First real `/v1` handler. Replicates `LocalRpcMethod::RequestPairingQr` semantics over HTTP.

**Files:**
- Modify: `crates/minos-backend/src/http/v1/pairing.rs`
- Test: `crates/minos-backend/tests/v1_pairing.rs`

**Wire contract:**

```text
POST /v1/pairing/tokens
Headers: X-Device-Id, X-Device-Role: agent-host, optional X-Device-Secret, optional X-Device-Name
Body:    { "host_display_name": "string" }
200:     { "qr_payload": { "v": 2, "backend_url": "...", "host_display_name": "...",
                            "pairing_token": "...", "expires_at_ms": 0,
                            "cf_access_client_id"?: "...", "cf_access_client_secret"?: "..." } }
401:     { "error": { "code": "unauthorized", "message": "..." } }
500:     { "error": { "code": "internal", "message": "..." } }
```

Body schema is exactly `minos_protocol::RequestPairingQrParams`; success body is exactly `minos_protocol::RequestPairingQrResponse` — reuse the types so wire compatibility is automatic.

- [x] **Step 1: Write the failing happy-path test**

`crates/minos-backend/tests/v1_pairing.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_domain::DeviceId;
use minos_protocol::RequestPairingQrResponse;

mod common;

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

#[tokio::test]
async fn post_pairing_tokens_mints_qr_payload_for_agent_host() {
    let state = backend_state().await;
    let mut app = router(state);
    let device_id = DeviceId::new();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header("x-device-id", device_id.to_string())
        .header("x-device-role", "agent-host")
        .header("x-device-name", "Mac")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::json!({ "host_display_name": "Fan's Mac" })))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: RequestPairingQrResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.qr_payload.v, 2);
    assert_eq!(resp.qr_payload.host_display_name, "Fan's Mac");
    assert!(!resp.qr_payload.pairing_token.is_empty());
    assert!(resp.qr_payload.expires_at_ms > 0);
}

#[tokio::test]
async fn post_pairing_tokens_rejects_ios_client() {
    let state = backend_state().await;
    let mut app = router(state);
    let device_id = DeviceId::new();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header("x-device-id", device_id.to_string())
        .header("x-device-role", "ios-client")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::json!({ "host_display_name": "x" })))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn post_pairing_tokens_rejects_missing_device_id() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/tokens")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::json!({ "host_display_name": "x" })))
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
```

- [x] **Step 2: Run — must fail**

```bash
cargo test -p minos-backend --test v1_pairing post_pairing_tokens
```

Expected: compile errors (the `pairing` module is empty) or 404 for `/v1/pairing/tokens`.

- [x] **Step 3: Implement the handler**

`crates/minos-backend/src/http/v1/pairing.rs`:

```rust
//! `POST /v1/pairing/*` and `DELETE /v1/pairing` handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, post};
use axum::{Json, Router};
use minos_domain::DeviceRole;
use minos_protocol::{
    PairingQrPayload, RequestPairingQrParams, RequestPairingQrResponse,
};
use serde::Serialize;

use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new().route("/pairing/tokens", post(post_tokens))
    // pairing/consume + DELETE /pairing added in Tasks B1/B2
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

fn err_body(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope { error: ErrorBody { code, message: message.into() } })
}

async fn post_tokens(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<RequestPairingQrParams>,
) -> Result<Json<RequestPairingQrResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate_role(&state.store, &headers, DeviceRole::AgentHost)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err_body("unauthorized", m)),
            auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m)),
        })?;

    let (token, expires) = state
        .pairing
        .request_token(outcome.device_id, state.token_ttl)
        .await
        .map_err(|e| {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "request_token failed");
            (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", e.to_string()))
        })?;

    let qr_payload = PairingQrPayload {
        v: 2,
        backend_url: state.public_cfg.public_url.clone(),
        host_display_name: params.host_display_name,
        pairing_token: token.as_str().to_string(),
        expires_at_ms: expires.timestamp_millis(),
        cf_access_client_id: state.public_cfg.cf_access_client_id.clone(),
        cf_access_client_secret: state.public_cfg.cf_access_client_secret.clone(),
    };
    Ok(Json(RequestPairingQrResponse { qr_payload }))
}
```

- [x] **Step 4: Run — must pass**

```bash
cargo test -p minos-backend --test v1_pairing post_pairing_tokens
```

Expected: 3 tests PASS.

- [x] **Step 5: Workspace acceptance**

```bash
cargo xtask check-all
```

- [x] **Step 6: Commit**

```bash
git add crates/minos-backend/src/http/v1/pairing.rs \
        crates/minos-backend/tests/v1_pairing.rs
git commit -m "feat(backend): POST /v1/pairing/tokens"
```

---

## Phase B — Pairing HTTP routes & client switchover

After this phase, both daemon and mobile use HTTP for pairing. The WS `LocalRpcMethod::{RequestPairingQr, Pair, ForgetPeer}` handlers still exist (deleted in Phase D).

### Task B1: `POST /v1/pairing/consume`

**Why:** Replaces `LocalRpcMethod::Pair`. Lets the iPhone redeem the pairing token over HTTP — *before* opening the WebSocket — so the iPhone's WS handshake can carry the freshly-issued `X-Device-Secret` from the start.

**Wire contract:**

```text
POST /v1/pairing/consume
Headers: X-Device-Id, X-Device-Role: ios-client, optional X-Device-Name
        (NO X-Device-Secret on first pair; existing rows must NOT have a hash)
Body:    { "token": "string", "device_name": "string" }
200:     { "peer_device_id": "...", "peer_name": "...", "your_device_secret": "..." }
401:     { "error": { "code": "unauthorized", ... } }
409:     { "error": { "code": "pairing_state_mismatch" | "pairing_token_invalid", ... } }
500:     { "error": { "code": "internal", ... } }
```

The handler also must push `Event::Paired` to the issuer (Mac) over its live WS — same compensation logic as the existing `handle_pair`.

**Files:**
- Modify: `crates/minos-backend/src/http/v1/pairing.rs`
- Modify: `crates/minos-backend/tests/v1_pairing.rs`
- Add type: `crates/minos-protocol/src/messages.rs` — a `PairConsumeRequest` and reuse-by-extension of `PairResponse`.

- [x] **Step 1: Add request/response types in `minos-protocol`**

In `crates/minos-protocol/src/messages.rs`, append:

```rust
/// Request body for `POST /v1/pairing/consume`. Distinct from
/// [`PairRequest`] because the HTTP route derives `device_id` from the
/// `X-Device-Id` header, not the body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairConsumeRequest {
    pub token: PairingToken,
    pub device_name: String,
}
```

Re-export from `crates/minos-protocol/src/lib.rs`:

```rust
pub use messages::PairConsumeRequest;
```

`PairResponse` is already defined and is the right success shape.

- [x] **Step 2: Write the failing tests**

Append to `crates/minos-backend/tests/v1_pairing.rs`:

```rust
use minos_backend::pairing::PairingService;
use minos_backend::store::devices::insert_device;
use minos_domain::{DeviceRole, PairingToken};
use minos_protocol::{PairConsumeRequest, PairResponse};
use std::time::Duration as StdDuration;

#[tokio::test]
async fn post_pairing_consume_happy_path_returns_secret_and_pairs() {
    let state = backend_state().await;

    // Pre-seed a Mac issuer + token (mirrors what /v1/pairing/tokens does).
    let mac_id = DeviceId::new();
    insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    let svc = PairingService::new(state.store.clone());
    let (token, _expires) = svc.request_token(mac_id, StdDuration::from_secs(300)).await.unwrap();

    let mut app = router(state.clone());
    let consumer_id = DeviceId::new();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", consumer_id.to_string())
        .header("x-device-role", "ios-client")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::to_value(PairConsumeRequest {
            token: token.clone(),
            device_name: "iPhone".into(),
        }).unwrap()))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);

    let resp: PairResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");
    assert_eq!(resp.your_device_secret.as_str().len(), 43);

    // Pairing committed
    let pair = minos_backend::store::pairings::get_pair(&state.store, mac_id).await.unwrap();
    assert_eq!(pair, Some(consumer_id));
}

#[tokio::test]
async fn post_pairing_consume_invalid_token_returns_409() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", DeviceId::new().to_string())
        .header("x-device-role", "ios-client")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::to_value(PairConsumeRequest {
            token: PairingToken::generate(),
            device_name: "iPhone".into(),
        }).unwrap()))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "pairing_token_invalid");
}

#[tokio::test]
async fn post_pairing_consume_rejects_agent_host_role() {
    let state = backend_state().await;
    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/consume")
        .header("x-device-id", DeviceId::new().to_string())
        .header("x-device-role", "agent-host")
        .header(header::CONTENT_TYPE, "application/json")
        .body(json_body(serde_json::to_value(PairConsumeRequest {
            token: PairingToken::generate(),
            device_name: "iPhone".into(),
        }).unwrap()))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
```

- [x] **Step 3: Run — must fail**

```bash
cargo test -p minos-backend --test v1_pairing post_pairing_consume
```

Expected: 404 / not found.

- [x] **Step 4: Implement the handler**

In `crates/minos-backend/src/http/v1/pairing.rs`:

Add `consume` to the router and implement:

```rust
pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/pairing/tokens", post(post_tokens))
        .route("/pairing/consume", post(post_consume))
}

async fn post_consume(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(params): Json<minos_protocol::PairConsumeRequest>,
) -> Result<Json<minos_protocol::PairResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    use crate::error::BackendError;
    use minos_protocol::{Envelope, EventKind};

    let outcome = auth::authenticate_role(&state.store, &headers, DeviceRole::IosClient)
        .await
        .map_err(|e| match e {
            auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err_body("unauthorized", m)),
            auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m)),
        })?;
    let consumer_id = outcome.device_id;

    let pairing_outcome = match state
        .pairing
        .consume_token(&params.token, consumer_id, params.device_name.clone())
        .await
    {
        Ok(o) => o,
        Err(BackendError::PairingTokenInvalid) => {
            return Err((StatusCode::CONFLICT, err_body("pairing_token_invalid", "pairing token is unknown, expired, or already consumed")));
        }
        Err(BackendError::PairingStateMismatch { actual }) => {
            let msg = if actual == "self" { "device cannot pair with itself".to_string() } else { format!("peer already paired (state: {actual})") };
            return Err((StatusCode::CONFLICT, err_body("pairing_state_mismatch", msg)));
        }
        Err(e) => {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "consume_token failed");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", e.to_string())));
        }
    };

    let issuer_id = pairing_outcome.issuer_device_id;

    // Push Event::Paired to the issuer's live WS, if any. If issuer is offline
    // OR the queue rejects, compensate (clear the pairing) — same as
    // envelope::local_rpc::handle_pair.
    let issuer_handle = state.registry.get(issuer_id);
    let consumer_secret_str = pairing_outcome.consumer_secret.as_str().to_string();

    let mac_name = match crate::store::devices::get_device(&state.store, issuer_id).await {
        Ok(Some(row)) => row.display_name,
        _ => "Mac".to_string(),
    };

    if let Some(issuer_handle) = issuer_handle {
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: consumer_id,
                peer_name: params.device_name.clone(),
                your_device_secret: pairing_outcome.issuer_secret.clone(),
            },
        };
        *issuer_handle.paired_with.write().await = Some(consumer_id);
        if let Err(e) = state.registry.try_send_current(&issuer_handle, frame) {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, issuer = %issuer_id, "Event::Paired delivery failed; compensating");
            *issuer_handle.paired_with.write().await = None;
            let _ = state.pairing.forget_pair(consumer_id).await;
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", "failed to deliver pairing secret to issuer; pairing rolled back")));
        }
    } else {
        tracing::warn!(target: "minos_backend::v1::pairing", issuer = %issuer_id, "issuer offline at pair time; compensating");
        let _ = state.pairing.forget_pair(consumer_id).await;
        return Err((StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", "issuer is offline; pairing rolled back")));
    }

    Ok(Json(minos_protocol::PairResponse {
        peer_device_id: issuer_id,
        peer_name: mac_name,
        your_device_secret: minos_domain::DeviceSecret(consumer_secret_str),
    }))
}
```

- [x] **Step 5: Run — must pass**

```bash
cargo test -p minos-backend --test v1_pairing
```

The "happy path" test seeds the issuer device row directly and skips the `Event::Paired` delivery (no live WS), which means the handler will hit the "issuer offline → compensate" branch. Adjust the happy-path test to spin up an actual session OR seed the registry with a `SessionHandle`. **Use the registry-seed approach** — it keeps the test self-contained:

Add a `seed_live_session` helper at the top of `v1_pairing.rs`:

```rust
fn seed_live_session(
    state: &minos_backend::http::BackendState,
    device_id: DeviceId,
    role: DeviceRole,
) -> tokio::sync::mpsc::Receiver<minos_protocol::Envelope> {
    use minos_backend::session::SessionHandle;
    let (handle, outbox_rx) = SessionHandle::new(device_id, role);
    state.registry.insert(handle);
    outbox_rx
}
```

Update `post_pairing_consume_happy_path_returns_secret_and_pairs` to call `seed_live_session(&state, mac_id, DeviceRole::AgentHost)` before sending the request, and after asserting the response, also assert the session received `Event::Paired`:

```rust
    let mut outbox = seed_live_session(&state, mac_id, DeviceRole::AgentHost);
    // ... send the request, assert response ...
    let frame = outbox.recv().await.expect("issuer receives Event::Paired");
    match frame {
        minos_protocol::Envelope::Event { event: minos_protocol::EventKind::Paired { peer_device_id, .. }, .. } => {
            assert_eq!(peer_device_id, consumer_id);
        }
        other => panic!("expected Event::Paired, got {other:?}"),
    }
```

Re-run; expected: PASS.

If `SessionHandle::new` is private to the `session` module, mark it `pub` (or expose a `pub(crate)` constructor — check the current visibility in `crates/minos-backend/src/session/mod.rs` and tighten as needed). The same struct is already used by `ws_devices.rs`, so visibility tweaks should be local.

- [x] **Step 6: Workspace acceptance**

```bash
cargo xtask check-all
```

- [x] **Step 7: Commit**

```bash
git add crates/minos-backend/src/http/v1/pairing.rs \
        crates/minos-backend/tests/v1_pairing.rs \
        crates/minos-protocol/src/messages.rs \
        crates/minos-protocol/src/lib.rs
git commit -m "feat(backend): POST /v1/pairing/consume"
```

### Task B2: `DELETE /v1/pairing`

**Why:** Replaces `LocalRpcMethod::ForgetPeer`. Idempotent teardown.

**Wire contract:**

```text
DELETE /v1/pairing
Headers: X-Device-Id, X-Device-Role, X-Device-Secret (required, must verify)
204:     no body — pair existed and was torn down
404:     { "error": { "code": "pairing_state_mismatch", "message": "..." } } — was unpaired
```

We choose 204 over 200 because the response body conveys nothing the client doesn't already know; the server-side state is what matters. Keeping the same JSON-error body for the 404 case so error parsing is uniform.

- [x] **Step 1: Tests**

Append to `crates/minos-backend/tests/v1_pairing.rs`:

```rust
#[tokio::test]
async fn delete_pairing_tears_down_and_pushes_unpaired() {
    let state = backend_state().await;
    // Seed a paired Mac+iPhone with a verifying secret on the iPhone side.
    let mac_id = DeviceId::new();
    let ios_id = DeviceId::new();
    insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0).await.unwrap();
    insert_device(&state.store, ios_id, "iPhone", DeviceRole::IosClient, 0).await.unwrap();

    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, ios_id, &hash).await.unwrap();
    minos_backend::store::pairings::insert_pairing(&state.store, mac_id, ios_id, 0).await.unwrap();

    let mut mac_outbox = seed_live_session(&state, mac_id, DeviceRole::AgentHost);

    let mut app = router(state.clone());
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/v1/pairing")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(minos_backend::store::pairings::get_pair(&state.store, mac_id).await.unwrap(), None);

    // Mac receives Event::Unpaired
    let frame = mac_outbox.recv().await.unwrap();
    assert!(matches!(frame, minos_protocol::Envelope::Event { event: minos_protocol::EventKind::Unpaired, .. }));
}

#[tokio::test]
async fn delete_pairing_when_unpaired_returns_404() {
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0).await.unwrap();
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, id, &hash).await.unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/v1/pairing")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "pairing_state_mismatch");
}
```

- [x] **Step 2: Run — must fail**

```bash
cargo test -p minos-backend --test v1_pairing delete_pairing
```

- [x] **Step 3: Implement**

In `crates/minos-backend/src/http/v1/pairing.rs`:

```rust
pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/pairing/tokens", post(post_tokens))
        .route("/pairing/consume", post(post_consume))
        .route("/pairing", delete(delete_pairing))
}

async fn delete_pairing(
    State(state): State<BackendState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<ErrorEnvelope>)> {
    use minos_protocol::{Envelope, EventKind};

    let outcome = auth::authenticate(&state.store, &headers).await.map_err(|e| match e {
        auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err_body("unauthorized", m)),
        auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", m)),
    })?;
    if !outcome.authenticated_with_secret {
        return Err((StatusCode::UNAUTHORIZED, err_body("unauthorized", "X-Device-Secret required for forget")));
    }

    let peer = match state.pairing.forget_pair(outcome.device_id).await {
        Ok(Some(peer)) => peer,
        Ok(None) => {
            return Err((StatusCode::NOT_FOUND, err_body("pairing_state_mismatch", "session is not paired; nothing to forget")));
        }
        Err(e) => {
            tracing::warn!(target: "minos_backend::v1::pairing", error = %e, "forget_pair failed");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err_body("internal", e.to_string())));
        }
    };

    let unpaired = Envelope::Event { version: 1, event: EventKind::Unpaired };

    // Caller's own live session (if any).
    if let Some(self_handle) = state.registry.get(outcome.device_id) {
        *self_handle.paired_with.write().await = None;
        let _ = state.registry.try_send_current(&self_handle, unpaired.clone());
    }
    // Peer's live session (if any).
    if let Some(peer_handle) = state.registry.get(peer) {
        *peer_handle.paired_with.write().await = None;
        let _ = state.registry.try_send_current(&peer_handle, unpaired);
    }

    Ok(StatusCode::NO_CONTENT)
}
```

- [x] **Step 4: Run — must pass**

```bash
cargo test -p minos-backend --test v1_pairing
```

- [x] **Step 5: Workspace acceptance + commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/v1/pairing.rs \
        crates/minos-backend/tests/v1_pairing.rs
git commit -m "feat(backend): DELETE /v1/pairing"
```

### Task B3: Daemon switches `request_pairing_token` and `forget_peer` to HTTP

**Why:** Daemon stops sending `LocalRpcMethod::{RequestPairingQr, ForgetPeer}` over WS; uses `reqwest` against the new `/v1` routes.

**Files:**
- Modify: `Cargo.toml` (workspace) — add `reqwest` to `[workspace.dependencies]`.
- Modify: `crates/minos-daemon/Cargo.toml` — depend on `reqwest`.
- Create: `crates/minos-daemon/src/relay_http.rs`.
- Modify: `crates/minos-daemon/src/relay_client.rs` — replace bodies of `request_pairing_token` (line 256) and `forget_peer` (line 289). Remove the `mac_name` argument plumbing if it's no longer needed by the WS dispatcher.

- [ ] **Step 1: Add the workspace dep**

In root `Cargo.toml`, under `[workspace.dependencies]`, after `tokio-tungstenite`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

In `crates/minos-daemon/Cargo.toml`, add to `[dependencies]`:

```toml
reqwest = { workspace = true }
```

- [ ] **Step 2: Write the failing test for the HTTP client wrapper**

`crates/minos-daemon/src/relay_http.rs` will own the wrapper. Add a unit test for URL derivation (the only pure piece — actual HTTP round-trips are exercised by integration tests next phase).

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_http_base_from_ws_url() {
        assert_eq!(http_base("ws://127.0.0.1:8787/devices").unwrap(), "http://127.0.0.1:8787");
        assert_eq!(http_base("wss://example.com/devices").unwrap(), "https://example.com");
        assert_eq!(http_base("wss://example.com:443/devices").unwrap(), "https://example.com:443");
    }
}
```

- [ ] **Step 3: Run — must fail**

```bash
cargo test -p minos-daemon relay_http
```

Expected: module not found.

- [ ] **Step 4: Implement `relay_http.rs`**

```rust
//! HTTP client for the backend's `/v1/*` control plane.
//!
//! Built on `reqwest`. Stamps the same `X-Device-*` and CF-Access
//! headers as the WS client.

use std::time::Duration;

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    PairingQrPayload, RequestPairingQrParams, RequestPairingQrResponse,
};
use reqwest::Client;
use serde::Deserialize;

use crate::config::RelayConfig;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

pub struct RelayHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_role: &'static str,
    device_name: String,
    config: RelayConfig,
}

impl RelayHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        device_name: String,
        config: RelayConfig,
    ) -> Result<Self, MinosError> {
        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| MinosError::BackendInternal { message: format!("reqwest build: {e}") })?;
        Ok(Self {
            client,
            base,
            device_id,
            device_role: "agent-host",
            device_name,
            config,
        })
    }

    pub async fn request_pairing_qr(
        &self,
        host_display_name: String,
    ) -> Result<PairingQrPayload, MinosError> {
        let url = format!("{}/v1/pairing/tokens", self.base);
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-name", &self.device_name);
        let req = stamp_cf(req, &self.config);
        let resp = req
            .json(&RequestPairingQrParams { host_display_name })
            .send()
            .await
            .map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status.is_success() {
            let body: RequestPairingQrResponse = resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode RequestPairingQrResponse: {e}"),
            })?;
            Ok(body.qr_payload)
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let req = self
            .client
            .delete(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let req = stamp_cf(req, &self.config);
        let resp = req.send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else if status == reqwest::StatusCode::NOT_FOUND {
            // Idempotent: nothing to forget is fine.
            Ok(())
        } else {
            Err(decode_error(status, resp).await)
        }
    }
}

fn stamp_cf(req: reqwest::RequestBuilder, cfg: &RelayConfig) -> reqwest::RequestBuilder {
    let mut req = req;
    if let Some(id) = cfg.cf_access_client_id.as_deref() {
        req = req.header("cf-access-client-id", id);
    }
    if let Some(sec) = cfg.cf_access_client_secret.as_deref() {
        req = req.header("cf-access-client-secret", sec);
    }
    req
}

fn connect_err(url: &str, e: reqwest::Error) -> MinosError {
    if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
        MinosError::CfAuthFailed { url: url.into(), message: e.to_string() }
    } else {
        MinosError::ConnectFailed { url: url.into(), message: e.to_string() }
    }
}

async fn decode_error(status: reqwest::StatusCode, resp: reqwest::Response) -> MinosError {
    let body: Result<ErrorEnvelope, _> = resp.json().await;
    match body {
        Ok(env) => MinosError::BackendInternal {
            message: format!("backend {} ({}): {}", status, env.error.code, env.error.message),
        },
        Err(_) => MinosError::BackendInternal { message: format!("backend {}", status) },
    }
}

pub(crate) fn http_base(ws_url: &str) -> Option<String> {
    let url = url::Url::parse(ws_url).ok()?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => other,
    };
    let host = url.host_str()?;
    let port = url
        .port()
        .map(|p| format!(":{p}"))
        .unwrap_or_default();
    Some(format!("{scheme}://{host}{port}"))
}
```

Add `pub mod relay_http;` to `crates/minos-daemon/src/lib.rs`. Add `url` to dependencies if not already present.

- [ ] **Step 5: Wire into `RelayClient`**

In `crates/minos-daemon/src/relay_client.rs`, add an `http: Arc<RelayHttpClient>` field to `Inner`, populate it in `RelayClient::spawn`, and rewrite the two methods:

```rust
pub async fn request_pairing_token(&self) -> Result<RelayQrPayload, MinosError> {
    let qr = self
        .inner
        .http
        .request_pairing_qr(self.inner.mac_name.clone())
        .await?;
    let (cf_access_client_id, cf_access_client_secret) = qr_cf_access_or_host_env(
        qr.cf_access_client_id,
        qr.cf_access_client_secret,
        &self.inner.config,
    );
    Ok(RelayQrPayload {
        v: qr.v,
        backend_url: qr.backend_url,
        host_display_name: qr.host_display_name,
        pairing_token: minos_domain::PairingToken(qr.pairing_token),
        expires_at_ms: qr.expires_at_ms,
        cf_access_client_id,
        cf_access_client_secret,
    })
}

pub async fn forget_peer(&self) -> Result<(), MinosError> {
    let secret = self.inner.secret.clone().ok_or_else(|| MinosError::DeviceNotTrusted {
        message: "no device secret persisted; cannot forget pairing".into(),
    })?;
    self.inner.http.forget_pairing(&secret).await
}
```

The daemon currently stores `secret: Option<DeviceSecret>` on `DispatchCtx`. Move/duplicate it into `Inner` so `forget_peer` can access it without going through the dispatch task — it's a `Clone`, so this is cheap.

- [ ] **Step 6: Run — must pass**

```bash
cargo test -p minos-daemon
```

The existing `relay_client_smoke` integration tests that exercise pairing-flow happy paths must still pass — they assert via `RelayClient::request_pairing_token` and `RelayClient::forget_peer`, both of which now route through HTTP. If those tests use a fake WS-only backend, point them at the real `minos-backend` test harness (or extend the fake to serve the two HTTP routes too — see the existing test at `crates/minos-daemon/tests/relay_client_smoke.rs` and adjust).

- [ ] **Step 7: Workspace acceptance**

```bash
cargo xtask check-all
```

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml \
        crates/minos-daemon/Cargo.toml \
        crates/minos-daemon/src/relay_http.rs \
        crates/minos-daemon/src/lib.rs \
        crates/minos-daemon/src/relay_client.rs \
        crates/minos-daemon/tests/
git commit -m "feat(daemon): use HTTP for pairing token + forget"
```

### Task B4: Mobile switches `pair_with_qr_json` and `forget_peer` to HTTP

**Why:** Mobile stops opening a secret-less WebSocket purely to send `LocalRpcMethod::Pair`. New flow:

1. Parse QR
2. Save backend URL + CF tokens to store
3. **POST `/v1/pairing/consume`** — get `{peer_device_id, peer_name, your_device_secret}`
4. Save the device secret
5. Open the WebSocket *with* the secret → `Connected`

**Files:**
- Modify: `crates/minos-mobile/Cargo.toml` — depend on `reqwest`.
- Create: `crates/minos-mobile/src/http.rs` — HTTP client wrapper.
- Modify: `crates/minos-mobile/src/client.rs` — replace bodies of `pair_with_qr_json` and `forget_peer`. Make `connect` always require a secret (delete the `Option<&str>` parameter for `device_secret`).

- [ ] **Step 1: Add reqwest**

In `crates/minos-mobile/Cargo.toml`:

```toml
reqwest = { workspace = true }
```

- [ ] **Step 2: Tests for the HTTP wrapper**

In `crates/minos-mobile/src/http.rs`, define the wrapper. Tests stay in `crates/minos-mobile/tests/http_smoke.rs`:

```rust
use minos_backend::http::{router, test_support::backend_state};
use minos_domain::{DeviceId, DeviceRole};
use minos_mobile::http::MobileHttpClient;
use minos_protocol::PairConsumeRequest;

#[tokio::test]
async fn pair_consume_round_trips_against_real_backend() {
    let state = backend_state().await;
    let mac_id = DeviceId::new();
    minos_backend::store::devices::insert_device(&state.store, mac_id, "Mac", DeviceRole::AgentHost, 0).await.unwrap();
    let svc = minos_backend::pairing::PairingService::new(state.store.clone());
    let (token, _) = svc.request_token(mac_id, std::time::Duration::from_secs(300)).await.unwrap();

    // Seed a live Mac session so consume can deliver Event::Paired.
    let (handle, mut mac_outbox) = minos_backend::session::SessionHandle::new(mac_id, DeviceRole::AgentHost);
    state.registry.insert(handle);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let consumer_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), consumer_id, None).unwrap();
    let resp = client.pair_consume(PairConsumeRequest {
        token,
        device_name: "iPhone".into(),
    }).await.unwrap();

    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");
    assert_eq!(resp.your_device_secret.as_str().len(), 43);
    let _ = mac_outbox.recv().await.unwrap(); // Event::Paired delivered
}
```

Add to `crates/minos-mobile/Cargo.toml` `[dev-dependencies]`:

```toml
minos-backend = { path = "../minos-backend", features = ["test-support"] }
```

- [ ] **Step 3: Run — must fail**

```bash
cargo test -p minos-mobile --test http_smoke
```

Expected: module not found.

- [ ] **Step 4: Implement `crates/minos-mobile/src/http.rs`**

```rust
//! HTTP client for the backend's `/v1/*` control plane.

use std::time::Duration;

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse, PairConsumeRequest,
    PairResponse, ReadThreadParams, ReadThreadResponse,
};
use reqwest::Client;
use serde::Deserialize;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

pub struct MobileHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_role: &'static str,
    cf_access: Option<(String, String)>,
}

impl MobileHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        cf_access: Option<(String, String)>,
    ) -> Result<Self, MinosError> {
        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        let client = Client::builder().timeout(HTTP_TIMEOUT).build()
            .map_err(|e| MinosError::BackendInternal { message: format!("reqwest build: {e}") })?;
        Ok(Self { client, base, device_id, device_role: "ios-client", cf_access })
    }

    pub async fn pair_consume(&self, req: PairConsumeRequest) -> Result<PairResponse, MinosError> {
        let url = format!("{}/v1/pairing/consume", self.base);
        let r = self.client.post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role);
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.json(&req).send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode PairResponse: {e}"),
            })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let r = self.client.delete(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT || status == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    // list_threads / read_thread / get_thread_last_seq added in Task C4.
}

fn stamp_cf(req: reqwest::RequestBuilder, cf: Option<&(String, String)>) -> reqwest::RequestBuilder {
    let mut req = req;
    if let Some((id, sec)) = cf {
        req = req.header("cf-access-client-id", id).header("cf-access-client-secret", sec);
    }
    req
}

fn connect_err(url: &str, e: reqwest::Error) -> MinosError {
    if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
        MinosError::CfAuthFailed { url: url.into(), message: e.to_string() }
    } else {
        MinosError::ConnectFailed { url: url.into(), message: e.to_string() }
    }
}

async fn decode_error(status: reqwest::StatusCode, resp: reqwest::Response) -> MinosError {
    let body: Result<ErrorEnvelope, _> = resp.json().await;
    match body {
        Ok(env) => MinosError::RpcCallFailed { method: format!("http {}", status), message: format!("{}: {}", env.error.code, env.error.message) },
        Err(_) => MinosError::BackendInternal { message: format!("backend {}", status) },
    }
}

pub(crate) fn http_base(ws_url: &str) -> Option<String> {
    let url = url::Url::parse(ws_url).ok()?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => other,
    };
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}"))
}
```

Add `pub mod http;` to `crates/minos-mobile/src/lib.rs`.

- [ ] **Step 5: Rewrite `MobileClient::pair_with_qr_json`**

In `crates/minos-mobile/src/client.rs`, replace the body of `pair_with_qr_json` (lines 234-287 in the current file):

```rust
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
    let cf = match (qr.cf_access_client_id.clone(), qr.cf_access_client_secret.clone()) {
        (Some(id), Some(sec)) => {
            self.store.save_cf_access(&id, &sec).await?;
            Some((id, sec))
        }
        _ => None,
    };

    let _ = self.state_tx.send(ConnectionState::Pairing);

    // Step 1: redeem the pairing token over HTTP. The backend records both
    // device-secret hashes and pushes Event::Paired to the Mac before
    // returning, so by the time we get the response the Mac is already
    // updated.
    let http = crate::http::MobileHttpClient::new(&qr.backend_url, self.device_id, cf.clone())?;
    let pair_resp = http.pair_consume(minos_protocol::PairConsumeRequest {
        token: minos_domain::PairingToken(qr.pairing_token),
        device_name: self.self_name.clone(),
    }).await?;

    let device_secret = pair_resp.your_device_secret.clone();
    self.store.save_device(&self.device_id, &device_secret).await?;

    // Step 2: now open the WS with the freshly-issued secret. From here on
    // every connect carries X-Device-Secret.
    self.connect(&qr.backend_url, device_secret.as_str(), cf).await?;

    let _ = self.state_tx.send(ConnectionState::Connected);
    Ok(())
}
```

Change the signature of `connect`:

```rust
async fn connect(
    &self,
    url: &str,
    device_secret: &str,
    cf_access: Option<(String, String)>,
) -> Result<(), MinosError> {
    // existing body, but the `if let Some(sec) = device_secret` branch
    // becomes unconditional.
}
```

Update `resume_persisted_session` (still has a secret in the store) and any other callers; the existing one passes `Some(device_secret.as_str())` → just unwrap to `&str`.

Replace `forget_peer`:

```rust
pub async fn forget_peer(&self) -> Result<(), MinosError> {
    let backend_url = self.store.load_backend_url().await?;
    let device = self.store.load_device().await?;
    let cf = self.store.load_cf_access().await?;

    if let (Some(url), Some((_, secret))) = (backend_url.as_deref(), device.as_ref()) {
        let http = crate::http::MobileHttpClient::new(url, self.device_id, cf)?;
        let _ = http.forget_pairing(secret).await; // best-effort
    }

    self.store.clear_all().await?;
    self.shutdown_outbound().await;
    let _ = self.state_tx.send(ConnectionState::Disconnected);
    Ok(())
}
```

- [ ] **Step 6: Run — must pass**

```bash
cargo test -p minos-mobile
cargo test -p minos-mobile --test http_smoke
```

Existing `pair_with_qr_json` tests likely seed a fake WS server; rewrite them to spin up `minos-backend`'s test router (matching the http_smoke pattern). The fake-peer binary at `crates/minos-mobile/src/bin/fake-peer.rs` continues to exercise the post-pair WS flow.

- [ ] **Step 7: Workspace acceptance + commit**

```bash
cargo xtask check-all
git add crates/minos-mobile/Cargo.toml \
        crates/minos-mobile/src/http.rs \
        crates/minos-mobile/src/lib.rs \
        crates/minos-mobile/src/client.rs \
        crates/minos-mobile/tests/
git commit -m "feat(mobile): use HTTP for pairing consume + forget"
```

---

## Phase C — Threads HTTP routes & mobile switchover

### Task C1: `GET /v1/threads`

**Wire contract:**

```text
GET /v1/threads?limit=50&before_ts_ms=1714000000000&agent=codex
Headers: X-Device-Id, X-Device-Role, X-Device-Secret (must verify, must be paired)
200:     ListThreadsResponse  (existing minos_protocol type)
401:     unauthorized | unauthorized (not paired → same code as today's local_rpc)
```

`agent` is optional; if omitted, all agents are returned. `before_ts_ms` is optional; if omitted, returns the newest page.

**Files:**
- Create: `crates/minos-backend/src/http/v1/threads.rs` (was a stub)
- Test: `crates/minos-backend/tests/v1_threads.rs`

- [ ] **Step 1: Test**

`crates/minos-backend/tests/v1_threads.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::pairing::PairingService;
use minos_backend::store::{devices::insert_device, pairings::insert_pairing};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::ListThreadsResponse;

mod common;

async fn paired_pair(state: &minos_backend::http::BackendState) -> (DeviceId, DeviceId, minos_domain::DeviceSecret) {
    let mac = DeviceId::new();
    let ios = DeviceId::new();
    insert_device(&state.store, mac, "Mac", DeviceRole::AgentHost, 0).await.unwrap();
    insert_device(&state.store, ios, "iPhone", DeviceRole::IosClient, 0).await.unwrap();

    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, ios, &hash).await.unwrap();
    insert_pairing(&state.store, mac, ios, 0).await.unwrap();
    (mac, ios, secret)
}

#[tokio::test]
async fn get_threads_returns_owner_scoped_list() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    // Seed two threads owned by the Mac.
    minos_backend::store::threads::upsert(&state.store, "thr_a", AgentName::Codex, &mac_id.to_string(), 100, 200, 1, None, None).await.unwrap();
    minos_backend::store::threads::upsert(&state.store, "thr_b", AgentName::Claude, &mac_id.to_string(), 300, 400, 2, None, None).await.unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=50")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: ListThreadsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.threads.len(), 2);
}

#[tokio::test]
async fn get_threads_unpaired_returns_401() {
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::IosClient, 0).await.unwrap();
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, id, &hash).await.unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=10")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}
```

- [ ] **Step 2: Run — fail**

```bash
cargo test -p minos-backend --test v1_threads get_threads
```

- [ ] **Step 3: Implement**

`crates/minos-backend/src/http/v1/threads.rs`:

```rust
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use minos_protocol::{
    GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse, ReadThreadParams,
    ReadThreadResponse,
};
use serde::{Deserialize, Serialize};

use crate::http::auth;
use crate::http::BackendState;

pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/threads", get(list_threads))
        .route("/threads/:thread_id/events", get(read_thread))
        .route("/threads/:thread_id/last_seq", get(get_thread_last_seq))
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope { error: ErrorBody }
#[derive(Debug, Serialize)]
struct ErrorBody { code: &'static str, message: String }

fn err(code: &'static str, message: impl Into<String>) -> Json<ErrorEnvelope> {
    Json(ErrorEnvelope { error: ErrorBody { code, message: message.into() } })
}

async fn require_paired_session(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<minos_domain::DeviceId, (StatusCode, Json<ErrorEnvelope>)> {
    let outcome = auth::authenticate(&state.store, headers).await.map_err(|e| match e {
        auth::AuthError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, err("unauthorized", m)),
        auth::AuthError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m)),
    })?;
    if !outcome.authenticated_with_secret {
        return Err((StatusCode::UNAUTHORIZED, err("unauthorized", "X-Device-Secret required")));
    }
    let owner = crate::store::pairings::get_pair(&state.store, outcome.device_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, err("unauthorized", "session is not paired")))?;
    Ok(owner)
}

#[derive(Debug, Deserialize)]
struct ListThreadsQuery {
    limit: u32,
    before_ts_ms: Option<i64>,
    agent: Option<minos_domain::AgentName>,
}

async fn list_threads(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Query(q): Query<ListThreadsQuery>,
) -> Result<Json<ListThreadsResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let owner = require_paired_session(&state, &headers).await?;
    let owner_s = Some(owner.to_string());
    let threads = crate::store::threads::list(&state.store, owner_s.as_deref(), q.agent, q.before_ts_ms, q.limit.min(500))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?;
    let next_before_ts_ms = threads.last().map(|t| t.last_ts_ms);
    Ok(Json(ListThreadsResponse { threads, next_before_ts_ms }))
}

#[derive(Debug, Deserialize)]
struct ReadThreadQuery {
    from_seq: Option<u64>,
    limit: u32,
}

async fn read_thread(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
    Query(q): Query<ReadThreadQuery>,
) -> Result<Json<ReadThreadResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let owner = require_paired_session(&state, &headers).await?;

    // The existing handler in envelope/local_rpc.rs:157 reused the bulk of
    // the read logic (owner probe, fresh translator state, end_reason
    // probe, pagination cursor). Move that body into a helper
    // `crate::ingest::history::read` so HTTP and WS share it. For this
    // task, the simplest change is to copy-extract the body verbatim into
    // such a helper and call it from here.
    let params = ReadThreadParams { thread_id: thread_id.clone(), from_seq: q.from_seq, limit: q.limit };
    let resp = crate::ingest::history::read_thread(&state, owner, params).await
        .map_err(|e| match e {
            crate::ingest::history::HistoryError::NotFound => (StatusCode::NOT_FOUND, err("thread_not_found", format!("thread not found: {thread_id}"))),
            crate::ingest::history::HistoryError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal", m)),
        })?;
    Ok(Json(resp))
}

#[derive(Debug, Deserialize)]
struct LastSeqQuery {}

async fn get_thread_last_seq(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
) -> Result<Json<GetThreadLastSeqResponse>, (StatusCode, Json<ErrorEnvelope>)> {
    let _ = require_paired_session(&state, &headers).await?;
    let last_seq = crate::store::raw_events::last_seq(&state.store, &thread_id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, err("internal", e.to_string())))?;
    Ok(Json(GetThreadLastSeqResponse { last_seq }))
}
```

The `read_thread` body needs the helper. **Inside this same task**, also create `crates/minos-backend/src/ingest/history.rs`:

```rust
//! Pure history-read helper extracted from envelope::local_rpc::handle_read_thread.
//! Used by HTTP `/v1/threads/{id}/events` and (during the migration window)
//! still by the WS `LocalRpcMethod::ReadThread` handler.

use minos_protocol::{ReadThreadParams, ReadThreadResponse};

use crate::http::BackendState;

pub enum HistoryError {
    NotFound,
    Internal(String),
}

pub async fn read_thread(
    state: &BackendState,
    owner_id: minos_domain::DeviceId,
    params: ReadThreadParams,
) -> Result<ReadThreadResponse, HistoryError> {
    // Move the body of handle_read_thread (envelope/local_rpc.rs:157-331)
    // here verbatim, replacing:
    //   - `ctx.store` → `&state.store`
    //   - `owner_id.to_string()` lookup unchanged
    //   - `err("...", "...")` returns → `HistoryError::{NotFound, Internal}`
    //   - tracing targets keep their existing strings
    todo!("paste in the existing body, lightly adapted")
}
```

Implement the body by copying from `envelope/local_rpc.rs:160-331` and adjusting the error returns. Replace the WS handler's body with a call to this helper too — that's a Phase-D-adjacent cleanup, but doing it now means the helper has two callers and must stay in sync. Keep the WS handler thin:

```rust
async fn handle_read_thread(ctx: &LocalRpcContext<'_>, params: &serde_json::Value) -> LocalRpcOutcome {
    let Some(owner_id) = *ctx.session.paired_with.read().await else {
        return err("unauthorized", "read_thread requires a paired session");
    };
    let p: minos_protocol::ReadThreadParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(e) => return err("bad_request", format!("invalid params: {e}")),
    };
    // We need a BackendState here. Pass one through the WS dispatch context
    // OR thread the individual fields the helper needs (store + translators
    // are the only ones; public_cfg is unused here). The WS dispatcher
    // currently threads them as separate args — adjust LocalRpcContext to
    // expose a synthesised BackendState handle. This is a one-shot edit.
    todo!("call crate::ingest::history::read_thread(...)");
}
```

Given the scope, the simpler path is: **keep the WS handler's existing body untouched in this task** (still calls store/translators directly). The HTTP handler gets the helper. They will diverge briefly, then Phase D deletes the WS handler entirely. Note this divergence in the commit message.

Add `pub mod history;` to `crates/minos-backend/src/ingest/mod.rs`.

- [ ] **Step 4: Run — pass**

```bash
cargo test -p minos-backend --test v1_threads
```

- [ ] **Step 5: Workspace acceptance + commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/http/v1/threads.rs \
        crates/minos-backend/src/ingest/history.rs \
        crates/minos-backend/src/ingest/mod.rs \
        crates/minos-backend/tests/v1_threads.rs
git commit -m "feat(backend): GET /v1/threads + /v1/threads/{id}/events,last_seq"
```

### Task C2: Skipped — folded into C1

(`/v1/threads/{id}/events` and `/v1/threads/{id}/last_seq` ship with C1 because the route file is small enough to land together. Phase boundary kept for tracking only.)

- [ ] **Step 1: confirm both routes are exercised by tests**

Add to `crates/minos-backend/tests/v1_threads.rs`:

```rust
#[tokio::test]
async fn get_thread_events_paginates() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    minos_backend::store::threads::upsert(&state.store, "thr_a", AgentName::Codex, &mac_id.to_string(), 100, 100, 0, None, None).await.unwrap();
    minos_backend::store::raw_events::insert(&state.store, "thr_a", AgentName::Codex, 1, &serde_json::json!({"method":"item/agentMessage/delta","params":{"delta":"Hi"}}), 100).await.unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads/thr_a/events?limit=10")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::ReadThreadResponse = serde_json::from_value(body).unwrap();
    assert!(!resp.ui_events.is_empty());
}

#[tokio::test]
async fn get_thread_last_seq_returns_max() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret) = paired_pair(&state).await;
    minos_backend::store::threads::upsert(&state.store, "thr_a", AgentName::Codex, &mac_id.to_string(), 100, 100, 0, None, None).await.unwrap();
    minos_backend::store::raw_events::insert(&state.store, "thr_a", AgentName::Codex, 7, &serde_json::json!({"method":"x"}), 100).await.unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads/thr_a/last_seq")
        .header("x-device-id", ios_id.to_string())
        .header("x-device-role", "ios-client")
        .header("x-device-secret", secret.as_str())
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.last_seq, 7);
}
```

- [ ] **Step 2: Run — pass**

```bash
cargo test -p minos-backend --test v1_threads
```

- [ ] **Step 3: Commit**

```bash
git add crates/minos-backend/tests/v1_threads.rs
git commit -m "test(backend): cover /v1/threads/{id}/{events,last_seq}"
```

### Task C3: Mobile switches thread reads to HTTP

- [ ] **Step 1: Extend `MobileHttpClient` with the three thread methods**

In `crates/minos-mobile/src/http.rs`:

```rust
impl MobileHttpClient {
    // ... existing ...

    pub async fn list_threads(&self, secret: &DeviceSecret, params: ListThreadsParams) -> Result<ListThreadsResponse, MinosError> {
        let mut url = format!("{}/v1/threads?limit={}", self.base, params.limit);
        if let Some(before) = params.before_ts_ms { url.push_str(&format!("&before_ts_ms={before}")); }
        if let Some(agent) = params.agent { url.push_str(&format!("&agent={}", serde_json::to_string(&agent).unwrap().trim_matches('"'))); }
        let r = self.client.get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal { message: format!("decode ListThreadsResponse: {e}") })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn read_thread(&self, secret: &DeviceSecret, params: ReadThreadParams) -> Result<ReadThreadResponse, MinosError> {
        let mut url = format!("{}/v1/threads/{}/events?limit={}", self.base, params.thread_id, params.limit);
        if let Some(from) = params.from_seq { url.push_str(&format!("&from_seq={from}")); }
        let r = self.client.get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal { message: format!("decode ReadThreadResponse: {e}") })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn get_thread_last_seq(&self, secret: &DeviceSecret, thread_id: &str) -> Result<GetThreadLastSeqResponse, MinosError> {
        let url = format!("{}/v1/threads/{}/last_seq", self.base, thread_id);
        let r = self.client.get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal { message: format!("decode GetThreadLastSeqResponse: {e}") })
        } else {
            Err(decode_error(status, resp).await)
        }
    }
}
```

- [ ] **Step 2: Switch `MobileClient::{list_threads, read_thread, get_thread_last_seq}`**

In `crates/minos-mobile/src/client.rs`, replace the bodies (lines 306-354):

```rust
pub async fn list_threads(&self, req: ListThreadsParams) -> Result<ListThreadsResponse, MinosError> {
    let (backend_url, secret, cf) = self.http_creds().await?;
    let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
    http.list_threads(&secret, req).await
}

pub async fn read_thread(&self, req: ReadThreadParams) -> Result<ReadThreadResponse, MinosError> {
    let (backend_url, secret, cf) = self.http_creds().await?;
    let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
    http.read_thread(&secret, req).await
}

pub async fn get_thread_last_seq(&self, req: GetThreadLastSeqParams) -> Result<GetThreadLastSeqResponse, MinosError> {
    let (backend_url, secret, cf) = self.http_creds().await?;
    let http = crate::http::MobileHttpClient::new(&backend_url, self.device_id, cf)?;
    http.get_thread_last_seq(&secret, &req.thread_id).await
}

async fn http_creds(&self) -> Result<(String, minos_domain::DeviceSecret, Option<(String, String)>), MinosError> {
    let backend_url = self.store.load_backend_url().await?.ok_or_else(|| MinosError::StoreCorrupt {
        path: "backend_url".into(), message: "missing backend_url".into() })?;
    let (_, secret) = self.store.load_device().await?.ok_or_else(|| MinosError::StoreCorrupt {
        path: "device".into(), message: "missing device secret".into() })?;
    let cf = self.store.load_cf_access().await?;
    Ok((backend_url, secret, cf))
}
```

- [ ] **Step 3: Run**

```bash
cargo test -p minos-mobile
```

Existing tests for `list_threads`/`read_thread`/`get_thread_last_seq` need to spin up an HTTP backend (same pattern as `http_smoke`). Update or replace.

- [ ] **Step 4: Workspace + commit**

```bash
cargo xtask check-all
git add crates/minos-mobile/src/http.rs crates/minos-mobile/src/client.rs crates/minos-mobile/tests/
git commit -m "feat(mobile): use HTTP for thread queries"
```

---

## Phase D — Cleanup (delete dead WS RPC)

After Phase C, no production caller hits the WS `LocalRpcMethod::*` handlers. Phase D deletes them and the surrounding plumbing.

### Task D1: Delete `LocalRpc` types from `minos-protocol`

**Files:**
- Modify: `crates/minos-protocol/src/envelope.rs`
- Modify: `crates/minos-protocol/src/lib.rs`
- Modify: `crates/minos-protocol/tests/envelope_golden.rs` — drop `local_rpc_*` fixtures

- [ ] **Step 1: Confirm there are no callers**

```bash
rg 'LocalRpcMethod|LocalRpcOutcome|RpcError|Envelope::LocalRpc' crates/
```

Should return only:
- `crates/minos-protocol/src/envelope.rs` (the definitions themselves)
- `crates/minos-backend/src/envelope/{mod,local_rpc}.rs` (deleted in D2)
- Test fixtures (deleted in D5)

If any production file still references them, go back and finish Phase B/C for that subsystem before continuing.

- [ ] **Step 2: Edit `crates/minos-protocol/src/envelope.rs`**

Remove the `LocalRpc { ... }` and `LocalRpcResponse { ... }` arms of `enum Envelope`. Remove `LocalRpcMethod`, `LocalRpcOutcome`, `RpcError` types. Keep `Envelope::{Forward, Forwarded, Event, Ingest}`. Update doc comments to drop references to local-RPC.

- [ ] **Step 3: Update re-exports**

In `crates/minos-protocol/src/lib.rs`, remove the lines that re-export the deleted types.

- [ ] **Step 4: Run**

```bash
cargo build -p minos-protocol
```

Compilation will surface any remaining import. Fix and re-run.

```bash
cargo test -p minos-protocol
```

- [ ] **Step 5: Update golden tests**

In `crates/minos-protocol/tests/envelope_golden.rs`, delete the `local_rpc_*` fixture files under `tests/golden/envelope/` (`local_rpc_ping.json`, `local_rpc_request_pairing_qr.json`, `local_rpc_pair.json`, `local_rpc_response_ok.json`, `local_rpc_response_err.json` — confirm via `ls tests/golden/envelope/`). Drop the corresponding `assert_eq!(...)` blocks in the test file.

```bash
cargo test -p minos-protocol --test envelope_golden
```

- [ ] **Step 6: Commit**

```bash
git add crates/minos-protocol/src/envelope.rs \
        crates/minos-protocol/src/lib.rs \
        crates/minos-protocol/tests/
git commit -m "refactor(protocol): remove LocalRpc envelope types"
```

### Task D2: Delete backend WS local-RPC dispatcher

**Files:**
- Delete: `crates/minos-backend/src/envelope/local_rpc.rs`
- Modify: `crates/minos-backend/src/envelope/mod.rs` — delete the `LocalRpc` arm in the dispatch loop, delete `pub mod local_rpc;`, delete unused imports

- [ ] **Step 1**

```bash
rm crates/minos-backend/src/envelope/local_rpc.rs
```

In `envelope/mod.rs`, find the `match envelope { ... }` block in `dispatch_envelope` (or wherever the `LocalRpc` arm currently lives) and delete that arm. Remove `pub mod local_rpc;` and any `use local_rpc::handle;` imports.

- [ ] **Step 2: Run**

```bash
cargo test -p minos-backend
```

The integration tests `tests/v1_pairing.rs` and `tests/v1_threads.rs` continue to cover the same behaviour through HTTP. WS-level integration tests for `LocalRpc{Ping,RequestPairingQr,Pair,ForgetPeer,ListThreads,ReadThread,GetThreadLastSeq}` are deleted with the dispatcher.

- [ ] **Step 3: Workspace + commit**

```bash
cargo xtask check-all
git add crates/minos-backend/src/envelope/
git commit -m "refactor(backend): remove WS LocalRpc dispatcher"
```

### Task D3: Delete daemon `send_local_rpc` and pending-map

**Files:**
- Modify: `crates/minos-daemon/src/relay_client.rs`

- [ ] **Step 1**

Delete:
- `Inner.next_id`, `Inner.pending`, `RelayClient::alloc_id`, `RelayClient::pending_map`, `RelayClient::send_local_rpc`
- The `LOCAL_RPC_TIMEOUT` constant
- The `Pending` type alias and `HashMap<u64, oneshot::Sender<...>>` machinery
- The dispatch-task branch that handles inbound `LocalRpcResponse` envelopes (look for `Envelope::LocalRpcResponse { id, outcome, .. } => { /* lookup pending, send */ }`)
- The `rpc_error_to_minos` helper if it's no longer referenced

Keep:
- `Inner.out_tx`, the dispatcher itself (it still relays `Forward`, `Forwarded`, `Ingest`, `Event`)
- Reconnect/backoff/auth logic — none of that depends on `LocalRpc`

- [ ] **Step 2: Run**

```bash
cargo test -p minos-daemon
cargo xtask check-all
```

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/src/relay_client.rs
git commit -m "refactor(daemon): drop send_local_rpc + pending-map"
```

### Task D4: Delete mobile `local_rpc` and pending DashMap

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1**

Delete:
- `MobileClient.next_rpc_id`, `MobileClient.pending` fields
- `MobileClient::local_rpc` method
- `LOCAL_RPC_TIMEOUT` constant
- The `recv_loop` branch that routes `LocalRpcResponse` into the pending map

Update `recv_loop`'s remaining match arms to handle only `Envelope::Event { event: EventKind::{Unpaired, ServerShutdown, UiEventMessage, Paired, PeerOnline, PeerOffline} }` and `Envelope::Forwarded`. (If `Forwarded` isn't routed today, leave that alone — out of scope.)

- [ ] **Step 2: Run**

```bash
cargo test -p minos-mobile
cargo xtask check-all
```

- [ ] **Step 3: Commit**

```bash
git add crates/minos-mobile/src/client.rs
git commit -m "refactor(mobile): drop local_rpc + pending DashMap"
```

### Task D5: Tidy + final acceptance

- [ ] **Step 1: Search for stragglers**

```bash
rg 'local_rpc|LocalRpc|LocalRpcMethod|LocalRpcOutcome' crates/ docs/superpowers/specs/
```

Anything that surfaces in `docs/superpowers/specs/*` is documentation; if a spec section explicitly defines the WS LocalRpc methods (e.g. spec §6.1 in `minos-relay-backend-design.md`), update it to reference `/v1/*` HTTP routes instead, since those are now the authoritative definition.

If any code still references the deleted symbols, go back to the appropriate task and finish.

- [ ] **Step 2: Run the full workspace test suite**

```bash
cargo xtask check-all
```

Per the user's standing instruction this is the gate before commit.

- [ ] **Step 3: Manual smoke**

Spin up the backend and pair end-to-end on real hardware (or the existing fake-peer + local backend in CI). Confirm:

1. Mac calls `POST /v1/pairing/tokens` → gets QR
2. iPhone scans → calls `POST /v1/pairing/consume` → gets `device_secret`
3. iPhone opens WS with `X-Device-Secret` → gets `Event::PeerOnline` for the Mac
4. Mac receives `Event::Paired` over its already-open WS
5. iPhone calls `GET /v1/threads` → gets list
6. iPhone calls `DELETE /v1/pairing` → both sockets see `Event::Unpaired`

Document any deviations in the commit message of this task.

- [ ] **Step 4: Commit any spec/doc updates**

```bash
git add docs/superpowers/specs/
git commit -m "docs: update minos-relay-backend-design for /v1 HTTP control plane"
```

---

## Self-review

**Spec coverage:**

- /v1/pairing/tokens → Task A3 ✓
- /v1/pairing/consume → Task B1 ✓
- DELETE /v1/pairing → Task B2 ✓
- /v1/threads → Task C1 ✓
- /v1/threads/{id}/events → Task C1 (Step 3 implementation) ✓
- /v1/threads/{id}/last_seq → Task C1 ✓
- Daemon HTTP migration → B3 ✓
- Mobile pairing migration → B4 ✓
- Mobile threads migration → C3 ✓
- Cleanup → D1–D5 ✓

**Placeholder scan:**

The `read_thread` body in `crate::ingest::history::read_thread` is described as "paste in the existing body, lightly adapted" — that is a directive, not a placeholder. The body it asks the engineer to copy is in `crates/minos-backend/src/envelope/local_rpc.rs:160-331` and is already in the codebase. Acceptable.

The deletion list in D3/D4 names the symbols by their current definitions; the engineer can `rg` them and remove. Acceptable.

**Type consistency:**

- `RequestPairingQrParams`, `RequestPairingQrResponse`, `PairingQrPayload`, `PairConsumeRequest`, `PairResponse`, `ListThreadsParams`, `ListThreadsResponse`, `ReadThreadParams`, `ReadThreadResponse`, `GetThreadLastSeqResponse` — all already defined in `crates/minos-protocol/src/messages.rs` except `PairConsumeRequest` which is added in Task B1 Step 1.
- `AuthOutcome { device_id, role, authenticated_with_secret }` — defined in A1 Step 3, consumed in A3, B1, B2, C1.
- `MobileHttpClient::pair_consume` returns `PairResponse`; consumer in B4 Step 5 reads `pair_resp.your_device_secret` — matches `PairResponse.your_device_secret: DeviceSecret`.
- `RelayHttpClient::request_pairing_qr` returns `PairingQrPayload`; consumer in B3 Step 5 reads `qr.{v, backend_url, host_display_name, pairing_token, expires_at_ms, cf_access_*}` — matches the struct.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/07-http-control-plane-split.md`.

**Per the user's standing rule for this repo: execute directly in the main conversation, not via subagent dispatch.** The plan is structured as four phases of small commits; the engineer (you) writes each test, runs it, implements, runs again, runs `cargo xtask check-all`, and commits before moving to the next step.

Suggested cadence: complete one Task per back-and-forth turn so the user can review each commit's diff if they want.
