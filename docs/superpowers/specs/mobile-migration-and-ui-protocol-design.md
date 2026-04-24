# Minos · Mobile Migration + Unified UI Protocol — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-24 |
| Owner | fannnzhang |
| Repository | `github.com/peterich-rs/minos` (public) |
| Proposed branch | `feat/mobile-and-ui-protocol` (worktree at `../minos-worktrees/mobile-and-ui-protocol`) |
| Supersedes (partial) | `flutter-app-and-frb-pairing-design.md` wholesale (Tailscale-direct pairing path no longer used); `minos-relay-backend-design.md` §6.1 `request_pairing_token` (renamed + expanded payload); §5.3 crate name `minos-relay` (renamed to `minos-backend`) |
| Related ADRs | 0001–0012 retained; proposes 0013–0015 (see §15) |

---

## 1. Context

The relay backend from `minos-relay-backend-design.md` shipped in plan 04 (`79bcbdf feat(relay): plan 04 — minos-relay backend (broker over WSS) (#1)`). The Mac agent runtime from plan 04's sister spec (`codex-app-server-integration-design.md`) also shipped: `minos-agent-runtime` spawns `codex app-server`, translates its notifications into the placeholder `AgentEvent` enum, and the macOS menu bar observes `AgentState`.

What has **not** happened yet:

1. The Flutter app still carries `flutter-app-and-frb-pairing-design.md`'s shape: it pairs directly to a Mac on a Tailscale IP via JSON-RPC 2.0, there is no envelope-aware client, and `MobileClient::pair_with(QrPayload)` expects a `{ip, port, token}` QR. Nothing in the mobile binary knows the word "relay".
2. `AgentEvent` is codex-shaped (TokenChunk / ToolCall / ToolResult / Reasoning / Done / Raw) and codex-only; claude / gemini integrations are deferred to `pty-agent-claude-gemini-design.md` but would produce different event shapes even if wired in.
3. There is no UI data model on the wire — the mobile app, once it reaches a `Connected` state, has nothing to render. Chat UI is blocked on both "how does data arrive" and "what shape is it in".

This spec closes those gaps. Two concerns are bundled here because they share one wire contract and because splitting them would force implementation to either (a) land the transport without knowing what it carries, or (b) define a payload schema without a validated delivery path. Keeping them together lets a single end-to-end integration test — *host emits raw codex events → backend stores + translates → mobile reads translated stream* — gate the whole thing.

What is **not** in this spec:

- The actual chat UI (Remodex-style bubbles, streaming text animation, tool-call cards, markdown rendering). A subsequent spec (tentative: `streaming-chat-ui-design.md`) owns that. This spec ships a deliberately plain debug viewer so the data contract can be validated without UI polish.
- Claude and Gemini translator implementations. The `AgentKind` enum values exist and translator function signatures are declared; bodies return `Err(TranslationNotImplemented)` until `pty-agent-claude-gemini-design.md` lands.
- Importing pre-existing codex rollout files (e.g. `~/.codex/sessions/*.jsonl`). Only sessions started through `start_agent` are ingested.
- Multi-host / remote-host deployments (Linux agent-host reaching a cloud backend). MVP assumes the agent-host and backend are on the same machine; host reaches backend via `127.0.0.1` loopback and does not need CF Access credentials.

---

## 2. Goals

### 2.1 MVP (this spec's scope)

1. **Crate rename.** `minos-relay` → `minos-backend`, with matching renames in env vars (`MINOS_RELAY_*` → `MINOS_BACKEND_*`), xtask commands (`relay-run` → `backend-run`), default DB filename (`minos-relay.db` → `minos-backend.db`), log-file prefix (`relay-*.xlog` → `backend-*.xlog`), and all source / doc references. No compatibility shim — we are pre-deployment.
2. **New crate `minos-ui-protocol`.** Holds the authoritative `UiEventMessage` type definition and the three translator functions `translate_codex` / `translate_claude` / `translate_gemini`. Each translator takes one raw CLI-native event (as `serde_json::Value`) and returns `Vec<UiEventMessage>` (one raw event can produce zero, one, or several UI events). Codex translator is fully implemented with fixtures; claude / gemini translators return `Err(TranslationNotImplemented)` until their own spec lands.
3. **Deprecate `AgentEvent`.** Drop the enum entirely from `minos-protocol::events`. `minos-agent-runtime` no longer translates; it forwards raw codex WS notifications untouched to `minos-backend`. The RPC method `subscribe_events` on `MinosRpc` is removed (it was never consumed by mobile; the new data path replaces it).
4. **New envelope variant `Envelope::Ingest`.** Agent-host → backend push channel. Carries `(agent, thread_id, seq, payload, ts_ms)` with the raw native JSON payload.
5. **New event variant `EventKind::UiEventMessage`.** Backend → mobile live broadcast. Carries the translated `UiEventMessage` plus correlation metadata (`thread_id`, `seq`, `ts_ms`).
6. **New `LocalRpcMethod` methods.** `ListThreads` (paginated thread list) and `ReadThread` (event history for one thread). Both return translated `UiEventMessage` data, never raw. Plus: rename `RequestPairingToken` → `RequestPairingQr` and widen its response to carry the full QR payload including CF Access Service Token fields.
7. **Backend persistence delta.** Two new SQLite tables: `threads` (one row per session, with title + lifecycle metadata) and `raw_events` (append-only `(thread_id, seq)` keyed log of raw CLI events). Only raw is stored; translation happens on-read (live fan-out and history queries).
8. **Credential centralisation.** CF Access Service Token moves from the Mac-side Keychain (spec §9.4 of `minos-relay-backend-design.md`) to the backend's env var configuration. Backend embeds those tokens into every issued pairing QR; mobile receives them via QR and stores them in its own Keychain. Agent-host on the same box connects via loopback and never sees CF tokens.
9. **Mobile relay migration.** `minos-mobile::MobileClient` rewrites its connection path from `ws://<tailscale-ip>:7878` to `wss://<backend-url>/devices` (with full envelope handling, CF Access headers, `X-Device-*` headers), replaces `pair_with(QrPayload)` with a new pairing flow driven by `LocalRpc::Pair`, and grows support for the three new `EventKind` messages (`UiEventMessage`, `PeerOnline`, `PeerOffline`, `ServerShutdown`).
10. **Mobile minimal viewer UI.** A two-page "debug viewer" to prove the end-to-end path: `ThreadListPage` shows a list of threads; tapping a row opens `ThreadViewPage`, which renders each `UiEventMessage` as a plain `ListTile` row. No input box, no chat bubbles, no markdown, no streaming animation. Chat UI is a follow-up spec.
11. **Tooling.** `cargo xtask check-all` keeps passing; worktree-aware rename; `cargo xtask backend-run` replaces `cargo xtask relay-run`; new fixture harness for `minos-ui-protocol::translate_codex` reuses `minos-agent-runtime::test_support::FakeCodexServer`'s scripted notifications as seed data.

### 2.2 Non-goals (explicit deferrals)

- **Chat UI.** Streaming token animation, tool-call cards with expand/collapse, markdown rendering, file-diff views, approval inboxes. → `streaming-chat-ui-design.md` (follow-up).
- **Claude / Gemini actual translation.** `AgentKind::Claude` / `AgentKind::Gemini` values exist, translator stubs compile, but real mapping + fixtures wait on `pty-agent-claude-gemini-design.md`.
- **Streaming tool-call arguments.** MVP ships `ToolCallPlaced { args_json: String }` + `ToolCallCompleted`. When streaming is needed later, three additional variants (`ToolCallStreamingStarted`, `ToolCallArgsDelta`, `ToolCallArgsFinalized`) are pure additions to the `UiEventMessage` enum; no version bump.
- **Rollout file import.** Existing `~/.codex/sessions/*.jsonl` from before this spec ships, or from direct-terminal codex use, are not imported into backend. Only events observed through `start_agent` go in.
- **Multi-host.** Single agent-host (on the same box as backend) in MVP. Remote Linux-host bootstrap, multi-host routing, host authentication beyond local loopback — all future specs.
- **End-to-end encryption of forwarded payloads.** Unchanged from `minos-relay-backend-design.md` §2.2: WSS + CF Access is the ring-0 guarantee; payload encryption is P2.
- **Retention / pruning of `raw_events`.** MVP stores forever. Pruning policy waits on real usage data.
- **Mobile sending input.** Mobile cannot type a message to an agent in this spec. That is a chat-UI concern. `send_user_message` RPC still exists on the Rust side (for future use), but no mobile-side surface invokes it.
- **Telemetry / usage reporting.** Unchanged; P3.

### 2.3 Testing philosophy (inherited, binding)

Unit tests across Rust and Flutter cover **logic only**. Widget tests are added only where the logic they exercise lives inside a Dart layer and cannot be reached via unit tests — specifically, the minimal viewer's "a list of `UiEventMessage` renders N rows" sanity check. No XCUITest, no integration_test scenarios. The real-device smoke gate (§12.5) is the sole functional-level validation.

### 2.4 UI-per-phase rule (inherited, binding)

This phase adds **two plain pages** (`ThreadListPage` + `ThreadViewPage`) and nothing else. No input, no buttons beyond "back" and "refresh". The chat-UI spec is free to rewrite both pages wholesale.

---

## 3. Tech Stack and Defaults

Inherits the stack from `minos-relay-backend-design.md` §3 and `flutter-app-and-frb-pairing-design.md` §3. Deltas:

| Area | Change |
|---|---|
| Rust workspace | Add one crate: `minos-ui-protocol`. No new external deps (reuses `serde`, `serde_json`, `thiserror`, `tracing`, `uuid` from the workspace) |
| `minos-backend` (renamed) | No dependency changes; new env vars `MINOS_BACKEND_CF_ACCESS_CLIENT_ID` and `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` |
| `minos-mobile` | No new external deps; frb regen produces new Dart types |
| Flutter | No new pubs; reuses `shadcn_ui`, `flutter_riverpod`, `mobile_scanner`, `flutter_secure_storage` (promoted from "P1" to "now" — needed for `cf_access_*` + `device_secret` + `backend_url` persistence). `flutter_secure_storage` version `^9.2.0` |
| CI | No change to matrix. Adds a `cargo sqlx prepare --check --workspace` step carried over from relay spec |

### 3.1 Relationship to `minos-agent-runtime`

`minos-agent-runtime` already owns the codex app-server connection (`crates/minos-agent-runtime/src/codex_client.rs`). In this spec it:

- **Loses** its `translate.rs` body (the codex → `AgentEvent` mapping). The file is removed; its golden fixtures move to `minos-ui-protocol/tests/golden/codex/`.
- **Gains** an ingest client: when `AgentRuntime::start` runs, it receives raw codex notifications on its WS read loop and hands each one to a new `Ingestor` handle that knows the `SessionHandle` to `minos-backend`. Exact wiring described in §5.4.

### 3.2 Relationship to `macos-relay-migration` worktree

The empty worktree at `../minos-worktrees/macos-relay-migration` created before this spec was started is superseded by this spec's work and can be deleted after the implementation plan ships. No code was committed to it.

---

## 4. Architecture Overview

```
 ┌────────────────── Cloudflare Edge ──────────────────┐
 │ minos.fan-nn.top                                    │
 │ Access policy: allow fannnzhang@...                 │
 │ Service Token validates non-browser clients         │
 └──────────────────┬───────────────────────┬──────────┘
                    │ WSS (mobile)          │ — (browser admin, future)
                    │ CF-Access-*           │
                    │ X-Device-*            │
                    ▼                       ▼
 ┌──────────── Backend box (Mac in MVP) ───────────────┐
 │ cloudflared tunnel (outbound QUIC)                  │
 │                                                     │
 │ ┌────────────────────────┐                          │
 │ │ minos-backend          │ 127.0.0.1:8787           │
 │ │   axum + tokio         │                          │
 │ │   SQLite:              │                          │
 │ │    devices             │                          │
 │ │    pairings            │                          │
 │ │    pairing_tokens      │                          │
 │ │    threads          NEW│                          │
 │ │    raw_events       NEW│                          │
 │ │   dispatcher:          │                          │
 │ │    LocalRpc (+ NEW     │                          │
 │ │      list_threads,     │                          │
 │ │      read_thread,      │                          │
 │ │      request_pairing_qr│                          │
 │ │     renamed from       │                          │
 │ │     request_pairing_   │                          │
 │ │     token)             │                          │
 │ │    Forward / Forwarded │                          │
 │ │    Ingest           NEW│                          │
 │ │    Event (+ NEW        │                          │
 │ │      UiEventMessage)   │                          │
 │ │   translators:         │                          │
 │ │    minos-ui-protocol   │                          │
 │ └──────────┬─────────────┘                          │
 │            │ WS loopback (no CF, no WSS)            │
 │            │                                        │
 │ ┌──────────▼─────────────┐                          │
 │ │ agent-host (Mac in MVP)│                          │
 │ │  minos-daemon          │                          │
 │ │   minos-agent-runtime: │                          │
 │ │    spawns codex child  │                          │
 │ │    WS client           │                          │
 │ │    Ingestor (NEW)      │                          │
 │ │  pairing client only   │                          │
 │ │    (no envelope peer   │                          │
 │ │     routing needed —   │                          │
 │ │     host is data src,  │                          │
 │ │     mobile is data     │                          │
 │ │     sink, backend is   │                          │
 │ │     authoritative)     │                          │
 │ └────────────────────────┘                          │
 └─────────────────────────────────────────────────────┘

  (mobile box, not co-located)
  ┌─────────────────────────────────────────────────────┐
  │ Minos (iOS)                                         │
  │  role: ios-client                                   │
  │  WS client (Flutter via frb → Rust)                 │
  │    MobileClient (rewritten)                         │
  │    consumes:                                        │
  │      Event::UiEventMessage (live)                   │
  │      LocalRpc::ListThreads / ReadThread responses   │
  │    emits:                                           │
  │      LocalRpc::Pair (one-shot)                      │
  │      LocalRpc::ListThreads / ReadThread             │
  │  Keychain:                                          │
  │    backend_url, device_id, device_secret,           │
  │    cf_access_client_id, cf_access_client_secret     │
  │  UI:                                                │
  │    PairingPage (existing, QR schema v2 updated)     │
  │    ThreadListPage (NEW, simple list)                │
  │    ThreadViewPage (NEW, plain ListTile per event)   │
  └─────────────────────────────────────────────────────┘
```

### 4.1 Three roles, three credential stores

| Role | Device | What it stores | How it authenticates to backend |
|---|---|---|---|
| Backend | Mac (MVP; future Linux/cloud) | CF Access Service Token (env var), SQLite DB | N/A — it *is* the backend |
| Agent-host | Mac (MVP) | `device_id`, `device_secret` (post-pair), `backend_url` | `127.0.0.1:8787` loopback, no CF token, no WSS; just `X-Device-*` headers |
| Mobile client | iPhone | `backend_url`, `device_id`, `device_secret`, `cf_access_client_id`, `cf_access_client_secret` — all in Keychain via `flutter_secure_storage` | `wss://<backend_url>` → CF edge → tunnel → backend; sends `CF-Access-*` + `X-Device-*` headers |

CF Access Service Token lives only in two places: the user's Cloudflare dashboard (authoritative) and the backend's env var (distribution source). It is never persisted on the agent-host. It reaches mobile only via the one-shot pairing QR.

### 4.2 Process model

Unchanged from current: backend and agent-host run in the same macOS process (Minos.app's embedded tokio runtime hosts both via separate crates). The backend binary `minos-backend` also exists as a standalone crate that can be run alone (`cargo xtask backend-run`) for development; in production Mac builds, `Minos.app`'s `DaemonBootstrap` may optionally spawn the backend too, but the simpler path keeps them decoupled and the backend run as a separate `launchd` service alongside `cloudflared`. This is described in more detail in §13.

For the implementation plan phase: start with the "backend as standalone binary, Mac.app connects to it via loopback" shape. Integrating them into one process is an optimisation for a later pass.

### 4.3 Protocol stack (top-down for mobile)

```
Cloudflare edge (Access + TLS)
  → HTTP/2 or QUIC tunnel
    → cloudflared
      → HTTP/1.1 Upgrade on 127.0.0.1:8787
        → WebSocket (tungstenite via axum)
          → Envelope JSON frames (v:1, kind-tagged)
            → either:
               · LocalRpc* (pair / list_threads / read_thread / ...)
               · Event (peer_online, peer_offline, server_shutdown,
                       ui_event_message NEW, unpaired, paired)
               · Forwarded (peer-to-peer RPC routed; unused in MVP
                            but preserved for future chat-input path)
```

For the agent-host talking to backend, drop TLS and CF layers — it is a plain loopback `ws://127.0.0.1:8787/devices`.

---

## 5. Crates and Their Changes

### 5.1 Rename `minos-relay` → `minos-backend`

Full rename, no alias crate. This is an atomic commit. Every touchpoint:

| Surface | Before | After |
|---|---|---|
| Crate directory | `crates/minos-relay/` | `crates/minos-backend/` |
| Cargo.toml workspace member | `minos-relay` | `minos-backend` |
| Library crate name | `minos_relay` | `minos_backend` |
| Binary crate name | `minos-relay` | `minos-backend` |
| All `use minos_relay::*` | | `use minos_backend::*` |
| xtask subcommand | `cargo xtask relay-run`, `relay-db-reset` | `cargo xtask backend-run`, `backend-db-reset` |
| Env var prefix | `MINOS_RELAY_LISTEN / _DB / _LOG_DIR / _TOKEN_TTL` | `MINOS_BACKEND_LISTEN / _DB / _LOG_DIR / _TOKEN_TTL` |
| New env vars | — | `MINOS_BACKEND_CF_ACCESS_CLIENT_ID`, `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` |
| Default DB filename | `./minos-relay.db`, `~/Library/Application Support/minos-relay/db.sqlite` | `./minos-backend.db`, `~/Library/Application Support/minos-backend/db.sqlite` |
| xlog prefix | `relay` (e.g. `relay-20260424.xlog`) | `backend` |
| Migrations dir | `crates/minos-relay/migrations/` | `crates/minos-backend/migrations/` |
| Doc language | "relay" | "backend" |
| ADR references | `relay-backend-design` | `backend-design` (filename unchanged since already shipped, but all new prose uses "backend") |

Pre-existing local databases (dev-only) are **not** migrated. Any developer with a populated `minos-relay.db` simply restarts with a fresh `minos-backend.db` — we are pre-deployment.

### 5.2 New crate: `minos-ui-protocol`

```
crates/minos-ui-protocol/
├── Cargo.toml
├── src/
│   ├── lib.rs              # re-exports: UiEventMessage, AgentKind alias (= minos_domain::AgentName), ThreadEndReason, MessageRole
│   ├── message.rs          # UiEventMessage enum + related types
│   ├── codex.rs            # translate_codex: serde_json::Value → Vec<UiEventMessage>
│   ├── claude.rs           # translate_claude: stub returning Err(TranslationNotImplemented)
│   ├── gemini.rs           # translate_gemini: stub returning Err(TranslationNotImplemented)
│   └── error.rs            # TranslationError (local type; surfaces via minos_domain::MinosError::TranslationFailed at crate boundary)
└── tests/
    ├── golden.rs           # rstest harness discovering tests/golden/**/*
    └── golden/
        └── codex/
            ├── thread_started.input.json
            ├── thread_started.expected.json
            ├── item_agent_message_delta.input.json
            ├── item_agent_message_delta.expected.json
            └── ...           # see §12.1 for required coverage
```

**Dependencies:**

```toml
[package]
name = "minos-ui-protocol"
version = "0.1.0"
edition = "2021"

[dependencies]
minos-domain = { path = "../minos-domain" }
serde        = { workspace = true, features = ["derive"] }
serde_json   = { workspace = true }
thiserror    = { workspace = true }
tracing      = { workspace = true }
uuid         = { workspace = true, features = ["v4"] }

[dev-dependencies]
rstest           = { workspace = true }
pretty_assertions = { workspace = true }

[lints]
workspace = true
```

`minos-ui-protocol` does **not** depend on `minos-protocol` — it defines its own types and is consumed by `minos-protocol` (for placing `UiEventMessage` inside `EventKind`). This dependency direction keeps `minos-ui-protocol` a leaf crate that backend / mobile / future surfaces can all include without pulling envelope / rpc definitions.

**Public API:**

```rust
// src/lib.rs
pub mod message;
pub use message::{UiEventMessage, MessageRole, ThreadEndReason};
pub use minos_domain::AgentName as AgentKind;  // alias, never redefine

mod codex;
mod claude;
mod gemini;
mod error;

pub use codex::translate as translate_codex;
pub use claude::translate as translate_claude;
pub use gemini::translate as translate_gemini;
pub use error::TranslationError;

/// One-shot dispatch by agent kind. Convenience for backend's translator loop.
pub fn translate(
    agent: AgentKind,
    raw_payload: &serde_json::Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    match agent {
        AgentKind::Codex  => translate_codex(raw_payload),
        AgentKind::Claude => translate_claude(raw_payload),
        AgentKind::Gemini => translate_gemini(raw_payload),
    }
}
```

### 5.3 Drop `AgentEvent`

`crates/minos-protocol/src/events.rs` currently re-exports `AgentEvent` from `minos-domain`. Both the enum itself and its re-export are deleted. `crates/minos-domain/tests/golden/agent_event_raw.json` is deleted.

Affected call-sites that must be rewritten or deleted:

| Location | Action |
|---|---|
| `crates/minos-agent-runtime/src/translate.rs` | Deleted; responsibilities move to `minos-ui-protocol::codex`. Fixtures migrate with it |
| `crates/minos-agent-runtime/src/runtime.rs` | The `broadcast::Sender<AgentEvent>` becomes `broadcast::Sender<IngestEvent>` carrying raw payloads (see §5.4). The state watch + lifecycle unchanged |
| `crates/minos-daemon/src/rpc_server.rs::subscribe_events` | **Removed**. The RPC method `subscribe_events` on `MinosRpc` is deleted from the trait and all impls. Mobile uses the new `Event::UiEventMessage` + `ReadThread` path instead |
| `crates/minos-protocol/src/rpc.rs` | Remove the `#[method(name = "subscribe_events")]` declaration |
| `crates/minos-ffi-uniffi/src/lib.rs` | No derives needed (`AgentEvent` was never on UniFFI surface per spec); confirm no stray references |
| `crates/minos-ffi-frb/` | frb-regen picks up deletion. Regenerate bindings; the Dart side `AgentEvent.*` symbols vanish |
| `apps/macos/Minos/Presentation/AgentSegmentView.swift` | Unchanged — observes `AgentState`, not `AgentEvent` |

### 5.4 `minos-agent-runtime` changes

Two structural changes:

1. **Stop translating.** `src/translate.rs` is deleted; `src/codex_client.rs`'s inbound pump hands each raw `Notification` (or `ServerRequest`) to a new `Ingestor` rather than running it through a translation table.
2. **Add the `Ingestor`.** A new struct that owns a WS connection to the backend (same envelope protocol as mobile, but authenticated as a host) and wraps each raw event in `Envelope::Ingest`.

```rust
// crates/minos-agent-runtime/src/ingest.rs

pub(crate) struct Ingestor {
    /// Outbound channel into the backend WS session.
    tx: tokio::sync::mpsc::Sender<Envelope>,
    /// Per-thread seq counter; persists across reconnects.
    seqs: dashmap::DashMap<String, u64>,
}

impl Ingestor {
    pub fn push(
        &self,
        agent: AgentKind,
        thread_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), SendError> {
        let seq = self.next_seq(thread_id);
        let env = Envelope::Ingest {
            version: 1,
            agent,
            thread_id: thread_id.to_string(),
            seq,
            payload,
            ts_ms: current_unix_ms(),
        };
        self.tx.try_send(env).map_err(SendError::from)
    }

    fn next_seq(&self, thread_id: &str) -> u64 { /* fetch_add on the dashmap entry */ }
}
```

The WS client connecting to backend is not `minos-transport::WsClient` directly because:

- The host talks envelopes, not jsonrpsee methods; `WsClient` wraps jsonrpsee.
- The host is a small special client — keeping a bespoke envelope-aware WS loop in `ingest.rs` is simpler than generalising `WsClient`.

For MVP the host-side WS client is ~200 lines of `tokio-tungstenite` doing: connect → send initial `LocalRpc::Pair` (if first boot) or silent auth via `X-Device-*` headers (if already paired) → loop sending `Envelope::Ingest` and receiving `Envelope::Event` (so the host sees `ServerShutdown` etc.). If this grows a second user later, promote to `minos-transport`.

Seq reset semantics: `(thread_id, seq)` is globally unique in `raw_events`. On reconnect, the host keeps its in-memory `seqs` map; the backend idempotent-inserts (`INSERT ... ON CONFLICT (thread_id, seq) DO NOTHING`) so retransmits after a brief disconnect don't double-write. If the host restarts, its in-memory map is lost, but the backend starts returning `IngestSeqConflict` for the old seqs — at which point the host requests the last persisted seq for each active thread via a new LocalRpc helper (§6.3) before resuming. For MVP and the "host never restarts while a thread is active" common case, this conflict path is a safety net, not a primary flow.

### 5.5 `minos-backend` changes (beyond the rename)

New modules and responsibilities:

```
crates/minos-backend/src/
├── ...                          # existing
├── ingest/
│   ├── mod.rs                   # Envelope::Ingest dispatcher
│   └── translate.rs             # calls minos_ui_protocol::translate, fans out EventKind::UiEventMessage
├── store/
│   ├── threads.rs               # CRUD on the new threads table
│   └── raw_events.rs            # append-only insert + range queries
└── http/
    └── ws_devices.rs            # extended to route Envelope::Ingest to ingest::dispatch,
                                 # plus new LocalRpc handlers (list_threads / read_thread / request_pairing_qr)
```

Backend pairing role: the pairing table gains the ability to record the agent-host device (`role = 'agent-host'` instead of `mac-host` — rename for platform neutrality). Agent-host's `device_secret` gating is identical to mobile's; the same pair-then-paired-mode FSM applies.

### 5.6 `minos-mobile` changes

Full rewrite of `crates/minos-mobile/src/client.rs` — though the public `MobileClient` surface preserves the `pair_with_json` entry point (shape updated to new QR schema v2) and the `events_stream` / `current_state` pair. Internal:

- Remove all Tailscale-related code (none of `minos-mobile::WsClient` construction with tailnet URLs survives).
- Replace WS client with envelope-speaking loop (reusing / subclassing the one in `ingest.rs` would be ideal; for MVP it's ok to have two similar loops — §5.4 note applies).
- Add persistent state fields for `backend_url`, `cf_access_client_id`, `cf_access_client_secret`, `device_id`, `device_secret`. The `PairingStore` trait is extended (new methods on the trait, `InMemoryPairingStore` updated; Dart-side `FlutterSecureStoragePairingStore` is new — see §10).
- Add inbound handlers for `Event::UiEventMessage` (forwarded through a new `ui_events_stream: broadcast::Receiver<UiEventMessage>`) and `Event::PeerOnline / PeerOffline / ServerShutdown / Unpaired` (feed existing `ConnectionState` state machine).
- Add outbound helpers `list_threads(limit, before_ts_ms, agent)` and `read_thread(thread_id, from_seq, limit)` returning `Vec<UiEventMessage>`.

### 5.7 `minos-daemon` changes

`minos-daemon` was designed as the composition root of the Mac process — but in the new architecture the WS server on port 7878 is gone (mobile connects to backend, not to daemon). What remains:

- Spawn and supervise `AgentRuntime` (unchanged surface).
- Spawn and supervise the `Ingestor`'s WS connection to backend.
- Provide `DaemonHandle` to Swift UI (unchanged).
- `start_autobind` no longer binds a public WS; it establishes the host's WS connection to backend instead. Rename to `start_ingest_link` in the impl; Swift keeps calling `startAutobind` for one release, then renamed in a follow-up. (Or rename both at once in this spec — decided at implementation time; `DaemonBootstrap` in Swift is small.)

Swift-side changes end here; the macOS chat UI is not in scope.

### 5.8 `minos-protocol` changes

| Item | Change |
|---|---|
| `Envelope` | Add `Ingest { v, agent: AgentKind, thread_id: String, seq: u64, payload: Value, ts_ms: i64 }` |
| `EventKind` | Add `UiEventMessage { thread_id: String, seq: u64, ui: UiEventMessage, ts_ms: i64 }` |
| `LocalRpcMethod` | Rename `RequestPairingToken` → `RequestPairingQr`. Add `ListThreads`, `ReadThread` |
| `events::AgentEvent` | **Delete** (§5.3) |
| `rpc::MinosRpc` | Remove `subscribe_events` method; keep `start_agent / send_user_message / stop_agent` (still routed peer-to-peer via `Forward`) |
| `messages.rs` | Add `ListThreadsRequest`, `ListThreadsResponse`, `ThreadSummary`, `ReadThreadRequest`, `ReadThreadResponse`, `PairingQrPayload` (new — see §7.2) |

### 5.9 Sharing matrix (post-changes)

| Crate | backend | agent-host | mobile | browser admin (future) |
|---|---|---|---|---|
| `minos-domain` | ✓ | ✓ | ✓ | ✓ |
| `minos-protocol` | ✓ | ✓ | ✓ | ✓ |
| `minos-ui-protocol` | ✓ | — | ✓ | ✓ |
| `minos-pairing` | ✓ (store) | ✓ (store) | ✓ (store) | — |
| `minos-transport` | — | partial (§5.4) | partial (§5.6) | — |
| `minos-cli-detect` | — | ✓ | — | — |
| `minos-daemon` | — | ✓ | — | — |
| `minos-mobile` | — | — | ✓ | — |
| `minos-agent-runtime` | — | ✓ | — | — |
| `minos-backend` | ✓ | — | — | — |
| `minos-ffi-uniffi` | — | ✓ | — | — |
| `minos-ffi-frb` | — | — | ✓ | — |

---

## 6. Protocol: Envelope + UI Event Model

### 6.1 Envelope delta

Adding one variant to `Envelope` and one to `EventKind`. Everything else preserved.

```rust
// crates/minos-protocol/src/envelope.rs (delta)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    // ... existing: LocalRpc, LocalRpcResponse, Forward, Forwarded, Event

    /// Host → Backend. Host pushes a single raw agent-native event for
    /// persistence and fan-out. No response is expected; delivery is
    /// at-least-once via retry, and idempotence is enforced by the
    /// (thread_id, seq) primary key in `raw_events`.
    Ingest {
        #[serde(rename = "v")] version: u8,
        agent: AgentKind,                  // = minos_domain::AgentName
        thread_id: String,
        seq: u64,                          // per-thread monotonic, host-assigned
        payload: serde_json::Value,        // raw CLI JSON
        ts_ms: i64,
    },
}
```

```rust
// events: extend EventKind
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    // ... existing: Paired, PeerOnline, PeerOffline, Unpaired, ServerShutdown

    /// Backend → Mobile. One translated UI event from the backend's
    /// live fan-out. `seq` matches the corresponding `raw_events` row
    /// so mobile can resume with `read_thread(thread_id, from_seq)`.
    UiEventMessage {
        thread_id: String,
        seq: u64,
        ui: minos_ui_protocol::UiEventMessage,
        ts_ms: i64,
    },
}
```

Golden JSON fixture (added to the existing `crates/minos-protocol/tests/golden/envelope/` harness):

```json
{
  "kind": "ingest",
  "v": 1,
  "agent": "codex",
  "thread_id": "thr_abc",
  "seq": 42,
  "payload": { "method": "item/agentMessage/delta", "params": { "delta": "Hi" } },
  "ts_ms": 1714000000000
}
```

```json
{
  "kind": "event",
  "v": 1,
  "type": "ui_event_message",
  "thread_id": "thr_abc",
  "seq": 42,
  "ui": {
    "kind": "text_delta",
    "message_id": "msg_def",
    "text": "Hi"
  },
  "ts_ms": 1714000000000
}
```

### 6.2 `LocalRpcMethod` additions and renames

```rust
// crates/minos-protocol/src/envelope.rs
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum LocalRpcMethod {
    Ping,
    RequestPairingQr,     // was RequestPairingToken
    Pair,
    ForgetPeer,
    ListThreads,          // NEW
    ReadThread,           // NEW
}
```

| Method | Caller role | Pre-state | Params | Success result | Errors |
|---|---|---|---|---|---|
| `request_pairing_qr` | `agent-host` | Paired OR Unpaired | `{"host_display_name": "..."}` — name is host-chosen, will be shown on mobile | `{"qr_payload": PairingQrPayload}` (see §7.2) | `CfAccessMisconfigured` if backend lacks CF env vars and the host's mobile-facing deploy requires them |
| `pair` | `ios-client` | Unpaired | `{"token": "...", "device_name": "..."}` | `{"peer_device_id": "...", "peer_name": "...", "your_device_secret": "..."}` (unchanged) | `PairingTokenInvalid`, `PairingStateMismatch` |
| `list_threads` | any paired role | Paired | `{"limit": u32, "before_ts_ms": Option<i64>, "agent": Option<AgentKind>}` | `{"threads": [ThreadSummary], "next_before_ts_ms": Option<i64>}` | — |
| `read_thread` | any paired role | Paired | `{"thread_id": "...", "from_seq": Option<u64>, "limit": u32}` | `{"ui_events": [UiEventMessage], "next_seq": Option<u64>, "thread_end_reason": Option<ThreadEndReason>}` | `ThreadNotFound`, `TranslationFailed` |
| `forget_peer` | any | Paired | `{}` | `{"ok": true}` | `Unauthorized` if unpaired |
| `ping` | any | any | `{}` | `{"ok": true}` | never |

`ThreadSummary` shape:

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub agent: AgentKind,
    pub title: Option<String>,                 // CLI-provided or backend fallback (§7)
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,                    // derived from raw_events rows (not precise message count — see §6.6)
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
}
```

### 6.3 Host-only helper LocalRpc

One more method, callable only by agent-host role, used on host reboot to resume ingest:

```rust
// Added to LocalRpcMethod:
GetThreadLastSeq,
```

Params: `{"thread_id": "..."}`; result `{"last_seq": u64}` (returns `0` if thread unknown). Allows the host to determine where to resume its `seq` counter after a crash-restart; any `Envelope::Ingest` with `seq <= last_seq` is dropped server-side via the unique `(thread_id, seq)` constraint.

### 6.4 `UiEventMessage` shape (authoritative)

```rust
// crates/minos-ui-protocol/src/message.rs

use minos_domain::AgentName as AgentKind;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEventMessage {
    // ── Thread lifecycle ───────────────────────────────
    ThreadOpened {
        thread_id: String,
        agent: AgentKind,
        title: Option<String>,
        opened_at_ms: i64,
    },
    ThreadTitleUpdated {
        thread_id: String,
        title: String,
    },
    ThreadClosed {
        thread_id: String,
        reason: ThreadEndReason,
        closed_at_ms: i64,
    },

    // ── Message boundaries ─────────────────────────────
    MessageStarted {
        message_id: String,                    // backend-synthesised UUIDv4; never a CLI-native id
        role: MessageRole,
        started_at_ms: i64,
    },
    MessageCompleted {
        message_id: String,
        finished_at_ms: i64,
    },

    // ── Message content (inside an open message) ─────────
    TextDelta {
        message_id: String,
        text: String,                          // append-only; mobile concatenates
    },
    ReasoningDelta {
        message_id: String,
        text: String,
    },

    // ── Tool calls (non-streaming args in MVP; see §2.2 for forward-compat plan) ──
    ToolCallPlaced {
        message_id: String,
        tool_call_id: String,                  // backend-synthesised UUIDv4
        name: String,
        args_json: String,
    },
    ToolCallCompleted {
        tool_call_id: String,
        output: String,
        is_error: bool,
    },

    // ── Meta / escape hatches ─────────────────────────
    Error {
        code: String,                          // snake_case of minos_domain::ErrorKind
        message: String,
        message_id: Option<String>,
    },
    Raw {
        kind: String,                          // native CLI method name the translator didn't recognise
        payload_json: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,                                    // reserved for host/network/server status injections
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadEndReason {
    UserStopped,
    AgentDone,
    Crashed { message: String },
    Timeout,
    HostDisconnected,                          // host WS dropped while thread was open
}
```

### 6.5 Design rationale (condensed)

- **Event-level, not message-level.** One wire frame = one incremental step. Mobile's state machine aggregates by `message_id` to assemble a message. Live and history go through the same pipeline (both emit `Vec<UiEventMessage>`), so the renderer has one code path.
- **`message_id` is backend-synthesised UUID.** Codex/claude/gemini each have their own id schemes; using a stable UUID generated at `MessageStarted` translation time means the mobile state machine never branches on agent kind.
- **`TextDelta` vs `ReasoningDelta` separate.** Mobile can choose to fold/collapse reasoning independently. Codex's o-series reasoning stream and claude's thinking block both map here; gemini (no reasoning stream) never emits `ReasoningDelta`.
- **`Raw` is a forward-compat escape.** When a codex notification's method doesn't match any branch in `translate_codex`, the translator emits a single `Raw { kind, payload_json }`. Mobile defaults to ignoring; the debug viewer in §10 shows it as `[Raw] <kind>: <payload preview>`.
- **`MessageRole::System` is kept.** Future: the backend synthesises "host disconnected" / "CF Access misconfigured" / "translation failed" system messages inlined into the thread's UI stream. MVP does not emit them yet; leaving the variant in place avoids a schema version bump when it starts being used.
- **No file-diff variant.** Patches arrive as `ToolCallPlaced { name: "apply_patch", args_json }` + `ToolCallCompleted { output }`. Future chat-UI spec may add a `FileEditPatch` variant; that addition is schema-additive.

### 6.6 Version strategy

- Envelope `v: 1` is unchanged. Adding `Envelope::Ingest` and `EventKind::UiEventMessage` are **additive** enum variants — serde deserialisers on older clients receive `"kind": "ingest"` and fall through their exhaustive match with an "unknown kind" error, closing the socket with WS code `4400`. Backend emits only what clients negotiated to understand. For MVP all three parties ship at the same version, so no negotiation is needed.
- `UiEventMessage` enum itself is permitted to grow new variants without any version bump. Mobile must treat unknown `kind` values as "ignore, log a warning" — not as protocol violations. Rust deserialises unknown variants via `#[serde(other)]` catch-all? No — serde_json on a tagged enum will fail on unknown tag; we handle this by **always** serialising a Raw fallback at the translator layer, so a future variant only appears if both ends were updated. Mobile built against an older `minos-ui-protocol` will simply fail to decode the frame and emit a `TranslationFailed` warning; the frame is skipped. This is acceptable because the backend is the source and can always elect to downgrade (emit `Raw { kind, payload_json }`) when it knows the peer is older. In MVP no downgrade logic exists; when it's needed, a `negotiate` LocalRpc on first connect can communicate min/max UiEventMessage versions.
- `message_count: u32` in `ThreadSummary` is "number of raw events of known-message kinds" — an approximation sufficient for list preview. It is not a precise count of rendered message bubbles on the mobile side.

---

## 7. Pairing Flow Rebuild

### 7.1 Credential distribution

```
  Cloudflare Dashboard (authoritative)
        │
        │ 1. User creates Service Token,
        │    copies Client-Id + Client-Secret
        ▼
  Backend host (= same Mac box in MVP)
  ┌──────────────────────────────────────────────────────┐
  │ env vars (set in launchd plist or shell before start)│
  │  MINOS_BACKEND_CF_ACCESS_CLIENT_ID=...               │
  │  MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET=...           │
  │                                                      │
  │ Backend reads on startup; stores in memory.          │
  │ Never written to the SQLite DB.                      │
  └───────────────┬─────────────────────────────┬────────┘
                  │                             │
                  │ 2. Agent-host requests      │ 3. Backend embeds CF token
                  │    pairing QR via           │    pair into the QR payload
                  │    LocalRpc                 │    it signs and returns
                  │                             │    to the host
                  ▼                             │
         Agent-host displays QR                 │
         (never sees CF token value)            │
                  │                             │
                  │ 4. Phone scans QR           │
                  ▼                             │
         Phone parses QR payload ──────────────┘
                  │
                  │ 5. Phone stores in Keychain
                  │    (flutter_secure_storage):
                  │      backend_url
                  │      cf_access_client_id
                  │      cf_access_client_secret
                  │      (+ later: device_id, device_secret)
                  │
                  │ 6. Phone connects to backend
                  │    wss://<backend_url>/devices
                  │    headers: CF-Access-Client-* (from Keychain)
                  │             X-Device-Id: <new UUIDv4>
                  │             X-Device-Role: ios-client
                  │
                  │ 7. Phone calls LocalRpc::Pair
                  │    with pairing_token from QR
                  │
                  ▼
         Backend validates token → issues device_secret → Paired
                  │
                  │ 8. Phone stores device_secret in Keychain
                  ▼
         Mobile re-establishes WS with X-Device-Secret on next connect
```

### 7.2 `PairingQrPayload` schema (v2)

```rust
// crates/minos-protocol/src/messages.rs
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairingQrPayload {
    /// QR payload version. Clients that see a v they don't understand
    /// reject the scan with a clear "update app" message.
    pub v: u8,                                 // = 2 in this spec (was 1 implicitly in Tailscale era)
    pub backend_url: String,                   // e.g. "wss://minos.fan-nn.top/devices"
    pub host_display_name: String,             // echoed from host's request_pairing_qr arg
    pub pairing_token: String,                 // 32-byte hex
    pub expires_at_ms: i64,                    // 5 min from issuance
    /// CF Access Service Token: present when backend is reached through
    /// a CF-gated hostname. When backend is configured without CF token
    /// env vars (dev-only), these fields are omitted — scanning such a
    /// QR tells the phone "connect without CF headers".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_secret: Option<String>,
}
```

Example (with CF):

```json
{
  "v": 2,
  "backend_url": "wss://minos.fan-nn.top/devices",
  "host_display_name": "fannnzhang's MacBook",
  "pairing_token": "7f3e...a1b2",
  "expires_at_ms": 1714000300000,
  "cf_access_client_id": "abcd1234.access",
  "cf_access_client_secret": "e5f6..."
}
```

QR image encoding policy:
- JSON is serialised with compact separators.
- Rendered as QR code ECC level `M` (medium), which is sufficient for ~350 bytes and has reasonable scan robustness.
- Mobile uses existing `mobile_scanner` to decode.

Security note: the QR is displayed only on the user's own screen (`Minos.app` menu → "Show QR" on Mac), scanned only by the user's own phone camera. Exposure window is 5 minutes. The CF Client Secret's blast radius is "can reach `minos.fan-nn.top`", which is gated behind the application-layer `X-Device-Secret` check; so embedding the CF token in QR is no worse than embedding the pairing token itself.

### 7.3 Pairing timing diagram

```
[Host]                                  [Backend]                        [iPhone]

boot
 │
 ├─ WS connect ws://127.0.0.1:8787/devices
 │   headers: X-Device-Id  (from Keychain, or new if first boot)
 │            X-Device-Role: agent-host
 │            (no CF-Access-* — loopback)
 │            (no X-Device-Secret — unpaired until done)
 │                                          │
 │                                          ├─ no device row → insert
 │                                          │   (secret_hash = NULL, role = agent-host)
 │                                          ├─ session = UNPAIRED
 │                                          └─ WS 101 Upgrade
 │ ◄── Event{type: "unpaired"} ─────────────│
 │
 │ user clicks "Show QR" in menu bar
 │ LocalRpc{method: "request_pairing_qr",
 │          params: {host_display_name: "fannnzhang's MacBook"}}
 │ ──────────────────────────────────────►
 │                                          ├─ generate 32B token
 │                                          ├─ hash + insert pairing_tokens
 │                                          │     (issuer = mac device_id)
 │                                          ├─ read CF env vars
 │                                          └─ build PairingQrPayload
 │ ◄── LocalRpcResponse{status: "ok",
 │        result: {qr_payload: {...v:2...}}}
 │
 ├─ serialise qr_payload → render QR image
 │ user points phone at screen
 │                                                                           │
 │                                                                           │ app first-boot
 │                                                                           │   no stored creds
 │                                                                           │   → PairingPage
 │                                                                           │
 │                                                                           │ camera reads QR
 │                                                                           │ parse payload; store
 │                                                                           │   backend_url, cf_access_*
 │                                                                           │   in Keychain
 │                                                                           │
 │                                                                           │ open WS connect
 │                                                                  wss://<backend_url>/devices
 │                                                                  headers:
 │                                                                    CF-Access-Client-Id
 │                                                                    CF-Access-Client-Secret
 │                                                                    X-Device-Id: <new UUIDv4>
 │                                                                    X-Device-Role: ios-client
 │                                          ◄──────────────────────────────── │
 │                                          ├─ CF edge validates tokens → OK
 │                                          ├─ no device row → insert
 │                                          └─ WS 101 Upgrade
 │                                          ── Event{type: "unpaired"} ────►
 │                                                                           │
 │                                          ◄── LocalRpc{method: "pair",
 │                                              params: {token, device_name}}
 │                                          ├─ token valid, not expired, not consumed
 │                                          ├─ mark consumed_at
 │                                          ├─ gen DeviceSecret_mac (argon2id)
 │                                          ├─ gen DeviceSecret_phone (argon2id)
 │                                          ├─ insert pairings row (mac, phone)
 │                                          └─ upgrade both sessions to PAIRED
 │ ◄── Event{type: "paired",                                                 │
 │        peer_device_id: <phone>,                                           │
 │        peer_name: <name>,                                                 │
 │        your_device_secret: <mac_secret>} ────────────────────────────────►│
 │                                          ── LocalRpcResponse{status:ok,   │
 │                                              result: {peer_device_id,     │
 │                                                       peer_name,          │
 │                                                       your_device_secret}}│
 │                                                                           │
 │ Mac daemon stores device_secret in Keychain                               │
 │ (next reconnect sends X-Device-Secret for PAIRED mode)                    │
 │                                                                           │
 │                                                                     phone Keychain stores
 │                                                                       device_id, device_secret
 │                                                                     UI → ThreadListPage
```

### 7.4 Reconnect and peer status

Already specified in `minos-relay-backend-design.md` §7.2 and §7.4; preserved verbatim. The only change is that the host's reconnect is over `127.0.0.1:8787`, not over CF edge.

`EventKind::UiEventMessage` delivery is **not** replayed on reconnect — if mobile misses live events while offline, it calls `ReadThread{from_seq}` to catch up. The `ReadThread` response is authoritative; live fan-out starts delivering new events from the moment mobile is back online. Dedup is by `seq` on the mobile side: mobile keeps a per-thread `max_seq_seen` watermark; any `UiEventMessage` with `seq <= max_seq_seen` is dropped.

### 7.5 Errors unique to the new flow

| Scenario | Error | Surface |
|---|---|---|
| Backend env var `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` absent but listener is publicly exposed | `CfAccessMisconfigured` raised at `request_pairing_qr` time; host's menu bar shows "配置不全:后端缺 CF Access 凭据" | Error surfaces to user at QR time, before any QR is shown |
| Host runs against a production backend but was never paired + backend has a stale device row from a different backend install | `DeviceNotTrusted` on WS connect; host wipes its local device creds and restarts the pairing flow | Blocked; host emits a Swift alert |
| `pair()` via relay succeeds but phone's subsequent reconnect hits `4401` (wrong secret) | Phone wipes local Keychain and returns to `PairingPage` | Phone UX inherited from relay spec |
| Mobile's QR scan yields `v > 2` | Reject with "App 版本过旧,请升级" | Inherits from existing scan-validation code |

---

## 8. Data Flow

### 8.1 Session start → live delivery

```
[User on Mac]  clicks "启动 Codex(测试)" in menu bar          (debug build; or via a future chat-UI "new thread" button)
  │
  ├─ AppState.startAgent() → DaemonHandle::start_agent(.codex)
  │
  ▼
[Rust: minos-agent-runtime]
  AgentRuntime::start(Codex)
    ├─ spawns codex app-server child (unchanged)
    ├─ connects WS to codex (unchanged)
    ├─ JSON-RPC initialize + thread/start → thread_id = "thr_abc"
    ├─ state_tx.send(Running { thread_id: "thr_abc", .. })
    ├─ Ingestor.push(Codex, "thr_abc", <payload of thread/started>) seq=1
    │
    (now every codex Notification on the WS read loop:)
    ├─ Notification{ method: "item/agentMessage/delta", params: {...} }
    │   → Ingestor.push(Codex, "thr_abc", <raw notification JSON>) seq=2
    ├─ Notification{ method: "item/toolCall/started", params: {...} }
    │   → Ingestor.push(Codex, "thr_abc", <raw>) seq=3
    └─ ...

[Rust: agent-host WS client → backend]
  Envelope::Ingest { v: 1, agent: codex, thread_id: "thr_abc", seq: N, payload: <raw>, ts_ms }
  sent as one WS text frame per raw event

[Rust: minos-backend ingest dispatcher]
  On Envelope::Ingest:
    ├─ INSERT INTO raw_events (thread_id, seq, agent, payload_json, ts_ms)
    │   ON CONFLICT (thread_id, seq) DO NOTHING
    ├─ UPSERT INTO threads (thread_id, agent, last_ts_ms, ...)
    ├─ minos_ui_protocol::translate(agent, &payload)
    │   → Vec<UiEventMessage>     (zero, one, or several)
    │
    (for each translated UiEventMessage ui:)
    ├─ for every paired mobile session currently Online:
    │   session.outbox.send(Envelope::Event {
    │     v: 1,
    │     event: EventKind::UiEventMessage { thread_id, seq, ui, ts_ms }
    │   })

[Rust: mobile MobileClient]
  On inbound Envelope::Event { event: UiEventMessage { thread_id, seq, ui, ts_ms } }:
    ├─ if seq <= max_seq_seen[thread_id]: drop (dedup)
    ├─ else: forward ui to Dart via StreamSink<UiEventMessage>
    └─ update max_seq_seen[thread_id] = seq

[Dart: ThreadViewPage]
  Riverpod `threadEventsProvider(thread_id).stream` yields the UI event.
  ListTile row is appended.
```

The translator is synchronous (CPU-only); backend does the translate inline on the WS ingest handler. If the translator throws `TranslationError::Unknown`, backend still persists the raw (it was inserted before translate was called) and emits no fan-out event for this raw — but also emits a single `EventKind::UiEventMessage { ui: UiEventMessage::Error { code: "translation_failed", ... } }` so the mobile user isn't left wondering.

### 8.2 History load on mobile first open

```
ThreadListPage.onLoad
 │
 ├─ ref.read(minosCoreProvider).listThreads(limit: 50, before: null, agent: null)
 │   → Dart → frb → Rust MobileClient::list_threads
 │     → Envelope::LocalRpc { method: list_threads, params: {...} } to backend
 │     ← Envelope::LocalRpcResponse { status: ok, result: {threads: [...]} }
 │
 └─ builds list. Each row tappable.

ThreadViewPage.onLoad(thread_id)
 │
 ├─ ref.read(minosCoreProvider).readThread(thread_id, from_seq: 0, limit: 500)
 │   → LocalRpc::read_thread → backend
 │     → SELECT * FROM raw_events WHERE thread_id = ? AND seq >= ? ORDER BY seq LIMIT ?
 │     → for each row: minos_ui_protocol::translate → collect Vec<UiEventMessage>
 │     ← LocalRpcResponse { status: ok, result: { ui_events: [...], next_seq: Option } }
 │
 ├─ if next_seq is Some: auto-load next page (simple paging in MVP — clicks a "Load More")
 │
 └─ subscribe to live `ui_events_stream` for this thread_id
    from_seq for live dedup = last seq from history response
```

### 8.3 Host restart mid-session (resume)

```
[Host]  process dies mid-stream, codex child is killed (kill_on_drop)
[Backend]  host's WS disconnects; session dropped from registry
           but raw_events rows for "thr_abc" up to seq=9 are persisted
[Host]  restarts
  ├─ AgentRuntime::start — no codex yet; state = Idle
  └─ in MVP, there's no auto-resume of the prior codex session
     (codex sessions are not resumable across app restarts in this phase;
      the thread is considered orphaned — backend emits
      ThreadClosed { reason: HostDisconnected } when the WS drops)

[User]  opens mobile → sees "thr_abc" in ThreadListPage with a "closed (host disconnected)" badge
```

Resuming a live codex session across a host restart is not in scope. The thread is archived by the backend's disconnection detector; the mobile viewer shows it as closed.

### 8.4 Forget peer (unchanged)

Mechanically identical to `minos-relay-backend-design.md` §7.4. New detail: on `unpaired` the mobile also clears its `ui_events_stream` watermarks and thread list caches.

### 8.5 Backend shutdown

Also unchanged from the parent spec §7.5. New: `ServerShutdown` now also triggers mobile to stop any in-flight `ReadThread` pagination.

---

## 9. Persistence (SQLite)

### 9.1 New tables

```sql
-- 0004_threads.sql
CREATE TABLE threads (
    thread_id         TEXT PRIMARY KEY,                         -- host-generated, e.g. codex thread_id
    agent             TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    owner_device_id   TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    title             TEXT,                                     -- nullable; populated by ThreadTitleUpdated translator output or backend fallback
    first_ts_ms       INTEGER NOT NULL,
    last_ts_ms        INTEGER NOT NULL,
    ended_at_ms       INTEGER,
    end_reason        TEXT,                                     -- NULL until ended; JSON-serialised ThreadEndReason
    message_count     INTEGER NOT NULL DEFAULT 0                -- incremented per raw_events insert; approximate for UI preview
) STRICT;

CREATE INDEX idx_threads_last_ts  ON threads(last_ts_ms DESC);
CREATE INDEX idx_threads_owner    ON threads(owner_device_id, last_ts_ms DESC);
```

```sql
-- 0005_raw_events.sql
CREATE TABLE raw_events (
    thread_id    TEXT NOT NULL REFERENCES threads(thread_id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    agent        TEXT NOT NULL CHECK (agent IN ('codex','claude','gemini')),
    payload_json TEXT NOT NULL,                                 -- serialised serde_json::Value as text
    ts_ms        INTEGER NOT NULL,
    PRIMARY KEY (thread_id, seq)
) STRICT;

CREATE INDEX idx_raw_events_thread_seq ON raw_events(thread_id, seq);
```

### 9.2 Title population order

On every `Envelope::Ingest`:

1. `translate(agent, payload)` → `Vec<UiEventMessage>`.
2. If the vec contains a `ThreadTitleUpdated { title }`, UPDATE `threads.title`.
3. ELSE, if `threads.title` is NULL AND the vec contains a `MessageStarted { role: User }` followed by a `TextDelta { text }` (or equivalent path), set `threads.title` to the first 80 characters of that text and additionally emit a synthetic `ThreadTitleUpdated { thread_id, title }` in the fan-out for mobile to pick up.
4. ELSE, `threads.title` remains NULL until (2) or (3) fires.

Rationale for the synthetic fallback: `ListThreads` shows titles to the user; leaving a thread title-less is bad UX. First-user-message prefix is the common heuristic (mirrors ChatGPT/Claude mobile UI).

### 9.3 DB file location (updated)

Dev: `./minos-backend.db` in CWD by default, override via `MINOS_BACKEND_DB=...`. Prod on Mac: `~/Library/Application Support/minos-backend/db.sqlite`.

### 9.4 Retention

None in MVP. `raw_events` grows forever. At 1 KB per raw event and ~200 events per session at a generous estimate, 500 sessions = 100 MB; acceptable. Pruning policy (e.g., keep last 90 days) is a P1 concern gated on actual usage.

### 9.5 Migration strategy

`sqlx::migrate!("./migrations")` applies sequentially. Existing dev databases from the relay era are not migrated — developers delete `minos-relay.db` or start fresh.

---

## 10. Mobile Deliverables

### 10.1 `MobileClient` rewrite

`crates/minos-mobile/src/client.rs` changes summarised:

```rust
pub struct MobileClient {
    store: Arc<dyn PairingStore>,
    ws: Arc<tokio::sync::Mutex<Option<WsClient>>>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    ui_events_tx: broadcast::Sender<(String, u64, UiEventMessage, i64)>,  // (thread_id, seq, ui, ts_ms)
    device_id: DeviceId,
    self_name: String,
}

impl MobileClient {
    pub async fn pair_with_qr_json(&self, qr_json: String) -> Result<PairOutcome, MinosError>;
    pub async fn list_threads(&self, req: ListThreadsRequest) -> Result<ListThreadsResponse, MinosError>;
    pub async fn read_thread(&self, req: ReadThreadRequest) -> Result<ReadThreadResponse, MinosError>;
    pub fn events_stream(&self) -> watch::Receiver<ConnectionState>;
    pub fn ui_events_stream(&self) -> broadcast::Receiver<(String, u64, UiEventMessage, i64)>;
}
```

The `PairingStore` trait extends to persist / retrieve:

```rust
#[async_trait::async_trait]
pub trait PairingStore: Send + Sync {
    async fn load_backend_url(&self) -> Result<Option<String>, MinosError>;
    async fn save_backend_url(&self, url: &str) -> Result<(), MinosError>;
    async fn load_cf_access(&self) -> Result<Option<(String, String)>, MinosError>;
    async fn save_cf_access(&self, id: &str, secret: &str) -> Result<(), MinosError>;
    async fn load_device(&self) -> Result<Option<(DeviceId, DeviceSecret)>, MinosError>;
    async fn save_device(&self, id: &DeviceId, secret: &DeviceSecret) -> Result<(), MinosError>;
    async fn clear_all(&self) -> Result<(), MinosError>;
}
```

`InMemoryPairingStore` preserved for Rust unit tests. A new Dart-side implementation `FlutterSecureStoragePairingStore` is wired from Dart through an frb callback; its implementation lives in Dart and calls `flutter_secure_storage` directly. (This unblocks ever persisting secrets on iOS — plan 03 had it as Tier B.)

### 10.2 Flutter `MinosCore` changes

```dart
abstract class MinosCoreProtocol {
  // existing
  Future<void> pairWithQrJson(String qrJson);
  Stream<ConnectionState> get connectionStates;
  ConnectionState get currentConnectionState;

  // new
  Future<List<ThreadSummary>> listThreads({int limit = 50, DateTime? before, AgentKind? agent});
  Future<ThreadPage> readThread(String threadId, {int? fromSeq, int limit = 500});
  Stream<ThreadEvent> get uiEvents;  // ThreadEvent = (String threadId, int seq, UiEventMessage ui, DateTime ts)
}
```

### 10.3 Riverpod providers (new)

| Provider | Shape | Responsibility |
|---|---|---|
| `threadListProvider` | `FutureProvider<List<ThreadSummary>>` | Calls `listThreads` once; pull-to-refresh triggers refetch |
| `threadEventsProvider(threadId)` | `AsyncNotifier<List<UiEventMessage>>` | Load history via `readThread`, then `ref.listen` to `uiEvents` stream for live append |
| `backendUrlProvider` | `FutureProvider<String?>` | Reads from Keychain; used by `_Router` to decide between PairingPage and the new list/view pages |

### 10.4 Pages

```
apps/mobile/lib/presentation/
├── pages/
│   ├── pairing_page.dart                # existing, QR schema v2 parser
│   ├── thread_list_page.dart            # NEW
│   ├── thread_view_page.dart            # NEW
│   ├── home_page.dart                   # REMOVED (its "已连接" placeholder is replaced by thread_list_page)
│   └── permission_denied_page.dart      # unchanged
└── widgets/
    ├── qr_scanner_view.dart             # unchanged
    ├── debug_paste_qr_sheet.dart        # unchanged
    ├── thread_list_tile.dart            # NEW
    └── ui_event_tile.dart               # NEW
```

**`ThreadListPage`** layout:
- `ShadCard` list, one per `ThreadSummary`.
- Each row: agent icon (text badge: `CDX`/`CLD`/`GEM`), title (or `"<untitled>"` if null), last-modified timestamp, small "ended" badge if `ended_at_ms` is set.
- Tappable → navigates to `ThreadViewPage(threadId)`.
- Pull-to-refresh triggers `ref.invalidate(threadListProvider)`.

**`ThreadViewPage`** layout:
- Scrollable `ListView.builder` of `UiEventTile` rows, one per `UiEventMessage`.
- Each tile is deliberately plain: shows the variant name (e.g., `TextDelta`), `message_id` (truncated), and the primary content field (`text`, `args_json`, `output`, `code + message`, etc.). `Text` wrapping no styling. `Monospace` font.
- Below the ListView, a thin status bar shows "live" (connected + receiving) or "history only" (not currently subscribed).
- No send input, no buttons other than "back".

### 10.5 Frb regen changes

```
apps/mobile/lib/src/rust/
├── minos_ui_protocol.g.dart    # NEW — mirrors of UiEventMessage, ThreadEndReason, MessageRole
├── envelope.g.dart             # updated with Ingest + EventKind::UiEventMessage
├── messages.g.dart             # updated with new LocalRpc method types
└── ...
```

`crates/minos-ffi-frb/src/api/minos.rs`:

- Add `#[frb(mirror(...))]` for `UiEventMessage`, `ThreadEndReason`, `MessageRole`, `ThreadSummary`, `AgentKind` (= `AgentName`).
- Add `MobileClient::list_threads`, `MobileClient::read_thread` with async wrappers.
- Add `MobileClient::subscribe_ui_events(sink: StreamSink<ThreadEvent>)` bridging the Rust broadcast receiver.

---

## 11. Error Handling

### 11.1 `MinosError` additions

```rust
#[error("unauthorized: {reason}")]
// unchanged (from relay spec)
Unauthorized { reason: String },

// NEW
#[error("cf access misconfigured at backend: {reason}")]
CfAccessMisconfigured { reason: String },

#[error("ingest seq conflict for thread {thread_id}: seq {seq} already present")]
IngestSeqConflict { thread_id: String, seq: u64 },

#[error("thread not found: {thread_id}")]
ThreadNotFound { thread_id: String },

#[error("translation not implemented for agent {agent}")]
TranslationNotImplemented { agent: String },

#[error("translation failed for agent {agent}: {message}")]
TranslationFailed { agent: String, message: String },
```

Matching `ErrorKind` variants. `user_message` strings (zh / en):

| ErrorKind | zh | en |
|---|---|---|
| CfAccessMisconfigured | 后端未正确配置 Cloudflare Access 凭据 | Backend Cloudflare Access credentials are not configured |
| IngestSeqConflict | 事件序号冲突 | Event sequence conflict |
| ThreadNotFound | 找不到该线程 | Thread not found |
| TranslationNotImplemented | 该 CLI 尚未接入协议翻译 | Translator not implemented for this CLI |
| TranslationFailed | 事件翻译失败 | Event translation failed |

### 11.2 UI failure modes

| # | Trigger | Error | UI |
|---|---|---|---|
| U1 | Mobile scans QR whose `v > 2` | `PairingQrVersionUnsupported` (new `ErrorKind`) | Toast: "App 版本过旧,请升级" |
| U2 | `ListThreads` when not paired | `Unauthorized` | Toast + route back to `PairingPage` |
| U3 | `ReadThread` returns `ThreadNotFound` | `ThreadNotFound` | Page shows empty state "线程不存在" |
| U4 | Backend emits `UiEventMessage::Error { code: "translation_failed", .. }` | — (not a `MinosError`) | Rendered as a grey-tinted row in the thread view |
| U5 | Mobile's `ui_events_stream` subscription lags (broadcast channel full) | — | Rust logs `warn!`; Dart silently re-subscribes; watermark-based dedup means no data loss; at worst one re-fetch via `ReadThread` |
| U6 | `pair()` via QR that was generated before backend was restarted (token was lost from memory + pairing_tokens GC'd) | `PairingTokenInvalid` | Toast: "二维码已过期,请重新扫描" (same as relay spec) |
| U7 | `request_pairing_qr` while backend lacks CF env vars AND the host can see the public URL has CF Access enabled (detected by a 302 from CF) | `CfAccessMisconfigured` | Mac menu bar: 红点 + "配置不全:后端缺 CF Access 凭据" |

---

## 12. Testing Strategy

### 12.1 `minos-ui-protocol::translate_codex` fixtures

Fixture source: codex app-server **WebSocket notification JSON frames**, *not* `~/.codex/sessions/*.jsonl` rollout files. The two schemas differ; only the WS frames are what we consume.

Collection method:

1. Seed set: copy the scripted `EmitNotification` entries from `crates/minos-agent-runtime/src/test_support.rs::FakeCodexServer` into fixtures. These already represent the notifications the rest of the test suite depends on.
2. Augmentation set: add a `--dump-raw-notifications <path>` flag to `minos-agent-runtime`'s test-support build (hidden behind a `test-support` feature; not shipped in release). When running against a real codex in a maintainer's dev environment, it appends one JSON-per-line to the file. Maintainers select typical samples from that file and place them in `tests/golden/codex/`.
3. Synthetic edge cases: a handful of "hand-crafted" fixtures covering events the seed set doesn't naturally produce (e.g., empty message completion, tool call with no arguments, reasoning-only turn).

Required event coverage (plan 04 elaborates; spec-level list for alignment):

| Codex notification (method) | Translator output |
|---|---|
| `thread/started` | `[ThreadOpened]` |
| `item/started` (role=user) | `[MessageStarted { role: User }]` |
| `item/started` (role=agent) | `[MessageStarted { role: Assistant }]` |
| `item/agentMessage/delta` | `[TextDelta]` |
| `item/agentMessage/completed` | `[]` (signal absorbed; MessageCompleted awaits turn/completed) |
| `item/reasoning/delta` | `[ReasoningDelta]` |
| `item/reasoning/completed` | `[]` |
| `item/toolCall/started` | `[ToolCallPlaced]` (args buffered from subsequent arguments notification) |
| `item/toolCall/arguments` | `[]` (buffered internally by translator; translator keeps per-tool_call_id state) |
| `item/toolCall/completed` | `[ToolCallCompleted]` |
| `turn/completed` | `[MessageCompleted]` for open assistant message |
| `thread/archived` | `[ThreadClosed]` |
| unknown method | `[Raw]` |

Note: the translator is stateful — it needs to buffer partial tool call arguments, track which `message_id` is open, etc. State lives in a `CodexTranslatorState` struct held per-thread on the backend side (one instance per translator invocation chain for a given thread).

Test form:

```rust
// crates/minos-ui-protocol/tests/golden.rs
#[rstest]
fn codex_golden(#[files("tests/golden/codex/*.input.json")] input_path: PathBuf) {
    let expected_path = input_path.with_extension("").with_extension("expected.json");
    let input: serde_json::Value = serde_json::from_str(&fs::read_to_string(&input_path).unwrap()).unwrap();
    let expected: Vec<UiEventMessage> = serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();
    // The fixture file name encodes the thread id of the state; harness resets state at each fixture.
    let mut state = CodexTranslatorState::new("thr_test".into());
    let got = translate_codex(&mut state, &input).unwrap();
    assert_eq!(got, expected);
}
```

### 12.2 `minos-backend` integration

Two new tests under `crates/minos-backend/tests/`:

- **`ingest_roundtrip.rs`**: run a full `pair → host opens WS → host sends 5 `Envelope::Ingest` frames → mobile opens WS → mobile issues `ReadThread` → verify the 5 translated events are returned in order → mobile receives next live Ingest as `Event::UiEventMessage` → verify dedup`.
- **`list_threads.rs`**: `create 3 threads with different agent kinds and timestamps → ListThreads with various filters → verify filter correctness and ordering`.

Uses in-memory SQLite via `sqlx::SqlitePool::connect(":memory:")`.

### 12.3 `minos-mobile` Rust tests

- New unit: `pair_with_qr_json(v2_payload)` round trip against a mock backend (envelope-speaking; built on `tokio::io::duplex`).
- Extended unit: `ui_events_stream` dedup by watermark.
- Extended unit: `list_threads` / `read_thread` request serialisation.

### 12.4 Flutter widget tests (deliberately thin)

Two test files, no interaction-level coverage:

- `thread_list_page_test.dart`: mount page with a stub `MinosCoreProtocol` that returns 3 pre-built `ThreadSummary` objects. Assert 3 `ListTile` descendants are rendered.
- `thread_view_page_test.dart`: mount page with a stub returning 10 `UiEventMessage` values of mixed kinds. Assert 10 `UiEventTile` descendants are rendered and that specific variant names (e.g. "TextDelta") appear in the tile text.

No scroll, no tap, no async interactions in widget tests. Those come in the chat-UI spec.

### 12.5 Real-device smoke checklist

```
□ Backend: `cargo xtask backend-run` prints "migrations applied" and "listening on 127.0.0.1:8787"
□ cloudflared: `sudo cloudflared service list` shows minos running
□ `curl -H "CF-Access-Client-Id: $ID" -H "CF-Access-Client-Secret: $SECRET" https://minos.fan-nn.top/health` → 200
□ Mac app: launches, menu bar shows "等待配对"
□ "Show QR" → QR appears
□ Inspecting the QR JSON (dev tool): contains v=2, backend_url wss://, cf_access_client_id, cf_access_client_secret
□ iPhone: scan → within 5s Pair succeeds, routed to ThreadListPage (empty list)
□ On Mac: trigger a debug "start Codex" → send a test prompt → see codex reply in logs
□ Backend db: `sqlite3 ~/Library/Application\ Support/minos-backend/db.sqlite "SELECT count(*) FROM raw_events"` reports >0
□ iPhone: pull-to-refresh on ThreadListPage → one row appears (the new thread, with auto-title from first user message)
□ iPhone: tap row → ThreadViewPage lists UiEventMessage rows; TextDelta rows visible
□ Send another prompt on Mac → iPhone's ThreadViewPage appends new rows live
□ Kill codex (ctrl-C its process) → iPhone shows a new row: ThreadClosed { reason: HostDisconnected }
□ Stop backend → both UIs show "Reconnecting"; restart backend → auto-recover within 60s
□ "Forget this device" on Mac → iPhone clears Keychain; UI → PairingPage
□ Rotate CF tokens in the Zero Trust dashboard → edit backend env vars → restart backend → re-pair (old QR invalid)
```

16 boxes ticked = this spec complete.

### 12.6 CI deltas

`.github/workflows/ci.yml`:

- Rust job appends `cargo sqlx prepare --check --workspace` (if not already inherited).
- Rust job appends `cargo test -p minos-ui-protocol --test golden` (runs the fixture harness).
- Dart job's existing `flutter test` picks up the two new widget tests automatically.
- No new cross-lane orchestration; all tests are in-memory or stub-based.

---

## 13. Tooling and Operations

### 13.1 `cargo xtask` updates

| Command | Change |
|---|---|
| `cargo xtask relay-run` | **Renamed** to `backend-run` |
| `cargo xtask relay-db-reset` | **Renamed** to `backend-db-reset` |
| `cargo xtask check-all` | Unchanged in shape; `crates/*` glob picks up `minos-backend` and `minos-ui-protocol` automatically |
| `cargo xtask gen-frb` | Unchanged; regenerates Dart mirrors for the new types |
| `cargo xtask bootstrap` | Unchanged |
| `cargo xtask codex-smoke` | Unchanged; opt-in; still uses its own in-process scripted codex |

Per memory: **`cargo xtask check-all` runs before every commit in this worktree** (workspace-level gate; crate-scoped runs miss frb mirror drift).

### 13.2 `minos-backend` config surface

```
minos-backend [OPTIONS]

  --listen <addr>                    Default: 127.0.0.1:8787
                                     Env: MINOS_BACKEND_LISTEN
  --db <path>                        Default: ./minos-backend.db
                                     Env: MINOS_BACKEND_DB
  --log-dir <path>                   Default: ~/Library/Logs/Minos/
                                     Env: MINOS_BACKEND_LOG_DIR
  --log-level <level>                Default: info
                                     Env: RUST_LOG
  --token-ttl-secs <n>               Default: 300
                                     Env: MINOS_BACKEND_TOKEN_TTL
  --cf-access-client-id <id>         No default (dev may omit).
                                     Env: MINOS_BACKEND_CF_ACCESS_CLIENT_ID
  --cf-access-client-secret <secret> No default.
                                     Env: MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET
```

Config precedence: CLI arg overrides env var. If both CF args missing AND `MINOS_BACKEND_ALLOW_DEV=1` is set, backend starts and `request_pairing_qr` returns a payload with CF fields omitted. Otherwise, backend refuses to start with an explicit error.

### 13.3 Cloudflare tunnel runbook (delta)

`docs/ops/cloudflare-tunnel-setup.md` — still unwritten (owed from relay spec), now grows an additional step:

```
7. (after `sudo cloudflared service install`)
   Set backend env vars. Edit the LaunchDaemon plist at
   /Library/LaunchDaemons/com.minos.backend.plist (exact plist install path is a plan-level detail):

     <key>EnvironmentVariables</key>
     <dict>
       <key>MINOS_BACKEND_CF_ACCESS_CLIENT_ID</key>
       <string>....</string>
       <key>MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET</key>
       <string>....</string>
     </dict>
```

Or for development: `MINOS_BACKEND_CF_ACCESS_CLIENT_ID=... MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET=... cargo xtask backend-run`.

### 13.4 Logging

Sink unchanged (`mars-xlog`). New structured fields for ingest: `ingest_agent`, `ingest_thread_id`, `ingest_seq`, `ingest_payload_method` (extracted for codex events when the translator can identify it). Fan-out path adds `fanout_device_id`, `fanout_ui_kind`.

### 13.5 Worktree convention

Implementation happens in `../minos-worktrees/mobile-and-ui-protocol/` (branch `feat/mobile-and-ui-protocol`). The existing empty `../minos-worktrees/macos-relay-migration/` worktree is deleted after this plan's first commit (its intended scope is subsumed here).

---

## 14. Out of Scope / Roadmap

| Item | Phase | Rationale |
|---|---|---|
| Chat UI (Remodex-style bubbles, markdown, streaming animation, tool-call cards) | P1 next spec | The viewer here is a debug surface; real UI is its own design exercise |
| Mobile sending user messages to agent (needs input box + user → `send_user_message` RPC flow) | Same P1 spec as chat UI | Input surfaces live with the chat UI |
| Tool-call args streaming (`ToolCallStreamingStarted` + `ToolCallArgsDelta` + `ToolCallArgsFinalized`) | P1/P2, schema-additive | Defer until UI wants the visual effect; adding variants is schema-additive |
| Claude / Gemini translators (real bodies, fixtures) | P1 `pty-agent-claude-gemini-design.md` | Different transport (PTY vs WS); needs its own spec for raw event capture |
| Multi-host (remote Linux host reaching a cloud backend) | P2 | Requires host bootstrap token flow, host-side CF token provisioning |
| `raw_events` pruning / retention policy | P2 | Wait for real sizes |
| End-to-end encryption of payloads | P2 | Unchanged from relay spec |
| Browser admin console on `/admin` | P2 | Reserved path; UI + auth flow in a follow-up spec |
| Importing pre-existing rollout files | P3 | Niche; may be done by a separate "rollout-watcher" crate |
| Windows support for host | P3+ | Process / sandbox flags are POSIX-specific |
| Telemetry / usage reporting | P3 | Privacy policy + opt-in required |

---

## 15. ADR Index (proposed)

Three new ADRs to land alongside this spec's acceptance:

| # | Topic |
|---|---|
| 0013 | `minos-ui-protocol`: unified UI event shape — event-level, backend-translated, `Raw` escape hatch |
| 0014 | Backend-assembled pairing QR + CF Access Service Token as backend-held credential |
| 0015 | Rename `minos-relay` → `minos-backend` to reflect broadened responsibilities (DB storage, translation, credential distribution) |

ADR 0014 explicitly replaces the relay spec's §9.4 "CF token in Mac Keychain" decision.

---

## 16. File Inventory

**New files:**

```
crates/minos-ui-protocol/Cargo.toml
crates/minos-ui-protocol/src/lib.rs
crates/minos-ui-protocol/src/message.rs
crates/minos-ui-protocol/src/codex.rs
crates/minos-ui-protocol/src/claude.rs
crates/minos-ui-protocol/src/gemini.rs
crates/minos-ui-protocol/src/error.rs
crates/minos-ui-protocol/tests/golden.rs
crates/minos-ui-protocol/tests/golden/codex/*.json     (seed + augmentation fixtures)
crates/minos-backend/src/ingest/mod.rs
crates/minos-backend/src/ingest/translate.rs
crates/minos-backend/src/store/threads.rs
crates/minos-backend/src/store/raw_events.rs
crates/minos-backend/migrations/0004_threads.sql
crates/minos-backend/migrations/0005_raw_events.sql
crates/minos-backend/tests/ingest_roundtrip.rs
crates/minos-backend/tests/list_threads.rs
crates/minos-agent-runtime/src/ingest.rs
apps/mobile/lib/presentation/pages/thread_list_page.dart
apps/mobile/lib/presentation/pages/thread_view_page.dart
apps/mobile/lib/presentation/widgets/thread_list_tile.dart
apps/mobile/lib/presentation/widgets/ui_event_tile.dart
apps/mobile/lib/application/thread_list_provider.dart
apps/mobile/lib/application/thread_events_provider.dart
apps/mobile/test/unit/thread_list_controller_test.dart
apps/mobile/test/widget/thread_list_page_test.dart
apps/mobile/test/widget/thread_view_page_test.dart
docs/adr/0013-minos-ui-protocol-unified-event-shape.md
docs/adr/0014-backend-assembled-pairing-qr.md
docs/adr/0015-rename-relay-to-backend.md
```

**Renamed (directory rename is atomic):**

```
crates/minos-relay/ → crates/minos-backend/
  (includes src/**, migrations/**, Cargo.toml, tests/**, etc.)
```

**Modified files:**

```
Cargo.toml                                         Workspace member rename + add minos-ui-protocol
Cargo.lock                                         Regenerated
crates/minos-protocol/Cargo.toml                   Add dep on minos-ui-protocol
crates/minos-protocol/src/envelope.rs              Add Ingest variant; extend EventKind; LocalRpcMethod add ListThreads / ReadThread / RequestPairingQr (rename) / GetThreadLastSeq
crates/minos-protocol/src/events.rs                DELETE AgentEvent re-export
crates/minos-protocol/src/messages.rs              Add ListThreadsRequest/Response, ReadThreadRequest/Response, PairingQrPayload, ThreadSummary
crates/minos-protocol/src/rpc.rs                   Remove subscribe_events
crates/minos-protocol/tests/golden/envelope/*.json Update + add new fixtures for ingest + ui_event_message
crates/minos-domain/src/error.rs                   Add 5 MinosError variants + ErrorKind + user_message strings
crates/minos-domain/src/events.rs                  DELETE AgentEvent enum
crates/minos-domain/tests/golden.rs                DELETE agent_event_raw.json
crates/minos-agent-runtime/Cargo.toml              Remove placeholder translate deps if any; rely on raw Value
crates/minos-agent-runtime/src/translate.rs        DELETED (contents migrate to minos-ui-protocol/src/codex.rs)
crates/minos-agent-runtime/src/runtime.rs          Broadcast channel carries raw ingest tuples, not AgentEvent
crates/minos-agent-runtime/src/test_support.rs     Seed fixtures referenced by minos-ui-protocol; add --dump flag gated on feature
crates/minos-daemon/src/handle.rs                  subscribe_events removed; start_ingest_link wiring
crates/minos-daemon/src/rpc_server.rs              subscribe_events removed; agent-related methods preserved
crates/minos-daemon/src/subscription.rs            No change to state observers; event observer removed
crates/minos-daemon/src/agent.rs                   Remove event_stream fn; keep state observer
crates/minos-backend/Cargo.toml                    Add dep on minos-ui-protocol
crates/minos-backend/migrations/0001_devices.sql   CHECK role: 'mac-host' → 'agent-host' (in-place edit; no data migration, dev DBs recreated)
crates/minos-backend/src/config.rs                 Add cf_access_client_id / cf_access_client_secret / allow_dev fields + parsing
crates/minos-backend/src/http/ws_devices.rs        Route Envelope::Ingest; handle new LocalRpcMethods
crates/minos-backend/src/pairing/mod.rs            request_pairing_qr build + return PairingQrPayload
crates/minos-backend/src/session/registry.rs       Expose per-session role for fan-out filtering
crates/minos-ffi-uniffi/src/lib.rs                 Remove AgentEvent surface; unchanged agent state
crates/minos-ffi-frb/src/api/minos.rs              Mirror new types; add list_threads / read_thread / subscribe_ui_events
crates/minos-mobile/src/client.rs                  Rewrite for envelopes + new stream + new RPC methods
crates/minos-mobile/src/store.rs                   Extend PairingStore trait + InMemoryPairingStore
crates/minos-pairing/src/lib.rs                    QrPayload → PairingQrPayload v2
crates/minos-transport/src/*                       If server role removed previously, confirm no stray references
apps/mobile/lib/infrastructure/minos_core.dart     Add listThreads / readThread / uiEvents wrappers
apps/mobile/lib/domain/minos_core_protocol.dart    Same, at protocol layer
apps/mobile/lib/application/minos_providers.dart   Add backendUrlProvider + refactor to use new MinosCoreProtocol
apps/mobile/lib/presentation/app.dart              _Router reads backendUrlProvider + connectionStateProvider
apps/mobile/lib/presentation/pages/pairing_page.dart  Update to parse v2 QR
apps/mobile/pubspec.yaml                           Add flutter_secure_storage ^9.2.0
apps/macos/Minos/Application/AppState.swift        Remove any event subscription code that referenced subscribe_events; preserve agent state observer
apps/macos/Minos/Application/DaemonDriving.swift   subscribe_events removed from protocol
apps/macos/Minos/Infrastructure/DaemonBootstrap.swift  start_autobind → start_ingest_link naming alignment (optional)
xtask/src/main.rs                                  Rename relay-run to backend-run + add backend-db-reset
.github/workflows/ci.yml                           minos-ui-protocol test step; backend naming
README.md                                          Update relay → backend mentions
docs/superpowers/specs/minos-relay-backend-design.md  Front-matter: mark §6.1 and §9.4 partially superseded by this spec
```

**Deleted files:**

```
crates/minos-agent-runtime/src/translate.rs
crates/minos-domain/tests/golden/agent_event_raw.json
```

---
