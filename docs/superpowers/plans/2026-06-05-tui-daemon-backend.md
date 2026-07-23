# Rework TUI Local-Daemon Backend Integration

## Feasibility Assessment

当前代码已经具备这次改造的核心拼图：`minos-tui` 已经把运行时依赖收敛到 `AgentBackend`（`crates/minos-tui/src/backend/mod.rs`），`minos-daemon` 已经有可复用的 `AgentGlue` / `RpcServerImpl` / `DaemonHandle::start` 组合（`crates/minos-daemon/src/agent.rs`, `crates/minos-daemon/src/rpc_server.rs`, `crates/minos-daemon/src/handle.rs`），而 `jsonrpsee` 也已经在 workspace 中启用 client/server/macros。真正的难点不在“能不能连上 daemon”，而在“不要把现有 shared RPC 面污染掉”和“不要假设 translated history 足够恢复 TUI 状态”。因此这项工作对 live daemon-backed 会话完全可行；对 persisted session attach 也可行，但必须单独分 phase 处理 raw-ingest replay。Feasible with caveats.

## Current Surface Inventory

- `crates/minos-tui/src/backend/mod.rs` (lines 1-29) -- `AgentBackend` 目前是 TUI 唯一后端抽象，返回的都是 runtime 层类型。
- `crates/minos-tui/src/main.rs` (lines 20-99) -- CLI 目前只会构造 `EmbeddedBackend`，没有运行时 backend 选择。
- `crates/minos-tui/src/app.rs` (lines 17-176) -- `App<B>` 依赖 `ManagerEvent` 和 `RawIngest` live 推进 UI，本身不是“从 snapshot 恢复状态”的客户端。
- `crates/minos-tui/src/ui/status_bar.rs` -- status bar 只展示 CLI 检测结果，没有 backend 连接态。
- `crates/minos-daemon/src/handle.rs` (lines 73-177) -- `DaemonHandle::start` 负责拼装 `AgentGlue`、`RpcServerImpl` 和 relay client。
- `crates/minos-daemon/src/rpc_server.rs` (lines 1-123) -- 当前 `RpcServerImpl` 实现的是 shared `MinosRpcServer`，用于 relay forwarded host commands，不再是本地监听器。
- `crates/minos-daemon/src/agent.rs` (lines 168-420) -- 已有 `start_agent`、`resume_session`、`ensure_thread_registered`、`read_session_history`、`hydrate_translator` 等能力。
- `crates/minos-protocol/src/rpc.rs` (lines 1-96) -- `MinosRpc` 是 shared host/mobile JSON-RPC 面，不应该承载 TUI local-only streaming 需求。
- `crates/minos-protocol/src/messages.rs` (lines 595-727) -- 现有 `ThreadState` 可复用，但 `SessionSummary` / `GetSessionResponse` / `ReadSessionResponse` 是 mobile/history 导向，不匹配 TUI attach 需求。

## Design

Addressing review feedback:

- 不修改 shared `MinosRpc` 来承载 TUI local-only streaming。
- 不把默认 endpoint 固定成 `ws://127.0.0.1:9123`。
- 不再使用 `test|production` 命名；这里切换的是 backend 实现，不是环境。
- 不使用会丢 `ts_ms` / `at_ms` / `workspace` 的 lossy event mirror。
- 不再假设 translated history 足够恢复 TUI translator state。

### Key design decisions

1. 为 TUI 新增独立的 local-only RPC 面。
   Choice: 在 `minos-protocol` 新增 `local_rpc` module 和 `LocalDaemonRpc` trait。
   Rejected: 直接扩展 `crates/minos-protocol/src/rpc.rs` 中的 `MinosRpc`。
   Why: `MinosRpc` 现在是 shared host/mobile contract，`RpcServerImpl` 也被 `invoke_host_command` 复用；把 TUI 专用 subscription 混进去会污染 relay forwarded surface。

2. CLI 参数改成 `--backend embedded|daemon`。
   Choice: 使用实现语义命名。
   Rejected: `--mode test|production`。
   Why: 当前代码切换的是 `EmbeddedBackend` 与新 `DaemonBackend`；“test/production”掩盖真实行为，而且容易和 `AgentLaunchMode` 混淆。

3. local RPC 发布必须是 daemon startup 的显式能力，而不是 `DaemonHandle::start` 的隐式副作用。
   Choice: `DaemonHandle::start(..., local_rpc: Option<LocalRpcConfig>)` 或 builder/config 等价方案。
   Rejected: 在 `DaemonHandle::start` 内总是起一个本地 listener。
   Why: `status`、`pairing_qr`、`peers` 这类一次性 CLI 流程也会调用 `DaemonHandle::start`；它们不应该抢占 discovery file，也不应该暴露 TUI 控制面。

4. local RPC 用 loopback ephemeral port + discovery file。
   Choice: bind `127.0.0.1:0`，把实际 URL 写到 `~/.minos/run/tui-daemon-rpc.json`。
   Rejected: 固定监听 `127.0.0.1:9123`。
   Why: 固定端口会引入冲突、crash 后 stale endpoint、以及并行测试互相踩踏的问题。`--daemon-url` 只作为 override 保留。

5. Phase 1 只保证 live daemon-backed 会话；persisted session attach 单独进 Phase 4。
   Choice: 第一阶段支持 `detect_clis`、`start_agent`、`send_message`、`interrupt_session`、`close_session`、live ingest/manager events。
   Rejected: 在第一版里就承诺“接到现有 daemon 后自动列出并恢复全部 thread”。
   Why: 现有 protocol DTO 没有 workspace，也没有 raw ingest history；而 TUI 的 translator state 是 per-session stateful 的，不能靠 `UiEventMessage` 历史直接恢复。

6. 只要定义 wire mirror，就必须 lossless mirror 当前 runtime 语义。
   Choice: local RPC event DTO 保留 `RawIngest.ts_ms`、`ManagerEvent::SessionStateChanged.at_ms`、`InstanceCrashed.workspace`。
   Rejected: 原文那套省略字段或自造字段的 `Streaming*` 草案。
   Why: 这些字段一旦丢掉，后续 replay、debug 和状态对账都做不干净，而且当前 runtime 根本没有 `instance_id` 这个事件字段。

7. `App` 需要能够持有 runtime-selected backend，并把连接态暴露给 UI。
   Choice: `App` 改成持有 `Arc<dyn AgentBackend>`，或把泛型边界改为 `B: AgentBackend + ?Sized`，同时给 backend 增加 cheap connection snapshot。
   Rejected: 保持当前 `App<B: AgentBackend>` 的 sized 假设不变。
   Why: `main.rs` 一旦支持 runtime backend 选择，就不再只有一个具体类型；当前 `App<B>` 直接吃 `Arc<dyn AgentBackend>` 是编不过的。

### Concrete interfaces

File: `crates/minos-protocol/src/local_rpc.rs`

```rust
use jsonrpsee::proc_macros::rpc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalIngestFrame {
    pub session_id: String,
    pub agent: minos_domain::AgentName,
    pub payload: serde_json::Value,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocalManagerEvent {
    SessionAdded {
        session_id: String,
        workspace: String,
        agent: minos_domain::AgentName,
    },
    SessionStateChanged {
        session_id: String,
        old: crate::ThreadState,
        new: crate::ThreadState,
        at_ms: i64,
    },
    SessionClosed {
        session_id: String,
        reason: crate::CloseReason,
    },
    InstanceCrashed {
        workspace: String,
        affected_threads: Vec<String>,
    },
}

#[rpc(server, client, namespace = "minos_local")]
pub trait LocalDaemonRpc {
    #[method(name = "health")]
    async fn health(&self) -> jsonrpsee::core::RpcResult<crate::HealthResponse>;

    #[method(name = "list_clis")]
    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<crate::ListClisResponse>;

    #[method(name = "start_agent")]
    async fn start_agent(
        &self,
        req: crate::StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

    #[method(name = "send_user_message")]
    async fn send_user_message(
        &self,
        req: crate::SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "interrupt_session")]
    async fn interrupt_session(
        &self,
        req: crate::InterruptSessionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "close_session")]
    async fn close_session(
        &self,
        req: crate::CloseSessionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[subscription(name = "subscribe_ingest", item = LocalIngestFrame)]
    fn subscribe_ingest(&self);

    #[subscription(name = "subscribe_manager_events", item = LocalManagerEvent)]
    fn subscribe_manager_events(&self);
}
```

Phase 4 才扩展 persisted-thread attach 所需的接口：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSessionSnapshot {
    pub session_id: String,
    pub agent: minos_domain::AgentName,
    pub workspace: String,
    pub state: crate::ThreadState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadSessionRawHistoryResponse {
    pub events: Vec<LocalIngestFrame>,
    pub next_seq: Option<u64>,
}

#[method(name = "resume_session")]
async fn resume_session(
    &self,
    req: crate::GetSessionParams,
) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

#[method(name = "list_local_sessions")]
async fn list_local_sessions(&self) -> jsonrpsee::core::RpcResult<Vec<LocalSessionSnapshot>>;

#[method(name = "read_session_raw_history")]
async fn read_session_raw_history(
    &self,
    req: crate::ReadSessionParams,
) -> jsonrpsee::core::RpcResult<ReadSessionRawHistoryResponse>;
```

File: `crates/minos-tui/src/backend/mod.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendConnectionState {
    Embedded,
    Connected {
        endpoint: String,
    },
    Disconnected {
        endpoint: String,
        last_error: Option<String>,
    },
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>>;
    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome>;
    async fn send_message(&self, session_id: &str, text: &str) -> Result<()>;
    async fn interrupt_session(&self, session_id: &str) -> Result<()>;
    async fn close_session(&self, session_id: &str) -> Result<()>;
    async fn list_sessions(&self) -> Result<Vec<minos_agent_runtime::store_facing::ThreadSnapshot>>;
    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest>;
    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent>;
    fn connection_state(&self) -> BackendConnectionState;
}
```

File: `crates/minos-tui/src/main.rs`

```rust
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default)]
enum BackendKind {
    #[default]
    Embedded,
    Daemon,
}
```

Usage:

- `minos-daemon start --local-rpc`
- `minos-tui --backend daemon`
- `minos-tui --backend daemon --daemon-url ws://127.0.0.1:43123`

## Phased Implementation

## Phase 1: Add a local-only daemon control plane

**File: `crates/minos-protocol/src/local_rpc.rs`**

- Add the dedicated local RPC trait and the live-stream DTOs.
- Keep `LocalIngestFrame` and `LocalManagerEvent` lossless mirrors of runtime semantics.
- Do not modify `crates/minos-protocol/src/rpc.rs`.

**File: `crates/minos-protocol/src/lib.rs`**

- Export `local_rpc`.
- Leave the shared `rpc` and `messages` re-exports untouched.

**File: `crates/minos-daemon/src/local_rpc.rs`**

- Add a `jsonrpsee` server bootstrapper for the local TUI control plane.
- Bind to loopback only.
- Publish the resolved endpoint into a discovery file under `paths::run_dir()`.
- Own the subscription fanout for `AgentGlue::ingest_stream()` and `AgentManager::manager_event_stream()`.

**File: `crates/minos-daemon/src/lib.rs`**

- Export the local-RPC bootstrap/config types needed by CLI or app callers.

Rationale: 先把 TUI-facing transport 单独切出来，不污染 relay/mobile 已有 contract。

## Phase 2: Wire local RPC into the long-running daemon lifecycle

**File: `crates/minos-daemon/src/handle.rs`**

- Extend `DaemonHandle::start` with an explicit local-RPC config instead of enabling a listener unconditionally.
- Store the local server handle and discovery-file path on the daemon so `stop()` can clean both up.
- Keep one-shot callers free to opt out.

**File: `crates/minos-daemon/src/main.rs`**

- Add `StartArgs` switches for local RPC publication:
  - `--local-rpc` to enable publishing from `start`
  - optional `--local-rpc-addr` for deterministic tests or manual override
- Do not enable local RPC for ephemeral commands such as `status`, `pairing_qr`, `peers`, or `forget-peer`.
- Print the resolved endpoint or discovery-file path on startup when enabled.

**File: `crates/minos-daemon/src/paths.rs`**

- Reuse `run_dir()` for discovery-file placement.
- Add tiny helpers if cleanup needs to be centralized.

Rationale: local listener 属于长期运行的 daemon instance，不属于所有 `DaemonHandle` 调用方。

## Phase 3: Add daemon-backed live sessions to TUI

**File: `crates/minos-tui/src/backend/daemon.rs`**

- Implement `DaemonBackend` using the macro-generated `LocalDaemonRpcClient`.
- Start live subscription pumps inside `connect()` so callers拿到的是 ready backend，而不是一个还需要额外 `start_subscriptions()` 的半成品。
- Convert `LocalIngestFrame` / `LocalManagerEvent` into runtime `RawIngest` / `ManagerEvent`.
- Track current connection state and last transport error for the UI.
- Do not fabricate thread snapshots from `ListSessionsResponse`; that mapping is wrong and phase 3 does not need it.

**File: `crates/minos-tui/src/backend/mod.rs`**

- Export `DaemonBackend`.
- Add a cheap connection-state getter.
- Keep `list_sessions()` present, but treat it as live cache only until persisted-thread hydration lands.

**File: `crates/minos-tui/src/main.rs`**

- Replace the fixed `EmbeddedBackend` construction with backend selection.
- Validate CLI semantics:
  - `--max-instances` applies only to `embedded`
  - `--daemon-url` is an override for `daemon`
  - default daemon endpoint comes from the discovery file
- Keep `--agent` auto-start behavior working in both backends.

**File: `crates/minos-tui/src/app.rs`**

- Refactor `App` so it can hold `Arc<dyn AgentBackend>` at runtime, or relax the generic to `?Sized`.
- Refresh backend connection state during `Tick`.
- Surface disconnects to the UI instead of silently stopping ingest.
- Keep the current live-thread behavior unchanged for newly started sessions.

**File: `crates/minos-tui/src/ui/status_bar.rs`**

- Extend the status bar to show backend mode and connectivity, not only CLI detection.
- Example labels: `embedded`, `daemon connected`, `daemon disconnected`.

Rationale: 这一阶段只把 TUI 的运行时边界从 in-process 挪到 daemon 进程，不同时承诺“attach 任意旧线程”。

## Phase 4: Persisted-thread attach and history hydration

**File: `crates/minos-protocol/src/local_rpc.rs`**

- Add `resume_session`, `list_local_sessions`, and `read_session_raw_history`.
- Return raw ingest frames, not translated `UiEventMessage`s.

**File: `crates/minos-daemon/src/agent.rs`**

- Reuse the existing persisted-thread helpers:
  - `resume_session()` for explicit attach
  - store-backed history reads for raw event replay
- Do not reuse `read_session_history()` for TUI hydration; it returns translated UI events and cannot rebuild translator state.

**File: `crates/minos-tui/src/app.rs`**

- On daemon startup, optionally hydrate visible thread metadata from `list_local_sessions`.
- When the user selects or resumes a persisted session, replay raw history through the existing per-agent translators before live events continue.
- Lazily call `resume_session` before the first send/interrupt on a persisted session that is not currently registered in the daemon manager; the current `send_user_message` path does not call `ensure_thread_registered`.

**File: `crates/minos-tui/src/translation.rs`**

- Keep replay logic identical to live ingest handling so translator-state reconstruction stays deterministic.

Rationale: 只有这一阶段完成后，TUI 才真正支持“接管已经存在的 daemon thread”，而不是只能新开 daemon-backed 会话。

## Phase 5: Verification

**File: `crates/minos-daemon/tests/local_rpc.rs`**

- Add round-trip tests for `health`, `list_clis`, `start_agent`, `send_user_message`, and subscription delivery.
- Use `127.0.0.1:0` and read back the actual bound address; no fixed ports in tests.
- Run with `minos-agent-runtime` `test-support` so CI can exercise the live path without a manually started daemon process.

**File: `crates/minos-tui/tests/daemon_backend.rs`**

- Add an integration test that boots the local RPC server in-process, connects `DaemonBackend`, and asserts live ingest delivery.
- Add a regression test that disconnect updates backend connection state instead of silently hanging.

**Commands**

- `cargo check -p minos-protocol`
- `cargo test -p minos-daemon local_rpc`
- `cargo test -p minos-tui`
- `cargo run -p minos-daemon -- start --local-rpc`
- `cargo run -p minos-tui -- --backend embedded`
- `cargo run -p minos-tui -- --backend daemon`

Rationale: 这条链路必须能在 CI 里直接测，`#[ignore]` + “先手工起一个 daemon” 不是这个 surface 的合格验收方式。

## Architectural Notes

- Semver impact: 如果改动收敛在新的 `local_rpc` module，就不会破坏现有 `MinosRpc` shared contract。
- Object safety: `AgentBackend` 只要新增的是 non-generic cheap getter，就仍然是 object-safe。
- Not changed: relay/backend transport、forwarded host command dispatch、pairing、mobile-facing `MinosRpc` DTO 都不在本次变更范围。
- History caveat: `ReadSessionResponse.ui_events` 适合 `minos-daemon history` 文本输出，但不适合 TUI attach；TUI 需要 raw ingest replay 来重建 translator state。
- Dependency changes: `minos-tui` 增加 `minos-protocol` 和 `jsonrpsee`；不应该反向依赖 `minos-daemon`。
- Failure handling: missing discovery file、stale endpoint、mid-session disconnect 都必须是显式 UI-visible error，不能只表现成“没有线程”或“界面静默”。
- Security boundary: Phase 1 只绑定 loopback，并通过 `run_dir` 发布 discovery file；如果后续需要收紧同机同用户信任边界，再引入 local bearer token，而不是去扩展 shared `MinosRpc`。

## File Change Summary

- `crates/minos-daemon/src/handle.rs` -- make local TUI RPC publication explicit and lifecycle-managed.
- `crates/minos-daemon/src/lib.rs` -- export the local-RPC module/config surface.
- `crates/minos-daemon/src/local_rpc.rs` -- add the loopback local RPC server and live subscription fanout.
- `crates/minos-daemon/src/main.rs` -- publish local RPC only for the long-running `start` command and expose CLI flags.
- `crates/minos-protocol/src/lib.rs` -- export the new local-only RPC module.
- `crates/minos-protocol/src/local_rpc.rs` -- define the TUI-facing local daemon control plane and its wire DTOs.
- `crates/minos-tui/src/app.rs` -- allow runtime backend selection and surface backend connectivity.
- `crates/minos-tui/src/backend/daemon.rs` -- implement the daemon-backed `AgentBackend`.
- `crates/minos-tui/src/backend/mod.rs` -- export `DaemonBackend` and define backend connectivity state.
- `crates/minos-tui/src/main.rs` -- add `embedded|daemon` backend selection and validate backend-specific flags.
- `crates/minos-tui/src/ui/status_bar.rs` -- show backend mode and connectivity alongside CLI detection.
- `crates/minos-daemon/tests/local_rpc.rs` -- cover local control-plane RPC and subscription behavior.
- `crates/minos-tui/tests/daemon_backend.rs` -- cover daemon-backed TUI transport and disconnect behavior.
