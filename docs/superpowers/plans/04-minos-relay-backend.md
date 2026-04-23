# Minos · Relay Backend — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL — use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to work this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Mark each step complete only after its acceptance criterion passes.

> **This plan does not touch the macOS or iOS apps.** Their refactor to the new topology is a separate plan (future `05-client-refactor-to-relay.md`). Stop at the relay boundary: you prove correctness via two in-process fake WebSocket clients.

---

## Goal

Stand up a standalone Rust backend service `minos-relay` that:

1. Listens on `127.0.0.1:8787` (configurable) with `axum`.
2. Accepts authenticated WebSocket connections on `/devices`.
3. Routes envelope-framed messages between paired devices.
4. Implements a minimal local-RPC surface (`ping`, `request_pairing_token`, `pair`, `forget_peer`).
5. Persists devices, pairings, and pairing tokens in SQLite via `sqlx`.
6. Passes an end-to-end `#[tokio::test]` integration test where two fake clients pair, round-trip a `list_clis`-shaped forward message, and execute `forget_peer`.

The plan ends when `cargo xtask check-all` is green and the relay's own integration test (`crates/minos-relay/tests/e2e.rs`) passes in under 2 seconds.

## Reference documents (READ THESE FIRST)

Read in order. Do not start coding until you have finished all four:

1. `docs/superpowers/specs/minos-relay-backend-design.md` — the single source of truth. Every design decision below traces back to a section here. When this plan disagrees with the spec, **the spec wins** — flag the disagreement and ask.
2. `docs/adr/0009-broker-architecture-pivot.md` — why the relay exists.
3. `docs/adr/0011-broker-envelope-protocol.md` — envelope shape rationale. Critical for §6 of spec.
4. `docs/adr/0012-sqlite-via-sqlx.md` — persistence choices. Critical for §8 of spec.

Secondary:

- `docs/adr/0010-cloudflare-tunnel-and-access.md` — exposure; not load-bearing for relay code itself (the relay is cloudflare-agnostic).
- `docs/ops/cloudflare-tunnel-setup.md` — runbook, only needed if you verify end-to-end through a real tunnel (optional; fake-client tests are sufficient for plan acceptance).
- `docs/superpowers/specs/minos-architecture-and-mvp-design.md` — original MVP spec. Retained context; overridden where the relay spec says so.
- Existing ADRs `0001`–`0008`.

## Architecture in one sentence

Clients open outbound WebSocket to `/devices` carrying `CF-Access-*` + `X-Device-Id` (+ optional `X-Device-Secret`) headers; the relay verifies the business-layer secret, classifies the connection as *Unpaired* or *Paired*, dispatches `LocalRpc` envelopes itself, and opaquely forwards `Forward` envelopes to the paired peer as `Forwarded`; server-side state changes arrive as `Event` envelopes.

## Tech stack (locked)

- Rust stable channel (same as the workspace; already pinned).
- `axum` 0.7+ with `ws` feature.
- `tokio-tungstenite` 0.29 (for in-test fake clients; already in workspace deps).
- `sqlx` 0.8+ with `sqlite`, `runtime-tokio`, `macros`, `migrate` features.
- `argon2` 0.5+ for secret hashing.
- `subtle` 2.x for constant-time compare.
- `dashmap` 6.x for the session registry.
- `clap` 4.x for CLI flags.
- Existing workspace deps: `tokio`, `serde`, `serde_json`, `uuid`, `thiserror`, `tracing`, `mars-xlog`, `getrandom`, `base64`.

## Prerequisites (environment)

Before you start:

```bash
# You must be on the feature branch.
git branch --show-current   # expect: feat/relay-backend

# You must NOT modify these crates in this plan (they belong to plan 05):
#   crates/minos-daemon
#   crates/minos-mobile
#   crates/minos-transport   (client-side only; see step 15 below)
#   apps/macos
#   apps/mobile

# Install sqlx CLI if you don't have it (for `cargo sqlx prepare`):
cargo install sqlx-cli --no-default-features --features sqlite
```

## File structure (created or modified)

```
minos/
├── Cargo.toml                                 [modified — add workspace deps: axum, sqlx, argon2, subtle, dashmap; add minos-relay to members via existing glob]
├── crates/
│   ├── minos-domain/
│   │   └── src/
│   │       ├── ids.rs                         [modified — add DeviceSecret newtype]
│   │       ├── agent.rs                       [unchanged]
│   │       ├── error.rs                       [modified — add new variants; rewrite Tailscale-era strings (see spec §10.1)]
│   │       └── role.rs                        [new — DeviceRole enum]
│   ├── minos-protocol/
│   │   └── src/
│   │       ├── envelope.rs                    [new — Envelope, LocalRpcMethod, LocalRpcOutcome, EventKind, RpcError]
│   │       ├── lib.rs                         [modified — pub mod envelope; pub use envelope::*]
│   │       └── tests/
│   │           └── envelope_golden.rs         [new — golden JSON round-trips for every variant]
│   └── minos-relay/                           [NEW CRATE]
│       ├── Cargo.toml                         [new]
│       ├── migrations/
│       │   ├── 0001_devices.sql               [new]
│       │   ├── 0002_pairings.sql              [new]
│       │   └── 0003_pairing_tokens.sql        [new]
│       ├── src/
│       │   ├── main.rs                        [new — bin entry]
│       │   ├── config.rs                      [new — CLI + env parsing]
│       │   ├── error.rs                       [new — RelayError → MinosError boundary]
│       │   ├── http/
│       │   │   ├── mod.rs                     [new]
│       │   │   ├── health.rs                  [new]
│       │   │   └── ws_devices.rs              [new — WS upgrade + auth extraction]
│       │   ├── session/
│       │   │   ├── mod.rs                     [new]
│       │   │   ├── registry.rs                [new — DashMap<DeviceId, SessionHandle>]
│       │   │   └── heartbeat.rs               [new — ping/pong loop]
│       │   ├── envelope/
│       │   │   ├── mod.rs                     [new — dispatcher]
│       │   │   └── local_rpc.rs               [new — pair/ping/forget/request_pairing_token]
│       │   ├── pairing/
│       │   │   ├── mod.rs                     [new]
│       │   │   └── secret.rs                  [new — argon2 hash + verify]
│       │   └── store/
│       │       ├── mod.rs                     [new — pool + migrate!]
│       │       ├── devices.rs                 [new]
│       │       ├── pairings.rs                [new]
│       │       └── tokens.rs                  [new]
│       └── tests/
│           └── e2e.rs                         [new — full pair + forward + forget loop with two fake clients]
├── xtask/
│   └── src/main.rs                            [modified — add relay-run and relay-db-reset subcommands]
└── .github/workflows/
    └── ci.yml                                 [modified — add `cargo sqlx prepare --check` step]
```

## Steps

### 1. Workspace + crate scaffolding

- [ ] Add workspace deps to root `Cargo.toml`:
  - `axum = { version = "0.7", features = ["ws", "macros"] }`
  - `sqlx = { version = "0.8", default-features = false, features = ["sqlite", "runtime-tokio", "macros", "migrate"] }`
  - `argon2 = "0.5"`
  - `subtle = "2"`
  - `dashmap = "6"`
  - `clap = { version = "4", features = ["derive", "env"] }`
- [ ] `cargo new --bin crates/minos-relay` and populate its `Cargo.toml`:
  - `[package]` name `minos-relay`, inherit workspace version / edition / license
  - `[dependencies]`: `tokio`, `axum`, `sqlx`, `argon2`, `subtle`, `dashmap`, `clap`, `serde`, `serde_json`, `thiserror`, `uuid`, `getrandom`, `tracing`, `tracing-subscriber`, `mars-xlog`, `futures`, `async-trait`, `tokio-tungstenite`, `chrono`, `minos-domain` (path dep), `minos-protocol` (path dep)
  - `[dev-dependencies]`: `tokio-test`, `pretty_assertions`, `tempfile`, `rstest`, `proptest`, `tokio-tungstenite`
- [ ] `cargo check -p minos-relay` compiles the empty bin.

**Acceptance:** `cargo check --workspace` is green.

### 2. Domain additions

- [ ] In `crates/minos-domain/src/ids.rs`, add `DeviceSecret(pub String)` newtype with `generate()` (32B `getrandom` → base64url), `as_str()`, `Debug` must redact (`<redacted DeviceSecret>`); `Display` likewise redacted; `Serialize`/`Deserialize` as transparent String.
- [ ] New file `crates/minos-domain/src/role.rs` with `DeviceRole` enum: `MacHost` / `IosClient` / `BrowserAdmin`. `Serialize`/`Deserialize` as kebab-case. `FromStr` + `Display` for DB round-trip.
- [ ] `crates/minos-domain/src/lib.rs`: `pub mod role; pub use role::*;`.
- [ ] In `crates/minos-domain/src/error.rs`, add new variants per spec §10.1:
  - `Unauthorized { reason: String }`
  - `ConnectionStateMismatch { expected: String, actual: String }`
  - `EnvelopeVersionUnsupported { version: u8 }`
  - `PeerOffline { peer_device_id: String }`
  - `RelayInternal { message: String }`
  Each needs: `#[error(...)]`, matching `ErrorKind` variant, `kind()` arm, two `user_message` arms (zh + en).
- [ ] Rewrite `BindFailed` / `ConnectFailed` zh/en strings — remove Tailscale references, replace with relay-centric copy per spec §10.1.
- [ ] Add golden tests for `DeviceSecret` (not leaking secret in Debug output), `DeviceRole` (round-trip through JSON), and the new error `user_message` arms.

**Acceptance:** `cargo test -p minos-domain` green. Grep `git diff` confirms no Tailscale string remains.

### 3. Envelope protocol in `minos-protocol`

- [ ] New file `crates/minos-protocol/src/envelope.rs`. Define `Envelope`, `LocalRpcMethod`, `LocalRpcOutcome`, `EventKind`, `RpcError` per spec §6.
- [ ] Use `#[serde(tag = "kind", rename_all = "snake_case")]` on `Envelope`; version field `#[serde(rename = "v")]`.
- [ ] `crates/minos-protocol/src/lib.rs`: `pub mod envelope; pub use envelope::*;`.
- [ ] New file `crates/minos-protocol/tests/envelope_golden.rs`: for each variant write a golden JSON file under `crates/minos-protocol/tests/golden/envelope/<variant>.json` and assert `serde_json::from_str` + round-trip equals the fixture. Any PR that changes the schema must update golden.

**Acceptance:** `cargo test -p minos-protocol` green; golden JSON files exist for every variant.

### 4. SQLite migrations

- [ ] Create `crates/minos-relay/migrations/0001_devices.sql`, `0002_pairings.sql`, `0003_pairing_tokens.sql` per spec §8.1.
- [ ] All three use `STRICT` mode, `INTEGER NOT NULL` for timestamps (unix epoch millis), FK cascades on `ON DELETE CASCADE` where the spec says.
- [ ] `crates/minos-relay/src/store/mod.rs` exports `async fn connect(db_url: &str) -> Result<SqlitePool, RelayError>` that opens the pool, runs `sqlx::migrate!()`, and returns.
- [ ] One test `tests/store_smoke.rs`: create a temp DB, connect, check every table exists via `sqlx::query_scalar`.

**Acceptance:** `cargo test -p minos-relay` green at this point.

### 5. Store CRUD

- [ ] `store/devices.rs` — `insert_device(pool, id, name, role) -> Result<()>`; `upsert_secret_hash(pool, id, hash) -> Result<()>`; `get_device(pool, id) -> Result<Option<DeviceRow>>`; `get_secret_hash(pool, id) -> Result<Option<String>>`.
- [ ] `store/pairings.rs` — `insert_pairing(pool, a, b) -> Result<()>` (swap to canonical order `a < b`); `get_pair(pool, id) -> Result<Option<DeviceId>>`; `delete_pair(pool, a, b) -> Result<()>`.
- [ ] `store/tokens.rs` — `issue_token(pool, token_hash, issuer, expires_at) -> Result<()>`; `consume_token(pool, token_hash_candidate, now) -> Result<Option<IssuerId>>` (validates not expired, not consumed, marks consumed atomically); `gc_expired(pool, now) -> Result<u64>`.
- [ ] Use `sqlx::query!` macros for all queries. Run `cargo sqlx prepare` so `sqlx-data.json` lands at repo root; commit it.
- [ ] Unit tests per module exercising happy + error paths (expired token, missing device, double-consume).

**Acceptance:** `cargo test -p minos-relay` covers all three modules; `sqlx-data.json` committed.

### 6. Pairing service

- [ ] `pairing/secret.rs` — `hash_secret(plain: &DeviceSecret) -> Result<String, RelayError>` via argon2id; `verify_secret(plain: &str, hash: &str) -> Result<bool, RelayError>` wrapping `argon2::Argon2::verify_password` with `subtle::ConstantTimeEq` for the final byte compare.
- [ ] `pairing/mod.rs` — `PairingService` struct holding a `SqlitePool`. Methods:
  - `request_token(issuer: DeviceId, ttl: Duration) -> Result<(PairingToken, DateTime<Utc>), RelayError>` — generates token, hashes, inserts, returns plain + expires_at.
  - `consume_token(candidate: &PairingToken, consumer: DeviceId, consumer_name: String) -> Result<PairingOutcome, RelayError>` — on success returns `{issuer, issuer_secret, consumer_secret}`; writes pair + both secrets atomically.
  - `forget_pair(either_side: DeviceId) -> Result<Option<DeviceId>, RelayError>` — returns the peer's DeviceId so the caller can push the Unpaired event.
- [ ] Tests: property test for token entropy (reuse pattern from `minos-pairing`); integration tests for consume expired / consume twice / consume unknown.

**Acceptance:** `cargo test -p minos-relay` covers pairing; property test with 1000 iterations completes in <1s.

### 7. Session registry

- [ ] `session/registry.rs`:
  ```rust
  pub struct SessionRegistry(Arc<DashMap<DeviceId, SessionHandle>>);
  pub struct SessionHandle {
      pub device_id: DeviceId,
      pub paired_with: Arc<RwLock<Option<DeviceId>>>,
      pub outbox: mpsc::Sender<ServerFrame>,
  }
  ```
- [ ] Methods: `insert`, `remove`, `get` (returning `SessionHandle` clone which is cheap), `route(from: DeviceId, to: DeviceId, payload: Value) -> Result<(), RelayError>` (looks up `to`, sends `Forwarded` via `outbox`).
- [ ] Bounded mpsc (`capacity: 256`); on full, drop oldest outbound event with a warn! log (MVP backpressure policy — queuing across disconnect is P1).
- [ ] Unit tests for concurrent insert/remove (tokio::spawn × 100).

**Acceptance:** `cargo test -p minos-relay` covers session registry; no leaks (`Arc` strong count drops on session end).

### 8. Envelope dispatcher

- [ ] `envelope/mod.rs` — main per-connection loop:
  ```rust
  async fn run_session(
      ws: WebSocket,
      session: SessionHandle,
      registry: Arc<SessionRegistry>,
      pairing: Arc<PairingService>,
      store: SqlitePool,
  ) -> Result<(), RelayError>
  ```
  Runs three concurrent branches in `tokio::select!`:
    1. `ws.next()` → decode Envelope → dispatch `LocalRpc` / `Forward` / unknown → respond / route
    2. `session.outbox.recv()` → serialize ServerFrame → `ws.send(Message::Text(json))`
    3. heartbeat tick → send `Ping` frame; if no Pong within 60s → close session 1011
- [ ] `envelope/local_rpc.rs` — handles the four methods:
  - `Ping` → always returns `{ok: true}`
  - `RequestPairingToken` → reject if `role != MacHost`; reject if session is in Unpaired-and-unknown state (must have been handshake-verified); call `PairingService::request_token`
  - `Pair` → reject if session is already Paired; call `PairingService::consume_token`; on success, write `DeviceSecret` hash for the iOS client, also for the Mac (peer) if not already set; broadcast `Event::Paired` to both sessions
  - `ForgetPeer` → reject if Unpaired; call `PairingService::forget_pair`; broadcast `Event::Unpaired` to both sides
- [ ] Unknown envelope kind or unsupported version → send `LocalRpcResponse::Err(EnvelopeVersionUnsupported)` and close 4400.

**Acceptance:** dispatcher unit-testable via a mock WS pair; integration test in step 12 exercises the real loop.

### 9. HTTP handlers and auth

- [ ] `http/mod.rs` — axum `Router`:
  ```
  GET /health      → http/health.rs  (returns "ok" + relay version)
  GET /devices     → http/ws_devices.rs (WS upgrade handler)
  ```
- [ ] `http/ws_devices.rs`:
  1. Extract `X-Device-Id` header; if missing → `401`.
  2. Extract optional `X-Device-Role` (default MacHost for absent; TODO — confirm with spec; for MVP default `IosClient` if absent is fine because pairing flow will clarify).
  3. Extract optional `X-Device-Secret`.
  4. Look up device in store. Two paths:
     - No device row → insert as Unpaired; connection goes live in Unpaired mode. Emit `Event::Unpaired` immediately.
     - Device row exists with `secret_hash`:
       - If `X-Device-Secret` missing → reject `4401`.
       - If `verify_secret` fails → reject `4401`.
       - Else → find peer via `get_pair`, set `paired_with` on the session handle, go live in Paired mode.
  5. Perform `WebSocketUpgrade::on_upgrade(|ws| run_session(...))`.
  - Note: Cloudflare Access headers (`CF-Access-Client-Id/Secret`) are validated at the edge; relay does not re-validate, but logs their presence as a sanity check in dev builds.
- [ ] In Unpaired mode, dispatcher must reject every `LocalRpc` method except `Ping`, `RequestPairingToken` (if role `MacHost`), `Pair` (if role `IosClient`). Reject `Forward` entirely.

**Acceptance:** integration test can open a WS with/without headers and observe expected close codes.

### 10. Config + main

- [ ] `config.rs`:
  ```
  --listen <addr>      default 127.0.0.1:8787  env MINOS_RELAY_LISTEN
  --db <path>          default ./minos-relay.db  env MINOS_RELAY_DB
  --log-dir <path>     default ~/Library/Logs/Minos/ (or $TMPDIR/minos on non-mac)  env MINOS_RELAY_LOG_DIR
  --log-level <level>  default info  env RUST_LOG
  --token-ttl-secs <n> default 300  env MINOS_RELAY_TOKEN_TTL
  ```
- [ ] `main.rs`:
  1. Parse config
  2. Init tracing with `mars_xlog::XlogLayer` (name_prefix `relay`)
  3. Connect SQLite pool, run migrations
  4. Build `SessionRegistry`, `PairingService`
  5. Build axum Router with state (the three Arcs)
  6. Spawn token-GC task (every 60s call `gc_expired`)
  7. `axum::serve(listener, router).with_graceful_shutdown(shutdown_signal)` where `shutdown_signal` awaits SIGTERM/SIGINT
  8. On shutdown: broadcast `Event::ServerShutdown` to all sessions, give 500ms drain, close pool

**Acceptance:** `cargo run -p minos-relay -- --listen 127.0.0.1:8787 --db ./tmp.db` boots, logs "listening" + "migrations applied", responds 200 on `/health`.

### 11. xtask wiring

- [ ] In `xtask/src/main.rs` add two subcommands via `clap`:
  - `relay-run` → `cargo run -p minos-relay -- --listen 127.0.0.1:8787 --db ./minos-relay.db --log-level debug`
  - `relay-db-reset` → `rm -f ./minos-relay.db && cargo run -p minos-relay -- --db ./minos-relay.db --exit-after-migrate` (implement `--exit-after-migrate` in config.rs)
- [ ] Do not touch the existing `check-all` subcommand; the `crates/*` glob already includes `minos-relay`.

**Acceptance:** `cargo xtask relay-run` boots; `cargo xtask relay-db-reset` recreates schema.

### 12. End-to-end integration test

- [ ] `crates/minos-relay/tests/e2e.rs` — one `#[tokio::test]` running under 2s. Structure:
  1. Start relay on ephemeral port with `tempfile::NamedTempFile` DB.
  2. Open two raw `tokio-tungstenite` clients against the relay.
     - Client A: `X-Device-Id: <uuid>`, `X-Device-Role: mac-host`, no secret
     - Client B: `X-Device-Id: <uuid>`, `X-Device-Role: ios-client`, no secret
  3. A sends `LocalRpc { method: RequestPairingToken }` → asserts `LocalRpcResponse::Ok({ token, expires_at })`.
  4. B sends `LocalRpc { method: Pair, params: { token, device_name } }` → asserts `Ok({ peer_device_id, peer_name, your_device_secret })`.
  5. Both clients observe `Event::Paired` with each other's info.
  6. A sends `Forward { payload: { jsonrpc: "2.0", method: "list_clis", id: 1 } }`.
  7. B receives `Forwarded { from: A, payload: ... }`. B sends `Forward { payload: { jsonrpc: "2.0", result: [...], id: 1 } }`.
  8. A receives `Forwarded { from: B, payload: ... }`.
  9. A sends `LocalRpc { method: ForgetPeer }`. Both clients observe `Event::Unpaired`.
  10. A disconnects. B disconnects.
  11. Assertion: `devices` table has two rows, `pairings` table has zero rows, `pairing_tokens` has one row with `consumed_at` non-null.
- [ ] Add one negative test: B tries `Pair` with an invalid token → asserts `LocalRpcResponse::Err(PairingTokenInvalid)`.
- [ ] Add one negative test: A reconnects with wrong `X-Device-Secret` → WS close code `4401`.

**Acceptance:** `cargo test -p minos-relay --test e2e` green in <2s.

### 13. CI wiring

- [ ] `.github/workflows/ci.yml` — in the existing `rust` job, before `cargo clippy`:
  ```yaml
  - name: Verify sqlx offline metadata
    run: cargo sqlx prepare --check --workspace
  ```
- [ ] Ensure `sqlx-data.json` is listed in `.gitignore` removal (it IS committed).
- [ ] `cargo xtask check-all` locally must still pass — includes the relay's `cargo test`.

**Acceptance:** push a no-op commit; CI green.

### 14. What NOT to do in this plan

- [ ] Do not modify `crates/minos-daemon` — its refactor from server-to-client belongs in plan 05.
- [ ] Do not modify `crates/minos-mobile` — same.
- [ ] Do not modify `crates/minos-transport` beyond **adding** an `AuthHeaders` struct and a client-side constructor that attaches them; do NOT delete or rewire the existing server / client code. Plan 05 consumes the new struct.
- [ ] Do not modify `apps/macos/` or `apps/mobile/`. Plan 05 + 06.
- [ ] Do not ship `docs/ops/cloudflare-tunnel-setup.md` changes; it is already written.
- [ ] Do not invent new spec sections or new ADRs. If the spec is ambiguous, STOP and ask. Document the ambiguity in a comment on the PR.

## Acceptance (plan-level)

All checkboxes above checked, plus:

- [ ] `cargo xtask check-all` green on `feat/relay-backend`.
- [ ] `crates/minos-relay/tests/e2e.rs` green in <2s.
- [ ] Manual smoke: `cargo xtask relay-run` → `curl -i http://127.0.0.1:8787/health` returns `200 OK` with body containing `minos-relay` and its version.
- [ ] Spec §11.4 items 1–3 green; items 4+ are plan-05 scope.
- [ ] No new TODOs left unresolved in the relay source; any deferred item is explicitly listed under "Out of scope" in this plan and referenced by an issue or follow-up plan.

## Out of scope (explicit)

Deferred to plan 05 or later:

- `minos-daemon` and `minos-mobile` refactor to relay client role.
- macOS `Minos.app` and Flutter `Minos` app config flows for Service Tokens.
- QR payload schema change on the Mac UI (from `{host, port, token}` to `{backend_url, token, mac_name}`).
- Skills metadata on `AgentDescriptor`.
- Browser admin console on `/admin`.
- Peer-offline queueing.
- DeviceSecret rotation.
- Production deployment tooling (systemd unit for Linux).
- E2EE of `forward` payloads.
- `cargo deny` coverage audit of the new dependencies beyond what the existing `deny.toml` catches.

## Risks and open questions

Flag any of these that surprise you during implementation:

1. **`X-Device-Role` default.** Spec implies required; plan allows default `IosClient`. Confirm with §7.1 step 2 in the spec: the role is set at app install time and is always present. If spec is followed strictly, reject connections without the header with `4400`. Default here is pragmatic for dev; in prod the clients always set it.
2. **Heartbeat policy on Unpaired sessions.** Should an Unpaired session be timed out faster than a Paired one? Spec does not specify. Pick 60s Unpaired / 90s Paired and log the choice — easy to revisit.
3. **Token GC cadence.** Spec says 60s. Cheap; fine. Watch for test flakiness if the test creates + expires tokens faster than the GC.
4. **Argon2 parameters.** Use `argon2::Argon2::default()` for MVP. Revisit if verification appears as a hotspot in profiling (it won't at our rates).
5. **`sqlx-data.json` churn.** Every schema change requires re-running `cargo sqlx prepare`. Document this in `CONTRIBUTING.md` or a pinned README section so future contributors don't get bitten.

## Handoff back to the user after completion

When the plan is finished:

1. Post a PR on `feat/relay-backend` targeting `main`.
2. In the PR description, list:
   - Link to this plan.
   - Summary of what the plan delivered.
   - Call-out of any decisions that diverged from the spec (should be empty; if non-empty, explain why).
   - Link to the cloudflared runbook; note which step of the runbook has been tested end-to-end (likely only steps 1–6 locally; steps 7–12 require the user's Cloudflare account).
3. Label the PR `relay-backend` and `plan-04` for future traceability.
4. Do not merge. The user reviews, runs the local smoke manually, then approves.

## Reference pointers (single list)

| Kind | Path |
|---|---|
| Spec | `docs/superpowers/specs/minos-relay-backend-design.md` |
| ADR | `docs/adr/0009-broker-architecture-pivot.md` |
| ADR | `docs/adr/0010-cloudflare-tunnel-and-access.md` |
| ADR | `docs/adr/0011-broker-envelope-protocol.md` |
| ADR | `docs/adr/0012-sqlite-via-sqlx.md` |
| Runbook | `docs/ops/cloudflare-tunnel-setup.md` |
| Original MVP spec | `docs/superpowers/specs/minos-architecture-and-mvp-design.md` |
| Existing crates (do not break) | `crates/minos-domain`, `crates/minos-protocol`, `crates/minos-pairing`, `crates/minos-transport`, `crates/minos-daemon`, `crates/minos-mobile`, `crates/minos-cli-detect`, `crates/minos-ffi-uniffi`, `crates/minos-ffi-frb` |
| Branch | `feat/relay-backend` |
