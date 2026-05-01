# Minos · Mobile Account Auth + Agent Session — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-26 |
| Owner | fannnzhang |
| Repository | `github.com/peterich-rs/minos` (public) |
| Proposed branch | `feat/mobile-auth-and-agent-session` (worktree at `../minos-worktrees/mobile-auth-and-agent-session`) |
| Supersedes (partial) | `flutter-app-and-frb-pairing-design.md` §"Tier B" deferrals (auto-reconnect lands here); `mobile-migration-and-ui-protocol-design.md` §2.2 "Mobile sending input" deferral (lifted) |
| Related ADRs | 0001–0013 retained; proposes 0014 (account-auth bearer model) |

---

## 1. Context

Three things shipped in plans 03–05 plus the recent HTTP-control-plane split:

1. The Flutter app at `apps/mobile/` pairs with a Mac through the relay backend, persists pairing in `flutter_secure_storage`, subscribes to `UiEventMessage` events, and renders a flat-list debug viewer. This was done as a deliberately plain Tier A surface to validate the data contract — see `mobile-migration-and-ui-protocol-design.md` §2.4 "UI-per-phase rule".
2. `minos-daemon` exposes the Plan 04 RPC trio (`start_agent`, `send_user_message`, `stop_agent`) over JSON-RPC, dispatched via `Envelope::Forwarded` against the local `jsonrpsee` server. The macOS menu bar drives this trio in debug builds for manual smoke.
3. `minos-backend` authenticates every HTTP and WS request via a device-centric model: `X-Device-Id` (UUID) + `X-Device-Role` + `X-Device-Secret` (argon2id). Pairings are device↔device. Cloudflare Access service tokens front the relay edge.

What has **not** happened yet:

1. **Mobile cannot drive an agent session.** `MobileClient` in `crates/minos-mobile` never sends `Envelope::Forward`; it has no JSON-RPC `id` correlation, no outbound dispatch, no pending-request map. The frb surface exposes pair / list_threads / read_thread / subscribe_* and nothing else. The three agent RPCs exist only on the UniFFI surface for the Mac menu bar.
2. **There is no user-account model.** Identity is per-device; pairings are between two physical devices. There is no notion of "this iPhone and this Mac belong to the same user." This blocks any account-shaped product behavior (multi-device, account-owned Macs, app-store onboarding).
3. **There is no production-grade chat UI.** `ThreadViewPage` renders `UiEventMessage`s as flat `ListTile` rows with no bubbles, no streaming animation, no markdown, no input box, no Stop button, no tool-call rendering. `mobile-migration-and-ui-protocol-design.md` §2.2 explicitly defers all of this to a follow-up spec — this is that spec.
4. **Tier B reliability is missing.** No auto-reconnect loop, no foreground/background lifecycle awareness, no Mac-offline UX surface.

This spec closes those four gaps in one cohesive change. Three concerns are bundled because they share a common bootstrap path (route gates auth → pair → main), a common state-management plane (Riverpod), and a common verification surface (one real-device smoke run validates the whole product). Splitting account auth from agent dispatch would force either a useless intermediate ship state ("can register but can't use codex") or retrofitting auth into a subsequent migration; splitting chat UI from agent dispatch would force a "type a prompt against a flat-list viewer" intermediate that isn't a real product surface.

The product framing — explicitly stated by the project owner — is **slack.ai-style**: an account-based product where the iPhone is the primary client and account ownership is foundational, not optional. This is the key product differentiator from Remodex (`third_party/remodex/`), whose "local-first, device-pair-only" framing does not transfer.

---

## 2. Goals

### 2.1 MVP scope (this spec)

1. **Backend account model.** New tables `accounts`, `refresh_tokens`; existing `devices` table grows an `account_id` foreign key (nullable, populated on iOS device login and on Mac pairing).
2. **Four auth endpoints.** `POST /v1/auth/{register, login, refresh, logout}` with the contract in §5.2. Email + password registration, no email verification, no password reset, no OAuth/SSO, no 2FA.
3. **Bearer token rail.** Access JWT (HS256, 15-minute TTL, `did` claim binds to device) in `Authorization: Bearer`; opaque refresh token (30-day TTL, server-side hashable + revocable) for rotation. Coexists with the existing `X-Device-Secret` rail — does not replace it.
4. **Account-aware WS routing.** Mac → iOS forward path filters by `account_id` rather than walking the raw `pairings` table. iOS → Mac path is unchanged.
5. **Mobile RPC dispatch.** `MobileClient` learns to send `Envelope::Forward`, allocates monotonic JSON-RPC `id`s, maintains a pending oneshot map, matches inbound `Envelope::Forwarded` by id, applies per-RPC timeouts.
6. **Auto-reconnect loop.** Exponential backoff (1 s → 30 s cap), connection-lifecycle aware, integrates token refresh on 401, drains pending requests with `RequestDropped` on disconnect.
7. **Token persistence and lifecycle.** Dart `flutter_secure_storage` is the persistence boundary; Rust holds in-memory tokens and broadcasts `AuthStateFrame` updates over a frb stream that Dart subscribes to and re-persists. App-lifecycle hooks (`WidgetsBindingObserver` → `notify_foregrounded` / `notify_backgrounded`) drive reconnect responsiveness.
8. **frb surface additions.** Nine new methods on `MobileClient` (4 auth + 3 agent + 2 lifecycle), plus mirror types for `AuthSummary`, `AuthStateFrame`, `PersistedAuthState`, `StartAgentRequest`, `StartAgentResponse`. `MinosError` grows variants for the auth and dispatch failure modes.
9. **Riverpod state layer.** New providers `auth_provider`, `active_session_provider`, `secure_storage_provider`, `lifecycle_provider`. `root_route_decision` becomes a function of `(AuthState, ConnectionState, PairingState)`; auth gate has highest priority.
10. **Three new screens, two reworked screens.** New: `LoginPage`, `AccountSettingsSheet`. Reworked: `ThreadViewPage` (Remodex-style chat with bubbles, streaming text, tool-call cards, reasoning sections, sticky input + Send/Stop), `ThreadListPage` (Remodex-style list with new-thread CTA). `PairingPage` keeps current logic with visual alignment to Remodex's onboarding views.
11. **Markdown and syntax-highlighted code blocks** in assistant messages via `flutter_markdown_plus` + `flutter_highlight` (or equivalents).
12. **First-run migration handling.** On Spec 1 deploy, all existing dev installs (production has no users yet) are forced to clear local pairing on first launch and route to the login page; documented in §11.
13. **Tooling.** `cargo xtask check-all` keeps passing; new sqlx queries are committed via `cargo sqlx prepare`; `MINOS_JWT_SECRET` env var is required for `minos-backend` startup (panic-on-missing); CI fixture sets a static test value.

### 2.2 Non-goals (explicit deferrals)

- **Multi-device per account.** MVP enforces single-active-iPhone via "new login revokes existing refresh tokens for the account". Multi-device sync (same-account messages mirroring across phones in real time) is deferred. Data model already supports it (Mac is account-owned, not device-owned), but the client-side state-sync layer is not in scope.
- **Email verification, password reset, magic link, OAuth/SSO, 2FA.** Tier B / future spec. The MVP accepts that an attacker who knows a target's email can register first; this is documented and acceptable for a closed early-access user base.
- **APNs push.** Notifications when turns finish or stall. Requires Apple Developer push entitlement + backend APNs integration. Tier B.
- **Power-feature surfaces from Remodex.** Git operations, photo attachments, voice transcription, fast/plan mode toggles, reasoning-effort selector, access-mode switch, hand-off-to-Mac, subagents UI, queue follow-up. None of these ship in Spec 1; several need backend support that doesn't exist yet.
- **Mid-turn interrupt.** Stop in this spec maps to the existing `stop_agent` RPC, which terminates the agent process and closes the thread. Remodex's "interrupt mid-turn without ending session" requires a new backend RPC and is deferred.
- **`MacOnline` / `MacOffline` realtime signals.** Spec 1 detects Mac availability lazily (on screen entry / pull-to-refresh); no proactive backend push. Deferred to Spec 2.
- **Screenshot blocking, app-switcher blur, accessibility, i18n beyond what already exists.** All Tier B.
- **Performance benchmarking, retention pruning of `raw_events`.** No real users yet; revisit when there are.

### 2.3 Testing philosophy (inherited, binding)

Same rule as `mobile-migration-and-ui-protocol-design.md` §2.3 — unit and small integration tests cover **logic only**. Widget tests cover Dart logic that cannot be reached at the unit level (state-machine wiring inside widgets, validation behavior). No XCUITest. The real-device smoke checklist (§9.5) is the sole functional-level gate.

### 2.4 UI-per-phase rule (extension)

This phase replaces the deliberately-plain debug viewer from `mobile-migration-and-ui-protocol-design.md` with a production-grade chat surface. Future power-feature specs are free to add toggles and side-sheets to the same `ThreadViewPage`, but the bubble + streaming + input-bar core lands here and is not expected to churn.

---

## 3. Tech Stack and Defaults

Inherits from `minos-relay-backend-design.md` §3, `mobile-migration-and-ui-protocol-design.md` §3, and `flutter-app-and-frb-pairing-design.md` §3. Deltas:

| Area | Change |
|---|---|
| `minos-backend` | New deps: `jsonwebtoken`, `argon2` (already used in `pairing/secret.rs` — confirm version match), `tower-governor` for rate limit. New env var: `MINOS_JWT_SECRET` (required, ≥32 bytes). |
| `minos-mobile` | New internal modules: `auth`, `rpc`, `reconnect`. No new external deps; uses workspace `tokio`, `serde`, `chrono`. |
| `minos-protocol` | `MinosError` grows variants (§6.5); `StartAgentRequest`, `StartAgentResponse`, `SendUserMessageRequest`, `StopAgentRequest` already exist (per Plan 04); just need frb mirroring. |
| `minos-ffi-frb` | Adds 9 methods + 3 mirror structs + 1 mirror enum on `MobileClient`. No new deps. |
| Flutter | New pubs: `flutter_markdown_plus ^1` (or `gpt_markdown`, see §7.6 for selection criterion), `flutter_highlight ^0.7`. `flutter_secure_storage` already present. |
| CI | `cargo xtask check-all` adds: `MINOS_JWT_SECRET` set in fixture, sqlx prepare regen for new queries, Flutter test for new test files. Migration up/down smoke covered by existing `#[sqlx::test]` fixtures. |

---

## 4. Architecture Overview

```
iPhone (Flutter app)                Backend (minos-relay)              Mac (minos-daemon)
┌────────────────────┐              ┌─────────────────────┐            ┌──────────────────┐
│ Riverpod state     │              │ HTTP /v1/auth/*     │            │ jsonrpsee server │
│  - auth            │  HTTP+JWT    │  (new)              │            │  - start_agent   │
│  - pairing         │ ───────────► │ HTTP /v1/pairing/*  │            │  - send_user_msg │
│  - active session  │              │  (+account_id)      │            │  - stop_agent    │
│  - thread/messages │              │ HTTP /v1/threads/*  │ Forward    │  - pair / health │
│                    │  WS+JWT      │  (existing)         │ ◄────────► │                  │
│ frb (Rust)         │ ◄──────────► │                     │ Envelope   │ codex app-server │
│  - MobileClient    │  Envelope    │ WS: account-routed  │            │  adapter         │
│  - id correlation  │              │  - Forward dispatch │            │                  │
│  - reconnect       │              │  - UiEvent fan-out  │            │ CF Access auth   │
│  - secure storage  │              │ DB: accounts +      │            │ (unchanged)      │
│                    │              │  refresh_tokens +   │            │                  │
│                    │              │  pairing.account_id │            │                  │
└────────────────────┘              └─────────────────────┘            └──────────────────┘
```

Key invariants:

- The Mac daemon is **untouched** in this spec. All three Plan 04 RPCs already work end-to-end against `Envelope::Forward` dispatch — what changes is who sends those forwards (the iPhone, now) and how the backend routes them (by account, now).
- The agent → mobile event stream (`Envelope::Ingest` from Mac → backend persists + translates → `Envelope::Event::UiEventMessage` to mobile) is **untouched** as a wire path. What changes is the routing target on the backend side (account-aware fan-out) and the consumer on the mobile side (production chat UI instead of debug viewer).
- The existing device-centric auth (`X-Device-Id` + `X-Device-Secret`) **stays**. Bearer JWT is **layered on top** for iOS clients only. Mac daemon continues to use device-secret + CF Access; it does not log in.

### 4.1 End-to-end data flow: phone starts a codex session

```
1. User submits prompt on ThreadViewPage input bar.

2. ActiveSessionController.start(prompt, agent, cwd?)
   → frb call: MobileClient.start_agent(StartAgentRequest)

3. Rust MobileClient
   - allocates JSON-RPC id N from monotonic counter
   - constructs Envelope::Forward { id: N, method: "minos_start_agent", params }
   - sends over the existing WS write half
   - inserts (id N → oneshot::Sender) into pending map
   - returns Future<StartAgentResponse> awaiting the oneshot

4. Backend
   - parses Envelope::Forward
   - extracts account_id from the WS bearer auth context
   - SELECT mac_device FROM pairings JOIN devices ON … WHERE account_id = $1
   - forwards the Envelope to the resolved Mac daemon's WS session

5. Mac daemon (relay_client.rs:535, existing logic — no change)
   - parses Forwarded payload, dispatches into local jsonrpsee server
   - jsonrpsee invokes minos_start_agent → minos-agent-runtime → spawns codex app-server
   - thread_id is allocated, response goes back as Envelope::Forwarded { id: N, result }

6. Backend
   - routes the reply back to the same iPhone WS session that sent the original

7. Rust MobileClient
   - matches id N → removes pending entry → fires oneshot with the result

8. Dart
   - frb Future resolves with StartAgentResponse
   - ActiveSessionController transitions to SessionStreaming(thread_id, agent)
   - UI swaps Send button for Stop, shows "Agent thinking…" until first TextDelta

9. Codex begins emitting events — this path is unchanged from Plan 05:
   Mac codex → minos-agent-runtime → daemon Envelope::Ingest → backend persists into raw_events,
   translates via minos-ui-protocol::translate_codex into UiEventMessage,
   broadcasts as Envelope::Event::UiEventMessage to all of the account's connected iPhones.
   MobileClient.ui_events_stream broadcasts to the Dart subscriber, ActiveSessionController
   filters by thread_id, Riverpod renders deltas into the chat surface.

10. User taps Stop → MobileClient.stop_agent → same forward path → thread closes.
    User types follow-up prompt → MobileClient.send_user_message → same forward path → next turn.
```

The forward path is symmetric in structure (id N out → reply N back), and the existing UiEvent fan-out continues to deliver the streamed agent output. The new dispatch half is the only fundamental wire-protocol consumer added.

---

## 5. Backend Layer

### 5.1 Database migrations

Three new migrations under `crates/minos-backend/migrations/`:

**`0007_accounts.sql`**
```sql
CREATE TABLE accounts (
    account_id     TEXT PRIMARY KEY,             -- UUIDv4
    email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash  TEXT NOT NULL,                -- argon2id; reuses pairing/secret.rs hasher
    created_at     INTEGER NOT NULL,             -- unix epoch ms
    last_login_at  INTEGER
) STRICT;
CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
```

**`0008_refresh_tokens.sql`**
```sql
CREATE TABLE refresh_tokens (
    token_hash     TEXT PRIMARY KEY,             -- SHA-256 hex of plaintext (32B random → 64 hex)
    account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    revoked_at     INTEGER                       -- NULL = active
) STRICT;
CREATE INDEX idx_refresh_tokens_account ON refresh_tokens(account_id) WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_tokens_device ON refresh_tokens(device_id) WHERE revoked_at IS NULL;
```

**`0009_devices_account_link.sql`**
```sql
ALTER TABLE devices ADD COLUMN account_id TEXT REFERENCES accounts(account_id);
CREATE INDEX idx_devices_account ON devices(account_id) WHERE account_id IS NOT NULL;
```

Semantics:

- iOS device row gets `account_id` set on first successful login. A subsequent login on the same device with a different email overwrites this column (account switch); the prior account's refresh tokens for this device are revoked at `/v1/auth/login` time.
- agent-host (Mac) device row gets `account_id` populated during `POST /v1/pairing/consume` — the iOS client (already bearer-authenticated) is the call's principal, and the consume handler copies that principal's `account_id` onto the Mac device row.
- The existing `pairings` table is **not modified**. Account ownership is queried through a join: `devices.account_id`. This avoids breaking the undirected `(device_a, device_b)` invariant and keeps the schema delta tight.

### 5.2 Auth endpoints

All four endpoints require `X-Device-Id` (and the existing `X-Device-Secret` if the device row has a hash) — the device identity rail is independent of the user identity rail. Bearer tokens are **issued by** auth endpoints, not consumed by them (except logout).

| Method | Path | Body | Response | Status codes |
|---|---|---|---|---|
| POST | `/v1/auth/register` | `{ "email": "<rfc>", "password": "<min8>" }` | `{ "account": { "account_id", "email" }, "access_token", "refresh_token", "expires_in": 900 }` | 200 / 400 weak / 409 taken / 429 limited |
| POST | `/v1/auth/login` | `{ "email", "password" }` | same shape as register | 200 / 401 invalid / 429 limited |
| POST | `/v1/auth/refresh` | `{ "refresh_token" }` | `{ "access_token", "refresh_token", "expires_in": 900 }` | 200 / 401 |
| POST | `/v1/auth/logout` | `{ "refresh_token" }` | (empty) | 204 |

Behaviors:

- **Register.** Argon2id hash the password with the workspace's existing parameters, insert into `accounts`, write `device.account_id`, mint a JWT + refresh token pair, return both. Rate limit: per-IP 3/hr.
- **Login.** Verify password; on success, **revoke all existing refresh_tokens for that account** (per §1 single-active-iPhone strategy from the brainstorm); **forcibly close any WS sessions belonging to this account on devices other than the requesting one** via `session_registry.close_account_sessions(account_id, except: requesting_device_id)` so that the prior device experiences an immediate disconnect (its reconnect attempt then fails refresh and routes to login within ~2 s); update `device.account_id` (overwriting any prior account on this device); mint and return new tokens. Rate limit: per-IP 5/min and per-email 10/min, whichever fires first.
- **Refresh.** Look up `token_hash`, verify not revoked + not expired + matches `(account_id, device_id)` from the request's auth context, mint new pair, **revoke old refresh** (rotation). Rate limit: per-account 60/hr.
- **Logout.** Mark the supplied refresh token's `revoked_at`. Other refresh tokens for the same account (if multi-device were enabled) are not affected. Logout requires `Authorization: Bearer` of an active access token from the same account (defense-in-depth).

Email is normalized to lowercase before lookup (the `COLLATE NOCASE` index makes case-insensitive uniqueness server-enforced).

### 5.3 JWT shape and signing

- **Algorithm:** HS256 (single backend instance; no need for asymmetric).
- **Signing key:** `MINOS_JWT_SECRET` env var, ≥32 bytes, panic-on-missing at startup. CI fixture sets a deterministic test value.
- **TTL:** access 900 s (15 min); refresh 30 d.
- **Claims:**
  ```json
  {
    "sub": "<account_id>",
    "did": "<device_id>",
    "iat": <unix_seconds>,
    "exp": <unix_seconds>,
    "jti": "<uuidv4>"
  }
  ```
- **Verification.** On every protected request: parse, verify signature, check `exp`, check `did == X-Device-Id`. The `did` binding prevents an access token from being replayed by a different device that knows a victim's device-id.

### 5.4 Auth middleware coexistence

> **Superseded 2026-05-01 by [ADR-0020](../../adr/0020-server-centric-auth-and-account-pairs.md).** iOS rail no longer requires `X-Device-Secret`; bearer alone authenticates iOS. Mac rail (CF Access + device-secret) is unchanged. The table below documents the historical dual-rail design.


A new module `crates/minos-backend/src/auth/bearer.rs` extracts `Authorization: Bearer` and verifies the JWT. It coexists with the existing `crates/minos-backend/src/http/auth.rs` (device-secret) — different routes need different rails:

| Route | Device rail | Bearer rail | Notes |
|---|---|---|---|
| `/v1/auth/register` | `X-Device-Id` (+ optional secret) | — | Bearer not yet issued |
| `/v1/auth/login` | `X-Device-Id` (+ optional secret) | — | same |
| `/v1/auth/refresh` | `X-Device-Id` | — | refresh token in body authenticates |
| `/v1/auth/logout` | `X-Device-Id` + secret | required | double-authenticate |
| `/v1/pairing/consume` (iOS) | `X-Device-Id` + secret | required | account_id copied to pairing's Mac side |
| `/v1/pairing/consume` (Mac) | `X-Device-Id` + CF Access | — | unchanged |
| `/v1/threads/*` (iOS) | `X-Device-Id` + secret | required | thread queries filtered by account |
| `GET /devices` (WS upgrade, iOS) | existing | required | see §5.5 |
| `GET /devices` (WS upgrade, Mac) | existing | — | unchanged |

The middleware returns `AccountAuthOutcome { account_id: AccountId, device_id: DeviceId }` on success; handlers consume both. Failure paths produce a `401` with an opaque `WWW-Authenticate: Bearer` header.

### 5.5 WS routing change

The forward dispatch path lives in `crates/minos-backend/src/session/registry.rs`. Today, when a Mac sends `Envelope::Forward` (in the agent → ingest case) or receives one (in the iOS → daemon case), the routing logic walks `pairings` to find the partner device.

The change is in **one direction only** — Mac → iOS reply routing. iOS → Mac forward routing stays as-is (one Mac per pairing today; account-aware lookup gives the same answer).

```sql
-- Reply routing query: find the iOS device(s) of the account this Mac belongs to,
-- restricted to currently-online WS sessions.
SELECT ios.device_id
FROM pairings p
JOIN devices ios ON ios.device_id = (
    CASE WHEN p.device_a = $mac THEN p.device_b ELSE p.device_a END
)
JOIN devices mac ON mac.device_id = $mac
WHERE (p.device_a = $mac OR p.device_b = $mac)
  AND ios.account_id = mac.account_id
  AND ios.role = 'ios-client'
LIMIT 1;
```

Under the single-active-iPhone constraint (§1), at most one row matches.

The `Envelope::Forward` wire shape itself is **unchanged** — pure server-side routing logic.

The session registry also gains one helper used by login (§5.2):

```rust
pub fn close_account_sessions(&self, account_id: AccountId, except: DeviceId) -> usize;
// closes (drops the WS write half of) every currently-online session whose
// device.account_id == $account_id and device_id != $except;
// returns the count closed (used in tracing).
```

### 5.6 Rate limiting and security

- `tower-governor` (or a hand-rolled in-memory token bucket if `governor` import overhead is undesirable) with per-IP and per-email keys.
- TLS is enforced at the Cloudflare edge (existing); backend does not redirect.
- Argon2id parameters for password hashing reuse the workspace's existing `pairing/secret.rs` settings (`m=19456, t=2, p=1`), already proven against timing attacks in CI.
- Refresh tokens are stored as SHA-256 hex digests (deterministic for PK lookup); plaintext is only ever in transit. Same scheme as `pairing_tokens`.
- Logout / refresh both write `revoked_at`; verification queries always filter `WHERE revoked_at IS NULL`.

### 5.7 Files

```
crates/minos-backend/migrations/0007_accounts.sql              new
crates/minos-backend/migrations/0008_refresh_tokens.sql        new
crates/minos-backend/migrations/0009_devices_account_link.sql  new
crates/minos-backend/src/store/accounts.rs                     new
crates/minos-backend/src/store/refresh_tokens.rs               new
crates/minos-backend/src/auth/mod.rs                           new
crates/minos-backend/src/auth/bearer.rs                        new (JWT extract + verify)
crates/minos-backend/src/auth/jwt.rs                           new (sign + verify helpers)
crates/minos-backend/src/auth/passwords.rs                     new (argon2id wrapper)
crates/minos-backend/src/http/v1/auth.rs                       new (4 endpoints)
crates/minos-backend/src/http/v1/mod.rs                        modify (register router)
crates/minos-backend/src/http/v1/pairing.rs                    modify (write account_id on consume)
crates/minos-backend/src/http/v1/threads.rs                    modify (bearer gate + account filter)
crates/minos-backend/src/http/ws_devices.rs                    modify (iOS WS bearer gate)
crates/minos-backend/src/session/registry.rs                   modify (account-aware Mac→iOS routing)
crates/minos-backend/src/config.rs                             modify (jwt_secret field)
crates/minos-backend/Cargo.toml                                modify (jsonwebtoken, governor)
```

---

## 6. Rust Core (`minos-mobile`) and frb Layer

### 6.1 `MobileClient` internal state additions

```rust
pub struct MobileClient {
    // existing: connection_state_tx, ui_events_tx, store, device_id, …

    // new
    auth: Arc<RwLock<Option<AuthSession>>>,
    pending: Arc<DashMap<u64, oneshot::Sender<RpcReply>>>,
    next_id: Arc<AtomicU64>,
    ws_outbound: mpsc::UnboundedSender<WsOutFrame>,   // write-half handle paired with existing read loop
    reconnect: Arc<ReconnectController>,
    auth_state_tx: watch::Sender<AuthStateFrame>,
}

struct AuthSession {
    access_token: String,
    access_expires_at: Instant,
    refresh_token: String,
    account: AuthSummary,
}

pub struct AuthSummary {
    pub account_id: String,
    pub email: String,
}
```

### 6.2 Outbound dispatch and id correlation

A new module `crates/minos-mobile/src/rpc.rs` owns the dispatch primitive:

```rust
pub(crate) async fn forward_rpc<P: Serialize, R: DeserializeOwned>(
    client: &MobileClient,
    method: &str,
    params: P,
    timeout: Duration,
) -> Result<R, MinosError> {
    let id = client.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    client.pending.insert(id, tx);

    let envelope = Envelope::Forward { id, method: method.into(), params: serde_json::to_value(params)? };
    client.ws_outbound
        .send(WsOutFrame::Text(serde_json::to_string(&envelope)?))
        .map_err(|_| MinosError::not_connected())?;

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(RpcReply::Ok(v))) => serde_json::from_value(v).map_err(Into::into),
        Ok(Ok(RpcReply::Err(e))) => Err(e.into()),
        Ok(Err(_recv_error)) => Err(MinosError::request_dropped()),  // pending was drained on disconnect
        Err(_elapsed) => {
            client.pending.remove(&id);
            Err(MinosError::timeout())
        }
    }
}
```

The inbound match arm is added to `crates/minos-mobile/src/client.rs::handle_text_frame`:

```rust
EnvelopeKind::Forwarded { id, result } => {
    if let Some((_, tx)) = self.pending.remove(&id) {
        let _ = tx.send(result);  // tx send-failure means caller dropped — no harm
    } else {
        tracing::debug!(id, "Forwarded with no pending entry — late or duplicate, dropping");
    }
}
```

Per-RPC timeouts (single source in `rpc.rs`):

| RPC | Timeout |
|---|---|
| `start_agent` | 60 s (codex spawn is slow on first call) |
| `send_user_message` | 10 s |
| `stop_agent` | 10 s |

### 6.3 Auto-reconnect controller

`crates/minos-mobile/src/reconnect.rs`:

```rust
pub(crate) struct ReconnectController {
    state: RwLock<State>,
}
struct State {
    delay: Duration,                  // 1 s start, ×2 each failure, cap 30 s
    consecutive_failures: u32,
    last_connected_at: Option<Instant>,
    foreground: bool,
}
```

Algorithm:

1. On disconnect: drain `pending` map (each pending `oneshot::Sender` receives `RpcReply::Err(RequestDropped)`); transition `connection_state` to `Disconnected`; schedule reconnect after `state.delay`.
2. Before each reconnect attempt: check `auth.access_expires_at`; if `< now + 2 min`, attempt `POST /v1/auth/refresh` first. On 401 → `auth_state_tx.send(RefreshFailed(err))`, exit reconnect loop, do not auto-retry (Dart routes to login).
3. WS upgrade with `Authorization: Bearer <access>` + existing `X-Device-*` headers.
4. On success: emit `Connected`; if `last_connected_at.elapsed() > 60 s` reset `delay` to 1 s; set `consecutive_failures = 0`.
5. On `notify_foregrounded()`: reset `delay` to 1 s and trigger an immediate reconnect attempt if disconnected.
6. On `notify_backgrounded()`: pause the reconnect loop (cancel pending timers); preserve in-memory state.

### 6.4 Token persistence and lifecycle

The persistence boundary is **Dart**, not Rust. Rust holds tokens in memory; Dart owns `flutter_secure_storage`. The handoff is via the `subscribe_auth_state` frb stream:

```
App start (Dart):
  1. flutter_secure_storage.read("auth") → Option<PersistedAuthState>
  2. MobileClient::new_with_persisted_state(pairing, persisted_auth)
  3. Rust hydrates AuthSession in-memory (no auto-refresh; defer to first network call)
  4. Dart subscribes to subscribe_auth_state stream

On register / login success (Rust → Dart):
  - Rust executes HTTP, stores AuthSession in-memory
  - Rust auth_state_tx.send(Authenticated(summary))
  - Dart receives, persists PersistedAuthState to secure storage

On refresh (Rust → Dart):
  - Same path; new tokens replace old in-memory and in storage

On logout (Rust → Dart):
  - Rust executes HTTP, clears AuthSession
  - Rust auth_state_tx.send(Unauthenticated)
  - Dart receives, deletes secure storage entry
```

**WS connect lifetime is gated on AuthState.** While `AuthUnauthenticated` (or hydrating with no persisted auth), `MobileClient` does **not** attempt to open the WS. On transition to `Authenticated`, the reconnect controller starts and performs the initial connect. On transition to `Unauthenticated` or `RefreshFailed`, the WS is torn down, pending requests drained with `RequestDropped`, and the reconnect loop is halted.

Proactive refresh: a background `tokio::spawn` task polls `auth.access_expires_at` once per minute; if `< now + 2 min` it calls refresh.

### 6.5 frb additions

In `crates/minos-ffi-frb/src/api/minos.rs`:

```rust
impl MobileClient {
    // existing methods preserved

    pub async fn register(&self, email: String, password: String) -> Result<AuthSummary, MinosError>;
    pub async fn login(&self, email: String, password: String) -> Result<AuthSummary, MinosError>;
    pub async fn refresh_session(&self) -> Result<(), MinosError>;
    pub async fn logout(&self) -> Result<(), MinosError>;

    pub fn subscribe_auth_state(&self) -> StreamSink<AuthStateFrame>;
    pub fn persisted_auth_state(&self) -> Option<PersistedAuthState>;

    pub async fn start_agent(
        &self,
        agent: AgentName,
        thread_id: Option<String>,    // None = new thread
        prompt: String,
        cwd: Option<String>,           // None = backend default ~/codex-workspace
    ) -> Result<StartAgentResponse, MinosError>;

    pub async fn send_user_message(&self, thread_id: String, text: String) -> Result<(), MinosError>;
    pub async fn stop_agent(&self, thread_id: String) -> Result<(), MinosError>;

    pub fn notify_foregrounded(&self);
    pub fn notify_backgrounded(&self);
}
```

Mirror types:

```rust
#[frb(mirror(AuthSummary))]
pub struct _AuthSummary { pub account_id: String, pub email: String }

#[frb(mirror(AuthStateFrame))]
pub enum _AuthStateFrame {
    Unauthenticated,
    Authenticated(AuthSummary),
    Refreshing,
    RefreshFailed(MinosError),
}

#[frb(mirror(PersistedAuthState))]
pub struct _PersistedAuthState {
    pub access_token: String,
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
    pub account: AuthSummary,
}

// StartAgentRequest / StartAgentResponse / SendUserMessageRequest / StopAgentRequest
// already exist in minos-protocol; add #[frb(mirror)] for each.
```

`MinosError` (in `minos-protocol::error`) gains variants:

```rust
pub enum ErrorKind {
    // existing variants preserved
    InvalidCredentials,
    EmailTaken,
    WeakPassword,
    RateLimited { retry_after_s: u32 },
    Unauthorized,
    AuthRefreshFailed,
    NotConnected,
    RequestDropped,
    Timeout,
    NotPaired,
    MacOffline,
    InvalidQrPayload,
    PairingTokenExpired,
    AgentStartFailed { reason: String },
    AgentNotRunning,
}
```

### 6.6 HTTP client adjustments

`crates/minos-mobile/src/http.rs` wraps every outgoing HTTP call in an `AuthAwareClient` that:

- injects `Authorization: Bearer <access>` if `AuthSession.is_some()`;
- retries once on a 401 by triggering `refresh_session()`;
- on a second 401 emits `AuthStateFrame::RefreshFailed` and surfaces `MinosError::AuthRefreshFailed` to the caller;
- preserves the existing `X-Device-*` header injection.

The four auth endpoints are added as methods on `AuthAwareClient` next to the existing pairing/threads endpoints.

### 6.7 Files

```
crates/minos-mobile/src/auth.rs                    new (AuthSession, AuthSummary, AuthStateFrame)
crates/minos-mobile/src/rpc.rs                     new (forward_rpc + WsOutFrame + RpcReply)
crates/minos-mobile/src/reconnect.rs               new (ReconnectController)
crates/minos-mobile/src/http.rs                    modify (AuthAwareClient + 4 auth endpoints)
crates/minos-mobile/src/client.rs                  modify (wire auth/rpc/reconnect, lifecycle hooks, Forwarded arm)
crates/minos-mobile/src/store.rs                   modify (PersistedAuthState shape)
crates/minos-mobile/src/lib.rs                     modify (re-exports)
crates/minos-protocol/src/error.rs                 modify (new ErrorKind variants)
crates/minos-ffi-frb/src/api/minos.rs              modify (9 new methods, 3 mirror structs, 1 mirror enum)
flutter_rust_bridge.yaml                           possibly modify (codegen hints)
```

---

## 7. Flutter / Dart Layer

### 7.1 Domain layer (sealed states)

`apps/mobile/lib/domain/auth_state.dart`:
```dart
sealed class AuthState { const AuthState(); }
class AuthUnauthenticated extends AuthState { const AuthUnauthenticated(); }
class AuthAuthenticated extends AuthState {
  final AccountSummary account;
  const AuthAuthenticated(this.account);
}
class AuthRefreshing extends AuthState { const AuthRefreshing(); }
class AuthRefreshFailed extends AuthState {
  final MinosError error;
  const AuthRefreshFailed(this.error);
}
```

`apps/mobile/lib/domain/active_session.dart`:
```dart
sealed class ActiveSession { const ActiveSession(); }
class SessionIdle extends ActiveSession { const SessionIdle(); }
class SessionStarting extends ActiveSession {
  final String prompt;
  const SessionStarting(this.prompt);
}
class SessionStreaming extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionStreaming(this.threadId, this.agent);
}
class SessionAwaitingInput extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionAwaitingInput(this.threadId, this.agent);
}
class SessionStopped extends ActiveSession {
  final String threadId;
  const SessionStopped(this.threadId);
}
class SessionError extends ActiveSession {
  final String? threadId;
  final MinosError error;
  const SessionError(this.threadId, this.error);
}
```

### 7.2 ActiveSession state machine

```
Idle ──start()──► Starting ──RPC reply──► Streaming ──UiEvent.MessageCompleted──► AwaitingInput
                              │                                                       │
                              └─── RPC fail ──► Error                          send()──┘
                                                                                      │
                                                      ──RPC reply──► Streaming
                                                                                      │
                                                          ──MessageCompleted──► AwaitingInput

(any of Streaming / AwaitingInput) ──stop()──► Stopped (then Idle on user navigation away or new thread)
(any state) ──ConnectionState.disconnected──► Error(network) (user retries after reconnect)
```

The controller subscribes to `MobileClient.subscribe_ui_events`, filters by current `thread_id`, and advances the state machine on `MessageCompleted` / `ThreadClosed` / `Error` UiEvent variants.

### 7.3 Riverpod providers

```
application/auth_provider.dart                  AsyncNotifier<AuthState>
                                                - bootstrap reads secure storage → frb hydrate → subscribe
                                                - register / login / logout methods
                                                - listens to subscribe_auth_state, persists on each
application/active_session_provider.dart        Notifier<ActiveSession> per-thread
                                                - start / send / stop methods
                                                - state machine transitions on UiEvents
application/secure_storage_provider.dart        Provider<AuthSecureStorage>
application/lifecycle_provider.dart             Provider<AppLifecycleObserver>
application/root_route_decision.dart            modify
application/minos_providers.dart                modify (bootstrap order: secure storage → auth → core)
application/thread_events_provider.dart         modify (expose thread-scoped UiEvent stream)
```

### 7.4 Routing gate

```dart
sealed class RootRoute {}
class RouteSplash extends RootRoute {}                        // while auth is bootstrapping
class RouteLogin extends RootRoute { final MinosError? errorBanner; }
class RoutePairing extends RootRoute {}
class RouteThreadList extends RootRoute { final bool macOffline; }

RootRoute decide(AuthState a, ConnectionState c, PairingState p) => switch (a) {
  AuthUnauthenticated()         => RouteLogin(errorBanner: null),
  AuthRefreshFailed(:final e)   => RouteLogin(errorBanner: e),
  AuthRefreshing()              => RouteSplash(),
  AuthAuthenticated() when p.isUnpaired => RoutePairing(),
  AuthAuthenticated()           => RouteThreadList(macOffline: !c.isConnected),
};
```

Auth state has highest priority; pairing second; main flow last.

### 7.5 Screen inventory and Remodex mapping

| Spec 1 screen | Remodex counterpart | Status |
|---|---|---|
| `LoginPage` | none (Remodex has no accounts) | new |
| `PairingPage` | `Views/Onboarding/*` | modify (visual alignment, logic preserved) |
| `ThreadListPage` | sidebar / thread list views | modify (visual rework) |
| `ThreadViewPage` | `Views/Timeline/*` main chat | modify (full rework — see §7.6) |
| `AccountSettingsSheet` | `Views/Settings/*` (subset) | new — email + logout + version only |

Explicitly **not** in Spec 1: git ops, photo attachments, voice transcription, reasoning-effort selector, access-mode switch, fast/plan mode toggles, subagents, hand-off-to-Mac, in-app notifications.

### 7.6 ThreadViewPage widget breakdown

```
presentation/widgets/chat/
  message_bubble.dart        user/assistant differentiation, left/right alignment, avatar slot
  streaming_text.dart        consumes UiEvent.TextDelta, animated cursor while streaming
  reasoning_section.dart     collapsible, consumes UiEvent.ReasoningDelta
  tool_call_card.dart        ToolCallPlaced/Completed, default-collapsed, expandable args/result view
  input_bar.dart             sticky bottom, Send/Stop button gated on ActiveSession state
  message_meta_row.dart      timestamp, model name (small caption row)
```

Markdown and syntax highlighting: `flutter_markdown_plus ^1` is the default pick. `gpt_markdown ^1` is an alternative optimized for streaming partial markdown — to be evaluated in the implementation plan based on whether `flutter_markdown_plus` produces visual artifacts during streaming reflows. Decision deferred to plan phase; both packages have similar API surface.

Scroll behavior: when a new message arrives, auto-scroll only if the user is within ~120 px of the bottom; otherwise preserve scroll position and show a "↓ N new" floating button.

### 7.7 LoginPage widget breakdown

```
presentation/widgets/auth/
  auth_form.dart             email + password + (mode == register ? confirm_password : null) fields,
                             primary submit button, mode toggle (Tab or Segmented control)
  auth_error_banner.dart     red banner, 6 s auto-dismiss, maps MinosError variants to localized strings
```

Validation:
- email: `^[^\s@]+@[^\s@]+\.[^\s@]+$` regex (intentionally not full RFC 5322).
- password: client min 8, server re-validates.
- register: confirm matches password.
- Submit button disables + shows spinner during in-flight call.

### 7.8 Lifecycle integration

```dart
class _MinosAppLifecycle extends ConsumerStatefulWidget { ... }
class _MinosAppLifecycleState extends ConsumerState with WidgetsBindingObserver {
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final core = ref.read(minosCoreProvider);
    switch (state) {
      case AppLifecycleState.resumed: core.notifyForegrounded();
      case AppLifecycleState.paused:  core.notifyBackgrounded();
      default: break;
    }
  }
}
```

Wrapped around the `MaterialApp` in `presentation/app.dart`.

### 7.9 Dependency additions

`apps/mobile/pubspec.yaml`:
```yaml
dependencies:
  flutter_markdown_plus: ^1
  flutter_highlight: ^0.7
  # gpt_markdown: ^1   # alternative; decision in plan phase
  # flutter_secure_storage: ^9  — already present
```

### 7.10 Files

```
apps/mobile/lib/domain/auth_state.dart                              new
apps/mobile/lib/domain/active_session.dart                          new
apps/mobile/lib/domain/account.dart                                 new (AccountSummary)
apps/mobile/lib/application/auth_provider.dart                      new
apps/mobile/lib/application/active_session_provider.dart            new
apps/mobile/lib/application/secure_storage_provider.dart            new
apps/mobile/lib/application/lifecycle_provider.dart                 new
apps/mobile/lib/application/root_route_decision.dart                modify
apps/mobile/lib/application/minos_providers.dart                    modify
apps/mobile/lib/application/thread_events_provider.dart             modify
apps/mobile/lib/infrastructure/auth_secure_storage.dart             new
apps/mobile/lib/infrastructure/minos_core.dart                      modify
apps/mobile/lib/presentation/app.dart                               modify
apps/mobile/lib/presentation/pages/login_page.dart                  new
apps/mobile/lib/presentation/pages/account_settings_page.dart       new
apps/mobile/lib/presentation/pages/pairing_page.dart                modify
apps/mobile/lib/presentation/pages/thread_list_page.dart            modify
apps/mobile/lib/presentation/pages/thread_view_page.dart            modify
apps/mobile/lib/presentation/widgets/auth/auth_form.dart            new
apps/mobile/lib/presentation/widgets/auth/auth_error_banner.dart    new
apps/mobile/lib/presentation/widgets/chat/message_bubble.dart       new
apps/mobile/lib/presentation/widgets/chat/streaming_text.dart       new
apps/mobile/lib/presentation/widgets/chat/reasoning_section.dart    new
apps/mobile/lib/presentation/widgets/chat/tool_call_card.dart       new
apps/mobile/lib/presentation/widgets/chat/input_bar.dart            new
apps/mobile/lib/presentation/widgets/chat/message_meta_row.dart     new
apps/mobile/pubspec.yaml                                            modify
```

---

## 8. Error Handling

### 8.1 Auth error matrix

| Scenario | Server response | Rust → Dart | UI behavior |
|---|---|---|---|
| Login wrong password | 401 `{kind:"invalid_credentials"}` | `InvalidCredentials` | Login page red banner |
| Register email exists | 409 `{kind:"email_taken"}` | `EmailTaken` | Banner + auto-switch to login mode |
| Password <8 (server) | 400 `{kind:"weak_password"}` | `WeakPassword` | Banner |
| Rate limited | 429 + `Retry-After: <s>` | `RateLimited { retry_after_s }` | Submit disabled with countdown |
| Access expired (HTTP) | 401 | Rust silent refresh + retry once | User-invisible |
| Refresh revoked (single-device-strategy kicked in) | refresh 401 | `AuthRefreshFailed` | Route to login + banner "另一台设备登录,请重新登录" |
| Refresh network failure | network error | 3 backoff retries → `AuthRefreshFailed` | Same, banner "会话恢复失败" |
| Bootstrap with stale refresh | refresh 401 | `AuthRefreshFailed` | Route to login |
| Account switch on same device | new login revokes prior account's tokens | `Authenticated(newAccount)` | Riverpod invalidates prior account's caches + clears local pairing if `device.account_id` changed |
| WS handshake 401 | (upgrade fails) | `reconnect.rs` triggers refresh + retry; if refresh also 401 → `AuthRefreshFailed` | Same as refresh-revoked path |

### 8.2 Connection errors

| Scenario | Behavior |
|---|---|
| Bootstrap with no network | Splash → bootstrap fails → "no network" page + retry |
| WS disconnects mid-session | Silent reconnect with backoff; UI shows reconnect-banner; if reconnect within 5 s, banner does not appear |
| Reconnect fails for ≥1 min | Banner upgrades to "无法连接服务器"; reconnect loop continues; user is **not** logged out |
| Mac offline (WS up, but no online Mac for account) | ThreadList top banner "Mac 已离线"; ThreadView input disabled with hint |
| Send while Mac offline | RPC short-circuits to `MacOffline` error; SessionError card shown |
| Mac comes back online | MVP: detected lazily on next pull-to-refresh / new RPC. No proactive realtime signal (deferred to Spec 2) |

### 8.3 RPC / agent errors

| Scenario | Rust → Dart | UI behavior |
|---|---|---|
| `start_agent` codex CLI missing | `AgentStartFailed { reason: "codex CLI not found" }` | SessionError card, no auto-retry |
| `start_agent` invalid cwd | `AgentStartFailed { reason: "invalid cwd" }` | Same. MVP cwd default `~/codex-workspace`; no UI to override |
| `start_agent` 60 s timeout | `Timeout` | SessionError + Retry button |
| `send_user_message` while Streaming/Starting | UI button is disabled; backend safety net: returns `AgentNotRunning` | Should not happen at the UI level |
| `stop_agent` already stopped | Backend idempotent OK | UI ignores |
| Pending `start_agent` on disconnect | `RequestDropped` | SessionError ("连接中断,请重试") |
| UiEvent gap on reconnect | Frame seq vs `last_seq` mismatch → `read_thread(thread_id, since_seq)` to backfill | Self-healing, user-invisible |

### 8.4 Pairing errors

| Scenario | Behavior |
|---|---|
| Invalid QR JSON | Toast on pairing page; scanner continues |
| Pairing token expired (server 5 min TTL) | Toast "二维码已过期,请在 Mac 上重新生成" |
| `pair/consume` 401 (account token expired mid-pair) | Refresh + retry once; if still 401 → route to login |
| Pair succeeds but Mac immediately offline | Pairing complete → ThreadList with offline banner |
| Local stale pairing for prior account | Bootstrap detects `device.account_id` mismatch → `forget_peer` + route to pairing |

### 8.5 Cross-cutting

- **App lifecycle.** Foreground transition: refresh access token if expiring within 2 min, then immediate reconnect attempt. Background: pause reconnect loop. Long background after refresh-token TTL: refresh fails → login.
- **Logout mid-session.** Order: best-effort `stop_agent` (2 s timeout, do not block) → `logout` POST → clear secure storage → route to login. Pairing **not** dropped.
- **Account switch on same device.** Invalidates Riverpod caches; calls `forget_peer` because backend has rebound `device.account_id`.
- **First-run migration.** See §11.
- **Concurrent sends.** Send button is force-disabled in Streaming/Starting; double-tap on Stop is server-idempotent.
- **Input length.** Prompt limited to 8000 chars client-side; longer disables Send with hint.
- **Security pessimism.** No FLAG_SECURE / app-switcher blur in Spec 1 (Tier B).

---

## 9. Testing and Verification

### 9.1 Rust workspace tests

| Crate | Required tests |
|---|---|
| `minos-backend` (unit) | argon2id roundtrip; JWT sign/verify + `did` mismatch rejection; refresh rotation revokes prior; rate-limit bucket overflow returns 429 |
| `minos-backend` (sqlx integration) | See §9.2 |
| `minos-mobile` (unit) | `forward_rpc` monotonic id allocation; pending insert/remove on success/timeout; disconnect drains all pending with `RequestDropped`; reconnect backoff sequence (1, 2, 4, 8, 16, 30 cap) |
| `minos-mobile` (integration with `fake-peer`) | See §9.3 |
| `minos-protocol` | `MinosError` new-variant serde roundtrip |
| `minos-ffi-frb` | Existing codegen-drift guard (`cargo xtask check-all`) covers mirror types |

### 9.2 Backend integration test scenarios

Each as `#[sqlx::test]` against a fresh in-memory DB:

```
auth_register_login_refresh_logout_happy_path
auth_register_duplicate_email_returns_409
auth_login_wrong_password_returns_401
auth_login_revokes_existing_refresh_tokens_for_account
auth_refresh_with_revoked_token_returns_401
auth_refresh_rotation_old_token_invalidated
auth_logout_revokes_only_current_refresh_token
auth_rate_limit_login_returns_429_with_retry_after

ws_handshake_ios_without_bearer_returns_401
ws_handshake_ios_with_bearer_did_mismatch_returns_401
ws_handshake_mac_with_cf_access_unchanged

pairing_consume_ios_writes_account_id_to_pairing_record
pairing_consume_ios_without_bearer_returns_401
pairing_consume_mac_without_bearer_unchanged_works

routing_mac_to_ios_filters_by_account_id
routing_mac_to_ios_with_no_account_match_returns_not_paired
routing_ios_to_mac_unchanged
```

### 9.3 `fake-peer` extension

`crates/minos-mobile/src/bin/fake-peer.rs` grows scriptable subcommands:

```bash
cargo run -p minos-mobile --bin fake-peer --features cli -- \
    register --email a@b.com --password testpass1
# expects: tokens written, WS established

cargo run -p minos-mobile --bin fake-peer --features cli -- \
    smoke-session --thread-id new --prompt "ping"
# expects: Forward(start_agent) → reply → thread_id printed → UiEventMessage stream tail
```

New integration test `crates/minos-mobile/tests/e2e_register_login_dispatch_start_agent.rs`: spins an in-process axum backend (using existing `#[sqlx::test]` style fixtures) and an in-process fake Mac handler; drives `MobileClient.start_agent` end-to-end; asserts the reply Future resolves with the synthetic `thread_id`.

### 9.4 Flutter tests

```
test/domain/active_session_machine_test.dart
test/domain/auth_state_machine_test.dart
test/application/root_route_decision_test.dart
test/application/auth_provider_test.dart                  (frb mock)
test/widgets/auth/auth_form_test.dart                     (validation + submit-disabled)
test/widgets/chat/streaming_text_test.dart                (TextDelta accumulation)
test/widgets/chat/message_bubble_test.dart                (alignment + cursor while streaming)
test/widgets/chat/input_bar_test.dart                     (gating on ActiveSession state)
```

### 9.5 Real-device smoke checklist (manual gate)

```
□ Fresh install → register new email → auto-login → pairing page
□ Mac: cargo run -p minos-daemon -- start --print-qr → iPhone scans → pair OK
□ Main chat → Send "Hello" → see codex streamed reply, characters land progressively
□ Tap Stop mid-stream → stream halts, session enters Stopped
□ Send follow-up prompt → next streaming turn
□ Background 30 s → foreground → WS reconnects (banner flashes)
□ Force-quit app → reopen → auto-login (refresh succeeds) → previous thread visible
□ Login same account on second iPhone → first iPhone bumped to login within ~2 s (single-device verify)
□ Mac: stop daemon → iPhone shows "Mac 已离线" banner, input disabled
□ Mac: start daemon → iPhone pull-to-refresh → banner clears, can continue
□ Settings → Logout → routed to login → secure storage cleared (reinstall still requires fresh login)
□ Three wrong passwords on login → 429 → button countdown
□ Airplane mode → banner "重连中" → restore network → auto-recovery
```

### 9.6 CI integration (`cargo xtask check-all`)

The existing workflow already gates Rust + Swift + Flutter + frb-drift. Additions:

- `MINOS_JWT_SECRET` set in CI fixture (otherwise backend startup panics in test harness).
- `cargo sqlx prepare --check --workspace` continues to gate; new queries must commit refreshed `.sqlx/*.json`.
- Migration up/down smoke is implicitly covered by `#[sqlx::test]` running each migration on fixture creation.
- New Flutter tests above are picked up by `fvm flutter test` in the existing `flutter` lane.

---

## 10. Acceptance Criteria

```
[functional] A new user, on a fresh install, can register → pair → start codex → see the first
             streamed token in under 90 s for an experienced operator on a non-cold codex spawn.
[functional] All 13 items in §9.5 real-device smoke checklist pass.
[quality]    cargo xtask check-all is green.
[quality]    All tests listed in §9.1–§9.4 pass and run in CI on every PR.
[security]   Passwords stored as argon2id; JWT secret read from env;
             curl -XPOST /v1/auth/login without X-Device-Id returns 401;
             access-token replay with mismatched X-Device-Id returns 401;
             revoked refresh token rejected.
[migration]  Spec-1 first launch on a stale dev install clears local pairing and routes to login,
             with no panic, no crash, no orphaned secure-storage entries.
[docs]       This spec doc committed on main; subsequent plan doc committed when writing-plans
             produces it; README gains a section describing login + agent session as the new flow.
```

---

## 11. Migration and First-Run Handling

There are no production users (the relay backend is not yet open to end-users; pairing requires CF Access service tokens that are dev-only at present). All existing installs are dev / maintainer machines.

On first launch of a Spec-1 build:

1. `auth_provider` bootstrap reads `flutter_secure_storage`. If no `auth` entry exists (the pre-Spec-1 case for all current installs), state is `AuthUnauthenticated` and routing goes to the login page.
2. If a stale `pairing` entry exists from before Spec 1, it is **left intact in storage** but is **not used** until login completes. After login, the bootstrap inspects whether the on-device pairing record is consistent with the new account (specifically: does `device.account_id` returned by the backend match the locally-cached pairing identity). If not — which it won't on first migration, since the device row didn't have an account before — `forget_peer` is invoked and the user is routed to the pairing page.
3. From the user's perspective: open new build → login page → log in → pairing page → re-scan QR → main flow. No special "migration" UI; it falls out of the normal route gates.

The plan document derived from this spec must include a step verifying step 3 against a real prior install.

---

## 12. Risks and Open Questions

### 12.1 Risks

- **Pending-map leak under id wraparound.** `next_id: AtomicU64` wrapping is not a practical concern for a single device's lifetime, but the pending map should still cap entry count (e.g. 1024) and reject new dispatches with `MinosError::not_connected()` when full. The plan covers this defense.
- **WS write half ownership.** The current `client.rs` likely owns the WS read half via a long-running task; the write half needs to be wrapped in an `mpsc::UnboundedSender` for the dispatch path to feed it. The plan must read the existing wiring before touching it (do not assume — verify against `client.rs:handle_text_frame` and surrounding code).
- **`flutter_markdown_plus` streaming flicker.** Partial markdown ("```rust\nfn foo(") may render artifacts during streaming. If smoke surfaces this, switch to `gpt_markdown` (drop-in alternative) or wrap in a debounced render pass. Plan should include a manual "stream a 1KB code block, observe no flicker" verification step.
- **Backend `governor` import.** `tower-governor` is the conventional choice but has had recent middleware-API churn. If it does not slot cleanly into the existing axum middleware stack, fall back to a hand-rolled token bucket. Plan to verify with a quick PoC before committing the dep.
- **Account switch race.** If the user logs in as account B on a device that was account A's, the brief window between "new device.account_id written" and "Riverpod cache invalidated" can show stale account-A data. Plan must order: sign out → secure storage clear → cache invalidate → sign in.

### 12.2 Open questions (resolved during brainstorm)

> **Superseded 2026-05-01 by [ADR-0020](../../adr/0020-server-centric-auth-and-account-pairs.md).** The "single device" decision below is no longer current. Pair model is now (mac_device, mobile_account); iOS auth is bearer-only.

All architectural questions were resolved during the spec discussion:

- Bearer header vs cookie → bearer (§3, §6.6 rationale).
- Single device vs multi-device → single (§5.2 login revokes existing refresh).
- WS auth model → account-aware routing (§5.5 model γ).
- State ownership → Rust handles transport, Dart handles UI/session state (§6, §7).
- Stop semantics → maps to existing `stop_agent` (terminates agent process, closes thread).
- Mac availability signals → lazy detection in MVP, proactive deferred to Spec 2.
- Email verification, password reset → both deferred to Tier B.

No questions remain open at the design stage.

---

## 13. Files Inventory (full)

Combined from §5.7, §6.7, §7.10:

```
crates/minos-backend/migrations/0007_accounts.sql
crates/minos-backend/migrations/0008_refresh_tokens.sql
crates/minos-backend/migrations/0009_devices_account_link.sql
crates/minos-backend/src/store/accounts.rs
crates/minos-backend/src/store/refresh_tokens.rs
crates/minos-backend/src/auth/mod.rs
crates/minos-backend/src/auth/bearer.rs
crates/minos-backend/src/auth/jwt.rs
crates/minos-backend/src/auth/passwords.rs
crates/minos-backend/src/http/v1/auth.rs
crates/minos-backend/src/http/v1/mod.rs                                 modify
crates/minos-backend/src/http/v1/pairing.rs                             modify
crates/minos-backend/src/http/v1/threads.rs                             modify
crates/minos-backend/src/http/ws_devices.rs                             modify
crates/minos-backend/src/session/registry.rs                            modify
crates/minos-backend/src/config.rs                                      modify
crates/minos-backend/Cargo.toml                                         modify

crates/minos-mobile/src/auth.rs
crates/minos-mobile/src/rpc.rs
crates/minos-mobile/src/reconnect.rs
crates/minos-mobile/src/http.rs                                         modify
crates/minos-mobile/src/client.rs                                       modify
crates/minos-mobile/src/store.rs                                        modify
crates/minos-mobile/src/lib.rs                                          modify
crates/minos-mobile/tests/e2e_register_login_dispatch_start_agent.rs

crates/minos-protocol/src/error.rs                                      modify

crates/minos-ffi-frb/src/api/minos.rs                                   modify
flutter_rust_bridge.yaml                                                possibly modify

apps/mobile/lib/domain/auth_state.dart
apps/mobile/lib/domain/active_session.dart
apps/mobile/lib/domain/account.dart
apps/mobile/lib/application/auth_provider.dart
apps/mobile/lib/application/active_session_provider.dart
apps/mobile/lib/application/secure_storage_provider.dart
apps/mobile/lib/application/lifecycle_provider.dart
apps/mobile/lib/application/root_route_decision.dart                    modify
apps/mobile/lib/application/minos_providers.dart                        modify
apps/mobile/lib/application/thread_events_provider.dart                 modify
apps/mobile/lib/infrastructure/auth_secure_storage.dart
apps/mobile/lib/infrastructure/minos_core.dart                          modify
apps/mobile/lib/presentation/app.dart                                   modify
apps/mobile/lib/presentation/pages/login_page.dart
apps/mobile/lib/presentation/pages/account_settings_page.dart
apps/mobile/lib/presentation/pages/pairing_page.dart                    modify
apps/mobile/lib/presentation/pages/thread_list_page.dart                modify
apps/mobile/lib/presentation/pages/thread_view_page.dart                modify
apps/mobile/lib/presentation/widgets/auth/auth_form.dart
apps/mobile/lib/presentation/widgets/auth/auth_error_banner.dart
apps/mobile/lib/presentation/widgets/chat/message_bubble.dart
apps/mobile/lib/presentation/widgets/chat/streaming_text.dart
apps/mobile/lib/presentation/widgets/chat/reasoning_section.dart
apps/mobile/lib/presentation/widgets/chat/tool_call_card.dart
apps/mobile/lib/presentation/widgets/chat/input_bar.dart
apps/mobile/lib/presentation/widgets/chat/message_meta_row.dart
apps/mobile/pubspec.yaml                                                modify

apps/mobile/test/domain/active_session_machine_test.dart
apps/mobile/test/domain/auth_state_machine_test.dart
apps/mobile/test/application/root_route_decision_test.dart
apps/mobile/test/application/auth_provider_test.dart
apps/mobile/test/widgets/auth/auth_form_test.dart
apps/mobile/test/widgets/chat/streaming_text_test.dart
apps/mobile/test/widgets/chat/message_bubble_test.dart
apps/mobile/test/widgets/chat/input_bar_test.dart
```

---
