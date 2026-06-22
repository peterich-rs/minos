# Host 守护进程 (minos-daemon) 架构文档

> 本文档详细描述 `minos-daemon` crate 的架构、模块划分和关键逻辑。

## 概述

`minos-daemon` 是运行在 Mac（或未来其他平台）上的守护进程，负责：连接后端 relay、管理本地 AI agent 子进程、处理配对、持久化本地状态。它是 macOS 应用的 Rust 核心，也通过 JSON-RPC 与 TUI 通信。

**源码路径**: `crates/minos-daemon/`

## CLI 子命令 (`src/main.rs`)

| 子命令 | 描述 |
|--------|------|
| `doctor` | 打印解析路径、设备 ID、relay URL |
| `list-clis` | 检测本地安装的 CLI agent |
| `host-skills` | 列出/管理 host skills |
| `status` | 连接 relay，打印状态快照 |
| `pairing-qr` | 连接 relay，打印配对 QR JSON |
| `peers` | 列出已配对设备 |
| `forget-peer` | 忘记配对设备 |
| `threads` | 列出持久化线程 |
| `thread <id>` | 查看线程详情 |
| `start` | 启动守护进程（长驻） |

### 启动序列（`Start` 命令）

1. 初始化 logging（mars-xlog）
2. 解析路径，加载 `LocalState`（或用新 DeviceId 初始化）
3. 可选配置本地 JSON-RPC 服务器（给 TUI 用）
4. 调用 `DaemonHandle::start_with_local_rpc()`
5. 可选打印配对 QR
6. 阻塞等待 SIGINT/SIGTERM
7. 调用 `handle.stop()`（优雅停机）

## 核心架构: `DaemonHandle` (`src/handle.rs`)

`DaemonHandle` 是 daemon 的主入口，包裹 `Arc<DaemonInner>`。

### 内部组件

```
DaemonInner {
    relay_client:      RelayClient,        // → 后端 relay 连接
    agent_glue:        AgentGlue,          // → 本地 agent 管理
    local_rpc:         Option<RpcServer>,  // → TUI JSON-RPC 服务器
    peer_store:        PeerStore,          // → 配对设备信息
    relay_link_rx:     watch::Receiver,    // → relay 连接状态
    peer_rx:           watch::Receiver,    // → 配对状态
}
```

### UniFFI 暴露给 Swift 的方法

| 方法 | 用途 |
|------|------|
| `start()` / `stop()` | 生命周期 |
| `current_relay_link()` / `subscribe_relay_link()` | Relay 连接状态 |
| `current_peer()` / `subscribe_peer()` | 配对状态 |
| `pairing_qr()` | 生成配对 QR |
| `forget_peer()` / `forget_peer_device()` | 管理配对 |
| `start_agent()` / `send_user_message()` | Agent 操作 |
| `interrupt_thread()` / `close_thread()` | 线程管理 |
| `subscribe_agent_state()` / `current_agent_state()` | Agent 状态 |

### Observer 协议（Swift 实现）

- `ConnectionStateObserver` — 连接状态变化
- `AgentStateObserver` — Agent 状态变化
- `RelayLinkStateObserver` — Relay 链路状态
- `PeerStateObserver` — 配对状态

## Relay 连接 (`src/relay_client.rs`)

### 连接生命周期

1. 等待 host 安装令牌（poll + Notify）
2. 进入 `run_once` 循环:
   - 验证令牌: `POST /v1/host/installations/self`
   - 获取 WS ticket: `POST /v1/host/realtime/ws-ticket`
   - WebSocket 升级: `tokio_tungstenite::connect_async`
   - 双向分发循环（inbound `ServerFrame`, outbound `ClientFrame`, ping/pong）
3. 断开后: 指数退避重连（1s → 2s → 4s → 8s → 16s → 30s 封顶）

### 入站帧路由

- `Hello`: 自动订阅 `host:{device_id}` topic
- `SubscribeAck`: 设置 `RelayLinkState::Connected`
- `DurableEvent`: 路由到 `route_durable_event`
  - `host_command_issued` → 调用本地 RPC
  - `host_linked` / `host_unlinked` → 刷新 peers
  - `host_force_close` → 关闭连接

### 心跳

每 30s 发 Ping，45s Pong 超时。

## HTTP Relay 客户端 (`src/relay_http.rs`)

`RelayHttpClient` 基于 `openwire::Client`，使用 Ed25519 签名认证请求。

| 端点 | 用途 |
|------|------|
| `POST /v1/host/bootstrap/nonce` | 获取请求签名 nonce |
| `POST /v1/host/pairing/request-code` | 请求配对 QR 码 |
| `POST /v1/host/pairing/redeem` | 赎回配对码 |
| `POST /v1/host/realtime/ws-ticket` | 获取 WS ticket |
| `POST /v1/host/installations/self` | 验证安装令牌 + 获取 peers |
| `DELETE /v1/pairing` | 忘记配对 |
| `POST /v1/me/peers/query` | 查询配对设备 |

### 请求签名

- Ed25519 密钥对存储在 `~/.minos/secrets/host-bootstrap-key.json`
- 请求头: `ed25519-sig: <signature>`, `ed25519: <public_key>`

## Agent 管理 (`src/agent.rs`)

### `AgentGlue` — daemon 侧的 Agent 管理器

桥接三个关注点：
1. **`AgentManager`**（来自 `minos-agent-runtime`）：多工作区 CLI 实例管理
2. **`IngestCoalescer` + `EventWriter`**：预分配 seq/projection，单写者 SQLite 本地持久化
3. **Watch channel**：镜像最新线程状态

subagent 也是普通 thread，只是在 `ThreadAdded` / `ThreadSummary` / `LocalThreadSnapshot` 上携带 `parent_thread_id`。daemon 收到 `ThreadAdded { parent_thread_id: Some(parent) }` 时复用父线程的 conversation 插入子线程行，且不增加 `conversations.agent_session_count`。TUI 通过现有 `list_conversation_agent_sessions` 得到父子 thread，不新增 `list_subagents` RPC。

Codex app-server 启动分两段超时：initialize handshake 默认 5 秒，`thread/start` 默认 30 秒。后者独立配置为 `AgentRuntimeConfig.thread_start_timeout`，因为线程创建会受 workspace 初始化、skills/MCP 注入和 Codex 冷启动状态影响。

Teamwork MCP 注入不依赖单一外部 sidecar。`AgentRuntimeConfig` 优先使用 `MINOS_TEAMWORK_MCP_BIN` 或同目录 `minos-teamwork-mcp`；找不到时，`minos-daemon __minos-teamwork-mcp` hidden 子命令可直接作为 stdio MCP server。TUI 托管 daemon 时当前可执行文件是 `minos-tui`，同一逻辑会回落到 `minos-tui __minos-teamwork-mcp`，因此 `minos-tui --backend daemon` 不要求用户额外构建 MCP bin。

### 支持的 Agent

`codex`, `claude`, `gemini`, `opencode`

### 关键操作

| 方法 | 描述 |
|------|------|
| `start_agent()` | 启动新 agent 会话 |
| `start_agent_with_session_id()` | 指定 session ID 启动 |
| `send_user_message()` | 发送文本到运行中的 agent |
| `dispatch_message()` | 自动 start-or-resume + send |
| `resolve_approval()` | 响应审批请求 |
| `respond_opencode_permission()` | 响应 opencode 权限请求 |
| `respond_opencode_question()` | 回答 opencode question 请求 |
| `interrupt_thread()` | 中断运行中的线程 |
| `close_thread()` | 优雅关闭线程 |
| `delete_thread()` | 删除线程 + 事件 |
| `resume_thread()` | 恢复挂起的线程 |
| `list_threads()` / `get_thread()` | 查询线程 |

### 线程状态

```
Starting → Idle → Running { turn_started_at_ms }
         ↕       ↕
    Suspended { reason: UserInterrupt | CodexCrashed | DaemonRestart | InstanceReaped }
         ↕
    Resuming → Running
         ↓
    Closed { reason: UserClose | TerminalError }
```

### 事件持久化流程

1. `AgentManager` 发出 `RawIngest`。`RawIngest` 不再以 `serde_json::Value` 作为主数据面，而是携带 `RawBody::InlineBytes` 或 `RawBody::Artifact`。
2. `AgentGlue` bridge 先交给 `IngestCoalescer`，按线程读取本地 `last_seq`，在本地/live fanout 之前生成稳定 `seq`、projection、checksum 和 canonical `IngestChunk`。
3. `AgentGlue` 将同一个 `IngestChunk` 分两路处理：`EventWriter::write_chunk()` 批量写 SQLite；`IngestSyncHandle::submit_live()` 非阻塞尝试实时上传。
4. 大于等于 `INLINE_RAW_BODY_THRESHOLD`（16 KiB）的 raw body 写入本地 `ArtifactStore`，SQLite `events` 行只保存 artifact metadata；小 body 以内联 bytes 保存。
5. `EventWriter` 只负责本地事务提交和 `projection_json` 持久化，不再构造 relay frame，也不 await relay outbound queue。
6. SQLite 提交后，daemon 本地订阅者收到 `LocalIngestFrame { seq, ui_events }`。WS 在线时，live sync worker 发送 `ClientFrame::HostIngestLiveBatch`；WS 断开或 live 队列满时只记录 dirty range，不保留 payload backlog。

## 本地存储 (`src/store/`)

### SQLite Schema（5 个 migration）

- **0001**: `schema_version`, `workspaces`, `threads`, `events` 表
- **0002**: `projects` 表，`threads` 增加 `project_id`
- **0003**: `chat_rooms`, `chat_messages`, `chat_agent_sessions`, `chat_mcp_commands`
- **0004**: teamwork MCP 状态
- **0005**: `ingest_sync_state`，记录 backend ack 水位和 host 本地 dirty range

`threads.parent_thread_id` 表示 subagent 归属。顶层 thread 为 `NULL`；subagent 行引用父 thread，事件仍写入各自 thread 的 `events`，因此历史回放和实时 fanout 不需要单独的数据面。

### `EventWriter` (`src/store/event_writer.rs`)

- 单写者任务保证每线程单调 `seq`
- 5ms 批量窗口提高吞吐
- 等待父线程行存在（指数退避重试）
- 只写本地 SQLite，不转发 relay，不受 WS 背压影响
- 持久化 `body_kind/body_inline/artifact_*`，避免把大 JSON DOM 在通道中重复 clone
- 持久化 `projection_json`，TUI replay 不再依赖重新读取完整 raw payload
- 区分 `live` 和 `jsonl_recovery` 事件来源

### Ingest Sync (`src/ingest_sync.rs`)

- 在线时按 5ms/50 条窗口把 live `IngestChunk` 打包为 `HostIngestLiveBatch`，使用非阻塞 enqueue；队列满时标记 dirty range。
- 断线期间 Agent 继续输出并写本地 SQLite，上传通道不堆正文 payload，只更新 `ingest_sync_state`。
- 重连并订阅 host topic 后发送 `HostGapManifest`，只包含 thread、seq range、bytes、event_count、时间范围和 running 状态。
- backend 接受 `HostGapManifest` 后通过同一条 WS 发送 `PullIngestRange`，host 从 SQLite 读取 range 后回 `HostIngestPullResponse`。
- 收到 `HostIngestAck` / `PullAck` 后推进本地 backend ack 水位。

### ArtifactStore (`src/store/artifacts.rs`)

- 根目录: daemon SQLite 所在目录下的 `artifacts/`
- 布局: `artifacts/<thread_id>/<artifact_id>`
- `artifact_id`: `art_<sha256>`
- 生命周期: `delete_thread()` 删除线程行后同步删除该线程 artifact 目录
- 读取: 本地 JSON-RPC `read_artifact_range(thread_id, artifact_id, offset, limit)` 返回 range bytes 与 total/eof

### 文件状态

- `local-state.json`: DeviceId + PeerRecord
- `device-secret.json`: Host 安装令牌
- `host-bootstrap-key.json`: Ed25519 签名密钥对
- 所有文件存储在 `~/.minos/secrets/`（0o700 权限）

## JSONL 恢复 (`src/jsonl_recover.rs`)

- 读取 `~/.codex/sessions/{codex_session_id}.jsonl` 恢复缺失事件
- 当前作为显式本地修复工具保留；生产同步安全机制是 `HostGapManifest` + `PullIngestRange`，不是旧 checkpoint reconciler。

## 配对 QR 生成 (`src/relay_pairing.rs`)

### 流程

1. CLI 或 Swift 调用 `DaemonHandle::pairing_qr()`
2. HTTP 客户端获取 bootstrap nonce，Ed25519 签名，POST `/v1/host/pairing/request-code`
3. 返回 `RelayQrPayload`（v, host_display_name, pairing_token, expires_at_ms）
4. 启动赎回循环：每 2s 轮询 `/v1/host/pairing/redeem` 直到手机确认
5. 成功后持久化 `DeviceSecret`，通知 dispatch 任务进行 WS 连接

## RPC 服务器 (`src/rpc_server.rs`)

`RpcServerImpl` 实现 `MinosRpcServer` trait，路由后端转发的命令。

支持的方法: `health`, `list_clis`, `list_host_skills`, `write_host_skill_config`, `start_agent`, `send_user_message`, `approval_decision`, `respond_opencode_question`, `interrupt_thread`, `close_thread`, `list_threads`, `get_thread`

### `invoke_host_command()` — 分发函数

路由 method 字符串到对应 RPC handler，支持 `minos_*` 和 `agent_session.*` 命名空间。

## 本地 RPC 服务器 (`src/local_rpc.rs`)

`LocalRpcImpl` 实现 `LocalDaemonRpcServer` trait，服务 TUI。

额外方法: `delete_thread`, `resume_thread`, `respond_opencode_question`, `read_thread_raw_history`, `read_group_chat`
订阅: `subscribe_ingest()` 和 `subscribe_manager_events()`

### 发现机制

写入 `tui-daemon-rpc.json` 到 `$MINOS_HOME/run/`，TUI 读取该文件发现 WS 地址。

## 模块连接图

```
main.rs
  └── DaemonHandle (handle.rs)
        ├── RelayClient (relay_client.rs)
        │     ├── RelayHttpClient (relay_http.rs) — HTTP 控制面
        │     ├── RpcServerImpl (rpc_server.rs) — 路由转发命令
        │     └── IngestSyncHandle (ingest_sync.rs) — ack/pull/manifest 路由
        ├── AgentGlue (agent.rs)
        │     ├── AgentManager (minos-agent-runtime) — CLI 进程管理
        │     ├── IngestCoalescer (ingest_coalescer.rs) — seq/projection/chunk 生成
        │     ├── EventWriter (store/event_writer.rs) — SQLite 本地写入
        │     └── LocalStore (store/mod.rs) — SQLite 连接池
        ├── LocalRpcImpl (local_rpc.rs) — TUI JSON-RPC 服务器
        ├── Subscription (subscription.rs) — UniFFI observer 桥接
        ├── device_secret_store.rs — Host 令牌持久化
        ├── host_bootstrap_key_store.rs — Ed25519 密钥持久化
        ├── local_state.rs — DeviceId + PeerRecord JSON
        ├── relay_pairing.rs — RelayQrPayload, PeerRecord
        └── jsonl_recover.rs — Codex JSONL 恢复
```
