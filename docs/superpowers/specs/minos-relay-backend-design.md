# Minos Relay · Backend Architecture and Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-23 |
| Owner | fannnzhang |
| Repository | `github.com/peterich-rs/minos` (public) |
| Branch | `feat/relay-backend` |
| Supersedes (partial) | `minos-architecture-and-mvp-design.md` §4 (topology), §6 (data flows), §7.4 rows 1–2 (Tailscale-specific failure modes) |
| Related ADRs | 0001–0008 retained; proposes 0009–0012 (see §12) |

---

## 1. Context

The original MVP spec assumed peer-to-peer over Tailscale: the Mac binds a JSON-RPC WebSocket server on its `100.x.y.z` address; the iPhone dials it directly over the tailnet. Practice exposed three mismatches:

1. **User-facing friction.** Tailscale requires per-device install + sign-in + tailnet choice + MagicDNS understanding before the app ever launches. For a "slock.ai-style remote coding control" product, that's a prerequisite wall no end-user should hit.
2. **Future direction.** The roadmap points at a backend-centric hub (multi-Mac host registration, browser admin console, later cloud agent coordination). P2P forecloses those without a second architectural pivot.
3. **Already-owned infrastructure.** The owner operates a Cloudflare-managed domain (`fan-nn.top`) with `cloudflared` installed and authenticated. A single named tunnel (`minos.fan-nn.top`) in front of a relay service is simpler to explain, deploy, and scale than the tailnet prerequisite.

This spec pivots Minos from **P2P-over-Tailscale** to **broker-over-public-WSS**: a new standalone Rust service (`minos-relay`) is the only thing anyone connects to. The Mac daemon and iOS client both become outbound WebSocket clients of the relay. `cloudflared` exposes `minos-relay` via `wss://minos.fan-nn.top`; Cloudflare Access Service Tokens gate the hostname at the edge; an application-layer device-id + device-secret scheme gates business authorization on top.

The existing `minos-architecture-and-mvp-design.md` continues to apply for everything the topology pivot does not touch (error enum, logging, test matrix, Clean Arch in Swift / Dart, shadcn / Riverpod stack). This spec is a diff, not a replacement.

---

## 2. Goals

### 2.1 MVP (this spec's scope)

1. **Relay bring-up.** A standalone `minos-relay` bin listening on `127.0.0.1:8787`, exposed at `wss://minos.fan-nn.top/devices` via `cloudflared`. Health endpoint at `/health` for liveness probes.
2. **Two-layer auth.**
   - Infrastructure: Cloudflare Access Service Token (`CF-Access-Client-Id` + `CF-Access-Client-Secret`) validated at the CF edge — backend never sees unauthenticated requests.
   - Business: `X-Device-Id` + `X-Device-Secret` headers verified on every connection.
3. **Connection-state gate.** Two modes per connection:
   - *Unpaired* (no secret / unknown device): may call `request_pairing_token`, `pair`, `ping`. All other RPCs reject with `MinosError::Unauthorized`.
   - *Paired* (secret verified): full RPC + `forward` routing enabled.
4. **QR-based pairing via broker.** Mac requests a one-shot pairing token from the relay; QR encodes only `{backend_url, token, mac_display_name}` (no IP/port — those are fixed). iPhone scans → `pair(token)` → relay persists pair record, issues `device_secret` to both sides, pushes `Paired` event.
5. **Envelope-based WS routing + HTTP control plane.** Backend-terminated control operations (issue/consume pairing tokens, forget peer, list/read threads, last-seq probe) ride a typed JSON-over-HTTP API at `POST/GET/DELETE /v1/*`. The WebSocket at `/devices` carries only the four envelope kinds the relay genuinely brokers: `forward` (client → relay, opaquely re-emitted to the peer as `forwarded`), `forwarded` (relay → client peer payload delivery), `event` (relay → client server-pushed state, e.g. `Paired` / `PeerOnline` / `UiEventMessage`), and `ingest` (agent-host → backend raw event stream). Business RPC schemas (`list_clis`, etc.) live in `forward.payload` and are never parsed by the relay.
6. **SQLite persistence from day one.** Devices, pairings, pairing tokens, and secret hashes survive relay restarts. No in-memory fallback.
7. **End-to-end RPC through broker.** iPhone calls `list_clis` → envelope forwarded to Mac host → Mac responds through same envelope path → iPhone surfaces result. Zero relay-side knowledge of the method's shape.
8. **Reconnect resilience.** WS client exponential backoff (`1s→2s→…→30s` cap); server-side session cleanup on dropped connections; `PeerOnline` / `PeerOffline` events pushed to the opposite side.
9. **Tool integration.** New crate wired into `cargo xtask check-all`; integration tests run the full pair-then-forward loop with two in-process fake clients.

### 2.2 Non-goals (this MVP)

- Browser admin console (reserves `/admin` WS path; UI + auth flow in a follow-up spec).
- Agent execution over the relay (codex `app-server`, claude / gemini PTY). Still P1.
- Multi-Mac / multi-iPhone cross-pairing. Single pair enforced; second pair replaces with confirmation.
- End-to-end encryption of `forward` payloads. Access + WSS transport security is accepted as the ring-0 guarantee for MVP.
- `DeviceSecret` rotation / revocation. To revoke, user calls `forget_peer` and re-pairs.
- Skills metadata (per-CLI agent skills discovery). Schema reserved, fields reserved on `AgentDescriptor`, but no population logic in MVP.
- Production deployment to an always-on Linux. Dev and first-cut prod both run on the Mac behind `caffeinate` / no-sleep.
- Automatic failback to P2P if relay is down. If relay is unreachable, clients show `Disconnected` and retry.

---

## 3. Tech Stack and Defaults

The stack inherits from the original spec; only deltas are listed here.

| Layer | Choice | Note |
|---|---|---|
| Backend HTTP/WS framework | **`axum` 0.7+** | tokio-native, `extract::ws` first-class, `tower` middleware stack. See §5 for why not `jsonrpsee` server / raw `tokio-tungstenite` |
| DB | **`sqlx` with `sqlite` feature + `rt-tokio`** | Async-native, compile-time query check (offline mode for CI), `sqlx::migrate!` for file-based migrations |
| Migration layout | `crates/minos-relay/migrations/XXXX_name.sql` | Plain SQL, numbered, run on startup |
| Password hashing | **`argon2` 0.5+** | Hashing `DeviceSecret` at rest. `blake3` considered but argon2 is the conservative default for credential material |
| Constant-time compare | **`subtle` 2.x** | `ConstantTimeEq` on secret verification path |
| UUID / random | Reuses workspace `uuid` + `getrandom` | No new deps |
| Tunnel exposure | `cloudflared` (binary, out of repo) | See §9.3 for ops runbook |
| Business auth gate | Cloudflare Access Service Token | Generated in Zero Trust dashboard, shipped to clients as configuration |
| Envelope wire format | Plain `serde_json::Value` payload, strongly typed envelope | See §6 |

### 3.1 Why depart from `jsonrpsee` on the backend

`jsonrpsee`'s server model assumes "clients call methods I handle." The relay's job is the opposite: most messages are *transit* — backend must not parse business payloads, or every new RPC becomes a backend change. Keeping `jsonrpsee` on the backend would force one of two bad shapes:

1. A catch-all `forward` method that defeats jsonrpsee's type safety.
2. Every business RPC duplicated on the backend with a passthrough body.

The envelope pattern cleanly separates "backend's RPCs" (small, stable: `request_pairing_token`, `pair`, `ping`, `forget_peer`) from "peer-to-peer RPCs" (unbounded, evolves with product). `jsonrpsee` remains authoritative for peer-to-peer schemas in `minos-protocol`; the relay only sees envelopes.

### 3.2 Cloudflare assumption

Users are expected to install `cloudflared` on the box that runs `minos-relay` and bind a named tunnel to the hostname. The relay itself is unaware of Cloudflare; from its perspective it listens on plain HTTP on `127.0.0.1:8787` and trusts that whatever reaches it has already cleared Access at the edge. The tunnel runbook lives at `docs/ops/cloudflare-tunnel-setup.md` (to be written in a follow-up PR).

---

## 4. Architecture Overview

```
 ┌───────────────────── Cloudflare Edge ─────────────────────┐
 │  minos.fan-nn.top (CNAME → tunnel UUID)                   │
 │  Access policy: allow fannnzhang@…                        │
 │  Service Token validates non-browser clients              │
 └─────────────────┬─────────────────────────┬───────────────┘
                   │ WSS                     │ WSS
                   │                         │
 ┌─────────────────┼─────────────────────────┼───────────────┐
 │   cloudflared (outbound QUIC tunnel to CF edge)           │
 │                 │                         │               │
 │       ┌─────────▼────────┐     ┌──────────▼─────────┐     │
 │       │ minos-relay      │     │  (future: /admin)  │     │
 │       │ 127.0.0.1:8787   │     │  browser console   │     │
 │       │ axum + tokio     │     └────────────────────┘     │
 │       │ ┌──────────────┐ │                                │
 │       │ │ WS handler   │ │ ← /devices                     │
 │       │ │ /devices     │ │                                │
 │       │ ├──────────────┤ │                                │
 │       │ │ Envelope     │ │                                │
 │       │ │ dispatcher   │ │                                │
 │       │ ├──────────────┤ │                                │
 │       │ │ Session reg  │ │ DashMap<DeviceId, Session>     │
 │       │ ├──────────────┤ │                                │
 │       │ │ Pairing svc  │ │ token issue/consume            │
 │       │ ├──────────────┤ │                                │
 │       │ │ SQLite store │ │ sqlx + migrations              │
 │       │ └──────────────┘ │                                │
 │       └──────────────────┘                                │
 │                                                           │
 │     (Mac box: relay + cloudflared + macOS app all local)  │
 └───────────────────────────────────────────────────────────┘

                   ▲                         ▲
                   │ outbound WSS            │ outbound WSS
                   │ CF-Access-* + X-Dev-*   │ CF-Access-* + X-Dev-*
                   │                         │
      ┌────────────┴──────────┐    ┌─────────┴──────────┐
      │ Minos.app (macOS)     │    │ Minos (iOS)        │
      │ role: mac-host        │    │ role: ios-client   │
      │ WS client (axum       │    │ WS client (Flutter │
      │   compatible)         │    │   via frb → Rust)  │
      │ cli-detect + host     │    │ pairing UI + chat  │
      │   RPC impls (list_clis│    │   surface (later)  │
      │   etc.)               │    │                    │
      └───────────────────────┘    └────────────────────┘
```

### 4.1 Process model

- **Relay box** (same Mac as the app in MVP): `minos-relay` bin + `cloudflared` launchd service. Both auto-start.
- **Mac app** (`Minos.app`): SwiftUI + UniFFI bridge + embedded tokio runtime (as before) + WS client to `wss://minos.fan-nn.top/devices` (new).
- **iOS app**: Flutter + frb + embedded tokio runtime + WS client to same URL.

### 4.2 Protocol stack (top-down)

`Cloudflare edge (Access + TLS)` → `HTTP/2 or QUIC tunnel to origin` → `cloudflared` → `HTTP/1.1 Upgrade` on `127.0.0.1:8787` → `WebSocket (tungstenite via axum)` → `Envelope JSON frames` → either `backend-local RPC` or `forwarded peer-to-peer JSON-RPC 2.0 payload`.

### 4.3 Deployment boundaries

| Boundary | Who authenticates | What fails if breached |
|---|---|---|
| Public internet → CF edge | TLS + Cloudflare Access | Edge drops at 401/403; relay never sees the request |
| CF edge → `cloudflared` tunnel | Cloudflare-issued tunnel certificate (invisible to us) | Tunnel refuses to route |
| `cloudflared` → `127.0.0.1:8787` | Loopback; no TLS | Would require local root on the box (assume compromised = game over anyway) |
| Relay → client (per WS) | `X-Device-Id` + `X-Device-Secret` | Unpaired mode only; RPCs reject |

---

## 5. Components

### 5.1 New and changed crates

| Crate | Status | MVP responsibility |
|---|---|---|
| `minos-relay` | **NEW**, bin | `axum` server, envelope dispatcher, session registry (`DashMap<DeviceId, SessionHandle>`), pairing service, SQLite store, cloudflared-agnostic |
| `minos-protocol` | Add `envelope` module | Envelope enum (`Forward` / `Forwarded` / `Event` / `Ingest`) + `EventKind` + the JSON request/response types for the HTTP `/v1/*` control plane (`PairConsumeRequest`, `PairResponse`, `PairingQrPayload`, `ListThreadsResponse`, …). Existing `#[rpc]` trait unchanged (still authoritative for peer-to-peer business RPCs) |
| `minos-domain` | Add types | `DeviceSecret` newtype; extend `MinosError` (see §8); `DeviceRole` enum (`MacHost` / `IosClient` / `BrowserAdmin`) |
| `minos-pairing` | Semantic refactor | State machine flips: pairing is now backend-mediated. `PairingStore` trait retained on clients for local-only credential storage; token-issuance logic moves into `minos-relay`'s pairing service |
| `minos-transport` | Scope narrowed | Server role **retired** in this MVP (nothing binds a public server any more). Client role (`WsClient::connect`, reconnect backoff, heartbeat loop) stays and grows an `auth: AuthHeaders` argument |
| `minos-daemon` | Internal refactor | Still the Mac-side composition root. Orchestrates: WS client to relay, `cli-detect`, local `PairingStore` (Keychain-backed via UniFFI). Name kept for continuity (renaming is a cosmetic follow-up) |
| `minos-cli-detect` | Unchanged | Still probes `codex` / `claude` / `gemini` on the Mac |
| `minos-mobile` | Lightly refactored | `MobileClient` now speaks envelopes, auth headers, no longer dials Tailscale IP |
| `minos-ffi-uniffi`, `minos-ffi-frb` | Untouched | Pure re-export shims |

### 5.2 `minos-relay` internal layout

```
crates/minos-relay/
├── Cargo.toml
├── migrations/
│   ├── 0001_devices.sql
│   ├── 0002_pairings.sql
│   └── 0003_pairing_tokens.sql
└── src/
    ├── main.rs              # bin entry: parse config, init tracing, run
    ├── config.rs            # env vars + CLI flags (listen addr, db path, log dir)
    ├── http/
    │   ├── mod.rs           # axum Router wiring
    │   ├── auth.rs          # shared header/auth classifier (used by /v1/* + /devices)
    │   ├── health.rs        # GET /health
    │   ├── ws_devices.rs    # GET /devices → WS upgrade
    │   └── v1/              # HTTP control plane (POST /v1/pairing/{tokens,consume},
    │                        # DELETE /v1/pairing, GET /v1/threads*)
    ├── session/
    │   ├── mod.rs           # SessionHandle, Session task
    │   ├── registry.rs      # DashMap<DeviceId, SessionHandle>
    │   └── heartbeat.rs     # ping/pong loop
    ├── envelope/
    │   └── mod.rs           # dispatch (Forward + Ingest only; control-plane is HTTP)
    ├── pairing/
    │   ├── mod.rs           # issue, consume, persist
    │   └── secret.rs        # argon2 hash + verify
    ├── store/
    │   ├── mod.rs           # sqlx pool + migrations
    │   ├── devices.rs
    │   ├── pairings.rs
    │   └── tokens.rs
    └── error.rs             # RelayError → maps to MinosError at boundary
```

### 5.3 Sharing matrix (post-pivot)

| Crate | Relay bin | Mac binary | iOS binary |
|---|---|---|---|
| `minos-domain` | ✓ | ✓ | ✓ |
| `minos-protocol` | ✓ (envelope only; `#[rpc]` trait not used by relay) | ✓ | ✓ |
| `minos-pairing` | ✓ (token service) | ✓ (client store) | ✓ (client store) |
| `minos-transport` | — | ✓ (client) | ✓ (client) |
| `minos-cli-detect` | — | ✓ | — |
| `minos-daemon` | — | ✓ | — |
| `minos-mobile` | — | — | ✓ |
| `minos-relay` | ✓ | — | — |
| `minos-ffi-uniffi` | — | ✓ | — |
| `minos-ffi-frb` | — | — | ✓ |

---

## 6. Protocol: Envelope (WebSocket) + HTTP `/v1/*` (control plane)

The backend exposes two surfaces:

1. **HTTP `/v1/*`** — JSON request/response routes for everything the
   backend itself terminates (issue/consume pairing tokens, tear down
   pairings, list/read threads). Authenticated per-request via the same
   `X-Device-Id` / `X-Device-Role` / `X-Device-Secret` (and
   `CF-Access-*`) header bundle the WebSocket uses.
2. **WebSocket `/devices`** — bi-directional channel that brokers
   peer-to-peer JSON-RPC payloads (`forward` / `forwarded`), pushes
   server-side state events (`event`), and accepts agent-host raw event
   streams (`ingest`). Every WS frame is one JSON object matching the
   `Envelope` enum below.

```rust
// crates/minos-protocol/src/envelope.rs
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    /// Client → Relay. Relay forwards opaquely to paired peer.
    Forward {
        #[serde(rename = "v")] version: u8,   // always 1 in MVP
        payload: serde_json::Value,           // opaque; JSON-RPC 2.0 by convention
    },
    /// Relay → Client. Peer sent you this.
    Forwarded {
        #[serde(rename = "v")] version: u8,
        from: DeviceId,
        payload: serde_json::Value,
    },
    /// Relay → Client. Server-side state push.
    Event {
        #[serde(rename = "v")] version: u8,
        #[serde(flatten)]
        event: EventKind,
    },
    /// Agent-host → Backend. Raw native event for persistence + fan-out.
    Ingest {
        #[serde(rename = "v")] version: u8,
        agent: AgentName,
        thread_id: String,
        seq: u64,
        payload: serde_json::Value,
        ts_ms: i64,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// Only fires on the Mac after iPhone consumes its token.
    Paired { peer_device_id: DeviceId, peer_name: String, your_device_secret: DeviceSecret },
    /// Paired peer WS is up.
    PeerOnline { peer_device_id: DeviceId },
    /// Paired peer WS dropped.
    PeerOffline { peer_device_id: DeviceId },
    /// Other side called forget_peer, or admin revoked.
    Unpaired,
    /// Relay is shutting down; clients should reconnect.
    ServerShutdown,
    /// Backend → Mobile. One translated UI event from backend's live fan-out.
    UiEventMessage { thread_id: String, seq: u64, ui: UiEventMessage, ts_ms: i64 },
}
```

### 6.1 HTTP control-plane routes

All routes require authenticated headers (`X-Device-Id`, `X-Device-Role`,
optional `X-Device-Secret`, optional `CF-Access-*`) — the same auth
classifier the WS handshake uses (`http::auth::authenticate`).

| Method + path | Caller role | Pre-state | Body / query | Success | Errors |
|---|---|---|---|---|---|
| `POST /v1/pairing/tokens` | `agent-host` | any | `{ "host_display_name": "..." }` | `{ "qr_payload": PairingQrPayload }` | `401` non-host, `403` CF-only edge state |
| `POST /v1/pairing/consume` | `ios-client` | Unpaired | `{ "token": "...", "device_name": "..." }` | `PairResponse { peer_device_id, peer_name, your_device_secret }` | `409 pairing_token_invalid`, `409 pairing_state_mismatch` |
| `DELETE /v1/pairing` | any (paired) | Paired | `X-Device-Secret` required | `204 No Content` | `404` if already unpaired |
| `GET /v1/threads` | any (paired) | Paired | `?limit&before_ts_ms&agent=` | `ListThreadsResponse` | `401` unpaired |
| `GET /v1/threads/{id}/events` | any (paired) | Paired, owns thread | `?limit&from_seq=` | `ReadThreadResponse` | `401` unpaired, `404` thread |
| `GET /v1/threads/{id}/last_seq` | any (paired) | Paired, owns thread | — | `GetThreadLastSeqResponse` | `401`, `404` |

After `POST /v1/pairing/consume` succeeds, the backend pushes
`Event::Paired` onto the issuer's live WebSocket so the Mac learns about
the iPhone without polling. After `DELETE /v1/pairing`, both sessions
receive `Event::Unpaired`.

Token TTL: **5 minutes**. Expired tokens are GC'd by a background task
every 60s. Consumed tokens are marked, never reused. Token hashing uses
SHA-256 (not argon2id): the 32-byte plaintext has ≥256 bits of entropy
and the short TTL bounds attacker exposure, so a deterministic hash
suffices and (unlike argon2's salted PHC output) supports direct
primary-key lookup.

### 6.2 Id semantics

- `Forward.payload` carries its own JSON-RPC 2.0 `id` field; the relay
  does not read it. Correlation of forwarded RPC responses is the
  client's problem (handled by `jsonrpsee` or equivalent on each side).
- HTTP `/v1/*` calls have no envelope id; reqwest correlates request
  with response at the transport layer.
- Backend never generates ids — it only routes.

### 6.3 Version field

`"v": 1` required on every WS envelope. Future breaking changes bump to
2; backend supports a window of versions during transitions. Clients
that see `"v"` they don't understand close the socket with WS code 4400.
HTTP routes are versioned via the `/v1/` path prefix; future revisions
land at `/v2/` rather than mutating the wire shape in place.

---

## 7. Data Flows

### 7.1 First-time pairing

```
[Mac]                                                    [Relay]                                    [iPhone]

1. app launches
   ├─ no device_id on disk → gen DeviceId (UUIDv4), persist
   └─ no device_secret    → connect as UNPAIRED

2. WS GET /devices
   headers: CF-Access-Client-Id/Secret
            X-Device-Id: <mac-uuid>
            X-Device-Role: mac-host
                                                    3. edge validates Service Token
                                                       relay: look up device_id → not found
                                                       → insert devices row (secret_hash = NULL)
                                                       → session = UNPAIRED
                                                       → WS 101 Upgrade
   ◄─────── Event{type: "unpaired"} ──────────────

4. user clicks "Show QR" in menu bar
   ├─ POST /v1/pairing/tokens
   │   headers: X-Device-Id (mac), X-Device-Role: agent-host, CF-Access-*
   │   body:    {"host_display_name": "Fan's Mac"}
   ─────────►                                      5. issue 32B token, hash it, store in
                                                       pairing_tokens (issuer=mac, ttl=5min)
   ◄─────── 200 OK { qr_payload: PairingQrPayload }
6. render QR: PairingQrPayload v2 (backend_url, token, host_display_name,
   expires_at_ms, optional CF-Access service-token fields)

                                                                               7. app launches
                                                                                  ├─ gen DeviceId
                                                                                  └─ connect UNPAIRED
                                                         ◄──────── WS GET /devices
                                                                   headers: X-Device-Id (iphone)
                                                                            X-Device-Role: ios-client
                                                       8. same unpaired handshake as step 3
                                                                                  9. user taps "Scan to pair"
                                                                                     camera yields QR
                                                                               ◄──── parse {token, ...}
                                                         ◄──── POST /v1/pairing/consume
                                                                headers: X-Device-Id (iphone),
                                                                         X-Device-Role: ios-client,
                                                                         CF-Access-*
                                                                body:    {"token": "...",
                                                                          "device_name": "..."}
                                                  10. consume token:
                                                      ├─ hash(input) match pairing_tokens row?
                                                      ├─ not expired, not consumed?
                                                      ├─ mark consumed_at
                                                      ├─ gen DeviceSecret_mac (32B), argon2 hash
                                                      ├─ gen DeviceSecret_phone (32B), argon2 hash
                                                      ├─ update devices rows with hashes
                                                      ├─ insert pairings row (mac, phone)
                                                      └─ upgrade both sessions to PAIRED
   ◄───── Event{type: paired,                    11. push Paired event to mac's live WS
           peer_device_id: phone,                    return PairResponse to phone over HTTP
           peer_name: "...",                                                             ─────────►
           your_device_secret: <mac secret>}
                                                                               ◄──── 200 OK PairResponse
                                                                                   { peer_device_id,
                                                                                     peer_name,
                                                                                     your_device_secret }

12. client stores secret in Keychain                                      13. client stores secret in Keychain
    UI: "Connected (1 device)"                                                UI: navigate to HomePage
```

### 7.2 Reconnect (post-pair, either side restarts)

```
[either side]                                             [Relay]

1. app launches → loads DeviceId + DeviceSecret from Keychain
2. WS GET /devices
   headers: CF-Access-Client-Id/Secret
            X-Device-Id: <uuid>
            X-Device-Secret: <base64url-secret>
                                                   3. edge validates Service Token
                                                      relay: look up device_id → found
                                                      ├─ argon2::verify(input, stored_hash)? → ok
                                                      ├─ load pairing record → peer = X
                                                      ├─ session_registry[device_id] = handle
                                                      ├─ session = PAIRED
                                                      └─ WS 101 Upgrade
   ◄──── Event{type: peer_online | peer_offline}   4. emit peer status to this client

                                                   5. if peer was offline, now notify peer too:
                                                      → peer's session gets Event{type:peer_online,
                                                                                   peer_device_id:this}
```

### 7.3 Business RPC through broker (iPhone asks Mac for CLI list)

```
[iPhone]                                 [Relay]                                 [Mac]

1. UI wants CLI list
   └─ minos-mobile calls MobileClient::list_clis()
   └─ jsonrpsee client builds JSON-RPC request
      wrap in Envelope::Forward{ v:1, payload: {jsonrpc, method:list_clis, id:42, params:{}} }
   ─────────►
                                    2. dispatch: Forward → lookup session for
                                       iphone's paired peer (mac) → present?
                                       ├─ yes: session.outbox.send(Forwarded{
                                       │         v:1, from:iphone, payload:<same>})
                                       └─ no:  synthesize JSON-RPC error
                                               "peer offline" and send back as
                                               Forwarded to sender (*)
                                                                         ─────────►
                                                                         3. dispatch: Forwarded payload
                                                                            looks like JSON-RPC 2.0 request
                                                                            → minos-daemon hands to its
                                                                              jsonrpsee server impl
                                                                         4. execute list_clis()
                                                                            → Vec<AgentDescriptor>
                                                                         5. JSON-RPC response:
                                                                            {jsonrpc, result:[...], id:42}
                                                                            wrap in Envelope::Forward
                                                                         ◄─────────
                                    6. Forward (reverse direction) → find mac's
                                       peer (iphone) online → send as Forwarded
   ◄─────────
7. envelope dispatcher unwraps Forwarded.payload,
   hands to local jsonrpsee client which correlates id:42
   → Future resolves with Vec<AgentDescriptor>
```

> (\*) Peer-offline handling: relay does not queue. If the callee is offline, relay immediately synthesizes a JSON-RPC error response (`code: -32001`, message: `"peer offline"`) and sends it back as `Forwarded`. Caller's jsonrpsee client sees a normal error future. This is a deliberate MVP choice — queuing across disconnections is P1.

### 7.4 Forget peer

```
[initiator]                              [Relay]                                [peer]

1. user taps "Forget this device"
   DELETE /v1/pairing
   headers: X-Device-Id, X-Device-Role, X-Device-Secret, CF-Access-*
   ─────────►
                                    2. find pairing row → delete
                                    3. emit Event{type: unpaired} to *both* live sessions
                                       ├─ issuer session → unpaired mode
                                       └─ peer session   → unpaired mode; secret invalidated
                                                                    ─────────►
                                                                    4. peer wipes local Keychain
                                                                       secret; UI → PairingPage
   ◄──── 204 No Content (HTTP)
   ◄──── Event{type: unpaired} (over WS)
5. wipe local Keychain secret; UI → PairingPage
```

### 7.5 Relay shutdown

```
[Relay] SIGTERM / SIGINT
1. stop accepting new connections
2. broadcast Event{type: server_shutdown} to every active session
3. drain outbox queues (up to 500ms)
4. close WS with code 1001 ("going away")
5. close DB pool, exit

[Client]
1. receives ServerShutdown → mark ConnectionState::Reconnecting{attempt:1}
2. waits backoff, reconnects when relay is back
```

---

## 8. Persistence (SQLite)

### 8.1 Schema

```sql
-- 0001_devices.sql
CREATE TABLE devices (
    device_id      TEXT PRIMARY KEY,          -- UUIDv4 string
    display_name   TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('mac-host','ios-client','browser-admin')),
    secret_hash    TEXT,                      -- argon2id; NULL while unpaired
    created_at     INTEGER NOT NULL,          -- unix epoch ms
    last_seen_at   INTEGER NOT NULL
) STRICT;

-- 0002_pairings.sql
-- Enforce undirected uniqueness by storing (a, b) with a < b.
CREATE TABLE pairings (
    device_a       TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    device_b       TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (device_a, device_b),
    CHECK (device_a < device_b)
) STRICT;

CREATE INDEX idx_pairings_a ON pairings(device_a);
CREATE INDEX idx_pairings_b ON pairings(device_b);

-- 0003_pairing_tokens.sql
CREATE TABLE pairing_tokens (
    token_hash        TEXT PRIMARY KEY,            -- SHA-256 hex digest of the plaintext token bearer (32B random → 64 hex chars). Deterministic for PK lookup; safe because tokens are one-shot and TTL ≤ 5 min.
    issuer_device_id  TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    consumed_at       INTEGER                     -- NULL until pair() succeeds
) STRICT;

CREATE INDEX idx_pairing_tokens_expires ON pairing_tokens(expires_at) WHERE consumed_at IS NULL;
```

### 8.2 Lifecycle

- **On startup**: `sqlx::migrate!("./migrations")` applies every pending migration in a single tx. Idempotent.
- **Token GC**: background tokio task wakes every 60s, deletes rows where `expires_at < now() AND consumed_at IS NULL`.
- **On `forget_peer`**: single DELETE on `pairings`; cascading FK cleans tokens if they referenced the forgotten device.
- **No "last_active session" row**: session state is in-memory only; DB represents persistent truth, memory represents current reality. A relay restart flushes all sessions; clients reconnect and re-populate.

### 8.3 DB file location

Dev: `./minos-relay.db` in CWD by default, override via `MINOS_RELAY_DB=/path/to.db`. Prod on Mac: `~/Library/Application Support/minos-relay/db.sqlite`.

---

## 9. Tooling and Operations

### 9.1 cargo xtask additions

| Command | Action |
|---|---|
| `cargo xtask relay-db-reset` | Delete `minos-relay.db` and re-run migrations. Dev convenience; never runs in prod |
| `cargo xtask relay-run` | Alias for `cargo run -p minos-relay -- --listen 127.0.0.1:8787 --db ./minos-relay.db` with sane dev defaults |
| `cargo xtask check-all` | Unchanged; now includes `minos-relay` by virtue of the `crates/*` glob in workspace |

### 9.2 Config surface (`minos-relay`)

```
minos-relay [OPTIONS]

  --listen <addr>         Default: 127.0.0.1:8787 (env: MINOS_RELAY_LISTEN)
  --db <path>             Default: ./minos-relay.db (env: MINOS_RELAY_DB)
  --log-dir <path>        Default: ~/Library/Logs/Minos/ (env: MINOS_RELAY_LOG_DIR)
  --log-level <level>     Default: info (env: RUST_LOG)
  --token-ttl-secs <n>    Default: 300 (env: MINOS_RELAY_TOKEN_TTL)
```

No CLI for admin tasks in MVP. `forget_peer` flows through the RPC path from a paired client.

### 9.3 cloudflared ops runbook (to ship as `docs/ops/cloudflare-tunnel-setup.md`)

Outline:

1. Prerequisites (Cloudflare account, zone `fan-nn.top`, `brew install cloudflared`).
2. `cloudflared tunnel login` — authorizes the machine.
3. `cloudflared tunnel create minos` — stores credentials.
4. `cloudflared tunnel route dns minos minos.fan-nn.top` — writes the CNAME.
5. Author `~/.cloudflared/config.yml` (already done during scaffolding, pinned to `http://localhost:8787`).
6. Zero Trust dashboard: create Access application on `minos.fan-nn.top`, allow owner email; generate a Service Token for non-browser clients; copy `Client ID` + `Client Secret` into Mac and iOS app configs.
7. `sudo cloudflared service install` — installs LaunchDaemon; service starts on boot.
8. Verification: `curl -H "CF-Access-Client-Id: ..." -H "CF-Access-Client-Secret: ..." https://minos.fan-nn.top/health` returns `200 OK` from the relay.

This runbook is a separate deliverable; the relay crate is agnostic to any of it.

### 9.4 Client config for Service Tokens

- **macOS app**: Service Token stored in app-level Keychain entry at first run (user pastes from dashboard); injected into every WS connect request. Not baked into the binary.
- **iOS app**: same pattern; paste during onboarding or scan from Mac's Keychain via the same QR flow (P1 improvement).

Service Token is device-class, not user-class — compromise requires rotating at the dashboard and redeploying client config. For single-user MVP this is acceptable.

### 9.5 Logging

Sink unchanged (`mars-xlog` via `tracing::Layer`). New `name_prefix`: `relay`. Relay log fields add `session_id` (in addition to the existing `device_id`, `peer_device_id`, `rpc_method`, `pairing_state`).

---

## 10. Error Handling

### 10.1 `MinosError` additions

New variants on the canonical enum (requires matching `ErrorKind` additions and zh/en strings in `crates/minos-domain/src/error.rs`):

```rust
#[error("unauthorized for this operation: {reason}")]
Unauthorized { reason: String },

#[error("relay connection state not suitable: expected {expected}, got {actual}")]
ConnectionStateMismatch { expected: String, actual: String },

#[error("envelope version unsupported: {version}")]
EnvelopeVersionUnsupported { version: u8 },

#[error("peer offline: {peer_device_id}")]
PeerOffline { peer_device_id: String },

#[error("relay internal error: {message}")]
RelayInternal { message: String },
```

Tailscale-specific zh/en strings in `BindFailed` / `ConnectFailed` are **rewritten** by this spec:

- `BindFailed`: no longer meaningful on clients (no client binds). Kept for the relay; string becomes "Cannot bind relay listen address; check MINOS_RELAY_LISTEN".
- `ConnectFailed`: new message "Cannot reach relay at `{url}`; check network and Cloudflare Access token".

### 10.2 New failure modes to handle

| # | Trigger | Error | UI behavior |
|---|---|---|---|
| R1 | Service Token missing or wrong | CF edge 401 (relay never sees request) | Client shows "Access token invalid; reconfigure in settings" |
| R2 | `X-Device-Id` present, `X-Device-Secret` wrong | Relay closes WS code 4401 | Client treats as `DeviceNotTrusted`, wipes local secret, returns to PairingPage |
| R3 | `pair()` with expired token | `PairingTokenInvalid` | iPhone UI: "QR expired, please rescan"; Mac UI auto-regenerates QR |
| R4 | `pair()` when Mac already paired with another iPhone | `PairingStateMismatch` | Mac UI confirms replace; iPhone UI: "Mac is already paired with another device" |
| R5 | Peer offline during `forward` | Relay returns `Forwarded` with synthesized JSON-RPC error `-32001 peer offline` | UI: "Mac offline, please check status" |
| R6 | Relay SIGTERM during active connection | `ServerShutdown` event | UI shows `Reconnecting`; client retries with backoff |
| R7 | Envelope `v` unknown | `EnvelopeVersionUnsupported` | WS close code 4400; client logs and retries; repeats ⇒ user must update app |
| R8 | SQLite file locked / corrupt on relay boot | relay refuses to start, exits 2 | Logged to `~/Library/Logs/Minos/relay-*.xlog`; Mac menu bar shows relay-down badge |

### 10.3 WS close code conventions

| Code | Meaning |
|---|---|
| `1000` | Normal close (client forget_peer, clean shutdown) |
| `1001` | `ServerShutdown` (relay going down) |
| `4400` | Bad envelope (version / schema) |
| `4401` | Auth failed (bad secret) |
| `4409` | State conflict (e.g., two simultaneous pair calls) |

---

## 11. Testing Strategy

### 11.1 Rust matrix additions

| Layer | Test type | Tools | Key targets |
|---|---|---|---|
| `minos-relay` envelope dispatcher | Unit | `serde_json` round-trips | Every `Envelope` variant round-trips; unknown `kind` rejects |
| `minos-relay` session registry | Unit | tokio test | concurrent insert / remove; cleanup on drop |
| `minos-relay` pairing service | Unit + property | `rstest`, `proptest` | token-generate uniqueness (1000-draw no collision); expire GC; argon2 verify constant-time path exercised |
| `minos-relay` SQLite store | Integration | `sqlx::test` (in-memory or temp file) | Migrations apply; CRUD for each table; FK cascades on `forget_peer` |
| `minos-relay` full stack | `#[tokio::test]` E2E | axum test server + two fake WS clients via `tokio-tungstenite` | `pair → forward → forward response → forget_peer` in a single test in <1s |
| `minos-protocol` envelope | Unit + golden JSON | `tests/golden/envelope/*.json` | Wire format frozen; PR bumps must update golden |

### 11.2 Cloudflare-dependent tests

Not in CI. A manual smoke checklist (§11.4) covers the CF path; automating requires Cloudflare API + test domain, deferred.

### 11.3 Client-side test updates

- `minos-mobile` unit tests: replace Tailscale-IP mocks with relay-URL mocks. `MobileClient::pair_with(qr_payload)` signature changes; tests follow.
- `apps/macos` XCTests: `AppState` subscribes to `DaemonHandle::events_stream` as before; event set grows (`PeerOnline`, `PeerOffline`, `ServerShutdown`) and tests cover all.
- `apps/mobile` widget tests: `HomePage` renders new `Reconnecting{attempt}` variant — already supported by `ConnectionState`.

### 11.4 Smoke acceptance checklist (hard gate for v0.2.0)

```
□ Relay: `cargo xtask relay-run` → logs "listening on 127.0.0.1:8787" + "migrations applied"
□ cloudflared: `sudo cloudflared service list` shows minos running
□ `curl -H CF-Access-* https://minos.fan-nn.top/health` → 200 OK
□ Mac app launches, menu bar shows "Awaiting pairing"
□ "Show QR" → QR displays
□ iPhone scans → within 5s both UIs show "Connected"
□ iPhone taps refresh → HomePage lists codex/claude/gemini with paths and versions
□ Restart iPhone app → reconnects without rescan
□ Restart Mac app → reconnects without rescan
□ Stop relay → both UIs show "Reconnecting"; restart relay → both auto-recover within 60s
□ "Forget this device" on Mac → iPhone immediately shows "Pairing revoked, please rescan"
□ Rotate Service Token in Zero Trust → both apps fail to connect with clear error; re-enter → recover
```

12 boxes ticked = relay MVP complete.

### 11.5 CI deltas

`.github/workflows/ci.yml`: `rust` job already runs workspace-wide tests. Add one step: `cargo sqlx prepare --check --workspace` to verify offline query metadata is up to date (else `cargo build` on CI without a DB fails).

---

## 12. Out of Scope / Roadmap

| Item | Phase | Rationale |
|---|---|---|
| Browser admin console on `/admin` | P1 | Reserves the path now; rendering + WS schema in follow-up spec |
| Skills discovery per CLI | P1 | AgentDescriptor gets `skills: Vec<SkillDescriptor>`; populated by an upgrade to `minos-cli-detect` |
| Queueing / replay during peer-offline | P1 | Relay currently errors `peer offline`; later grows a bounded outbox per pair |
| `DeviceSecret` rotation / revocation | P1.5 | Requires key-rotation UX; `forget_peer` + re-pair suffices for MVP |
| Production deploy on home Linux | P1.5 | Build pipeline + systemd unit; same binary, different box |
| E2EE `forward` payloads | P2 | Paired devices exchange X25519 keys via relay once, content-layer sealed thereafter |
| Multi-pair (one Mac, multiple mobiles) | P2 | Schema already undirected; need routing strategy (broadcast vs unicast) |
| Agent execution (codex app-server, PTY) | P1 (separate spec) | Uses the forward path unchanged; relay is agnostic |
| Telemetry / usage reporting | P3 | Requires privacy policy + opt-in UI |

---

## 13. ADR Index (proposed)

This spec implies four new ADRs; all to be authored alongside this spec's acceptance:

| # | Topic |
|---|---|
| 0009 | Broker architecture pivot: relay-centric WSS vs P2P Tailscale |
| 0010 | Exposure via Cloudflare Tunnel + Access (vs self-hosted TLS, vs Ngrok, vs bespoke relay in Workers) |
| 0011 | Envelope protocol for broker: kind-tagged routing instead of jsonrpsee-server |
| 0012 | SQLite + sqlx from day one (vs in-memory → later migrate) |

0001–0008 remain in force except where §1 and §10 explicitly override specific paragraphs (Tailscale assumptions, P2P data flows, Tailscale-specific error strings).
