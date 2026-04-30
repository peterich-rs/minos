# Minos · Codex App-Server Integration — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-23 |
| Owner | fannnzhang |
| Parent spec | `docs/superpowers/specs/minos-architecture-and-mvp-design.md` |
| Target plan | `docs/superpowers/plans/04-codex-app-server-integration.md` (to be written) |
| Predecessors | Plans 01–03 (`docs/superpowers/plans/0{1,2,3}-*.md`) |
| Roadmap slot | Parent spec §11 / P1 — `codex-app-server-integration.md` |

---

## 1. Context

Plans 01–03 landed the MVP frame: the nine Rust crates, the macOS `MenuBarExtra` app over UniFFI, and the iOS Flutter client talking to the Mac daemon over Tailscale + JSON-RPC 2.0. `subscribe_events` is declared on the `MinosRpc` trait but the server returns `-32601` "not implemented"; `AgentEvent` is declared in `minos-protocol::events` but has zero producers. Agent runtime was always scheduled as the first P1 step — §10.1 of the parent spec pins the shapes, §11 names the spec `codex-app-server-integration.md`.

This document is the design contract for that step. It delivers a **daemon-side bridge** to `codex app-server` plus an **observer-only macOS menubar addition**. There is intentionally no chat UI in either surface: the user-facing conversation layer is a separate P1 spec (`streaming-chat-ui-design.md`, to be brainstormed after this one lands). What this spec delivers is the full RPC + FFI surface the chat UI will consume, plus a debug-build smoke path so maintainers can exercise the bridge on their own workstation without the chat UI existing.

The scope is deliberately narrower than "ship Codex on mobile" — mobile gets no new UI this phase. The Flutter `MobileClient` facade stays frozen at its plan-03 shape; only the underlying `MinosRpc` contract (which mobile re-generates through frb) gains methods and `AgentEvent` gains a new variant.

---

## 2. Goals

### 2.1 In scope

1. New workspace crate **`minos-agent-runtime`** — owns the `codex app-server` child, speaks WS-loopback JSON-RPC to it, exposes an `AgentRuntime` handle with `start / send_user_message / stop / state_stream / event_stream`.
2. Real implementation of **`subscribe_events`** in `RpcServerImpl` (MVP stub removed).
3. Three new RPC methods on `MinosRpc`: `start_agent`, `send_user_message`, `stop_agent`, with typed request/response structs in `minos-protocol::messages`.
4. Two new FFI surfaces exported through UniFFI: agent control (`DaemonHandle::{start_agent, send_user_message, stop_agent}`) and `DaemonHandle::subscribe_agent_state(observer)` mirroring the existing `ConnectionStateObserver` pattern.
5. macOS menubar gains an **Agent segment** — read-only `AgentState` row in release builds; start / send-ping / stop debug buttons in debug builds.
6. `AgentEvent` enum gains a **`Raw { kind, payload_json }` variant** — additive; codex events outside the known-good mapping set are forwarded verbatim.
7. Workspace root convention: all codex sessions run in `$MINOS_HOME/workspaces`, created on demand. `$MINOS_HOME` resolves via the existing `default_minos_home()` in `minos-daemon`.
8. Codex process is run **sandboxed and approval-free** via `-c approval_policy=never -c sandbox_permissions=[...]`; any leaked approval `ServerRequest` is auto-rejected and forwarded as a `Raw` event.
9. Rust `cargo test` coverage on `minos-agent-runtime` for state machine, translation table, approval auto-reject, codex crash, broadcast fan-out.
10. Daemon integration test (`crates/minos-daemon/tests/agent_e2e.rs`) exercising the full `start_agent → send_user_message → receive event → stop_agent` pipeline against a scripted fake WS server standing in for codex.
11. Swift unit tests (`MinosTests/Application/AgentStateTests.swift`) over the new `AppState` methods and the debug-button paths.
12. `docs/adr/0009-codex-app-server-ws-transport.md` and `docs/adr/0010-agent-event-raw-variant.md`.

### 2.2 Out of scope (explicit deferrals)

| Item | Deferred to |
|---|---|
| Any chat / message history / streaming markdown UI on Mac or iOS | `streaming-chat-ui-design.md` (next P1 spec) |
| `respond_approval(request_id, decision)` RPC and the approval UX | Same |
| `AgentEvent` content-type expansion beyond `Raw` (e.g. typed `ApprovalRequired`, `PatchPreview`) | Driven by chat-ui needs, not here |
| Multi-session concurrency / `session_id` parameter on `subscribe_events` | P2+ (breaking change when it arrives) |
| Per-session workspace override (`start_agent(cwd: String)`) | Additive RPC parameter in a later spec |
| Flutter / iOS UI that calls the new RPCs | Re-generated frb bindings exist; no `MobileClient` surface in this phase |
| PTY-backed agents (`claude` / `gemini`) | `pty-agent-claude-gemini-design.md` (separate P1 spec) |
| Real-codex smoke as a default CI step | Stays opt-in via env flag; CI image does not install `codex` |
| `respond_approval` UX, approval inbox, diff preview panes | `streaming-chat-ui-design.md` |
| Codex authentication flow (login, token refresh) | Users run `codex login` once outside Minos before using; Minos never drives auth |

### 2.3 Testing philosophy (inherited, binding)

Unit tests across Rust and Swift cover **logic only**. UI-level, widget, SwiftUI Preview, and functional tests are integration concerns and are not written in this plan. The sole Swift test target in `apps/macos/MinosTests/` continues to be logic-layer (`AppStateTests`); `AgentSegmentView` is reached through the debug build on the maintainer's workstation, not through XCUITest.

### 2.4 UI-per-phase rule (inherited, binding)

The macOS menubar gains **exactly one** new read-only row plus three debug-build-only buttons. No chat surface, no message list, no history, no approval UX, no "choose workspace". Future specs freely rewrite the agent-facing Mac UI; there is no preservation tax.

Reachable `AgentState` values this phase renders:
- `Idle` (boot default; no codex child running)
- `Starting` (between `start_agent` RPC and codex `thread/started` notification)
- `Running { started_at, thread_id }` (codex thread live)
- `Crashed { reason }` (codex died without a `stop_agent` call)

The release build never surfaces a "start / send / stop" control; those paths are guarded by `#if DEBUG` so a shipped `.app` cannot trigger agent runtime by accident while the chat UI is still missing.

---

## 3. Assumptions from Plans 01–03

Treated as delivered and stable; this plan does not restructure them.

- **`minos-domain`**: `AgentName{Codex,Claude,Gemini}` (§5.1 parent), `AgentEvent{TokenChunk,ToolCall,ToolResult,Reasoning,Done}` (placeholder-five), `MinosError` + `ErrorKind` + `Lang` + `user_message` table, `ConnectionState`.
- **`minos-protocol`**: `MinosRpc` trait with `pair`, `health`, `list_clis`, `subscribe_events`. `subscribe_events` signature is `async fn subscribe_events(&self) -> SubscriptionResult` — **unchanged** in plan 04 (adding a `session_id` param is saved for a future breaking bump).
- **`minos-daemon`**: `DaemonHandle` (Arc-wrapped), `start_autobind`, `stop(&self)`, `pairing_qr`, `subscribe` + `ConnectionStateObserver`, `current_trusted_device`, `forget_device`, `logging::{init, today, set_debug}`. `FilePairingStore`. `default_minos_home()` (Linux/CLI = `$HOME/.minos`; macOS GUI = platform-native, unless `--minos-home` overrides).
- **`minos-transport`**: `WsServer`, `WsClient` (kept to the daemon's own JSON-RPC surface; **not reused** for the codex-side WS — see §5.1 rationale).
- **`minos-ffi-uniffi`**: feature-gated `uniffi` derives on domain / pairing / daemon types, `Subscription` + `ConnectionStateObserver` callback pattern. Two re-exported free functions (`init_logging`, `today_log_path`, `discover_tailscale_ip`, `set_debug`).
- **`minos-ffi-frb`**: frb v2 adapter exporting `MobileClient` to Dart; owns a dedicated Tokio runtime (per the `fix(ffi-frb): own a dedicated tokio runtime for state forwarding` commit) which will also host the agent-runtime background tasks if MobileClient ever re-exports them — not this phase.
- **`apps/macos`**: `MenuBarView` branching on `bootError / displayError / connectionState / trustedDevice / isShowingQr`, `AppState: @Observable`, `DaemonDriving` protocol, `ConnectionStateObserverAdapter`, `DaemonBootstrap`.
- **`apps/mobile`**: Flutter app with pairing + home pages; `MobileClient` facade reconnects and streams connection state via FRB.
- **xtask**: `check-all` orchestrates Rust + Swift + Flutter legs; `bootstrap` installs `uniffi-bindgen-swift`, `flutter_rust_bridge_codegen`, `cargo-deny`, `cargo-audit`, `xcodegen`, `swiftlint`. Flutter leg now scopes to `apps/mobile/lib test` (not `.`).
- **`codex`** CLI: externally installed by the user. This phase asserts `codex --version` returns a 0.2x+ release advertising `app-server` in its subcommand list (already the case on macOS 14+ and Linux via `brew install openai/codex/codex` or `cargo install codex-cli`). We do **not** vendor or pin codex.

---

## 4. Architecture

```
┌──────────────────────── Minos.app (single process) ────────────────────────┐
│ Swift / SwiftUI  (presentation / application / domain / infrastructure)    │
│   new: AgentSegmentView (MenuBarView child) + agentState on AppState       │
│                                                                            │
│                          UniFFI (async + callback interfaces)              │
│ ┌──────────────────────────────▼──────────────────────────────────────┐    │
│ │ libminos_ffi_uniffi.a   re-exports → DaemonHandle + new agent API   │    │
│ └──────────────────────────────┬──────────────────────────────────────┘    │
│                                │                                            │
│ minos-daemon (tokio, in-process)                                           │
│  ├─ plan 01-02: WsServer 100.x:7878..=7882 (jsonrpsee) — unchanged         │
│  ├─ plan 01-02: FilePairingStore, transport, pairing, detect — unchanged   │
│  └─ *NEW* AgentRuntime handle (always constructed; idle until start_agent) │
│                                     ↑                                      │
│                                     │ internal channels:                   │
│                                     │   watch::Sender<AgentState>          │
│                                     │   broadcast::Sender<AgentEvent>      │
│                                     │                                      │
│ ┌─────────────────────── minos-agent-runtime (new) ────────────────────┐   │
│ │ AgentRuntime state machine:                                          │   │
│ │   Idle → Starting → Running → Stopping → Idle                        │   │
│ │         └───────→ Crashed → Idle                                     │   │
│ │                                                                      │   │
│ │ CodexClient (owns WS + JSON-RPC plumbing):                           │   │
│ │   ┌─ process.rs: spawn & supervise codex subprocess                  │   │
│ │   ├─ codex_client.rs: tokio-tungstenite WS client + JSON-RPC framing │   │
│ │   ├─ translate.rs: codex notification → AgentEvent mapping table     │   │
│ │   └─ approvals.rs: auto-reject every approval ServerRequest          │   │
│ │                                                                      │   │
│ └───────────────────────────┬──────────────────────────────────────────┘   │
│                             │ ws://127.0.0.1:<port>  JSON-RPC 2.0          │
│                             ▼                                              │
│ ┌───────────── codex (external binary, spawned as child) ───────────────┐  │
│ │ codex app-server --listen ws://127.0.0.1:<port>                       │  │
│ │   -c approval_policy=never                                            │  │
│ │   -c sandbox_permissions=['disk-full-read-access',                    │  │
│ │                           'disk-write-folder=$MINOS_HOME/workspaces'] │  │
│ │   -c shell_environment_policy.inherit=all                             │  │
│ └───────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘

WS 100.x:7878 still serves mobile peers; the codex-side WS is strictly 127.0.0.1.
```

### 4.1 Process model

Single Mac process, as before. Two independent WS surfaces now coexist inside that process: the existing **outward** `WsServer` on the Tailscale IP (peer mobile clients) and the new **inward** WS client talking to the codex child over loopback. They share the same Tokio runtime.

### 4.2 Dependency shape

`minos-daemon` grows a path-dep on `minos-agent-runtime`. `minos-agent-runtime` depends on `minos-domain` (for `AgentEvent`, `MinosError`, `AgentName`), `tokio` (full — process + WS), `tokio-tungstenite`, `serde` + `serde_json`, `thiserror`, `tracing`, and `uuid` (for internal correlation). It does **not** depend on `minos-protocol` — the RPC trait lives one layer up in `minos-daemon`'s `RpcServerImpl`, which is where codex's native method names are translated into our `MinosRpc` method names.

`apps/mobile` regenerates its frb bindings (routine Phase C-style regen) to pick up:
- `AgentEvent::Raw` (new variant)
- `start_agent` / `send_user_message` / `stop_agent` on the generated `MinosRpcClient`

The Dart `MobileClient` facade **does not** re-export these. Chat-ui spec will.

### 4.3 Workspace root

Resolved once per `AgentRuntime::start` call:

```
workspace_root = $MINOS_HOME/workspaces
                 where $MINOS_HOME defaults to:
                   - $HOME/.minos             (Linux / CLI / test)
                   - platform-native dir      (macOS GUI: existing
                     daemon behaviour — see crates/minos-daemon/src/main.rs)
```

Created on demand (`std::fs::create_dir_all`) before spawning codex. Missing-parent / permission-denied → `MinosError::StoreIo { path, message }` (reusing the existing variant; this is not an agent-specific failure).

### 4.4 Logging

Unchanged channels. `minos-agent-runtime` uses `tracing` with targets `minos_agent_runtime::process`, `::codex_client`, `::translate`, `::state`. All logs land in the same `daemon_YYYYMMDD.xlog` via the existing `XlogLayer`.

Standard fields added this phase: `agent_name` (always `codex` for now), `thread_id`, `codex_pid`. These join the existing `device_id` / `peer_device_id` / `rpc_method` set.

### 4.5 Non-use of `minos-transport`

> **Note (2026-04-30):** The "raw framed JSON" wire format below is now
> typed end-to-end by `crates/minos-codex-protocol` (typify codegen of
> `schemas/`). `CodexClient::call_typed` / `notify_typed` carry method
> strings and response types from the trait, and inbound dispatch goes
> through generated `ServerRequest` / `ServerNotification` enums. See
> `codex-typed-protocol-design.md` and ADR
> `0019-codex-protocol-typed-codegen.md`.

`minos-transport` wraps jsonrpsee's server + client for the mobile↔daemon protocol (where both ends run our trait-generated code). Codex's WS surface is driven by codex's own JSON-RPC schema — method names differ, param shapes differ, no shared trait. Reusing `WsClient` here would mean bypassing jsonrpsee's type-safe client and writing raw `RpcMessage` / `RpcResponse` payloads anyway. We therefore talk to codex with `tokio-tungstenite` directly (raw framed JSON), matching codex's own client examples.

---

## 5. Components

### 5.1 New crate: `minos-agent-runtime`

#### Cargo.toml

```toml
[package]
name = "minos-agent-runtime"
version = "0.1.0"
edition = "2021"

[dependencies]
minos-domain     = { path = "../minos-domain" }
tokio            = { workspace = true, features = ["full"] }
tokio-tungstenite = { workspace = true }
serde            = { workspace = true, features = ["derive"] }
serde_json       = { workspace = true }
thiserror        = { workspace = true }
tracing          = { workspace = true }
uuid             = { workspace = true, features = ["v4"] }
url              = { workspace = true }
futures-util     = { workspace = true }

[dev-dependencies]
tokio            = { workspace = true, features = ["full", "test-util"] }

[lints]
workspace = true
```

#### Public types

```rust
// crates/minos-agent-runtime/src/state.rs

use minos_domain::AgentName;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Starting { agent: AgentName },
    Running {
        agent: AgentName,
        thread_id: String,
        started_at: SystemTime,
    },
    Stopping,
    Crashed { reason: String },
}
```

```rust
// crates/minos-agent-runtime/src/lib.rs

pub mod state;
pub mod translate;

pub use state::AgentState;

use minos_domain::{AgentEvent, AgentName, MinosError};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, watch};

pub struct AgentRuntimeConfig {
    pub workspace_root: PathBuf,
    pub codex_bin: Option<PathBuf>,           // default: PATH lookup
    pub ws_port_range: std::ops::RangeInclusive<u16>, // default 7879..=7883
    pub event_buffer: usize,                  // default 256
}

pub struct AgentRuntime { /* private */ }

impl AgentRuntime {
    pub fn new(cfg: AgentRuntimeConfig) -> Arc<Self>;

    pub async fn start(&self, agent: AgentName) -> Result<StartAgentOutcome, MinosError>;
    pub async fn send_user_message(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), MinosError>;
    pub async fn stop(&self) -> Result<(), MinosError>;

    pub fn current_state(&self) -> AgentState;
    pub fn state_stream(&self) -> watch::Receiver<AgentState>;
    pub fn event_stream(&self) -> broadcast::Receiver<AgentEvent>;
}

pub struct StartAgentOutcome {
    pub session_id: String, // = codex thread_id
    pub cwd: String,        // absolute path, canonicalised
}
```

#### Internal modules

| File | Role |
|---|---|
| `src/lib.rs` | Facade type, `AgentRuntime` + `Config` + re-exports |
| `src/state.rs` | `AgentState` enum + serde round-trip tests |
| `src/process.rs` | `CodexProcess` — spawn + supervise `tokio::process::Child`; `kill_on_drop(true)`; `stop_graceful` = SIGTERM → 3s grace → SIGKILL |
| `src/codex_client.rs` | Raw WS JSON-RPC 2.0 framing: connect to `ws://127.0.0.1:<port>`, pump inbound frames, send typed `Request` / parse `Response` / parse `Notification` / parse `ServerRequest` |
| `src/translate.rs` | Mapping table **codex notification method → `AgentEvent` variant** (known set) or `AgentEvent::Raw` (fallback). Pure function, heavily unit-tested |
| `src/approvals.rs` | Given a codex `ServerRequest` whose method is in the approval set, build the auto-reject response payload |
| `src/runtime.rs` | The state machine — glues process + codex_client + translate + approvals; holds `watch::Sender<AgentState>` + `broadcast::Sender<AgentEvent>` |

#### Start sequence

```
AgentRuntime::start(Codex)
 ├─ if state ≠ Idle → Err(AgentAlreadyRunning)
 ├─ create workspace_root if missing → (StoreIo on failure)
 ├─ pick port p in ws_port_range by bind-probing a throw-away TcpListener
 │    (same pattern as minos-daemon::start_autobind)
 ├─ state_tx.send(Starting { agent: Codex })
 ├─ spawn codex:
 │    Command::new(codex_bin.or_default())
 │      .args([ "app-server", "--listen",
 │              &format!("ws://127.0.0.1:{p}"),
 │              "-c", "approval_policy=never",
 │              "-c", &format!("sandbox_permissions=[\
 │                             'disk-full-read-access',\
 │                             'disk-write-folder={workspace_root}']"),
 │              "-c", "shell_environment_policy.inherit=all" ])
 │      .kill_on_drop(true)
 │      .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
 │      .spawn()?                                        → CodexSpawnFailed
 ├─ spawn stderr-drainer task (log lines at WARN)
 ├─ spawn supervisor task: child.wait() → on exit, state_tx.send(Crashed)
 ├─ connect WS client with retry (200ms × 15 = 3s timeout ceiling)
 │    → CodexConnectFailed on exhaustion
 ├─ JSON-RPC `initialize` request → wait response                  (5s timeout)
 ├─ JSON-RPC `thread/start` request { cwd: workspace_root }
 │    → response carries thread_id                                  (5s timeout)
 ├─ state_tx.send(Running { agent, thread_id, started_at: now })
 └─ return StartAgentOutcome { session_id: thread_id, cwd: workspace_root }

Any error along the chain:
  ├─ best-effort: send SIGTERM to child, wait 500ms
  ├─ state_tx.send(Idle)
  └─ propagate Err(e)
```

#### Send sequence

```
AgentRuntime::send_user_message(session_id, text)
 ├─ match state:
 │   ├─ Running { thread_id, .. } if thread_id == session_id → continue
 │   ├─ Running { thread_id, .. } else → Err(AgentSessionIdMismatch)
 │   └─ _ → Err(AgentNotRunning)
 ├─ JSON-RPC `turn/start` request {
 │      thread_id,
 │      items: [{ type: "text", text }],
 │    }                                                            (10s timeout)
 ├─ on response:
 │   ├─ Ok → return Ok(())
 │   └─ Err(JsonRpcError) → Err(CodexProtocolError { method: "turn/start", message })
 └─ on WS error → bubble as CodexProtocolError { method: "turn/start", message }
```

`send_user_message` does **not** wait for `turn/completed`; that arrives later as a broadcast event. This keeps the RPC fast and consistent with "fire-and-observe" semantics.

#### Stop sequence

```
AgentRuntime::stop()
 ├─ match state:
 │   ├─ Running { thread_id, .. } → continue with thread_id
 │   ├─ Idle | Crashed → return Ok(())   (idempotent)
 │   └─ Starting | Stopping → Err(AgentNotRunning)  (caller should retry/wait)
 ├─ state_tx.send(Stopping)
 ├─ best-effort JSON-RPC `turn/interrupt` { thread_id }              (500ms)
 ├─ best-effort JSON-RPC `thread/archive`  { thread_id }             (500ms)
 ├─ CodexProcess::stop_graceful():  SIGTERM → 3s wait → SIGKILL
 ├─ drop WS handle (task exits)
 ├─ state_tx.send(Idle)
 └─ Ok(())
```

Timeouts on the two codex RPCs are short on purpose: a polite goodbye is best-effort. The authoritative termination is the signal.

#### Crash detection

The supervisor task `tokio::spawn`ed during `start` does:

```
let exit = child.wait().await;
state_tx.send(Crashed { reason: reason_from(exit) });
// also close WS client side so blocking reads unblock
```

`reason_from(ExitStatus)` produces `"exit code N"` on Unix, `"signal SIGTERM"` on signal exit, `"terminated by minos (expected)"` when a companion atomic flag is set at `stop` time (so `stop` → `Idle`, not → `Crashed`).

#### Broadcast semantics

`event_stream()` returns a fresh `broadcast::Receiver<AgentEvent>` each call. The broadcast channel uses a bounded buffer (`event_buffer`, default 256). Slow subscribers that fall behind get `RecvError::Lagged(n)`; agent-runtime logs a warning at `warn!` level and does **not** attempt to reconnect them — the subscriber decides whether to resubscribe.

### 5.2 `minos-protocol` additions

#### `events.rs`

Add one variant:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TokenChunk { text: String },
    ToolCall { name: String, args_json: String },
    ToolResult { name: String, output: String },
    Reasoning { text: String },
    Done { exit_code: i32 },
    // NEW ── forward-compat escape hatch. `kind` is the codex method name
    // (e.g. `"item/plan/delta"`), `payload_json` is the raw `params` object
    // as a string. Consumers may render nothing for unknown `kind`.
    Raw { kind: String, payload_json: String },
}
```

Serde golden test file `crates/minos-domain/tests/golden/agent_event_raw.json`:

```json
{"type":"raw","kind":"item/plan/delta","payload_json":"{\"step\":\"compile\"}"}
```

added to the existing `golden.rs` round-trip harness.

#### `messages.rs`

Add request + response structs:

```rust
use crate::AgentName;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentRequest {
    pub agent: AgentName,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendUserMessageRequest {
    pub session_id: String,
    pub text: String,
}
```

Golden round-trip tests added alongside the existing message goldens.

#### `rpc.rs`

Append three methods to the `MinosRpc` trait:

```rust
#[method(name = "start_agent")]
async fn start_agent(
    &self,
    req: StartAgentRequest,
) -> jsonrpsee::core::RpcResult<StartAgentResponse>;

#[method(name = "send_user_message")]
async fn send_user_message(
    &self,
    req: SendUserMessageRequest,
) -> jsonrpsee::core::RpcResult<()>;

#[method(name = "stop_agent")]
async fn stop_agent(&self) -> jsonrpsee::core::RpcResult<()>;
```

`subscribe_events` is **not** modified (signature preserved — no `session_id` param).

### 5.3 `minos-domain` additions

#### `error.rs`

Append to `ErrorKind`:

```rust
CodexSpawnFailed,
CodexConnectFailed,
CodexProtocolError,
AgentAlreadyRunning,
AgentNotRunning,
AgentNotSupported,
AgentSessionIdMismatch,
```

Append to `MinosError`:

```rust
#[error("failed to spawn codex: {message}")]
CodexSpawnFailed { message: String },

#[error("failed to connect codex WS at {url}: {message}")]
CodexConnectFailed { url: String, message: String },

#[error("codex protocol error on {method}: {message}")]
CodexProtocolError { method: String, message: String },

#[error("agent is already running")]
AgentAlreadyRunning,

#[error("no agent session is running")]
AgentNotRunning,

#[error("agent {agent:?} not supported in this build")]
AgentNotSupported { agent: crate::AgentName },

#[error("session id does not match the active session")]
AgentSessionIdMismatch,
```

`ErrorKind::user_message` gains 14 arms (7 variants × 2 languages). Draft strings:

| ErrorKind | zh | en |
|---|---|---|
| CodexSpawnFailed | 无法启动 Codex CLI；请确认已安装 `codex` | Failed to launch codex CLI; is codex installed? |
| CodexConnectFailed | 无法连接 Codex 服务 | Could not reach codex app-server |
| CodexProtocolError | Codex 返回错误，请查看日志 | Codex returned an error — see log |
| AgentAlreadyRunning | Agent 已在运行 | An agent session is already running |
| AgentNotRunning | 当前没有 Agent 会话 | No agent session is running |
| AgentNotSupported | 这一期仅支持 Codex | Only Codex is supported in this phase |
| AgentSessionIdMismatch | 会话已失效，请重新启动 | Session is no longer active; please restart |

`kind()` exhaustive-match test gains seven cases; `every_error_kind_has_user_message_in_both_langs` gains seven entries.

### 5.4 `minos-daemon` additions

#### `src/agent.rs` (new)

Composition root that wires `AgentRuntime` into `DaemonInner` and exposes push-style FFI:

```rust
pub(crate) struct AgentGlue {
    runtime: Arc<AgentRuntime>,
    // Separate watch-to-observer forwarder, mirroring the Connection pattern.
    // See subscription.rs for the reusable helper.
}

impl AgentGlue {
    pub fn new(workspace_root: PathBuf) -> Self;
    pub async fn start_agent(&self, req: StartAgentRequest) -> Result<StartAgentResponse, MinosError>;
    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError>;
    pub async fn stop_agent(&self) -> Result<(), MinosError>;
    pub fn subscribe_state(&self, obs: Arc<dyn AgentStateObserver>) -> Arc<Subscription>;
    pub fn event_stream(&self) -> broadcast::Receiver<AgentEvent>;
    pub async fn shutdown(&self) -> Result<(), MinosError>; // called by DaemonHandle::stop
}
```

#### `src/subscription.rs` additions

Keep `ConnectionStateObserver` + `spawn_observer` unchanged. Add a **sibling pair** for agent state — not a generic helper:

```rust
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait AgentStateObserver: Send + Sync {
    fn on_state(&self, state: AgentState);
}

pub(crate) fn spawn_agent_observer(
    rx: watch::Receiver<AgentState>,
    observer: Arc<dyn AgentStateObserver>,
) -> Arc<Subscription> { /* mirror of spawn_observer */ }
```

Rationale: two concrete trait/helper pairs is ~40 lines of near-duplicate code; a generic helper would need a `Clone + Send + 'static` bound on the state type plus a closure emit fn, which costs more in type gymnastics than it saves. Reconsider if a third observer surface appears.

#### `src/handle.rs` additions

```rust
impl DaemonHandle {
    pub async fn start_agent(&self, req: StartAgentRequest) -> Result<StartAgentResponse, MinosError>;
    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError>;
    pub async fn stop_agent(&self) -> Result<(), MinosError>;
    pub fn subscribe_agent_state(&self, obs: Arc<dyn AgentStateObserver>) -> Arc<Subscription>;
    pub fn current_agent_state(&self) -> AgentState;
}
```

All delegate to `inner.agent.<method>`. Naming mirrors the existing pairing / subscribe pattern to keep the Swift surface regular.

`DaemonHandle::stop` adds an `agent.shutdown()` call before the existing `server.shutdown()` so a running codex child is killed before the WS server stops accepting peers.

#### `src/rpc_server.rs` changes

`RpcServerImpl::start_agent`, `send_user_message`, `stop_agent` simply delegate to the glue. `subscribe_events` is **rewritten** from the stub — it acquires a fresh `broadcast::Receiver<AgentEvent>` and forwards events into the jsonrpsee `SubscriptionSink`:

```rust
async fn subscribe_events(
    &self,
    pending: PendingSubscriptionSink,
) -> SubscriptionResult {
    let mut rx = self.agent.event_stream();
    let sink = pending.accept().await?;
    loop {
        match rx.recv().await {
            Ok(evt) => {
                sink.send(SubscriptionMessage::from_json(&evt)?).await?;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "subscribe_events subscriber lagged");
                // continue; next recv() will deliver the next live event
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}
```

#### `DaemonConfig` / `DaemonInner` glue

`DaemonInner` grows an `agent: Arc<AgentGlue>` field constructed eagerly in `start_autobind` with `AgentRuntimeConfig` defaults:

```rust
let workspace_root = minos_home_dir()?.join("workspaces");
let agent_cfg = AgentRuntimeConfig {
    workspace_root,
    codex_bin: None,
    ws_port_range: 7879..=7883,
    event_buffer: 256,
};
let agent = Arc::new(AgentGlue::new_with_runtime(AgentRuntime::new(agent_cfg)));
```

Eager construction means `subscribe_events` is valid even when no codex child is running — subscribers get an empty stream and see events as soon as `start_agent` lands. The codex process itself stays unborn until the first `start_agent`.

### 5.5 `minos-ffi-uniffi` additions

- `#[uniffi::Enum]` on `AgentState` (struct-shaped variants; Swift sees `case running(agent:String, threadId:String, startedAt:Date)` etc.).
- `#[uniffi::export(with_foreign)] pub trait AgentStateObserver { fn on_state(&self, state: AgentState); }`.
- New exported methods on `DaemonHandle`:
  ```rust
  #[uniffi::export(async_runtime = "tokio")]
  impl DaemonHandle {
      pub async fn start_agent(&self, req: StartAgentRequest) -> Result<StartAgentResponse, MinosError>;
      pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError>;
      pub async fn stop_agent(&self) -> Result<(), MinosError>;
      pub fn subscribe_agent_state(&self, observer: Arc<dyn AgentStateObserver>) -> Arc<Subscription>;
      pub fn current_agent_state(&self) -> AgentState;
  }
  ```
- `StartAgentRequest` / `StartAgentResponse` / `SendUserMessageRequest`: `#[uniffi::Record]` derives behind the existing `uniffi` feature flag.
- `AgentEvent` itself is **not on the UniFFI surface** this phase and remains so. Swift never observes events directly — the menubar only consumes `AgentState` via the observer. `AgentEvent` crosses the daemon boundary exclusively through the jsonrpsee `subscribe_events` subscription (serde JSON to mobile peers), not UniFFI. No UniFFI derives are added to it; no Swift consumer is generated.

### 5.6 `minos-ffi-frb` additions

- Regenerate frb bindings after protocol / domain changes land (`cargo xtask gen-frb`).
- `MobileClient` facade **unchanged** in this phase — the generated `MinosRpcClient` has the three new methods, but we do not surface them. Chat-ui spec adds the Dart-facing methods.
- `frb_drift_guard` in xtask catches any accidental manual edits to `apps/mobile/lib/src/rust/`.

### 5.7 macOS menubar additions

#### New view: `AgentSegmentView`

```
apps/macos/Minos/Presentation/AgentSegmentView.swift
```

Inserted in `MenuBarView`'s non-error / non-booting branches (unpaired AND paired layouts; after the pairing section, before the "显示今日日志 / 退出" tail). The **text row always reflects the current `AgentState`**, in both release and debug builds. The buttons below are **debug-only** (`#if DEBUG`) — release builds render the text row alone.

Idle (both builds):

```
──────────────
  Agent: Idle
  [启动 Codex（测试）]            ← only in debug
──────────────
```

Running (both builds show the same text; uptime ticks once per second):

```
──────────────
  Agent: Running · thread abc12 · 2m
  [发送 ping（测试）]   [停止 Codex]   ← only in debug
──────────────
```

Crashed (text in both builds; dismiss button in debug only):

```
──────────────
  Agent: Crashed · exit code 137
  [关闭提示]                      ← only in debug
──────────────
```

Starting / Stopping render as plain text rows ("Agent: Starting…" / "Agent: Stopping…") with no interactive controls in any build.

`AgentSegmentView.swift` owns the entire subtree and all buttons; `MenuBarView` just mounts it.

#### `AppState` additions

```swift
@Observable
final class AppState {
    // existing: connectionState, trustedDevice, bootError, displayError, isShowingQr, …

    var agentState: AgentState = .idle
    var agentError: MinosError?

    var agentSubscription: Subscription?

    func startAgent() async         // only called from debug button
    func sendAgentPing() async      // only called from debug button; text = "ping"
    func stopAgent() async          // only called from debug button
    func dismissAgentCrash()        // clears Crashed + agentError
}
```

`startAgent` / `sendAgentPing` / `stopAgent` call `daemon.startAgent(...)` / `daemon.sendUserMessage(...)` / `daemon.stopAgent()` respectively through the existing `DaemonDriving` protocol (now extended).

#### `DaemonDriving` additions

```swift
protocol DaemonDriving {
    // existing methods preserved

    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse
    func sendUserMessage(_ req: SendUserMessageRequest) async throws
    func stopAgent() async throws
    func subscribeAgentState(observer: AgentStateObserver) -> Subscription
    func currentAgentState() -> AgentState
}
```

`DaemonHandle+DaemonDriving.swift` picks up the new methods via a second extension; the UniFFI-generated `DaemonHandle` already exposes them thanks to §5.5.

#### `DaemonBootstrap` extension

After `bootstrap()` establishes `appState.daemon` and wires the connection observer, it also wires the agent observer:

```swift
let agentAdapter = AgentStateObserverAdapter { state in
    Task { @MainActor in appState.agentState = state }
}
appState.agentSubscription = daemon.subscribeAgentState(observer: agentAdapter)
appState.agentState = daemon.currentAgentState()
```

`shutdown()` cancels the subscription alongside the connection one.

---

## 6. Data Flow

### 6.1 Debug-build "start → send → stream → stop"

```
User clicks "启动 Codex（测试）" in menubar     (debug only)
 └─ AppState.startAgent()
     ├─ try: let resp = try await daemon.startAgent(.init(agent: .codex))
     │    ├─ Rust: AgentRuntime.start(Codex)
     │    │    ├─ state_tx.send(Starting { Codex })
     │    │    │      ↓ observer.on_state(.starting) → appState.agentState
     │    │    ├─ spawn codex child (kill_on_drop)
     │    │    ├─ supervisor task watching child.wait()
     │    │    ├─ WS connect ws://127.0.0.1:<port> (3s retry budget)
     │    │    ├─ JSON-RPC initialize
     │    │    ├─ JSON-RPC thread/start → thread_id "abc12"
     │    │    └─ state_tx.send(Running { Codex, "abc12", now })
     │    │           ↓ observer.on_state(.running) → appState.agentState
     │    └─ return StartAgentResponse { session_id: "abc12", cwd: "<ws>" }
     └─ appState.currentSession = resp (held for send/stop)

User clicks "发送 ping（测试）"
 └─ AppState.sendAgentPing()
     ├─ try: await daemon.sendUserMessage(
     │          .init(session_id: currentSession.session_id, text: "ping"))
     │    └─ Rust: CodexClient.call("turn/start", { thread_id: "abc12",
     │                                              items: [{text: "ping"}] })
     └─ codex begins streaming:
          Rust: codex_client reads WS frames →
            "item/started"       → AgentEvent::Raw
            "item/agentMessage/delta" x N → AgentEvent::TokenChunk
            "turn/completed"     → AgentEvent::Done { exit_code: 0 }
          → broadcast::Sender<AgentEvent> fires
          → RpcServerImpl::subscribe_events forwards to any mobile peer
          → xlog WARN-free emission; tracing debug records each event

User clicks "停止 Codex"
 └─ AppState.stopAgent()
     └─ daemon.stopAgent()
          └─ Rust: AgentRuntime.stop()
               ├─ state_tx.send(Stopping)
               ├─ polite turn/interrupt + thread/archive
               ├─ child.kill() (SIGTERM → 3s → SIGKILL)
               ├─ WS client drops → pending tasks unwind
               └─ state_tx.send(Idle)
                     ↓ observer → appState.agentState = .idle
```

### 6.2 Event flow to a mobile subscriber

```
(apps/mobile not used this phase; this is the forward-compat path)

Mobile client sends WS frame:
  { "jsonrpc":"2.0", "method":"minos_subscribe_events", "params":[], "id":42 }
 → daemon jsonrpsee subscription handler
     ├─ agent.event_stream() → broadcast::Receiver<AgentEvent>
     └─ loop: serialize event → sink.send
Mobile client receives notifications:
  { "jsonrpc":"2.0", "method":"minos_agent_event",
    "params":{"subscription":42,
              "result":{"type":"token_chunk","text":"Hello"}}}
```

### 6.3 Crash path

```
codex child dies (OOM, segfault, unexpected exit)
 └─ supervisor task sees child.wait() resolve
     ├─ if state == Stopping → state_tx.send(Idle)  (expected termination)
     └─ else → state_tx.send(Crashed { reason })    (unexpected)
                ↓ observer → appState.agentState = .crashed
                appState.agentError = .codexProtocolError(...) if initiation-time
                menubar renders crashed branch with "关闭提示"
```

`dismissAgentCrash()` clears `agentError` but does NOT change `agentState`; the next successful `startAgent` transitions `Crashed → Starting → Running` naturally.

### 6.4 Approval ServerRequest (expected to be rare)

> **Note (2026-04-30):** Approval method names and reply-payload shapes
> are now defined by `codex-typed-protocol-design.md` and ADR
> `0019-codex-protocol-typed-codegen.md`. The handler is exhaustive over
> the typed `ServerRequest` enum; the per-variant reject shape (`Denied`
> for v1 `ApplyPatchApproval` / `ExecCommandApproval`, `Decline` for v2
> `CommandExecution` / `FileChange`, empty `GrantedPermissionProfile`
> for `Permissions`) matches the schema. The illustrative `decision:
> "rejected"` literal below is preserved for historical context only —
> no schema accepts that string.

```
codex emits a ServerRequest:
  { "jsonrpc":"2.0","id":"req-xxx","method":"ExecCommandApproval",
    "params":{ ... } }
 └─ codex_client recognizes the id field → this is a request, not notification
     ├─ immediately send reply:
     │    { "jsonrpc":"2.0","id":"req-xxx","result":{ "decision":"rejected" }}
     ├─ log warn!(method = "ExecCommandApproval", "auto-rejected")
     └─ ALSO broadcast AgentEvent::Raw {
          kind: "server_request/ExecCommandApproval",
          payload_json: <stringified original params>
        }
        (so future chat-ui can surface "codex tried to run X, auto-rejected")
```

Because `approval_policy=never` is set in codex's config, this path should be dead code in practice; the Raw forward exists purely so the bridge is never silently complicit when codex does ask.

### 6.5 Daemon shutdown

```
User clicks "退出 Minos"
 └─ AppState.shutdown()
     ├─ subscription.cancel()       (ConnectionState observer)
     ├─ agentSubscription?.cancel() (AgentState observer)
     ├─ await daemon.stop()
     │    └─ Rust DaemonHandle::stop
     │         ├─ agent.shutdown() (= AgentRuntime.stop if Running)
     │         │    → kill codex child
     │         └─ server.shutdown()
     └─ NSApp.terminate(nil)
```

---

## 7. Error Handling

### 7.1 Rust → Swift mapping

All new `MinosError` variants are struct-shaped and cross UniFFI unchanged. Swift receives:

```swift
case codexSpawnFailed(message: String)
case codexConnectFailed(url: String, message: String)
case codexProtocolError(method: String, message: String)
case agentAlreadyRunning
case agentNotRunning
case agentNotSupported(agent: AgentName)
case agentSessionIdMismatch
```

`MinosError+Display.swift` gains seven new `switch` arms routing to the matching `ErrorKind`. Localized strings come from the single-source Rust table (no duplication).

### 7.2 UI policy

| Trigger | Swift handler | Display |
|---|---|---|
| `startAgent()` throws | `appState.agentError = e` + `agentState = .idle` | Inline banner, auto-dismiss 3s |
| `sendAgentPing()` throws | Same | Same |
| `stopAgent()` throws | Same | Same |
| Agent crash (no call in flight) | `agentState = .crashed { reason }` (no `agentError` unless chained) | Menubar agent segment shows the Crashed branch |

Crash paths stand alone from `displayError` — `MenuBarView` is already branching on boot/display errors; agent-segment error rendering is self-contained inside `AgentSegmentView`.

### 7.3 Errors this phase cannot trigger

- `AgentNotSupported { agent: Claude | Gemini }` — only surfaceable by a misbehaving mobile client sending a non-Codex enum value. Unit-tested but unreachable through menubar debug buttons (they hard-code `.codex`).
- `AgentSessionIdMismatch` — unreachable from menubar because the debug flow always uses the latest `currentSession`. Covered by unit tests only.

---

## 8. Testing Strategy

### 8.1 `minos-agent-runtime` unit tests

| Module | Scenario | Technique |
|---|---|---|
| `state` | Enum round-trip + serde for fixture variants | Unit |
| `translate` | Every known codex notification method → expected `AgentEvent` variant | Table-driven with `#[rstest::rstest]` over fixture JSON files |
| `translate` | Unknown method → `AgentEvent::Raw { kind, payload_json }` with faithful payload | Same |
| `approvals` | Every approval method name in codex's schema produces a correctly-shaped reject response JSON | Table-driven |
| `process` | `kill_on_drop` semantics (spawn + drop handle → child dies within N ms) | `#[tokio::test]` with `tokio::process::Command::new("sleep").arg("60")` |
| `process` | `stop_graceful` escalates SIGTERM → SIGKILL when child ignores SIGTERM | `tokio::test` with `trap`-style shell child |
| `codex_client` | JSON-RPC framing encode/decode on WS mock | Unit — mock WS via `tokio::io::duplex` |
| `runtime` | `start → ok → Running` transitions; `start → start` → `AgentAlreadyRunning` | `tokio::test` with a scripted FakeCodexServer |
| `runtime` | `stop` idempotent (Idle / Crashed → Ok; double-stop no panic) | Same |
| `runtime` | `send_user_message` with wrong session_id → `AgentSessionIdMismatch` | Same |
| `runtime` | Crash propagation: fake codex panics → state → `Crashed { reason }` | Same |
| `runtime` | Broadcast fan-out: two subscribers receive same sequence | Same |

`FakeCodexServer` lives in `crates/minos-agent-runtime/src/test_support.rs` behind a `test-support` Cargo feature — a tokio-tungstenite WS accept loop with a script queue (`VecDeque<Step>` where `Step` is `ExpectRequest { method, reply }` / `EmitNotification { method, params }` / `DieUnexpectedly`). The agent-runtime crate enables the feature in its own `[dev-dependencies]` (self-ref pattern) so its `tests/` integration files see it; the daemon crate enables it in its own `[dev-dependencies]` so `agent_e2e.rs` imports it as `minos_agent_runtime::test_support::FakeCodexServer`. Each test builds its own script.

### 8.2 Daemon integration (`crates/minos-daemon/tests/agent_e2e.rs`)

Full in-process exercise with `FakeCodexServer`:

```
1. Start FakeCodexServer on ephemeral port
2. Build DaemonHandle via start_autobind (fake WorkspaceRoot under tempdir)
3. Rewire AgentRuntime to connect to fake server's port (via test-only config hook)
4. Call daemon.start_agent(.codex) → expect Running state
5. Subscribe agent_state via observer; assert state sequence
6. Subscribe events via broadcast; assert events from a scripted turn
7. daemon.send_user_message(session_id, "ping") → fake asserts it received turn/start
8. Fake emits item/agentMessage/delta; harness asserts AgentEvent::TokenChunk received
9. Fake emits turn/completed; harness asserts AgentEvent::Done received
10. daemon.stop_agent() → fake's WS closes; child process killed
11. assert state returns to Idle
```

A "fake-port" injection seam exists on `AgentRuntimeConfig` so tests can skip the subprocess-spawn step and connect directly to the fake WS server's URL.

### 8.3 Swift logic tests (`MinosTests/Application/AgentStateTests.swift`)

Covered scenarios:

| Scenario | Setup | Assertion |
|---|---|---|
| Observer drives agentState | `MockDaemon.subscribeAgentState` stores observer; test pushes `.running(..)` | `appState.agentState == .running(...)` |
| `startAgent()` happy path (debug-only; covered regardless) | `MockDaemon.startAgent` returns `.init(session_id: "t1", cwd: "/w")` | `currentSession.session_id == "t1"` |
| `startAgent()` throws `.agentAlreadyRunning` | MockDaemon throws | `appState.agentError != nil`, agentState unchanged |
| `sendAgentPing()` calls with correct session_id | MockDaemon captures args | Captured args match currentSession.session_id + text "ping" |
| `stopAgent()` happy path | MockDaemon records | call-count == 1; `currentSession` cleared |
| `dismissAgentCrash()` clears `agentError`, leaves `agentState` alone | `agentState = .crashed(..)`, `agentError = ...` | `agentError == nil`, `agentState == .crashed(..)` |
| `shutdown()` cancels agentSubscription | MockSubscription + shutdown | `cancel()` call-count == 1 |

MockDaemon gains the three agent methods + state / subscription forwarders.

### 8.4 `xtask check-all` additions

- Default: no change. Runs Rust + Swift + Flutter legs as today. `minos-agent-runtime`'s unit tests run inside `cargo test --workspace`, `agent_e2e.rs` runs inside the same step (requires codex binary? **no** — fake server only).
- Opt-in: `MINOS_XTASK_WITH_CODEX=1 cargo xtask check-all` (or new `--with-codex` arg) appends an extra leg:
  ```
  cargo run -p minos-xtask --bin codex-smoke
  ```
  — spawns real `codex app-server`, sends a trivial `"reply with the word ok"` prompt, asserts a `token_chunk` containing `ok` arrives within 60s. Default skipped.

### 8.5 CI

No new CI leg. The daemon integration test runs inside `cargo test --workspace` on both linux and macos lanes. Real codex is not installed on either runner.

### 8.6 Done criteria

Plan 04 is **done** when ALL of:

1. `cargo xtask check-all` green on a fresh clone (no env vars).
2. `cargo test -p minos-agent-runtime` ≥ 12 passing tests covering each of the 12 scenarios in §8.1.
3. `cargo test -p minos-daemon --test agent_e2e` passes (fake codex round trip).
4. Swift `AgentStateTests` passes; existing `AppStateTests` still green.
5. Manual smoke on maintainer workstation with debug build: click "启动 Codex（测试）" → menubar shows Running with thread id → click "发送 ping（测试）" → within 10 s, `~/Library/Logs/Minos/daemon_YYYYMMDD.xlog` contains a `token_chunk` line whose text includes the codex response to "ping" → click "停止 Codex" → menubar shows Idle; `ps | grep codex` reports no zombie.
6. `MINOS_XTASK_WITH_CODEX=1 cargo xtask check-all` green on the maintainer's workstation (codex installed).

Items 1–4 are CI-enforced. 5 and 6 are maintainer-run pre-merge checks documented in the plan's closing checklist.

---

## 9. Tooling Notes

### 9.1 `$MINOS_HOME` and the workspace dir

The existing `default_minos_home()` inside `crates/minos-daemon/src/main.rs` is **promoted** to a reusable function `minos_daemon::paths::minos_home()` that both the daemon CLI (`main.rs`) and the in-process boot path (`AgentGlue::new`, via `start_autobind`) call. The CLI module's local copy is removed and replaced with a call-site. Behavior preserved:

- If `$MINOS_HOME` env var is set (or `--minos-home` arg) → use it.
- Else Linux / CLI → `$HOME/.minos`.
- Else macOS GUI → keep the daemon's current platform-native defaults; the **workspace** under it is always `<minos_home>/workspaces`, regardless of platform.

The `workspaces/` subdirectory is created on first `start_agent` via `std::fs::create_dir_all`. This directory is intended to be user-visible (they may `cd ~/.minos/workspaces` in a shell to inspect codex's output).

### 9.2 Codex CLI presence

Not checked at daemon startup. `start_agent` throws `CodexSpawnFailed { message: "codex: command not found" }` on the first invocation if absent. `which codex` is an explicit non-precondition for `cargo xtask check-all` (agent_e2e uses the fake server).

### 9.3 `uniffi-bindgen-swift` regeneration

Running `cargo xtask gen-uniffi` after the FFI changes produces new Swift enum members (`AgentState`, `AgentStateObserver`) and new method signatures. Regenerated `Generated/` is gitignored as today; the real check is that `xcodebuild build` succeeds with the regenerated content.

### 9.4 frb regeneration

Running `cargo xtask gen-frb` regenerates `apps/mobile/lib/src/rust/` with:
- `AgentEvent.raw(kind: String, payload_json: String)` enum arm
- `MinosRpcClient.start_agent(...)`, `send_user_message(...)`, `stop_agent(...)`

`frb_drift_guard` in xtask catches any staleness; CI's dart lane runs the regenerate + diff.

---

## 10. Out of Scope (reiterated)

| Item | Phase | Why not here |
|---|---|---|
| Mobile / iOS chat UI | `streaming-chat-ui-design.md` | UI-per-phase rule; mobile surface intentionally unchanged |
| `respond_approval` RPC + approval inbox / diff preview UX | Same | Needs UI; chat-ui spec owns approval flow end-to-end |
| Multi-session concurrency / per-session WS-server-per-session model | P2+ | No current use case; adding `session_id` to `subscribe_events` is a breaking change — deferred |
| Per-session workspace override via `StartAgentRequest.cwd` | Later additive spec | Current convention is one shared dir; mobile will pass when it has a UI for picking |
| PTY agents (claude, gemini) | `pty-agent-claude-gemini-design.md` | Separate spec; different transport |
| Agent auto-reconnect after crash | Later | Today's behavior: `Crashed` is terminal until user retries; keeps code simple |
| Codex auth / login / token refresh flows | Indefinitely | Out of Minos's remit; users run `codex login` once before use |
| Real-codex smoke in CI default | Deferred | Installing codex + maintaining model auth on CI runners is out of scope |
| Windows support | Out | `sandbox_permissions` flags use POSIX paths; Windows workspace handling is its own problem |

---

## 11. Open Questions

None remaining. Eight questions were posed and resolved during brainstorming:

1. Scope boundary → Scope B (bridge + menubar observer; no chat UI). §2.
2. Start trigger → Trigger B (RPC-primary; menubar observer + debug-only buttons). §5.7, §6.1.
3. Session cardinality → Session A (single session at a time; no `session_id` on `subscribe_events`). §5.2, §5.4.
4. Input channel → Full primitive (`start_agent` + `send_user_message` + `stop_agent`). §5.2.
5. Workspace dir → `$MINOS_HOME/workspaces`, shared across sessions this phase. §4.3, §9.1.
6. Transport to codex → WebSocket (`codex app-server --listen ws://127.0.0.1:<port>`); rationale in ADR 0009. §4, §5.1.
7. Event translation → `AgentEvent::Raw` variant as forward-compat escape hatch; rationale in ADR 0010. §5.2.
8. Approval handling → codex config `approval_policy=never` + sandbox-restricted; leaked approvals auto-reject and forward as `Raw`. §4, §6.4.

---

## 12. File Inventory

**New files:**

```
crates/minos-agent-runtime/Cargo.toml
crates/minos-agent-runtime/src/lib.rs
crates/minos-agent-runtime/src/state.rs
crates/minos-agent-runtime/src/process.rs
crates/minos-agent-runtime/src/codex_client.rs
crates/minos-agent-runtime/src/translate.rs
crates/minos-agent-runtime/src/approvals.rs
crates/minos-agent-runtime/src/runtime.rs
crates/minos-agent-runtime/src/test_support.rs            (`pub mod test_support` behind `feature = "test-support"`; exports `FakeCodexServer`; agent-runtime's own tests enable the feature via dev-dependency self-ref, daemon enables it via dev-dependency — no source duplication across crates)
crates/minos-agent-runtime/tests/translate_table.rs
crates/minos-agent-runtime/tests/runtime_e2e.rs
crates/minos-daemon/src/agent.rs
crates/minos-daemon/tests/agent_e2e.rs
crates/minos-domain/tests/golden/agent_event_raw.json
apps/macos/Minos/Presentation/AgentSegmentView.swift
apps/macos/Minos/Application/AgentStateObserverAdapter.swift
apps/macos/MinosTests/Application/AgentStateTests.swift
docs/adr/0009-codex-app-server-ws-transport.md
docs/adr/0010-agent-event-raw-variant.md
```

**Modified files:**

```
Cargo.toml                                                add minos-agent-runtime workspace member
crates/minos-domain/src/error.rs                          add 7 MinosError variants + ErrorKind + user_message
crates/minos-domain/tests/golden.rs                       include raw-event golden
crates/minos-protocol/src/events.rs                       AgentEvent::Raw variant + test
crates/minos-protocol/src/messages.rs                     Start/Send request+response types + goldens
crates/minos-protocol/src/rpc.rs                          add 3 #[method(...)] entries
crates/minos-daemon/Cargo.toml                            dep on minos-agent-runtime
crates/minos-daemon/src/lib.rs                            export agent module + re-export types
crates/minos-daemon/src/handle.rs                         wire AgentGlue into DaemonInner; 5 new methods
crates/minos-daemon/src/rpc_server.rs                     implement 3 new methods; rewrite subscribe_events
crates/minos-daemon/src/subscription.rs                   add AgentStateObserver + helper (generic or dedicated)
crates/minos-daemon/src/main.rs                           expose paths::minos_home() helper (for agent)
crates/minos-ffi-uniffi/src/lib.rs                        UniFFI derives on new types; re-exports
apps/macos/Minos/Application/AppState.swift               agentState, agentError, currentSession fields + methods
apps/macos/Minos/Application/DaemonDriving.swift          5 new protocol methods
apps/macos/Minos/Application/ObserverAdapter.swift        (if consolidated) or split into AgentStateObserverAdapter
apps/macos/Minos/Infrastructure/DaemonBootstrap.swift     wire agentSubscription
apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift  agent extension
apps/macos/Minos/Presentation/MenuBarView.swift           mount AgentSegmentView in unpaired + paired branches
apps/macos/MinosTests/TestSupport/MockDaemon.swift        implement new protocol methods
apps/mobile/lib/src/rust/**                               regenerated by frb; AgentEvent.raw + client methods
xtask/src/main.rs                                         optional --with-codex flag / MINOS_XTASK_WITH_CODEX env
xtask/src/codex_smoke.rs (new section, if separated)       codex-smoke binary for opt-in leg
README.md                                                 P1 landing note (Plan 04 Tier complete)
```

**Deleted:** none.

---

## 13. ADRs

Two new ADRs accompany Plan 04:

- **`docs/adr/0009-codex-app-server-ws-transport.md`** — why WebSocket loopback over stdio-pipe. Touches: debuggability (tcpdump-friendly), alignment with codex's first-class IDE-integration path, willingness to accept one extra port + child-supervision responsibility.
- **`docs/adr/0010-agent-event-raw-variant.md`** — why `AgentEvent::Raw` as a single escape hatch rather than growing the typed variant set per codex release. Touches: forward-compat across codex protocol churn, kept minimal because chat-ui spec will typed-up what it actually renders.

Both ADRs follow the same MADR 4.0 format as 0001–0008.
