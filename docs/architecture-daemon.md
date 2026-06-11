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
2. **`EventWriter`**：单写者 SQLite + relay 转发
3. **Watch channel**：镜像最新线程状态

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

1. `AgentManager` 通过 broadcast channel 发出 `RawIngest`
2. `AgentGlue` bridge 转发到 `EventWriter::write_live()`
3. `EventWriter` 批量写入（5ms 窗口，最大 100 条），分配单调 `seq`
4. SQLite 提交后，转发 `ClientFrame::HostStreamEvent` 到 relay outbound 队列

## 本地存储 (`src/store/`)

### SQLite Schema（3 个 migration）

- **0001**: `schema_version`, `workspaces`, `threads`, `events` 表
- **0002**: `projects` 表，`threads` 增加 `project_id`
- **0003**: `chat_rooms`, `chat_messages`, `chat_agent_sessions`, `chat_mcp_commands`

### `EventWriter` (`src/store/event_writer.rs`)

- 单写者任务保证每线程单调 `seq`
- 5ms 批量窗口提高吞吐
- 等待父线程行存在（指数退避重试）
- SQLite 提交后转发到 relay
- 区分 `live` 和 `jsonl_recovery` 事件来源

### 文件状态

- `local-state.json`: DeviceId + PeerRecord
- `device-secret.json`: Host 安装令牌
- `host-bootstrap-key.json`: Ed25519 签名密钥对
- 所有文件存储在 `~/.minos/secrets/`（0o700 权限）

## JSONL 恢复 (`src/jsonl_recover.rs`)

- 读取 `~/.codex/sessions/{codex_session_id}.jsonl` 恢复缺失事件
- 由 `Reconciliator` 在检测到间隙时触发

## Reconciliator (`src/reconciliator.rs`)

- 处理后端的 `IngestCheckpoint` 帧
- 比较后端的 `last_seq_per_thread` 与本地 `threads.last_seq`
- 从本地 DB 回放缺失事件到 relay
- 本地 DB 有缺口时委托给 `jsonl_recover`
- 优先处理活跃线程（running > idle > suspended）

## 配对 QR 生成 (`src/relay_pairing.rs`)

### 流程

1. CLI 或 Swift 调用 `DaemonHandle::pairing_qr()`
2. HTTP 客户端获取 bootstrap nonce，Ed25519 签名，POST `/v1/host/pairing/request-code`
3. 返回 `RelayQrPayload`（v, host_display_name, pairing_token, expires_at_ms）
4. 启动赎回循环：每 2s 轮询 `/v1/host/pairing/redeem` 直到手机确认
5. 成功后持久化 `DeviceSecret`，通知 dispatch 任务进行 WS 连接

## RPC 服务器 (`src/rpc_server.rs`)

`RpcServerImpl` 实现 `MinosRpcServer` trait，路由后端转发的命令。

支持的方法: `health`, `list_clis`, `list_host_skills`, `write_host_skill_config`, `start_agent`, `send_user_message`, `approval_decision`, `interrupt_thread`, `close_thread`, `list_threads`, `get_thread`

### `invoke_host_command()` — 分发函数

路由 method 字符串到对应 RPC handler，支持 `minos_*` 和 `agent_session.*` 命名空间。

## 本地 RPC 服务器 (`src/local_rpc.rs`)

`LocalRpcImpl` 实现 `LocalDaemonRpcServer` trait，服务 TUI。

额外方法: `delete_thread`, `resume_thread`, `read_thread_raw_history`, `read_group_chat`
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
        │     └── Reconciliator (reconciliator.rs) — 检查点对账
        ├── AgentGlue (agent.rs)
        │     ├── AgentManager (minos-agent-runtime) — CLI 进程管理
        │     ├── EventWriter (store/event_writer.rs) — SQLite 写入 + relay 转发
        │     └── LocalStore (store/mod.rs) — SQLite 连接池
        ├── LocalRpcImpl (local_rpc.rs) — TUI JSON-RPC 服务器
        ├── Subscription (subscription.rs) — UniFFI observer 桥接
        ├── device_secret_store.rs — Host 令牌持久化
        ├── host_bootstrap_key_store.rs — Ed25519 密钥持久化
        ├── local_state.rs — DeviceId + PeerRecord JSON
        ├── relay_pairing.rs — RelayQrPayload, PeerRecord
        └── jsonl_recover.rs — Codex JSONL 恢复
```
