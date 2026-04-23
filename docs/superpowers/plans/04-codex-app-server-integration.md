# Minos · Codex App-Server Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`. Execute **one phase per subagent, one validation gate per phase, and one commit per phase**. This document is intentionally phase-oriented; do not fall back to task-by-task micro-commits. All dispatched subagents run on `opus` — do not auto-downgrade.

**Goal:** Wire `codex app-server` into the macOS daemon as a supervised subprocess over a WebSocket loopback, implement `subscribe_events` end-to-end, and add three new RPC methods (`start_agent` / `send_user_message` / `stop_agent`) plus an observer-only Agent segment on the macOS menubar. The plan ends when `cargo xtask check-all` is green, the daemon integration test `agent_e2e.rs` passes against a scripted `FakeCodexServer`, and the maintainer has exercised the debug-build menubar buttons against a real `codex` install on their workstation.

**Architecture:** A new crate `minos-agent-runtime` owns the codex subprocess, the loopback WebSocket client, the event-translation table, and the approval auto-reject path. The daemon grows an `AgentGlue` that holds one `AgentRuntime`, re-exposes it through new `DaemonHandle` methods, and implements the previously-stubbed `subscribe_events` by forwarding `AgentRuntime`'s broadcast stream. Plan-02's observer pattern (`ConnectionStateObserver` + `spawn_observer`) is duplicated — not genericized — for `AgentState`. The Mac menubar gains exactly one new view (`AgentSegmentView`) whose interactive controls are guarded by `#if DEBUG`. No chat UI, no mobile RPC client surface — both land in the next P1 spec (`streaming-chat-ui-design.md`).

**Tech Stack:**
- Rust stable (inherited); new crate `minos-agent-runtime` with `tokio` full, `tokio-tungstenite`, `serde`/`serde_json`, `thiserror`, `tracing`, `uuid`, `url`, `futures-util`.
- jsonrpsee `subscribe_events` sink API (already pinned by plan 01; no version bump).
- UniFFI 0.31 derives (existing feature flag); `uniffi-bindgen-swift` for regeneration.
- `codex` CLI (external — user-installed; tests use `FakeCodexServer`, not the real binary).
- Xcode 26.2 for Swift compile + test (macOS lane); macOS 14+ runtime.
- `flutter_rust_bridge_codegen` 2.x for frb regeneration (Dart picks up additive changes — no UI work).

**Reference spec:** Implements `docs/superpowers/specs/codex-app-server-integration-design.md`. All behavior, RPC contract, error mapping, UI surface, and testing scope decisions live in the spec; this plan optimizes execution order and commit boundaries.

**Working directory note:** Runs on `main` alongside plans 01–03; single-developer repo. No worktree isolation required.

**Version drift policy:** Versions listed here are accurate as of 2026-04-23. If `cargo add` resolves to a higher minor when executed, prefer the resolved version unless compilation fails.

---

## File structure (target end-state)

```text
minos/
├── Cargo.toml                                            [modified: + minos-agent-runtime workspace member]
├── README.md                                             [modified: plan-04 landing note]
├── .github/workflows/ci.yml                              [unchanged — codex not installed on CI]
├── crates/
│   ├── minos-domain/
│   │   ├── src/error.rs                                  [modified: +7 MinosError variants, +7 ErrorKind arms, +14 user_message strings]
│   │   └── tests/golden/
│   │       └── agent_event_raw.json                      [new]
│   ├── minos-protocol/
│   │   ├── src/events.rs                                 [modified: AgentEvent::Raw variant + round-trip test]
│   │   ├── src/messages.rs                               [modified: StartAgentRequest/Response, SendUserMessageRequest + goldens]
│   │   └── src/rpc.rs                                    [modified: 3 new #[method(...)] entries]
│   ├── minos-agent-runtime/                              [new crate]
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── state.rs
│   │       ├── translate.rs
│   │       ├── approvals.rs
│   │       ├── process.rs
│   │       ├── codex_client.rs
│   │       ├── runtime.rs
│   │       └── test_support.rs                           [feature = "test-support"]
│   │   └── tests/
│   │       ├── translate_table.rs
│   │       └── runtime_e2e.rs
│   └── minos-daemon/
│       ├── Cargo.toml                                    [modified: + minos-agent-runtime path dep + test-support dev-dep]
│       ├── src/
│       │   ├── lib.rs                                    [modified: pub mod agent; pub mod paths; re-exports]
│       │   ├── handle.rs                                 [modified: AgentGlue field, 5 new methods, stop() wires agent.shutdown()]
│       │   ├── agent.rs                                  [new: AgentGlue composition]
│       │   ├── paths.rs                                  [new: promoted minos_home() helper]
│       │   ├── main.rs                                   [modified: call paths::minos_home() instead of local helper]
│       │   ├── rpc_server.rs                             [modified: 3 new methods + real subscribe_events]
│       │   └── subscription.rs                           [modified: + AgentStateObserver + spawn_agent_observer]
│       └── tests/
│           └── agent_e2e.rs                              [new]
├── crates/minos-ffi-uniffi/
│   └── src/lib.rs                                        [modified: AgentState derive, observer trait, new DaemonHandle methods]
├── apps/macos/
│   ├── Minos/
│   │   ├── Generated/                                    [regenerated by gen-uniffi; gitignored]
│   │   ├── Application/
│   │   │   ├── AppState.swift                            [modified: agentState, agentError, currentSession + methods]
│   │   │   ├── DaemonDriving.swift                       [modified: +5 agent methods on protocol]
│   │   │   └── AgentStateObserverAdapter.swift           [new]
│   │   ├── Infrastructure/
│   │   │   ├── DaemonBootstrap.swift                     [modified: wire agentSubscription]
│   │   │   └── DaemonHandle+DaemonDriving.swift          [modified: extension with agent methods]
│   │   └── Presentation/
│   │       ├── MenuBarView.swift                         [modified: mount AgentSegmentView]
│   │       └── AgentSegmentView.swift                    [new]
│   └── MinosTests/
│       ├── TestSupport/MockDaemon.swift                  [modified: implement +5 agent protocol methods]
│       └── Application/AgentStateTests.swift             [new]
├── apps/mobile/
│   └── lib/src/rust/                                     [regenerated by gen-frb; frb-drift guard enforces]
├── xtask/
│   └── src/main.rs                                       [modified: --with-codex / MINOS_XTASK_WITH_CODEX opt-in leg]
└── docs/adr/
    ├── 0009-codex-app-server-ws-transport.md             [new]
    └── 0010-agent-event-raw-variant.md                   [new]
```

---

## Current checkpoint

- Plans 01–03 landed; `cargo xtask check-all` is green; mobile iOS is up; FRB bridge is alive.
- `AgentEvent` exists in `minos-protocol::events` with 5 variants and zero producers; `MinosRpc::subscribe_events` is declared but the daemon implementation returns `-32601 "not implemented"`.
- `DaemonHandle::stop` is `&self`-taking (plan 02 refactor) and `DaemonInner` is `Arc`-wrapped, so adding new `&self` methods costs nothing structural.
- `minos-daemon::main.rs` already owns a `default_minos_home()` helper returning `$HOME/.minos`; promoting it to `paths::minos_home()` is the only refactor this plan takes outside the agent runtime.
- `codex` CLI is assumed installed on the maintainer's workstation; CI runners do NOT install it, and the daemon integration test does not require it (uses `FakeCodexServer`).

---

## Phase dependency graph

```text
Plans 01–03 landed.
 -> Phase A  Protocol + domain prep  (AgentEvent::Raw, new RPC types, 7 MinosError variants)
    -> Phase B  minos-agent-runtime — state/translate/approvals + FakeCodexServer test harness
       -> Phase C  minos-agent-runtime — process/codex_client/runtime state machine + integration tests
          -> Phase D  minos-daemon wiring — AgentGlue, new DaemonHandle methods, subscribe_events real impl, agent_e2e.rs
             -> Phase E  UniFFI + macOS menubar — gen-uniffi, AppState, AgentSegmentView, AgentStateTests
                -> Phase F  frb regen + xtask opt-in smoke + ADRs 0009/0010 + README landing
```

### Phase execution rules

1. One implementation subagent owns one phase end-to-end.
2. A phase is not done until its listed validation commands pass AND (where applicable) the required generated artifacts are regenerated and checked in.
3. Do not split a phase into multiple commits unless validation exposes a narrow repair inside that same phase.
4. The design spec remains the source of truth for behavior, layering, and UI scope; this plan optimizes execution order and commit boundaries.
5. If a phase needs a small adjacent config/doc change to make its own gate pass, keep that change in the same phase commit.
6. Never relax the unit-test-only rule (spec §2.3) or the UI-per-phase rule (spec §2.4) to unblock a later phase.
7. For UI-shaped work (Phase E's AgentSegmentView), dispatch the phase to the `frontend-design` specialist; other phases go to the generic implementer.

---

## Phase A · Protocol + domain prep

**Goal:** Land every pure-additive change to `minos-domain` and `minos-protocol` so Phase B can treat them as stable. No new logic — just enum variants, types, RPC method declarations, strings. Golden tests ensure Serde stability across crates.

**Scope:**

- `crates/minos-protocol/src/events.rs`:
  - Add `AgentEvent::Raw { kind: String, payload_json: String }` variant. Serde tag stays `type`, variant renames `snake_case`.
  - Add a unit test asserting the `raw` variant serializes to `{"type":"raw","kind":"...","payload_json":"..."}`.
- `crates/minos-domain/tests/golden/agent_event_raw.json`: new file with the canonical shape.
- `crates/minos-domain/tests/golden.rs`: add a round-trip test pair for the new golden file.
- `crates/minos-protocol/src/messages.rs`:
  - Add `StartAgentRequest { agent: AgentName }`, `StartAgentResponse { session_id: String, cwd: String }`, `SendUserMessageRequest { session_id: String, text: String }` — all with `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]`.
  - Per-type Serde round-trip unit tests.
- `crates/minos-protocol/src/rpc.rs`: add three `#[method(...)]` entries to the `MinosRpc` trait:
  ```rust
  #[method(name = "start_agent")]
  async fn start_agent(&self, req: StartAgentRequest) -> jsonrpsee::core::RpcResult<StartAgentResponse>;

  #[method(name = "send_user_message")]
  async fn send_user_message(&self, req: SendUserMessageRequest) -> jsonrpsee::core::RpcResult<()>;

  #[method(name = "stop_agent")]
  async fn stop_agent(&self) -> jsonrpsee::core::RpcResult<()>;
  ```
  `subscribe_events` signature stays exactly as it is.
- `crates/minos-domain/src/error.rs`:
  - Append 7 variants to `ErrorKind` (canonical order, follow the existing pattern):
    `CodexSpawnFailed, CodexConnectFailed, CodexProtocolError, AgentAlreadyRunning, AgentNotRunning, AgentNotSupported, AgentSessionIdMismatch`.
  - Append 7 matching struct-shaped variants to `MinosError` with `#[error("...")]` strings from spec §5.3.
  - Append 7 arms to `MinosError::kind`.
  - Append 14 arms (7 × 2 langs) to `ErrorKind::user_message`:

    | Kind | zh | en |
    |---|---|---|
    | CodexSpawnFailed | "无法启动 Codex CLI；请确认已安装 `codex`" | "Failed to launch codex CLI; is codex installed?" |
    | CodexConnectFailed | "无法连接 Codex 服务" | "Could not reach codex app-server" |
    | CodexProtocolError | "Codex 返回错误，请查看日志" | "Codex returned an error — see log" |
    | AgentAlreadyRunning | "Agent 已在运行" | "An agent session is already running" |
    | AgentNotRunning | "当前没有 Agent 会话" | "No agent session is running" |
    | AgentNotSupported | "这一期仅支持 Codex" | "Only Codex is supported in this phase" |
    | AgentSessionIdMismatch | "会话已失效，请重新启动" | "Session is no longer active; please restart" |

  - Update the two exhaustive-match tests (`kind_exhaustively_matches_every_variant`, `every_error_kind_has_user_message_in_both_langs`) to include the new variants and assert `cases.len() == 18`.

**Preserved constraints:**
- No modifications to `subscribe_events` signature — `async fn subscribe_events(&self) -> SubscriptionResult` stays byte-for-byte.
- No changes to existing `AgentEvent` variants; the new variant is additive at the end of the enum.
- `MinosError::AgentNotSupported { agent: AgentName }` carries the existing `AgentName` enum — no new cross-crate type surface.
- Do NOT touch `minos-ffi-uniffi` in this phase; UniFFI surface changes land in Phase E (where gen-uniffi regen runs end-to-end).
- Do NOT regenerate frb — that is Phase F (after all protocol/daemon code compiles).

**Files touched:**
- `crates/minos-protocol/src/events.rs`
- `crates/minos-protocol/src/messages.rs`
- `crates/minos-protocol/src/rpc.rs`
- `crates/minos-domain/src/error.rs`
- `crates/minos-domain/tests/golden/agent_event_raw.json` (new)
- `crates/minos-domain/tests/golden.rs`

**Validation:**
```bash
cargo fmt --check
cargo clippy -p minos-domain -p minos-protocol --all-targets -- -D warnings
cargo test -p minos-domain -p minos-protocol
cargo xtask check-all
```

**Commit boundary:**
```bash
git add crates/minos-domain crates/minos-protocol
git commit -m "feat(protocol): agent RPC surface + AgentEvent::Raw + 7 error variants"
```

---

## Phase B · `minos-agent-runtime` scaffold + pure-logic modules

**Goal:** Create the new crate and deliver every module that can be tested without spawning a subprocess or opening a socket. Phase C will glue these to real I/O.

**Scope:**

- `Cargo.toml` (workspace root): append `"crates/minos-agent-runtime"` to `members`.
- `crates/minos-agent-runtime/Cargo.toml`:
  ```toml
  [package]
  name = "minos-agent-runtime"
  version = "0.1.0"
  edition = "2021"

  [features]
  test-support = []

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
  minos-agent-runtime = { path = ".", features = ["test-support"] }
  tokio               = { workspace = true, features = ["full", "test-util", "macros"] }

  [lints]
  workspace = true
  ```
  Add missing workspace-level deps (`tokio-tungstenite`, `futures-util`, `url`) to the root `Cargo.toml [workspace.dependencies]` if absent; match versions used by existing crates (`minos-transport` already pulls `tokio-tungstenite`; `url` is used transitively via jsonrpsee).
- `src/lib.rs`: public facade — re-export `AgentRuntime`, `AgentRuntimeConfig`, `StartAgentOutcome`, `AgentState`, `translate::translate_notification`, `approvals::build_auto_reject`. Phase C fills in `AgentRuntime` itself; in Phase B ship the crate with a `#[allow(dead_code)]` placeholder so it compiles:
  ```rust
  pub mod state;
  pub mod translate;
  pub mod approvals;
  #[cfg(feature = "test-support")]
  pub mod test_support;

  pub use state::AgentState;
  ```
- `src/state.rs`:
  ```rust
  use minos_domain::AgentName;
  use std::time::SystemTime;
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(tag = "state", rename_all = "snake_case")]
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
  Unit tests: each variant round-trips via `serde_json`; `Idle` is the `Default`.
- `src/translate.rs`: pure function `translate_notification(method: &str, params: &serde_json::Value) -> minos_domain::AgentEvent`. Implement the mapping table from spec §5.2:
  - `item/agentMessage/delta` → `TokenChunk { text: params["delta"].as_str() }` (fall back to empty string if missing).
  - `item/reasoning/textDelta` / `item/reasoning/summaryTextDelta` → `Reasoning { text: params["delta"].as_str() }`.
  - `item/commandExecution/outputDelta` → `ToolResult { name: "shell", output: params["chunk"].as_str() }`.
  - `item/mcpToolCall/progress` where `params["phase"] == "started"` → `ToolCall { name: params["name"].as_str(), args_json: serde_json::to_string(&params["arguments"]).unwrap_or_default() }`; otherwise `ToolResult` with `output = params["result"]` stringified.
  - `turn/completed` → `Done { exit_code: 0 }` (codex does not expose non-zero; map any non-empty error field to `-1`).
  - Any other method → `Raw { kind: method.to_string(), payload_json: serde_json::to_string(params).unwrap_or_default() }`.

  Unit-test table with 8 entries: each of the 5 hard-mapped methods produces the expected `AgentEvent`, plus three `Raw` cases (`item/plan/delta`, `thread/tokenUsage/updated`, and a completely-unknown method `foo/bar/baz`).
- `src/approvals.rs`: pure function `build_auto_reject(request_id: serde_json::Value, method: &str) -> serde_json::Value`. Produces:
  ```json
  {"jsonrpc":"2.0","id":<request_id>,"result":{"decision":"rejected"}}
  ```
  Unit test: each of the 5 approval method names (`ApplyPatchApproval`, `ExecCommandApproval`, `FileChangeRequestApproval`, `PermissionsRequestApproval`, `CommandExecutionRequestApproval`) produces a syntactically valid response with the correct id and shape.
- `src/test_support.rs` (gated `#[cfg(feature = "test-support")]`): implement `FakeCodexServer` — a tokio-tungstenite WS accept loop backed by a `VecDeque<Step>` script:
  ```rust
  pub enum Step {
      ExpectRequest { method: String, reply: serde_json::Value },
      EmitNotification { method: String, params: serde_json::Value },
      EmitServerRequest { method: String, params: serde_json::Value },
      DieUnexpectedly,
  }

  pub struct FakeCodexServer { /* private: listener task handle, port */ }

  impl FakeCodexServer {
      pub async fn bind(script: Vec<Step>) -> (Self, u16) { /* ... */ }
      pub async fn stop(self) { /* abort task */ }
  }
  ```
  Implementation notes: accept **exactly one** client per test; drain the script in order; on `ExpectRequest`, read a frame, assert it's a JSON-RPC request with matching `method`, send back `{"jsonrpc":"2.0","id":<id>,"result":<reply>}`; on `EmitNotification`, send `{"jsonrpc":"2.0","method":...,"params":...}`; on `EmitServerRequest`, send a request with a fresh string id and record the id so the caller can later assert how the agent-runtime replied. `DieUnexpectedly` closes the WS abruptly.
- `tests/translate_table.rs`: integration test re-asserting the 8 translation cases from the unit test. Exists separately so the plan can run `cargo test -p minos-agent-runtime --test translate_table` in isolation during debugging.

**Preserved constraints:**
- `minos-agent-runtime` has NO dependency on `minos-protocol` (per spec §5.1 Cargo.toml). `AgentEvent` currently lives in `minos-protocol::events`; to let agent-runtime use it without a `minos-protocol` dep, Phase B relocates the enum to `minos-domain::events` and leaves `minos-protocol::events` as a one-line re-export (`pub use minos_domain::events::*;`). Downstream crates continue importing from `minos_protocol` unchanged.
- The relocation is done once, in Phase B, before agent-runtime first references the type. Phase A's `AgentEvent::Raw` variant is **added at the original location** (`minos-protocol::events`) and gets carried over during the Phase B move along with its golden test.
- FakeCodexServer uses `tokio_tungstenite::accept_async` directly — do not wrap `minos-transport::WsServer`.
- No `unsafe` blocks.
- `#![forbid(unsafe_code)]` at `src/lib.rs` top, mirroring `minos-protocol`.

**Files touched:**
- `Cargo.toml` (workspace)
- `crates/minos-domain/src/lib.rs` (re-export events module)
- `crates/minos-domain/src/events.rs` (new — move from protocol; `AgentEvent` + `Raw`)
- `crates/minos-protocol/src/events.rs` (becomes a one-line `pub use minos_domain::events::*;`)
- `crates/minos-agent-runtime/Cargo.toml` (new)
- `crates/minos-agent-runtime/src/{lib,state,translate,approvals,test_support}.rs` (new)
- `crates/minos-agent-runtime/tests/translate_table.rs` (new)

**Validation:**
```bash
cargo fmt --check
cargo clippy -p minos-agent-runtime -p minos-domain -p minos-protocol --all-targets -- -D warnings
cargo test -p minos-agent-runtime
cargo test -p minos-protocol          # regression: AgentEvent re-export path still serializes
cargo test -p minos-domain            # regression: golden still passes for new events module location
cargo xtask check-all
```

**Commit boundary:**
```bash
git add Cargo.toml crates/minos-agent-runtime crates/minos-domain crates/minos-protocol
git commit -m "feat(agent-runtime): scaffold crate + state/translate/approvals + FakeCodexServer"
```

---

## Phase C · `minos-agent-runtime` process + codex_client + runtime state machine

**Goal:** Fill in the three modules that touch real I/O — subprocess spawn, WS client, and the state-machine glue — and prove the whole stack against `FakeCodexServer` through an integration test.

**Scope:**

- `src/process.rs`: `pub(crate) struct CodexProcess { child: Option<tokio::process::Child> }` with:
  - `spawn(bin: &Path, args: &[&str]) -> Result<Self, MinosError>` using `tokio::process::Command` with `.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)`.
  - `stderr_drain(&mut self)` that consumes stderr into `tracing::warn!(target = "minos_agent_runtime::process", ...)` lines (spawn a task once per process).
  - `stop_graceful(&mut self) -> Result<ExitStatus, MinosError>`: `kill()` with SIGTERM → `tokio::time::timeout(Duration::from_secs(3), child.wait())` → on timeout, `start_kill()` (SIGKILL) + another wait.
  - Unit tests: `kill_on_drop` via `sleep 60`; `stop_graceful` escalation via a shell subprocess that traps SIGTERM (`bash -c 'trap "" TERM; sleep 30'`).
- `src/codex_client.rs`: `pub(crate) struct CodexClient` that owns one `tokio-tungstenite::WebSocketStream`. Public methods:
  - `async fn connect(url: &Url) -> Result<Self, MinosError>` with retry loop (15 × 200 ms, surface `CodexConnectFailed` on exhaustion).
  - `async fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, MinosError>` — implements JSON-RPC 2.0 framing with `uuid::Uuid::new_v4()` as request id; waits for the matching response in the WS stream while forwarding any interleaved notifications / server requests to a caller-provided callback channel.
  - `async fn next_inbound(&mut self) -> Option<Inbound>` where `Inbound` is `Notification { method, params } | ServerRequest { id, method, params } | Closed`.
  - `async fn reply(&mut self, id: serde_json::Value, result: serde_json::Value) -> Result<(), MinosError>` — used by the approval auto-reject path.
  - Unit tests use an in-memory `tokio::io::duplex()` pair with a hand-rolled peer sending canned frames.
- `src/runtime.rs`: the state-machine facade.

  ```rust
  pub struct AgentRuntime { inner: Arc<Inner> }

  struct Inner {
      cfg: AgentRuntimeConfig,
      state_tx: watch::Sender<AgentState>,
      state_rx: watch::Receiver<AgentState>,
      event_tx: broadcast::Sender<AgentEvent>,
      active: Mutex<Option<Active>>,
  }

  struct Active {
      process: CodexProcess,
      client: CodexClient,  // consumed by pump task; stored here only before spawn
      thread_id: String,
      started_at: SystemTime,
      pump_task: JoinHandle<()>,
      supervisor_task: JoinHandle<()>,
      expected_exit: Arc<AtomicBool>, // set by stop() so supervisor distinguishes
  }
  ```

  Implement the seven public methods listed in spec §5.1:
  - `new(cfg) -> Arc<Self>`: builds watch + broadcast channels (broadcast capacity = `cfg.event_buffer`, default 256), seeds `state_tx` with `AgentState::Idle`.
  - `async fn start(&self, agent: AgentName) -> Result<StartAgentOutcome, MinosError>`: runs the sequence from spec §5.1 (validate Idle, create workspace dir, port-probe in `cfg.ws_port_range`, spawn codex with the exact arg list from the spec, WS connect with retry, `initialize`, `thread/start`, broadcast Running). Any error rolls back by best-effort `child.start_kill()` and setting state back to Idle.
  - `async fn send_user_message(&self, session_id: &str, text: &str) -> Result<(), MinosError>`: validate Running state + session id matches, fire `turn/start`.
  - `async fn stop(&self) -> Result<(), MinosError>`: idempotent — Idle/Crashed return Ok(()) immediately; Running transitions Stopping → polite turn/interrupt + thread/archive (each ≤500ms) → `stop_graceful` → set `expected_exit`, state Idle.
  - `current_state()`, `state_stream()`, `event_stream()` are trivial reads.

  The `pump_task` reads inbound frames from `CodexClient` and:
  - Forwards notifications via `translate::translate_notification` → `event_tx.send(evt)`.
  - On `ServerRequest` with approval-method name, builds `approvals::build_auto_reject`, sends it back via `CodexClient::reply`, AND also broadcasts an `AgentEvent::Raw { kind: format!("server_request/{method}"), payload_json: serde_json::to_string(&params).unwrap_or_default() }`.
  - On `Closed`, exits; the supervisor_task is authoritative for state transitions.

  The `supervisor_task` awaits `child.wait()`; on exit, reads `expected_exit`:
  - `true` → state_tx.send(Idle).
  - `false` → state_tx.send(Crashed { reason }). `reason` = `"exit code N"` (Unix exit status) or `"signal <NAME>"` (decoded from `os::unix::process::ExitStatusExt::signal()`).

- `tests/runtime_e2e.rs`: three integration tests driving `AgentRuntime` against `FakeCodexServer`:
  1. Happy path: `start(Codex)` → expect state Starting then Running with correct thread_id; `send_user_message` → fake observes `turn/start`; fake emits `item/agentMessage/delta` with `{"delta": "Hello"}` → subscriber receives `AgentEvent::TokenChunk { text: "Hello" }`; `stop()` → state Idle.
  2. Approval auto-reject: fake emits a `ServerRequest` with method `ExecCommandApproval` → runtime replies with `{"decision":"rejected"}` AND broadcasts `AgentEvent::Raw { kind: "server_request/ExecCommandApproval", .. }`.
  3. Crash detection: fake executes `DieUnexpectedly` after `thread/start` → state transitions to `Crashed { reason: "signal SIGKILL" }` (or whatever the fake's close triggers). Subscribers observe the transition.

  Use `AgentRuntimeConfig` with a fake-port injection seam: `codex_bin = Some(PathBuf::from("/bin/sleep"))` (never actually invoked because tests should override the WS URL — see next bullet) OR add a test-only config field `test_ws_url: Option<Url>` that, when Some, skips subprocess spawn entirely and connects directly to the fake. The latter is cleaner — implement it:
  ```rust
  pub struct AgentRuntimeConfig {
      pub workspace_root: PathBuf,
      pub codex_bin: Option<PathBuf>,
      pub ws_port_range: std::ops::RangeInclusive<u16>,
      pub event_buffer: usize,
      /// Test-only seam: when Some, skip subprocess spawn and connect here
      /// directly. Production code must leave this as None.
      #[cfg(feature = "test-support")]
      pub test_ws_url: Option<url::Url>,
  }
  ```

**Preserved constraints:**
- `AgentRuntime::stop()` is idempotent (Idle/Crashed → Ok).
- `start()` returns `AgentAlreadyRunning` when state ≠ Idle.
- `send_user_message()` with a stale `session_id` returns `AgentSessionIdMismatch` — NOT `AgentNotRunning`.
- Approval auto-reject is unconditional for the 5 listed approval method names; any other ServerRequest method is logged at `warn!` and broadcast as a `Raw` event without a reply (codex will timeout the request; that's acceptable given codex's `approval_policy=never` means the request should never arrive at all).
- `event_tx` capacity 256; lagged subscribers log `warn!` but are not disconnected.

**Files touched:**
- `crates/minos-agent-runtime/src/{process,codex_client,runtime,lib}.rs`
- `crates/minos-agent-runtime/tests/runtime_e2e.rs` (new)

**Validation:**
```bash
cargo fmt --check
cargo clippy -p minos-agent-runtime --all-targets --all-features -- -D warnings
cargo test -p minos-agent-runtime --all-features
cargo xtask check-all
```

**Commit boundary:**
```bash
git add crates/minos-agent-runtime
git commit -m "feat(agent-runtime): process supervisor + WS client + runtime state machine"
```

---

## Phase D · `minos-daemon` integration

**Goal:** Wire `AgentRuntime` into `DaemonHandle`, implement the three new RPC methods and the real `subscribe_events`, add the Agent state observer FFI pattern, and prove it with an in-process integration test against `FakeCodexServer`.

**Scope:**

- `crates/minos-daemon/Cargo.toml`: add
  ```toml
  minos-agent-runtime = { path = "../minos-agent-runtime" }
  ```
  under `[dependencies]`. Add
  ```toml
  minos-agent-runtime = { path = "../minos-agent-runtime", features = ["test-support"] }
  ```
  under `[dev-dependencies]`.
- `crates/minos-daemon/src/paths.rs` (new): promote `default_minos_home()` from `main.rs`:
  ```rust
  use std::path::PathBuf;
  use minos_domain::MinosError;

  pub fn minos_home() -> Result<PathBuf, MinosError> {
      if let Ok(p) = std::env::var("MINOS_HOME") {
          return Ok(PathBuf::from(p));
      }
      let home = std::env::var("HOME").map_err(|_| MinosError::StoreIo {
          path: "$HOME".into(),
          message: "HOME env var not set".into(),
      })?;
      Ok(PathBuf::from(home).join(".minos"))
  }
  ```
  On macOS GUI, we still honor `$MINOS_HOME`; if unset, fall back to `$HOME/.minos` (the existing daemon behavior for CLI is preserved; GUI deliberately uses the same path for user-visible workspaces).
- `crates/minos-daemon/src/main.rs`: delete the local `default_minos_home` helper; replace the one call-site with `paths::minos_home()`.
- `crates/minos-daemon/src/subscription.rs`: add
  ```rust
  use minos_agent_runtime::AgentState;

  #[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
  pub trait AgentStateObserver: Send + Sync {
      fn on_state(&self, state: AgentState);
  }

  pub(crate) fn spawn_agent_observer(
      mut rx: tokio::sync::watch::Receiver<AgentState>,
      observer: std::sync::Arc<dyn AgentStateObserver>,
  ) -> std::sync::Arc<Subscription> {
      observer.on_state(rx.borrow().clone());
      let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
      tokio::spawn(async move {
          loop {
              tokio::select! {
                  biased;
                  _ = &mut cancel_rx => break,
                  r = rx.changed() => {
                      if r.is_err() { break; }
                      observer.on_state(rx.borrow().clone());
                  }
              }
          }
      });
      std::sync::Arc::new(Subscription::new(cancel_tx))
  }
  ```
  Two tests mirroring the existing `observer_receives_initial_and_subsequent_states` and `cancel_is_idempotent`.
- `crates/minos-daemon/src/agent.rs` (new):
  ```rust
  use std::path::PathBuf;
  use std::sync::Arc;
  use minos_agent_runtime::{AgentRuntime, AgentRuntimeConfig, AgentState, StartAgentOutcome};
  use minos_domain::{AgentEvent, AgentName, MinosError};
  use minos_protocol::{
      SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
  };
  use tokio::sync::{broadcast, watch};

  pub(crate) struct AgentGlue { runtime: Arc<AgentRuntime> }

  impl AgentGlue {
      pub(crate) fn new(workspace_root: PathBuf) -> Self {
          let runtime = AgentRuntime::new(AgentRuntimeConfig {
              workspace_root,
              codex_bin: None,
              ws_port_range: 7879..=7883,
              event_buffer: 256,
              #[cfg(feature = "test-support")]
              test_ws_url: None,
          });
          Self { runtime }
      }

      #[cfg(feature = "test-support")]
      pub(crate) fn new_with_test_ws(workspace_root: PathBuf, url: url::Url) -> Self {
          let runtime = AgentRuntime::new(AgentRuntimeConfig {
              workspace_root,
              codex_bin: None,
              ws_port_range: 7879..=7883,
              event_buffer: 256,
              test_ws_url: Some(url),
          });
          Self { runtime }
      }

      pub(crate) async fn start(&self, req: StartAgentRequest) -> Result<StartAgentResponse, MinosError> {
          if !matches!(req.agent, AgentName::Codex) {
              return Err(MinosError::AgentNotSupported { agent: req.agent });
          }
          let out = self.runtime.start(req.agent).await?;
          Ok(StartAgentResponse { session_id: out.session_id, cwd: out.cwd })
      }

      pub(crate) async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
          self.runtime.send_user_message(&req.session_id, &req.text).await
      }

      pub(crate) async fn stop(&self) -> Result<(), MinosError> { self.runtime.stop().await }

      pub(crate) fn current_state(&self) -> AgentState { self.runtime.current_state() }
      pub(crate) fn state_stream(&self) -> watch::Receiver<AgentState> { self.runtime.state_stream() }
      pub(crate) fn event_stream(&self) -> broadcast::Receiver<AgentEvent> { self.runtime.event_stream() }

      pub(crate) async fn shutdown(&self) -> Result<(), MinosError> { self.runtime.stop().await }
  }
  ```
- `crates/minos-daemon/src/lib.rs`:
  ```rust
  pub mod agent;
  pub mod paths;
  pub use agent::AgentGlue;
  pub use minos_agent_runtime::AgentState;
  pub use subscription::AgentStateObserver;
  ```
- `crates/minos-daemon/src/handle.rs`: inside `DaemonInner`, add `agent: Arc<AgentGlue>`. In `start_on_port_range` (or wherever `DaemonInner` is constructed), build it with `AgentGlue::new(paths::minos_home()?.join("workspaces"))`. Create the workspace dir lazily — not at daemon boot — so tests without a disk don't fail; `AgentRuntime::start` will `create_dir_all` when invoked.

  Append five methods to `impl DaemonHandle` (behind `#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]`):
  ```rust
  pub async fn start_agent(
      &self,
      req: StartAgentRequest,
  ) -> Result<StartAgentResponse, MinosError> {
      self.inner.agent.start(req).await
  }

  pub async fn send_user_message(
      &self,
      req: SendUserMessageRequest,
  ) -> Result<(), MinosError> {
      self.inner.agent.send_user_message(req).await
  }

  pub async fn stop_agent(&self) -> Result<(), MinosError> {
      self.inner.agent.stop().await
  }

  pub fn subscribe_agent_state(
      &self,
      observer: Arc<dyn AgentStateObserver>,
  ) -> Arc<Subscription> {
      crate::subscription::spawn_agent_observer(
          self.inner.agent.state_stream(),
          observer,
      )
  }

  pub fn current_agent_state(&self) -> AgentState {
      self.inner.agent.current_state()
  }
  ```
  Modify `DaemonHandle::stop(&self)` to first `self.inner.agent.shutdown().await?` before the existing server shutdown, so a running codex child is killed cleanly on app quit.

- `crates/minos-daemon/src/rpc_server.rs`:
  - Add the three new methods as thin delegates to `self.agent.<method>`. `RpcServerImpl` already holds an `Arc<AgentGlue>` — inject via its constructor.
  - Rewrite `subscribe_events`:
    ```rust
    async fn subscribe_events(
        &self,
        pending: jsonrpsee::server::PendingSubscriptionSink,
    ) -> jsonrpsee::core::SubscriptionResult {
        let mut rx = self.agent.event_stream();
        let sink = pending.accept().await?;
        loop {
            match rx.recv().await {
                Ok(evt) => {
                    let msg = jsonrpsee::server::SubscriptionMessage::from_json(&evt)?;
                    if sink.send(msg).await.is_err() { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "subscribe_events subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        Ok(())
    }
    ```

- `crates/minos-daemon/tests/agent_e2e.rs` (new): in-process end-to-end harness that matches spec §8.2:
  1. Start `FakeCodexServer` with a script emulating an initialize → thread/start → accept a turn/start → emit a TokenChunk → accept a turn/interrupt → close.
  2. Build a `DaemonHandle` via `start_autobind` with a test-only config pointing `AgentGlue` at `AgentGlue::new_with_test_ws(tmpdir, fake_url)`.
  3. Subscribe to `AgentState` + `AgentEvent` streams (using a local observer type).
  4. Call `handle.start_agent(StartAgentRequest { agent: AgentName::Codex })` → assert `AgentState::Running { thread_id, .. }`.
  5. Call `handle.send_user_message(...)` with matching session id → assert fake observes the `turn/start`.
  6. After fake emits the delta, assert the broadcast subscriber sees `AgentEvent::TokenChunk { text: "Hello" }`.
  7. Call `handle.stop_agent()` → assert `AgentState::Idle`.
  8. Call `handle.stop()` → clean teardown.

  Require a DaemonHandle test constructor that takes an injected `AgentGlue` — add it behind `#[cfg(test)]` or behind the `test-support` feature that's already on `minos-agent-runtime`. Prefer the feature-gated path so the glue stays in the production module tree and tests don't touch private fields.

**Preserved constraints:**
- `DaemonHandle::stop` still takes `&self` (plan 02 contract).
- `subscribe_events` signature on the trait is unchanged; only its implementation in `RpcServerImpl` is rewritten.
- No regressions in plan-02 tests — run the existing suite on every change.
- `FilePairingStore`, `WsServer`, `start_autobind` unchanged.

**Files touched:**
- `crates/minos-daemon/Cargo.toml`
- `crates/minos-daemon/src/{lib,main,handle,rpc_server,subscription,agent,paths}.rs`
- `crates/minos-daemon/tests/agent_e2e.rs` (new)

**Validation:**
```bash
cargo fmt --check
cargo clippy -p minos-daemon --all-targets --all-features -- -D warnings
cargo test -p minos-daemon --all-features
cargo xtask check-all
```

**Commit boundary:**
```bash
git add crates/minos-daemon
git commit -m "feat(daemon): AgentGlue + 3 new RPC methods + real subscribe_events"
```

---

## Phase E · UniFFI + macOS menubar

**Goal:** Make the new daemon surface reachable from Swift; add the `AgentSegmentView`, `AppState` plumbing, and `AgentStateTests` logic-only coverage. No chat UI; debug buttons are `#if DEBUG`.

**Dispatch:** Use the `frontend-design` specialist for this phase — SwiftUI layout decisions (mounting order, padding, timeline-driven uptime) benefit from specialized attention.

**Scope:**

- `crates/minos-ffi-uniffi/src/lib.rs`:
  - Enable `uniffi` feature on the `minos-agent-runtime` path dep (it already has the feature gate on `AgentStateObserver` in `subscription.rs`).
  - Add `#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]` to `AgentState` in `minos-agent-runtime/src/state.rs` (late-add belonging to Phase E's gate).
  - `AgentState::Running` carries `started_at: SystemTime`. UniFFI 0.31 ships built-in support for `std::time::SystemTime` → Swift `Date`, so no custom-type declaration is needed. If the build errors with "SystemTime not a supported type", fall back to the `DateTime<Utc>` → `SystemTime` pattern plan 02 used for `TrustedDevice.paired_at` — check `minos-ffi-uniffi/src/lib.rs` for the existing block.
  - `StartAgentRequest`, `StartAgentResponse`, `SendUserMessageRequest` get `#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]` in `minos-protocol/src/messages.rs` (late-add; plan 02 has the same feature-gate pattern in `minos-domain`, extend to `minos-protocol` with a new `uniffi` feature flag on its Cargo.toml).
  - `minos-ffi-uniffi/Cargo.toml` gains `minos-protocol = { path = "../minos-protocol", features = ["uniffi"] }` and `minos-agent-runtime = { path = "../minos-agent-runtime", features = ["uniffi"] }` (add a `uniffi` feature to both crates that re-exports the necessary deps).

- `cargo xtask gen-uniffi` regenerates `apps/macos/Minos/Generated/`. The grep smoke in xtask should catch the new types: `public enum AgentState`, `public protocol AgentStateObserver`, `public struct StartAgentRequest`. Extend the grep list accordingly in `xtask/src/main.rs`.

- Swift app additions:
  - `apps/macos/Minos/Application/AgentStateObserverAdapter.swift` (new): mirrors `ConnectionStateObserverAdapter`:
    ```swift
    final class AgentStateObserverAdapter: AgentStateObserver {
        let onUpdate: (AgentState) -> Void
        init(onUpdate: @escaping (AgentState) -> Void) { self.onUpdate = onUpdate }
        func onState(state: AgentState) { onUpdate(state) }
    }
    ```
  - `apps/macos/Minos/Application/DaemonDriving.swift`: add 5 methods to the protocol (mirror §5.7 of the spec).
  - `apps/macos/Minos/Infrastructure/DaemonHandle+DaemonDriving.swift`: extension forwarding the five new methods to the generated `DaemonHandle`.
  - `apps/macos/Minos/Application/AppState.swift`: add `agentState: AgentState = .idle`, `agentError: MinosError? = nil`, `currentSession: StartAgentResponse? = nil`, `agentSubscription: Subscription? = nil`. Add four `async` methods (`startAgent`, `sendAgentPing`, `stopAgent`, `dismissAgentCrash`). Wire `shutdown` to also cancel `agentSubscription`.
  - `apps/macos/Minos/Infrastructure/DaemonBootstrap.swift`: after the existing connection-observer wiring, add parallel agent-state wiring.
  - `apps/macos/Minos/Presentation/AgentSegmentView.swift` (new): the view described in spec §5.7 — `@Bindable var appState: AppState`; branches on `agentState`; `#if DEBUG` exposes buttons. Uses `TimelineView(.periodic(from: startedAt, by: 1))` for the uptime counter.
  - `apps/macos/Minos/Presentation/MenuBarView.swift`: insert `AgentSegmentView(appState: appState)` between the pairing block and the "显示今日日志 / 退出" tail, in both `unpairedContent` and `pairedContent` branches. Do not mount in `bootingContent` or `bootErrorContent`.

- `apps/macos/MinosTests/TestSupport/MockDaemon.swift`: implement the five new methods; record call counts + captured arguments for assertions.
- `apps/macos/MinosTests/Application/AgentStateTests.swift` (new): seven scenarios from spec §8.3. `MockDaemon` is instantiated fresh per test; Swift Testing (or XCTest, matching existing style) asserts on recorded state / captured arguments.

**Preserved constraints:**
- Release-build menubar never offers agent-control buttons. Enforced by `#if DEBUG` guards in `AgentSegmentView.swift` AND by the absence of any release-facing caller of `AppState.startAgent()` (only `AgentSegmentView`'s debug branch calls it).
- `AgentSegmentView` is the **only** Swift file allowed to introduce a new branch on `appState.agentState`; other views must remain oblivious to agent state.
- `MockDaemon` continues to implement `DaemonDriving` without importing UniFFI-generated types — tests use the plan-02 protocol seam.
- Logic-only test rule: no UI tests, no Preview snapshots, no XCUITest.

**Files touched:**
- `crates/minos-agent-runtime/{src/state.rs, Cargo.toml}` (add uniffi feature)
- `crates/minos-protocol/{src/messages.rs, Cargo.toml}` (add uniffi feature, derive Record)
- `crates/minos-ffi-uniffi/{src/lib.rs, Cargo.toml}`
- `apps/macos/Minos/Generated/*` (regenerated; gitignored)
- `apps/macos/Minos/Application/{AppState, DaemonDriving, AgentStateObserverAdapter}.swift`
- `apps/macos/Minos/Infrastructure/{DaemonBootstrap, DaemonHandle+DaemonDriving}.swift`
- `apps/macos/Minos/Presentation/{MenuBarView, AgentSegmentView}.swift`
- `apps/macos/MinosTests/TestSupport/MockDaemon.swift`
- `apps/macos/MinosTests/Application/AgentStateTests.swift`
- `xtask/src/main.rs` (extend gen-uniffi grep smoke)

**Validation:**
```bash
cargo xtask gen-uniffi
cargo xtask gen-xcode
cargo xtask check-all    # includes xcodebuild build + MinosTests
```
On the maintainer's workstation with a debug build, click through the debug buttons and confirm:
- "启动 Codex（测试）" → menubar shows `Running · thread <id> · 0s` within a few seconds.
- "发送 ping（测试）" → `daemon_YYYYMMDD.xlog` shows a `token_chunk` with codex's response.
- "停止 Codex" → menubar reverts to Idle; `ps | grep codex` shows no lingering child.

**Commit boundary:**
```bash
git add crates/minos-agent-runtime crates/minos-protocol crates/minos-ffi-uniffi apps/macos xtask
git commit -m "feat(macos): agent segment view + AgentStateObserver + logic tests"
```

---

## Phase F · frb regen + xtask opt-in smoke + ADRs + README landing

**Goal:** Regenerate Dart bindings, add the opt-in real-codex smoke, write the two ADRs, close out README.

**Scope:**

- `cargo xtask gen-frb`: regenerates `apps/mobile/lib/src/rust/`. Expected diff: `AgentEvent.raw(kind, payload_json)` arm on the Dart enum; `MinosRpcClient.start_agent(...)`, `send_user_message(...)`, `stop_agent(...)` methods on the generated client trait. `frb_drift_guard` in CI enforces a clean diff.
- `xtask/src/main.rs`: add a `codex_smoke_leg` that is invoked only when `MINOS_XTASK_WITH_CODEX=1` is set (or a new `--with-codex` flag is passed). The leg:
  1. Checks `which codex` → skips with a clear log if absent (same pattern as `flutter_leg`'s fvm check).
  2. Starts a fresh tokio runtime in the xtask process.
  3. Uses `minos_agent_runtime::AgentRuntime::new(...)` directly (add the crate as an xtask dep) to `start(Codex)` under a tempdir workspace.
  4. Calls `send_user_message(session_id, "reply with the word ok")` and spawns a subscriber on `event_stream()`.
  5. Waits up to 60 seconds for a `TokenChunk` whose `text` contains `ok` (case-insensitive).
  6. Calls `stop()`.
  7. Any failure exits nonzero with a clear error.

  The leg is appended to `check-all` **only** when the flag/env is present; the default lane does nothing.
- `docs/adr/0009-codex-app-server-ws-transport.md`: MADR 4.0 — Context (why codex, pairing with IDE-integration model), Decision (WebSocket loopback), Consequences (one extra port, explicit child supervision), Alternatives Rejected (stdio pipes — rejected for debuggability + alignment with codex's own examples).
- `docs/adr/0010-agent-event-raw-variant.md`: MADR 4.0 — Context (`AgentEvent` needs to survive codex protocol churn), Decision (single `Raw` variant as escape hatch), Consequences (mobile consumers need a no-op fallback), Alternatives Rejected (expanding `AgentEvent` per codex release — tight coupling; rewriting `AgentEvent` — breaks mobile bindings).
- `README.md`: update the "Status" / "Roadmap" block to reflect Plan 04 landed; add a bullet about debug-build menubar agent controls.

**Preserved constraints:**
- Do not add `codex` install steps to CI.
- `xtask codex_smoke_leg` must NOT run by default — only when the env var or `--with-codex` flag is set.
- ADRs follow the exact section structure used by 0001–0008.

**Files touched:**
- `apps/mobile/lib/src/rust/` (regenerated)
- `xtask/src/main.rs`, `xtask/Cargo.toml` (dep on minos-agent-runtime)
- `docs/adr/0009-codex-app-server-ws-transport.md` (new)
- `docs/adr/0010-agent-event-raw-variant.md` (new)
- `README.md`

**Validation:**
```bash
cargo xtask gen-frb
git diff --exit-code -- apps/mobile/lib/src/rust crates/minos-ffi-frb/src/frb_generated.rs
cargo xtask check-all
# Optional, on maintainer workstation with codex installed:
MINOS_XTASK_WITH_CODEX=1 cargo xtask check-all
```

**Commit boundary:**
```bash
git add apps/mobile/lib/src/rust xtask docs/adr README.md
git commit -m "feat(xtask,docs): frb regen + codex-smoke opt-in + ADRs 0009/0010"
```

---

## Phase G · Plan close-out

**Goal:** Final sanity pass; no new code. One small commit bumping the plan status annotations.

**Scope:**

- Verify every item in spec §8.6 Done criteria:
  1. `cargo xtask check-all` green on a fresh `cargo clean` workspace — ✅
  2. `cargo test -p minos-agent-runtime --all-features` ≥ 12 tests — ✅
  3. `cargo test -p minos-daemon --test agent_e2e` green — ✅
  4. Swift `AgentStateTests` green; `AppStateTests` still green — ✅
  5. Manual smoke on workstation debug build: start / ping / stop → log shows `token_chunk` → no zombie codex — ✅
  6. `MINOS_XTASK_WITH_CODEX=1 cargo xtask check-all` green locally — ✅
- Update the plan header "Status" / README status to indicate Plan 04 landed.

**Files touched:**
- `docs/superpowers/plans/04-codex-app-server-integration.md` (status annotation)
- `README.md` (if Phase F didn't fully cover status)

**Validation:**
```bash
cargo xtask check-all
```

**Commit boundary:**
```bash
git add docs/superpowers/plans/04-codex-app-server-integration.md README.md
git commit -m "docs: mark plan 04 landed"
```

---

## Plan self-review

A quick check against the spec before handoff.

**Spec coverage:**

| Spec section | Plan phase covering it |
|---|---|
| §2.1 #1 new crate `minos-agent-runtime` | Phase B + Phase C |
| §2.1 #2 real `subscribe_events` | Phase D (rpc_server.rs rewrite) |
| §2.1 #3 three new RPC methods | Phase A (declaration) + Phase D (implementation) |
| §2.1 #4 UniFFI agent surface | Phase E |
| §2.1 #5 macOS menubar Agent segment | Phase E (AgentSegmentView) |
| §2.1 #6 `AgentEvent::Raw` variant | Phase A (declaration) + Phase F (frb regen) |
| §2.1 #7 `$MINOS_HOME/workspaces` convention | Phase D (`paths::minos_home()` promotion) |
| §2.1 #8 codex config flags + approval auto-reject | Phase C (start sequence + approvals module) |
| §2.1 #9 `minos-agent-runtime` unit tests | Phase B + Phase C |
| §2.1 #10 daemon integration test | Phase D (agent_e2e.rs) |
| §2.1 #11 Swift unit tests | Phase E (AgentStateTests.swift) |
| §2.1 #12 ADRs 0009 + 0010 | Phase F |
| §7 error mapping (7 new variants) | Phase A (declaration) + Phase E (Swift `MinosError.kind` switch update) |
| §5.7 UI layout (3 `AgentState` renderings + release/debug split) | Phase E |

No gaps.

**Placeholder scan:** No TBD / TODO / "implement later" / "add error handling" strings remain.

**Type consistency:**
- `AgentState` referenced identically across Phase B/C/D/E.
- `StartAgentRequest { agent: AgentName }` → `StartAgentResponse { session_id: String, cwd: String }` — consistent spec §5.2 and Phase A/D/E.
- `AgentGlue::new(workspace_root: PathBuf)` (Phase D) vs. `AgentRuntimeConfig { workspace_root: PathBuf, ... }` (Phase C) — consistent.
- `AgentStateObserver::on_state(&self, state: AgentState)` Phase D → Swift `func onState(state: AgentState)` Phase E — matches UniFFI's naming transform.

No inconsistencies found.

---

## Execution options

**Plan complete and saved to `docs/superpowers/plans/04-codex-app-server-integration.md`. Two execution options:**

1. **Subagent-Driven (recommended)** — dispatch one subagent per phase, review between phases, fast iteration. Matches the existing plan-03 execution model.
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints for review.

Which approach?
