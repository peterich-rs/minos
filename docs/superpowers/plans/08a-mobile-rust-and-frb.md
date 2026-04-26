# Mobile Auth + Agent Session — Phases 4–7 (Mobile Rust + frb)

> **Companion to** `08-mobile-auth-and-agent-session.md`. Read that file's preamble (worktree, critical clarifications, phase map) before starting these tasks. **REQUIRED SUB-SKILL:** Use superpowers:subagent-driven-development.

These phases land the mobile-side dispatch surface that consumes the backend shipped in Phases 1–3. After Phase 7 the Dart side has new methods; UI work follows in `08b`.

---

## Phase 4: Mobile Rust HTTP Auth

### Task 4.1: Extend `PersistedPairingState` with auth fields

**Files:**
- Modify: `crates/minos-mobile/src/store.rs`

- [ ] **Step 1: Add fields**

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedPairingState {
    pub backend_url: Option<String>,
    pub device_id: Option<String>,
    pub device_secret: Option<String>,
    pub cf_access_client_id: Option<String>,
    pub cf_access_client_secret: Option<String>,

    // New (auth):
    pub access_token: Option<String>,
    pub access_expires_at_ms: Option<i64>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
}
```

- [ ] **Step 2: Extend `MobilePairingStore` trait**

Add async methods:

```rust
async fn save_auth(&self, access: String, access_expires_at_ms: i64, refresh: String, account_id: String, account_email: String) -> Result<(), MinosError>;
async fn load_auth(&self) -> Result<Option<PersistedAuth>, MinosError>;
async fn clear_auth(&self) -> Result<(), MinosError>;
```

Where `PersistedAuth` is a small struct holding all five fields.

- [ ] **Step 3: Implement on `InMemoryPairingStore`**

Mirror existing per-field methods. Update `from_parts` to take auth fields.

- [ ] **Step 4: Run tests**

```bash
cargo test -p minos-mobile
```

- [ ] **Step 5: Commit**

```bash
git add crates/minos-mobile/src/store.rs
git commit -m "feat(mobile): PersistedPairingState carries auth tokens"
```

---

### Task 4.2: `auth.rs` module — `AuthSession`, `AuthSummary`, `AuthStateFrame`

**Files:**
- Create: `crates/minos-mobile/src/auth.rs`
- Modify: `crates/minos-mobile/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
//! In-memory auth state held by `MobileClient`. Persistence lives in
//! `store.rs` via `flutter_secure_storage` on the Dart side. Spec §6.1.

use std::time::Instant;

use minos_domain::MinosError;
use minos_protocol::AuthSummary;

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub access_token: String,
    pub access_expires_at: Instant,
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
    pub account: AuthSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStateFrame {
    Unauthenticated,
    Authenticated { account: AuthSummary },
    Refreshing,
    RefreshFailed { error: MinosError },
}
```

- [ ] **Step 2: Wire mod**

In `crates/minos-mobile/src/lib.rs`:

```rust
pub mod auth;
```

- [ ] **Step 3: Cargo check + commit**

```bash
cargo check -p minos-mobile
git add crates/minos-mobile/src/auth.rs crates/minos-mobile/src/lib.rs
git commit -m "feat(mobile): AuthSession + AuthStateFrame types"
```

---

### Task 4.3: 4 auth endpoints on `MobileHttpClient`

**Files:**
- Modify: `crates/minos-mobile/src/http.rs`

- [ ] **Step 1: Add `register`**

```rust
impl MobileHttpClient {
    pub async fn register(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/register", self.base);
        let body = AuthRequest { email: email.into(), password: password.into() };
        let req = self.client.post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .json(&body);
        let req = stamp_cf(req, &self.cf_access);
        let resp = req.send().await.map_err(|e| connect_err(&url, e))?;
        decode_auth_resp(resp).await
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> { /* same shape, /v1/auth/login */ }

    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, MinosError> { /* /v1/auth/refresh, body { refresh_token } */ }

    pub async fn logout(&self, access_token: &str, refresh_token: &str) -> Result<(), MinosError> {
        // Adds Authorization: Bearer header. Uses 204 NO_CONTENT.
    }
}
```

- [ ] **Step 2: Add response decoder mapping kind→error**

```rust
async fn decode_auth_resp(resp: reqwest::Response) -> Result<AuthResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        return resp.json::<AuthResponse>().await.map_err(|e| MinosError::BackendInternal { message: e.to_string() });
    }
    // Try to parse `{ kind: "..." }`
    let retry_after = resp.headers().get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(60);
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
    Err(match (status.as_u16(), kind) {
        (400, "weak_password") => MinosError::WeakPassword,
        (401, "invalid_credentials") => MinosError::InvalidCredentials,
        (401, _) => MinosError::Unauthorized { reason: "auth failed".into() },
        (409, "email_taken") => MinosError::EmailTaken,
        (429, _) => MinosError::RateLimited { retry_after_s: retry_after },
        _ => MinosError::BackendInternal { message: format!("{status} {kind}") },
    })
}
```

- [ ] **Step 3: Tests**

In `tests/http_smoke.rs`, add `auth_register_round_trips_against_real_backend` mirroring the existing `pair_consume_round_trips` pattern.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-mobile --test http_smoke
git add crates/minos-mobile/src/http.rs crates/minos-mobile/tests/http_smoke.rs
git commit -m "feat(mobile): /v1/auth/* HTTP client methods"
```

---

### Task 4.4: Token-refresh interceptor on HTTP

**Files:**
- Modify: `crates/minos-mobile/src/http.rs`

- [ ] **Step 1: Add a `with_bearer` helper**

```rust
impl MobileHttpClient {
    fn build_authed_request(&self, method: reqwest::Method, path: &str, access: &str)
        -> reqwest::RequestBuilder
    {
        let url = format!("{}{}", self.base, path);
        let req = self.client.request(method, &url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("authorization", format!("Bearer {access}"));
        stamp_cf(req, &self.cf_access)
    }
}
```

> Auth-aware retry logic (refresh on 401 + retry once) lives in `client.rs::AuthAwareDispatch` — see Task 6.4. The HTTP client is intentionally dumb; it surfaces 401 and the layer above decides.

- [ ] **Step 2: Cargo check + commit**

```bash
cargo check -p minos-mobile
git add crates/minos-mobile/src/http.rs
git commit -m "refactor(mobile): build_authed_request helper for bearer routes"
```

---

## Phase 5: Mobile Rust RPC Dispatch

### Task 5.1: `RpcReply` + monotonic id state in `MobileClient`

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`
- Modify: `crates/minos-mobile/Cargo.toml`

- [ ] **Step 1: Add `dashmap` dep**

```toml
dashmap = { workspace = true }
```

- [ ] **Step 2: Add fields to `MobileClient`**

```rust
pub struct MobileClient {
    // existing fields preserved
    pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>>,
    next_id: Arc<AtomicU64>,
    auth_state_tx: watch::Sender<AuthStateFrame>,
    auth_state_rx: watch::Receiver<AuthStateFrame>,
    auth_session: Arc<RwLock<Option<AuthSession>>>,
}
```

Initialize in `new(...)`:

```rust
let pending = Arc::new(DashMap::new());
let next_id = Arc::new(AtomicU64::new(1));
let (auth_state_tx, auth_state_rx) = watch::channel(AuthStateFrame::Unauthenticated);
let auth_session = Arc::new(RwLock::new(None));
```

- [ ] **Step 3: Add `RpcReply` enum**

In a new file `crates/minos-mobile/src/rpc.rs`:

```rust
//! Outbound JSON-RPC dispatch over `Envelope::Forward`. Spec §6.2.
//!
//! The relay envelope is opaque payload-wise — JSON-RPC `{id, method,
//! params}` lives INSIDE `Envelope::Forward { payload }`. Reply
//! correlation reads the inner JSON-RPC `id`.

use serde_json::Value;

#[derive(Debug, Clone)]
pub enum RpcReply {
    Ok(Value),
    Err { code: i32, message: String },
}
```

- [ ] **Step 4: Wire mod**

In `lib.rs`:

```rust
pub mod rpc;
```

- [ ] **Step 5: Cargo check**

```bash
cargo check -p minos-mobile
```

- [ ] **Step 6: Commit**

```bash
git add crates/minos-mobile/Cargo.toml \
        crates/minos-mobile/src/client.rs \
        crates/minos-mobile/src/rpc.rs \
        crates/minos-mobile/src/lib.rs
git commit -m "feat(mobile): pending-map + auth state + RpcReply skeleton"
```

---

### Task 5.2: `forward_rpc` helper

**Files:**
- Modify: `crates/minos-mobile/src/rpc.rs`

- [ ] **Step 1: Implement**

```rust
use std::sync::atomic::Ordering;
use std::time::Duration;

use dashmap::DashMap;
use minos_domain::MinosError;
use minos_protocol::envelope::Envelope;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::{mpsc, oneshot};

const PENDING_CAP: usize = 1024;

pub(crate) async fn forward_rpc<P: Serialize, R: DeserializeOwned>(
    pending: &DashMap<u64, oneshot::Sender<RpcReply>>,
    next_id: &AtomicU64,
    outbox: &mpsc::Sender<Envelope>,
    method: &str,
    params: P,
    timeout: Duration,
) -> Result<R, MinosError> {
    if pending.len() >= PENDING_CAP {
        return Err(MinosError::NotConnected);
    }
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    pending.insert(id, tx);

    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": serde_json::to_value(params).map_err(|e| MinosError::BackendInternal { message: e.to_string() })?,
    });
    let env = Envelope::Forward { version: 1, payload };

    if outbox.send(env).await.is_err() {
        pending.remove(&id);
        return Err(MinosError::NotConnected);
    }

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(RpcReply::Ok(v))) => serde_json::from_value(v)
            .map_err(|e| MinosError::BackendInternal { message: e.to_string() }),
        Ok(Ok(RpcReply::Err { code, message })) => Err(map_rpc_err(code, message)),
        Ok(Err(_)) => Err(MinosError::RequestDropped),
        Err(_) => {
            pending.remove(&id);
            Err(MinosError::Timeout)
        }
    }
}

fn map_rpc_err(code: i32, message: String) -> MinosError {
    match code {
        -32001 => MinosError::PairingStateMismatch { message },
        -32003 => MinosError::DeviceNotTrusted { reason: message },
        _ => MinosError::RpcCallFailed { method: "forwarded".into(), message },
    }
}

use crate::rpc::RpcReply;
```

(Reorganize the file — `RpcReply` enum from Task 5.1 stays at the top, `forward_rpc` below.)

- [ ] **Step 2: Unit test for id allocation + insert/remove**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[tokio::test]
    async fn forward_rpc_timeout_removes_pending() {
        let pending = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (tx, _rx) = mpsc::channel::<Envelope>(8);
        let res: Result<Value, _> = forward_rpc(&pending, &next_id, &tx,
            "minos_health", serde_json::Value::Null, Duration::from_millis(50)).await;
        assert!(matches!(res, Err(MinosError::Timeout)));
        assert!(pending.is_empty());
    }
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p minos-mobile rpc::tests
git add crates/minos-mobile/src/rpc.rs
git commit -m "feat(mobile): forward_rpc dispatch primitive"
```

---

### Task 5.3: `Forwarded` arm in `handle_text_frame`

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Update `handle_text_frame` signature**

It currently takes `(text, ui_events_tx, state_tx)`. Add `pending: &DashMap<u64, oneshot::Sender<RpcReply>>`. Thread the new arg through `recv_loop` too.

- [ ] **Step 2: Add the arm**

After the existing `Envelope::Event` arm:

```rust
Envelope::Forwarded { payload, .. } => {
    let Some(id) = payload.get("id").and_then(|v| v.as_u64()) else {
        tracing::debug!(?payload, "mobile: Forwarded missing id");
        return;
    };
    let Some((_, tx)) = pending.remove(&id) else {
        tracing::debug!(id, "mobile: Forwarded with no pending entry");
        return;
    };
    let reply = if let Some(result) = payload.get("result") {
        RpcReply::Ok(result.clone())
    } else if let Some(err) = payload.get("error") {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-32000) as i32;
        let message = err.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
        RpcReply::Err { code, message }
    } else {
        RpcReply::Err { code: -32700, message: "malformed jsonrpc reply".into() }
    };
    let _ = tx.send(reply);
}
```

- [ ] **Step 3: Pass `pending` Arc into `recv_loop`**

In `connect()`, where `recv_handle` is spawned, clone `pending` and pass it to `recv_loop`. Update `recv_loop` and `handle_text_frame` signatures.

- [ ] **Step 4: Test**

In `crates/minos-mobile/src/client.rs` `#[cfg(test)] mod tests`, add a unit test that synthesises a `Forwarded` text frame, calls `handle_text_frame`, and asserts the corresponding pending entry was fired.

- [ ] **Step 5: Run + commit**

```bash
cargo test -p minos-mobile
git add crates/minos-mobile/src/client.rs
git commit -m "feat(mobile): handle inbound Forwarded → match by JSON-RPC id"
```

---

### Task 5.4: `start_agent` / `send_user_message` / `stop_agent` on MobileClient (Rust)

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Implement `start_agent`**

```rust
impl MobileClient {
    pub async fn start_agent(
        &self,
        agent: AgentName,
        prompt: String,
    ) -> Result<StartAgentResponse, MinosError> {
        let outbox = self.outbox.lock().await
            .clone()
            .ok_or(MinosError::NotConnected)?;
        let req = StartAgentRequest { agent };
        let resp: StartAgentResponse = forward_rpc(
            &self.pending, &self.next_id, &outbox,
            "minos_start_agent", req, Duration::from_secs(60),
        ).await?;
        // Deliver the prompt as the first user message.
        let send_req = SendUserMessageRequest { session_id: resp.session_id.clone(), text: prompt };
        let _: () = forward_rpc(
            &self.pending, &self.next_id, &outbox,
            "minos_send_user_message", send_req, Duration::from_secs(10),
        ).await?;
        Ok(resp)
    }

    pub async fn send_user_message(&self, session_id: String, text: String) -> Result<(), MinosError> {
        let outbox = self.outbox.lock().await.clone().ok_or(MinosError::NotConnected)?;
        let req = SendUserMessageRequest { session_id, text };
        let _: () = forward_rpc(&self.pending, &self.next_id, &outbox,
            "minos_send_user_message", req, Duration::from_secs(10)).await?;
        Ok(())
    }

    pub async fn stop_agent(&self) -> Result<(), MinosError> {
        let outbox = self.outbox.lock().await.clone().ok_or(MinosError::NotConnected)?;
        let _: () = forward_rpc(&self.pending, &self.next_id, &outbox,
            "minos_stop_agent", serde_json::Value::Null, Duration::from_secs(10)).await?;
        Ok(())
    }
}
```

> **Critical**: `forward_rpc::<_, ()>` — the daemon returns `null`/empty object, which `serde_json::from_value::<()>` accepts only on `Value::Null`. If the daemon returns `{}`, deserialize as `serde_json::Value` first then discard. If you hit this, switch the return-type binding to `serde_json::Value` and ignore.

- [ ] **Step 2: Drain pending on disconnect**

In the existing `Disconnected` transition path (`recv_loop` close/err arm, `Unpaired`/`ServerShutdown` event arm), drain `self.pending`:

```rust
// inside recv_loop close handler — needs `pending` arg
for entry in pending.iter() {
    // Can't move out of iter; collect keys first.
}
let keys: Vec<u64> = pending.iter().map(|e| *e.key()).collect();
for k in keys {
    if let Some((_, tx)) = pending.remove(&k) {
        let _ = tx.send(RpcReply::Err { code: -32099, message: "request dropped".into() });
    }
}
```

The `forward_rpc` callsite maps `RpcReply::Err` with code `-32099` → `MinosError::RequestDropped`. Update `map_rpc_err` accordingly.

- [ ] **Step 3: Test**

Add a `crates/minos-mobile/src/client.rs` test:

```rust
#[tokio::test]
async fn start_agent_returns_request_dropped_when_disconnected() {
    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let res = client.start_agent(AgentName::Codex, "ping".into()).await;
    assert!(matches!(res, Err(MinosError::NotConnected)));
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-mobile
git add crates/minos-mobile/src/client.rs crates/minos-mobile/src/rpc.rs
git commit -m "feat(mobile): start_agent/send_user_message/stop_agent over forward dispatch"
```

---

## Phase 6: Mobile Rust Auto-Reconnect + Lifecycle

### Task 6.1: `ReconnectController` skeleton

**Files:**
- Create: `crates/minos-mobile/src/reconnect.rs`
- Modify: `crates/minos-mobile/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
//! Auto-reconnect controller. Spec §6.3.

use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug)]
pub(crate) struct ReconnectController {
    state: RwLock<ReconnectState>,
}

#[derive(Debug)]
struct ReconnectState {
    delay: Duration,
    consecutive_failures: u32,
    last_connected_at: Option<Instant>,
    foreground: bool,
    paused: bool,
}

impl ReconnectController {
    pub fn new() -> Self {
        Self { state: RwLock::new(ReconnectState {
            delay: Duration::from_secs(1),
            consecutive_failures: 0,
            last_connected_at: None,
            foreground: true,
            paused: false,
        }) }
    }

    pub async fn next_delay(&self) -> Duration {
        self.state.read().await.delay
    }

    pub async fn record_failure(&self) {
        let mut s = self.state.write().await;
        s.consecutive_failures = s.consecutive_failures.saturating_add(1);
        s.delay = (s.delay * 2).min(Duration::from_secs(30));
    }

    pub async fn record_success(&self) {
        let mut s = self.state.write().await;
        let stable = s.last_connected_at
            .map(|t| t.elapsed() > Duration::from_secs(60))
            .unwrap_or(true);
        if stable { s.delay = Duration::from_secs(1); }
        s.consecutive_failures = 0;
        s.last_connected_at = Some(Instant::now());
    }

    pub async fn notify_foregrounded(&self) {
        let mut s = self.state.write().await;
        s.foreground = true;
        s.delay = Duration::from_secs(1);
        s.paused = false;
    }

    pub async fn notify_backgrounded(&self) {
        let mut s = self.state.write().await;
        s.foreground = false;
        s.paused = true;
    }

    pub async fn is_paused(&self) -> bool {
        self.state.read().await.paused
    }
}
```

- [ ] **Step 2: Wire mod**

```rust
mod reconnect;
pub(crate) use reconnect::ReconnectController;
```

- [ ] **Step 3: Unit tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backoff_caps_at_30s() {
        let r = ReconnectController::new();
        for _ in 0..10 { r.record_failure().await; }
        assert_eq!(r.next_delay().await, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn foreground_resets_delay() {
        let r = ReconnectController::new();
        r.record_failure().await;
        r.record_failure().await;
        r.notify_foregrounded().await;
        assert_eq!(r.next_delay().await, Duration::from_secs(1));
    }
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-mobile reconnect
git add crates/minos-mobile/src/reconnect.rs crates/minos-mobile/src/lib.rs
git commit -m "feat(mobile): ReconnectController backoff state machine"
```

---

### Task 6.2: Reconnect loop in `MobileClient`

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Add reconnect controller + loop task field**

In the struct, add `pub(crate) reconnect: Arc<ReconnectController>` and `reconnect_handle: Mutex<Option<JoinHandle<()>>>`.

- [ ] **Step 2: Spawn loop on `Authenticated` transition**

When `auth_state_tx.send(AuthStateFrame::Authenticated{..})` fires (after register/login/hydrate), spawn a task that:

1. Reads `reconnect.next_delay()`, sleeps that long.
2. Checks `reconnect.is_paused()` — if true, exit.
3. Checks `auth_session.access_expires_at`; if `< now + 2 min` calls refresh.
4. Calls `connect()`.
5. On success → `record_success()`. On failure → `record_failure()`, loop.

- [ ] **Step 3: Stop loop on `Unauthenticated` / `RefreshFailed`**

```rust
if let Some(h) = self.reconnect_handle.lock().await.take() { h.abort(); }
```

Drain `pending` with `RequestDropped`.

- [ ] **Step 4: Tests**

Use the existing `tests/envelope_client.rs` pattern (real backend) — add `reconnects_after_ws_drop_when_authenticated`. Use a fake-backend handler that closes the WS once, then accepts the second connection.

- [ ] **Step 5: Run + commit**

```bash
cargo test -p minos-mobile
git add crates/minos-mobile/src/client.rs
git commit -m "feat(mobile): reconnect loop gated on AuthState"
```

---

### Task 6.3: Lifecycle hooks

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Add public methods**

```rust
impl MobileClient {
    pub fn notify_foregrounded(&self) {
        let r = self.reconnect.clone();
        tokio::spawn(async move { r.notify_foregrounded().await; });
    }

    pub fn notify_backgrounded(&self) {
        let r = self.reconnect.clone();
        tokio::spawn(async move { r.notify_backgrounded().await; });
    }
}
```

> Sync wrappers because Dart calls these from `WidgetsBindingObserver` which runs on the main isolate; the actual state mutation is async-safe.

- [ ] **Step 2: Cargo check + commit**

```bash
cargo check -p minos-mobile
git add crates/minos-mobile/src/client.rs
git commit -m "feat(mobile): notify_foregrounded / notify_backgrounded"
```

---

### Task 6.4: Auth-aware HTTP retry + token refresh

**Files:**
- Modify: `crates/minos-mobile/src/client.rs`

- [ ] **Step 1: Add `register` / `login` / `refresh_session` / `logout` on `MobileClient`**

```rust
impl MobileClient {
    pub async fn register(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
        let http = self.http_client().await?;
        let resp = http.register(&email, &password).await?;
        self.set_auth_session(resp).await;
        Ok(self.auth_session.read().await.as_ref().unwrap().account.clone())
    }

    pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError> { /* same shape */ }

    pub async fn refresh_session(&self) -> Result<(), MinosError> {
        let session = self.auth_session.read().await.clone()
            .ok_or_else(|| MinosError::AuthRefreshFailed { message: "no session".into() })?;
        self.auth_state_tx.send(AuthStateFrame::Refreshing).ok();
        let http = self.http_client().await?;
        match http.refresh(&session.refresh_token).await {
            Ok(r) => {
                let mut s = self.auth_session.write().await;
                if let Some(s) = s.as_mut() {
                    s.access_token = r.access_token;
                    s.access_expires_at_ms = chrono::Utc::now().timestamp_millis() + (r.expires_in * 1000);
                    s.access_expires_at = Instant::now() + Duration::from_secs(r.expires_in as u64);
                    s.refresh_token = r.refresh_token;
                }
                self.auth_state_tx.send(AuthStateFrame::Authenticated { account: session.account }).ok();
                Ok(())
            }
            Err(e) => {
                self.auth_state_tx.send(AuthStateFrame::RefreshFailed { error: e.clone() }).ok();
                self.clear_auth_and_disconnect().await;
                Err(MinosError::AuthRefreshFailed { message: e.to_string() })
            }
        }
    }

    pub async fn logout(&self) -> Result<(), MinosError> {
        // Best-effort stop_agent (2s)
        let _ = tokio::time::timeout(Duration::from_secs(2), self.stop_agent()).await;
        let session = self.auth_session.read().await.clone();
        if let Some(s) = session {
            let http = self.http_client().await?;
            let _ = http.logout(&s.access_token, &s.refresh_token).await;
        }
        self.clear_auth_and_disconnect().await;
        Ok(())
    }
}
```

- [ ] **Step 2: Subscribe stream**

```rust
pub fn subscribe_auth_state(&self) -> watch::Receiver<AuthStateFrame> {
    self.auth_state_rx.clone()
}
```

- [ ] **Step 3: Tests**

Mock backend: assert that 401 on refresh transitions state to `RefreshFailed`.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-mobile
git add crates/minos-mobile/src/client.rs
git commit -m "feat(mobile): register/login/refresh/logout + AuthStateFrame stream"
```

---

## Phase 7: frb Surface

### Task 7.1: Add 4 auth methods to frb `MobileClient`

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: Add `pub use` imports at top**

In the `pub use` block near the top of `minos.rs`:

```rust
pub use minos_protocol::AuthSummary;
```

- [ ] **Step 2: Add methods**

In `impl MobileClient { ... }`:

```rust
pub async fn register(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
    self.0.register(email, password).await
}

pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError> {
    self.0.login(email, password).await
}

pub async fn refresh_session(&self) -> Result<(), MinosError> {
    self.0.refresh_session().await
}

pub async fn logout(&self) -> Result<(), MinosError> {
    self.0.logout().await
}
```

- [ ] **Step 3: Cargo check (just the FFI crate)**

```bash
cargo check -p minos-ffi-frb
```

- [ ] **Step 4: Commit**

```bash
git add crates/minos-ffi-frb/src/api/minos.rs
git commit -m "feat(frb): expose register/login/refresh/logout"
```

---

### Task 7.2: Add 3 agent methods + 2 lifecycle methods

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: Add imports**

```rust
pub use minos_protocol::{StartAgentResponse};
pub use minos_domain::AgentName;  // already imported per existing mirror
```

- [ ] **Step 2: Add methods**

```rust
pub async fn start_agent(
    &self,
    agent: AgentName,
    prompt: String,
) -> Result<StartAgentResponse, MinosError> {
    self.0.start_agent(agent, prompt).await
}

pub async fn send_user_message(
    &self,
    session_id: String,
    text: String,
) -> Result<(), MinosError> {
    self.0.send_user_message(session_id, text).await
}

pub async fn stop_agent(&self) -> Result<(), MinosError> {
    self.0.stop_agent().await
}

#[frb(sync)]
pub fn notify_foregrounded(&self) { self.0.notify_foregrounded(); }

#[frb(sync)]
pub fn notify_backgrounded(&self) { self.0.notify_backgrounded(); }
```

- [ ] **Step 3: Add `StartAgentResponse` mirror**

At the bottom (mirror block):

```rust
#[allow(dead_code)]
#[frb(mirror(StartAgentResponse))]
pub struct _StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}
```

- [ ] **Step 4: Cargo check + commit**

```bash
cargo check -p minos-ffi-frb
git add crates/minos-ffi-frb/src/api/minos.rs
git commit -m "feat(frb): expose start_agent/send/stop + lifecycle hooks"
```

---

### Task 7.3: `subscribe_auth_state` stream

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: Add the watch-channel subscriber**

Mirror the existing `subscribe_state` pattern (watch-channel forwarder). Define a frb-friendly mirror enum first:

```rust
#[derive(Debug, Clone)]
pub enum AuthStateFrame {
    Unauthenticated,
    Authenticated { account: AuthSummary },
    Refreshing,
    RefreshFailed { error: MinosError },
}

impl From<minos_mobile::auth::AuthStateFrame> for AuthStateFrame {
    fn from(f: minos_mobile::auth::AuthStateFrame) -> Self {
        match f {
            minos_mobile::auth::AuthStateFrame::Unauthenticated => Self::Unauthenticated,
            minos_mobile::auth::AuthStateFrame::Authenticated { account } => Self::Authenticated { account },
            minos_mobile::auth::AuthStateFrame::Refreshing => Self::Refreshing,
            minos_mobile::auth::AuthStateFrame::RefreshFailed { error } => Self::RefreshFailed { error },
        }
    }
}
```

> **Note**: NOT a `#[frb(mirror)]` — frb cannot mirror an enum from a non-`crate::api` module if the variants carry an external type with attribute conflicts. Instead, define a fresh enum here that frb codegen sees as a first-class Dart enum.

- [ ] **Step 2: Add subscribe method**

```rust
pub fn subscribe_auth_state(&self, sink: StreamSink<AuthStateFrame>) {
    let mut rx = self.0.subscribe_auth_state();
    frb_runtime().spawn(async move {
        // Emit current state immediately.
        let snapshot = AuthStateFrame::from(rx.borrow().clone());
        if sink.add(snapshot).is_err() { return; }
        while rx.changed().await.is_ok() {
            let f = AuthStateFrame::from(rx.borrow().clone());
            if sink.add(f).is_err() { break; }
        }
    });
}
```

- [ ] **Step 3: Cargo check + commit**

```bash
cargo check -p minos-ffi-frb
git add crates/minos-ffi-frb/src/api/minos.rs
git commit -m "feat(frb): subscribe_auth_state watch stream"
```

---

### Task 7.4: Add `AuthSummary` mirror

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: Add mirror block at the bottom**

```rust
#[allow(dead_code)]
#[frb(mirror(AuthSummary))]
pub struct _AuthSummary {
    pub account_id: String,
    pub email: String,
}
```

- [ ] **Step 2: Cargo check + commit**

```bash
cargo check -p minos-ffi-frb
git add crates/minos-ffi-frb/src/api/minos.rs
git commit -m "feat(frb): mirror AuthSummary"
```

---

### Task 7.5: Update `MinosError` mirror

**Files:**
- Modify: `crates/minos-ffi-frb/src/api/minos.rs`

- [ ] **Step 1: Add new variants to the `_MinosError` mirror**

For each `MinosError` variant added in Task 3.1, add a matching arm in the `_MinosError` mirror block. The mirror must be exhaustive — any missing variant fails codegen.

- [ ] **Step 2: Cargo check + commit**

```bash
cargo check -p minos-ffi-frb
git add crates/minos-ffi-frb/src/api/minos.rs
git commit -m "feat(frb): mirror new MinosError variants"
```

---

### Task 7.6: Regenerate frb bindings

**Files:**
- Generated: `apps/mobile/lib/src/rust/api/minos.dart` (and `.freezed.dart`)
- Generated: `crates/minos-ffi-frb/src/frb_generated.rs`

- [ ] **Step 1: Regenerate**

```bash
cargo xtask gen-frb
```

Expected: tool runs to completion, no errors.

- [ ] **Step 2: Run drift guard**

```bash
cargo xtask check-all
```

Expected: green. The drift guard regenerates and `git diff --exit-code`s — passing means the regenerated files are committed.

- [ ] **Step 3: Commit**

```bash
git add apps/mobile/lib/src/rust/ crates/minos-ffi-frb/src/frb_generated.rs
git commit -m "chore(frb): regenerate bindings for auth + agent + lifecycle"
```

---

## Checkpoint: Phase 4–7 → mobile Rust + frb ship

At this point:
- `minos-mobile` exposes auth, dispatch, reconnect, lifecycle as Rust APIs.
- `minos-ffi-frb` exposes those to Dart with mirror types.
- `cargo xtask check-all` is green.

Next: Flutter UI consumes these. See `08b-flutter-and-verification.md`.
