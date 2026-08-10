# Host 守护进程 (minos-daemon) 架构文档

> 本文档详细描述 `minos-daemon` crate 的架构、模块划分和关键逻辑。

## 概述

`minos-daemon` 是运行在 Mac（或未来其他平台）上的守护进程，负责：连接后端 relay、管理本地 AI agent 子进程、处理配对、持久化本地状态。它是 macOS 应用的 Rust 核心，也通过 JSON-RPC 与 Desktop 通信。

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
| `sessions` | 列出持久化线程 |
| `thread <id>` | 查看线程详情 |
| `start` | 启动守护进程（长驻） |

### 启动序列（`Start` 命令）

1. 初始化 logging（mars-xlog）
2. 解析路径，加载 `LocalState`（或用新 DeviceId 初始化）
3. 可选配置本地 JSON-RPC 服务器（给 Desktop 用）
4. 调用 `DaemonHandle::start_with_local_rpc()`（内部：open store → `mark_orphans_suspended` → **`prune_orphan_worktrees`** → agent/relay）
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
    local_rpc:         Option<RpcServer>,  // → Desktop JSON-RPC 服务器
    peer_store:        PeerStore,          // → 配对设备信息
    relay_link_rx:     watch::Receiver,    // → relay 连接状态
    peer_rx:           watch::Receiver,    // → 配对状态
}
```

### Desktop local RPC 方法

| 方法 | 用途 |
|------|------|
| `start()` / `stop()` | 生命周期 |
| `current_relay_link()` / `subscribe_relay_link()` | Relay 连接状态 |
| `current_peer()` / `subscribe_peer()` | 配对状态 |
| `pairing_qr()` | 生成配对 QR |
| `forget_peer()` / `forget_peer_device()` | 管理配对 |
| `start_agent()` / `send_user_message()` | Agent 操作 |
| `interrupt_session()` / `close_session()` | 线程管理 |
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
   - Host 实时：`Authorization: Bearer hit_*` 直连 `/ws/host`（无 host ticket）
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
| `POST /v1/host/pairing/redeem` | 赎回配对码 |
| `GET /ws/host` with `Authorization: Bearer hit_*` | Host 实时口（无 ticket） |
| `POST /v1/host/installations/self` | 验证安装令牌 + 获取 peers |
| `POST /v1/me/peers/query` | 查询配对设备 |

### 请求签名

- Ed25519 密钥对存储在 `~/.minos/secrets/host-bootstrap-key.json`
- 请求头: `ed25519-sig: <signature>`, `ed25519: <public_key>`

## Agent 管理 (`src/agent.rs`)

### `AgentGlue` — daemon 侧的 Agent 管理器

桥接三个关注点：
1. **`AgentManager`**（来自 `minos-agent-runtime`）：多工作区 CLI 实例管理
2. **`IngestCoalescer` + `EventWriter`**：projection + 父行就绪缓冲；**seq 仅在 SQLite 事务内分配**，单写者本地持久化
3. **Watch channel**：镜像最新线程状态

subagent 也是普通 thread，只是在 `SessionAdded` / `SessionSummary` / `LocalSessionSnapshot` 上携带 `parent_session_id`。daemon 收到 `SessionAdded { parent_session_id: Some(parent) }` 时复用父线程的 conversation 插入子线程行，且不增加 `conversations.agent_session_count`。Desktop 通过现有 `list_conversation_agent_sessions` 得到父子 thread，不新增 `list_subagents` RPC。conversation message 读路径会过滤 `session_id` 指向 subagent 的旧消息行，summary 的 preview/count 也只计算可见消息，避免 subagent transcript/result 污染 conversation 主时间线。

**Conversation 主时间线排序契约：** `chat_messages.message_seq`（SQLite rowid PK）是唯一排序键；list 按 `message_seq DESC` 分页，客户端 reverse 为 ASC 展示。`message_id` upsert 复用原 `message_seq`（body/metadata 可更新）。`created_at_ms` 仅展示。多 agent 完成顺序 = durable 落库顺序（finish/write order）；因果关系用 `reply_to_message_id` / `mentions` / `delegation_id` 表达（例如 MCP 委托 result 引用 request），不通过重排历史 seq。Agent 回合结果由 `conversation_completion` 在 turn boundary 写入 `agent-result:…`；subagent session 不写 conversation result。

**多端 vs 本机（双写者，按路径）：**

| 路径 | 本地 `agent-result:…` | Hub 协作气泡 |
|------|----------------------|--------------|
| Local-only / 未 Link | Daemon 工作台 SSOT | 无 |
| Mobile / `client_live` @agent | Host ingest 仍可写本地 workbench | **Hub TurnCompletionProjector**（per `origin_message_id`） |
| Desktop-native Linked | Daemon 写本地 workbench（规范 id = `agent-result:{conv}:{session}:{origin}`） | Desktop Outbox **`host_projection`** 上行同一 id（Hub 不二次 dispatch） |

- Daemon **不**在协议层 dual-write 到 Hub；**Desktop** 在 Linked 且 id 规范时仍可 `host_projection` uplink（`isCanonicalAgentResultId`）。  
- Collab / Hub-bound session：**禁止** message_key / `t{ms}` 回退当 agent-result 后缀；缺 `origin_message_id` 时 skip 写（fail-visible），避免非规范 id 污染 Hub。  
- Desktop Linked 读路径：Hub 气泡 SSOT + 本地 tool/git 合并；同 id Hub 优先。

**Agent 终态正文（last segment）：** Grok/Gemini 等 ACP agent 常在同一 `message_id` 下用多段 `agent_message_chunk` 输出中间进度（「正在定位…」），工具/思考会把 session 时间线拆成多个气泡。`conversation_completion` 与 session `ChatState` 对齐：tool / reasoning / subagent 事件关闭当前文本 segment，turn 结束时只把**最后一个未关闭的 assistant 文本段**写入 conversation；不会把全过程进度日志与最终摘要拼接成一条。工具后若无新的最终文本，则不回写中间进度。显式 `post_conversation_update` 仍可单独追加中途状态条。 Hub projector 复用同一 last-segment 语义（`minos-ui-protocol` 翻译）。

**Grok 投影（Phase A–C）：** `translate_grok` 在 daemon 投影层对齐 grok-build pager：
- **A**：读 `_meta.streamStartMs` / `agentTimestampMs` / `promptId`；`tool_call` 与 **agent_message_chunk** 上的 `streamStartMs` 变化会 `MessageCompleted` 当前 assistant text，下一段 agent text 用新 `message_id`。`agent_thought_chunk` 使用独立的 concurrent `streamStartMs`，**不得**据此关闭 text（否则 thought/text 交错会把正文拆成逐 token 气泡）。
- **B**：压制 todo/wait/task-output/spawn 等 plumbing（`grok/turn_activity` Raw 保留等待语义）；tool 标题优先 `rawInput.description` / path；thinking 附带 `elapsedMs` activity。
- **C**：`minos-acp-protocol` 补齐 tool kind/locations/rawInput/Failed、`agent_thought_chunk`、notification `_meta`；translator 从 raw JSON 抽取 content/locations/orphan update。
- **Grok ACP 双通道投影**（完整清单见 [architecture-grok-acp-projection.md](./architecture-grok-acp-projection.md)）：优先结构化 `raw_output`（pager 同款），再 `content`；禁止 dump ToolOutput JSON。覆盖 Edit→patch、Read→去模型行号 densify、Bash→`output_for_prompt`+ANSI strip、Grep→`file_matches`、ListDir 列表、Web/MCP/Skill 等。

Codex app-server 启动分两段超时：initialize handshake 默认 5 秒，`thread/start` 默认 30 秒。后者独立配置为 `AgentRuntimeConfig.thread_start_timeout`，因为线程创建会受 workspace 初始化、skills/MCP 注入和 Codex 冷启动状态影响。

Teamwork MCP 注入不依赖单一外部 sidecar。`AgentRuntimeConfig` 优先使用 `MINOS_TEAMWORK_MCP_BIN` 或同目录 `minos-teamwork-mcp`；找不到时，`minos-daemon __minos-teamwork-mcp` hidden 子命令可直接作为 stdio MCP server。Desktop / daemon 作为 host binary 时，同一逻辑回落到 `__minos-teamwork-mcp` hidden sidecar。

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
| `interrupt_session()` | 中断运行中的线程 |
| `close_session()` | 显式关闭线程（`Closed`，不可 resume） |
| `delete_session()` | 删除线程 + 事件 |
| `resume_session(session_id, auto_continue)` | 注册 + provider reattach；`auto_continue` 时在 `needs_continue` 下一发 CONTINUE |
| `list_sessions()` / `get_session()` | 查询线程 |

### 停机与 resume（host-local）

Managed Desktop 退出会调用 `DaemonHandle::stop`：

1. **`AgentGlue::shutdown`**：对每个内存中非 Closed 线程 `suspend_for_daemon_stop`（best-effort cancel + 内存态 `Suspended { DaemonRestart }`），并按停机前状态写入 `needs_continue`（`running`/`starting`/`resuming` → 1，idle/已 suspended → 0）。**同步** `suspend_thread_for_daemon_restart` 落库：mid-flight → durable `suspended`；idle → durable **`idle`**（不把回合间停机标成用户 Pause）。Manager event bridge **不**把 `Suspended{DaemonRestart}` 再写一次 SQLite（避免与同步落库竞态）。**不**调用 `close_session`。
2. `shutdown_instances` SIGTERM/SIGKILL provider 进程组。
3. 拆除 local RPC discovery + relay。

脏退出（kill -9）：下次 `DaemonHandle::start` 时 `mark_orphans_suspended` **只**把 `running`/`starting`/`resuming` 翻成 `suspended` + `daemon_restart` + `needs_continue=1`。**`idle` 保持 `idle`**（回合间进程死亡 ≠ 用户 Pause；下次 `resume_session` 在无 live process 时仍会 reattach）。

| 列 / 标志 | 含义 |
|-----------|------|
| `sessions.needs_continue`（`0001_initial`） | 进程死亡时是否在中途 turn；一发 CONTINUE 或用户消息抢占后清零 |
| `take_needs_continue` | 原子 clear + 返回原值；用户 send 与 open-time continue 互斥 |

`resume_session`：**仅 reattach**（到 Idle）。`auto_continue=true` 时若 `take_needs_continue` 成功则注入共享 `CONTINUE_PROMPT`（见 `minos-agent-runtime`）。`send_user_message` 在发用户文本前 `take_needs_continue`（**用户消息优先**，不注入 CONTINUE）。

显式 close / delete → `Closed`，不可 resume。`UserInterrupt` suspend 不置 `needs_continue`。子 agent 同样 suspend-not-close，但 open 路径只对 **最多一个** top-level `needs_continue` session 自动 continue。

CONTINUE 经 `synth_user_message_ingest` 写入历史，时间线上显示为 user 消息（有意取舍，不改 conversation message model）。

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

`Starting` / `Resuming` → `Suspended` 在 daemon stop 路径合法；`Suspended` → `Suspended` 允许改写 pause reason。

### 事件持久化流程

1. `AgentManager` 发出 `RawIngest`。`RawIngest` 不再以 `serde_json::Value` 作为主数据面，而是携带 `RawBody::InlineBytes` 或 `RawBody::Artifact`。
2. `AgentGlue` bridge 交给 `IngestCoalescer::admit`：父 `sessions` 行就绪则生成 **无 seq** 的 `PreparedIngest`（projection + conversation_id）；父行晚到则 **按 session 缓冲**（不静默 drop、不预先烧 seq），`SessionAdded` / 短轮询后 `drain_ready`。
3. `EventWriter::write_prepared` 在 SQLite 事务内用 `last_seq + 1` **分配 seq 并 commit**；失败则 backoff 重试 / 回写 FIFO retry 队首（seq 从未提交，无空洞）。Ingest actor 是唯一提交者；旧 frame 成功前不允许新 frame 进入 writer，保证 provider 顺序与 durable seq 一致。
4. **仅本地 commit 成功后** 才 `submit_live` + 向 Desktop 广播 `LocalIngestFrame { seq, ui_events }`（commit-then-upload）。
5. 大于等于 `INLINE_RAW_BODY_THRESHOLD`（16 KiB）的 raw body 写入本地 `ArtifactStore`，SQLite `events` 行只保存 artifact metadata；小 body 以内联 bytes 保存。
6. `EventWriter` 只负责本地事务提交和 `projection_json` 持久化，不再构造 relay frame，也不 await relay outbound queue。WS 在线时 live sync 发 `HostIngestLiveBatch`；断开或队列满时只记 dirty range。

## 本地存储 (`src/store/`)

### SQLite Schema（latest-only 单一 migration）

路径：`crates/minos-daemon/migrations/0001_initial.sql`（`sqlx::migrate!`）。开发态破坏性变更时清库重建，不做增量 ALTER 链。

| 表 | 内容 |
|----|------|
| `schema_version` | 应用侧版本记录（sqlx 另有 `_sqlx_migrations`） |
| `workspaces` | workspace root 注册 |
| `projects` | 项目（含 `workspace_path`） |
| `conversations` | 协作对话 + priority/progress/git work-unit 元数据（`branch`、`worktree_path`、`git_mode`、`git_dirty`、`git_head`） |
| `sessions` | agent session（`session_id`、runtime `agent`、nullable `bot_id`、`parent_session_id`、`needs_continue`、status…） |
| `conversation_agent_members` | roster membership PK `(conversation_id, bot_id)` + optional peer `brief` |
| `chat_messages` | conversation 主时间线（agent 行写 `bot_id`；含 reply/delegation/mentions；`sender_role` 含 system） |
| `chat_message_reactions` | 主时间线消息上的本机 emoji 反应（`message_id + emoji + actor`；host actor=`local`/`user`/`You`；幂等 toggle；非 Nostr kind:7；云端 social 延后） |
| `events` / `artifacts` | session transcript 事件与大 payload |
| `ingest_sync_state` | backend ack 水位 + host dirty range |
| `bot_identities` | 本机 bot 身份缓存（`bot_id`、display_name、runtime、model、effort、system_prompt、source=`user_configured`\|`host_runtime_seed`）；替代旧 `agent_profiles` |

`sessions.parent_session_id` 表示 subagent 归属。顶层 session 为 `NULL`；subagent 行引用父 session，事件仍写入各自 session 的 `events`，因此历史回放和实时 fanout 不需要单独的数据面。

### `EventWriter` (`src/store/event_writer.rs`)

- 单写者任务保证每线程单调 `seq`
- 5ms 批量窗口提高吞吐
- 等待父线程行存在（指数退避重试）
- 只写本地 SQLite，不转发 relay，不受 WS 背压影响
- 持久化 `body_kind/body_inline/artifact_*`，避免把大 JSON DOM 在通道中重复 clone
- 持久化 `projection_json`，Desktop replay 不再依赖重新读取完整 raw payload
- 区分 `live` 和 `jsonl_recovery` 事件来源

### Ingest Sync (`src/ingest_sync.rs`)

- 在线时按 5ms/50 条窗口把 live `IngestChunk` 打包为 `HostIngestLiveBatch`，使用非阻塞 enqueue；队列满时标记 dirty range。
- 断线期间 Agent 继续输出并写本地 SQLite，上传通道不堆正文 payload，只更新 `ingest_sync_state`。
- 重连并订阅 host topic 后发送 `HostGapManifest`，只包含 thread、seq range、bytes、event_count、时间范围和 running 状态。
- backend 接受 `HostGapManifest` 后通过同一条 WS 发送 `PullIngestRange`，host 从 SQLite 读取 range 后回 `HostIngestPullResponse`。
- 收到 `HostIngestAck` / `PullAck` 后推进本地 backend ack 水位。

### ArtifactStore (`src/store/artifacts.rs`)

- 根目录: daemon SQLite 所在目录下的 `artifacts/`
- 布局: `artifacts/<session_id>/<artifact_id>`
- `artifact_id`: `art_<sha256>`
- 生命周期: `delete_session()` 删除线程行后同步删除该线程 artifact 目录
- 读取: 本地 JSON-RPC `read_artifact_range(session_id, artifact_id, offset, limit)` 返回 range bytes 与 total/eof

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
2. Host Link：Desktop 登录同一账户后调用 `host.prepare_link` / `host.sign_link_proof` / `POST /v1/hosts/link` / `host.apply_link_token`
3. Daemon 持久化 `host_installation_token` 并连接 `/ws/host`
4. 启动赎回循环：每 2s 轮询 `/v1/host/pairing/redeem` 直到手机确认
5. 成功后持久化 `DeviceSecret`，通知 dispatch 任务进行 WS 连接

## RPC 服务器 (`src/rpc_server.rs`)

`RpcServerImpl` 实现 `MinosRpcServer` trait，路由后端转发的命令。

支持的方法: `health`, `list_clis`, `list_models`, `list_agent_profiles`, `create_agent_profile`, `update_agent_profile`, `delete_agent_profile`, `list_host_skills`, `write_host_skill_config`, `start_agent` (optional `profile_id` / `model` / `reasoning_effort` / `instructions`), `start_agent_in_conversation` (same), `send_user_message`, `approval_decision`, `respond_opencode_question`, `interrupt_session`, `close_session`, `list_sessions`, `get_session`

Host-local **bot identities**（`bot_identities` 表，见 `0001_initial`；wire 仍用 `list/create/update/delete_agent_profiles`，`id` ≡ `bot_id`，`instructions` ≡ `system_prompt`）cache personalized runtime+model+effort+system_prompt for local launch and offline roster. **Product bot identity SSOT is Hub `agents`** (global bot user + digital body); local rows are a Host cache / launch helper and must not mint a second multi-end identity. Offline create-conversation runtime labels map to stable seeds `local-rt-{runtime}` via `ensure_local_runtime_bot` (`source=host_runtime_seed`). See [global-bot-identity-design](superpowers/specs/global-bot-identity-design.md) and [bot-identity-session-separation Phase 2](superpowers/specs/2026-08-10-bot-identity-session-separation.md). Model discovery remains best-effort via Codex `model/list`, CLI probes (`grok models`, `opencode models`), or static aliases (Claude/Gemini).

**Profile name rules**: display names are also `@Name` mention tokens. `create_agent_profile` / `update_agent_profile` reject empty names and names containing whitespace, `#`, or `@` (breaks single-token routing / `agent#short` form). Desktop create form + `profileMentionInsert` enforce the same; non-clean names force `@p/<id>` insert as defense in depth.

**Launch resolution** (`AgentGlue::resolve_launch_options`): when `profile_id`/`bot_id` is set, load bot identity and require `request.agent == identity.runtime_agent`; merge launch fields with precedence **explicit request > identity > None**. Missing identity or agent mismatch → protocol error. Structured log on start includes `profile_id`, `bot_id`, `agent`, `session_id`.

**Agents capability SSOT**: runtime set and capability flags come from `minos_domain::AgentName` / `AgentDescriptor` (filled in `list_clis` via `minos-cli-detect`). Per-model effort ladders come only from `list_models` (`model_catalog.rs`); unsupported runtimes must return empty `supported_reasoning_efforts` — never invent default ladders for Claude/Gemini/OpenCode. Desktop projects these values and must not maintain a rival capability table.

### `invoke_host_command()` — 分发函数

路由 method 字符串到对应 RPC handler，支持 `minos_*` 和 `agent_session.*` 命名空间。

## 本地 RPC 服务器 (`src/local_rpc.rs`)

`LocalRpcImpl` 实现 `LocalDaemonRpcServer` trait，服务 Desktop。

额外方法: `delete_session`, `resume_session`, `respond_opencode_question`, `read_session_raw_history`, `list_conversations`, `create_conversation`, `update_conversation`, `remove_conversation_agent`, `list_conversation_messages`, `toggle_conversation_message_reaction`, `append_conversation_message`, `start_agent_in_conversation`, 以及 host-local git 服务：`git_get_status`, `git_get_diff`, `git_create_worktree`, `git_remove_worktree`, `git_ensure_identity`, `git_push_branch`, `git_open_pull_request`, `post_git_update`。

Conversation 元数据（`conversations` 表，见 `0001_initial`）：`priority`、`progress`（默认 `todo`）、git work-unit 绑定（`branch` / `worktree_path` / `git_mode` / `git_dirty` / `git_head`）。`create_conversation` 接受可选 `git_mode`：`worktree`（项目是 git 仓库时的**默认**）会在 `{repo_parent}/.minos-worktrees/<slug>-<id>` 下 `git worktree add -b minos/...` 并写入绑定；`inherit` 只快照 project workspace。创建成功后会向时间线发一条结构化 `worktree_created` git activity（body 内嵌 `<!--minos-git-activity:{...}-->`）。Agent **roster** 在 `conversation_agent_members`（PK=`(conversation_id, bot_id)` + 可选 `brief`）；wire `create_conversation` 仍接受 runtime labels `agents: [{ agent, brief? }]`，daemon 经 `ensure_local_runtime_bot` 转为 `local-rt-{runtime}` 后写入 membership。`ConversationRosterMember` 含 `bot_id` + runtime `agent` badge + optional `display_name`/`brief`。`LocalConversationSummary` 的 `participating_agents` 仍为 runtime 列表（从 roster bots 推导），`roster` 为完整成员。成员 `brief` 为空时 fallback 到该 `bot_id` 的 identity `description`（再 fallback 同 runtime 最新 description，≤500）。`start_agent_in_conversation` 按 `bot_id`（`profile_id` 或 local-rt seed）做 membership 检查，并将 roster briefing 注入 session-start instructions；session 行写入 `bot_id`。`add_conversation_agent` / `remove_conversation_agent` 变更 roster 时会：(1) 写 `sender_role=system` 协调消息（`[minos:system]…`）；(2) 广播 `LocalConversationEvent::RosterChanged`；(3) 向 **Idle** top-level session 注入 host 协调输入。MCP：`list_conversation_roster`。`remove_conversation_agent` 按 runtime 移除相关 bot memberships，关闭匹配 sessions（`roster_removed`）、取消 running delegations。列表聚合 `running_count` 与 `needs_attention_count`。首次 start 时若 progress 仍为 `todo` 则升为 `in_progress`。

Host git 实现位于 `crates/minos-daemon/src/git/`（`exec` / `snapshot` / `worktree` / `diff` / `identity` / `activity`），经 LocalDaemonRpc 暴露，供 Desktop/Mobile 共用；不做 forge hosting。`git_push_branch` 要求完整 `user.name`/`user.email` 且工作区干净，并对 `remote` 做 `^[A-Za-z0-9._/-]+$` 校验（拒绝 leading `-`）；`git_open_pull_request` 走本机 `gh pr create` 并自动 `post_git_update(pr_opened)`。`create_conversation_worktree` 按 repo toplevel 串行化（`Mutex`），分支是否已存在用 `git show-ref --verify` 判断。daemon 启动时 `prune_orphan_worktrees`：扫描各 project 的 `.minos-worktrees/`，删除 **DB 无引用** 的孤儿目录（不自动删 `minos/*` 分支）。`post_git_update` / `format_activity_body` 对 summary/url/subjects 等有 per-field 长度上限，防止灌库扇出。

Desktop：打开 conversation（Timeline mount）时调用 `git_get_status(refresh=true)` 刷新 header 上的 branch/dirty；create conversation 表单可显式选 `git_mode`（isolated worktree / project workspace）；时间线对 `git_activity` 消息渲染专用卡片（`GitActivityCard`）。

订阅: `subscribe_ingest()`、`subscribe_manager_events()` 和 `subscribe_conversation_events()`

`AgentGlue` 维护本地 manager event 与 conversation event 两条总线。agent runtime 状态事件通过 `subscribe_manager_events()` 进入 Desktop；`append_conversation_message`、daemon Teamwork MCP 的 `post_conversation_update` / `post_git_update` 和 delegation 可见消息写入成功后都会通过 `subscribe_conversation_events()` 发布 `ConversationMessageAppended { conversation_id, message_seq }`，Desktop 据此刷新当前 conversation。`toggle_conversation_message_reaction` 在本机 `chat_message_reactions` 上幂等 add/remove（host actor=`local`），并发布 `ConversationReactionToggled { conversation_id, message_id, reactions }`（完整聚合，UI 无需 re-list）；`list_conversation_messages` 同步嵌入各消息的 `reactions`。云端 social reactions 不在本层。daemon 的 `delegate_to_agent` 与 daemon embedded handler 共用 `TeamworkStore` 深度策略：delegated source thread 只能 delegate 回原 source agent，不能启动第三个 agent。`delegate_to_agent` 支持可选 `profile_id` / `target_profile`（name）；仅 `target_agent` 时 convenience 绑定该 runtime 最新 host profile，再经 `start_agent_in_conversation_with_options` 应用 launch 字段（与 RPC profile 启动同一语义，不在 MCP client 侧 merge model/effort/instructions）。`wait_delegation` 在 daemon 侧按 `timeout_ms` 阻塞到终态；`TeamworkStore` 用同 DB path 共享的 `DelegationSignalBus` 在 complete/cancel 时唤醒 waiter（fallback poll 2s，避免跨进程漏信号）。sidecar→daemon UDS 读超时对 `wait_delegation` 取 `timeout_ms + 5s` margin，其它请求保持 30s；连接失败 / 对端关闭 / daemon 拒绝分别映射 MCP `-32001` / `-32002` / `-32003`。daemon 从本地 `sessions` 表恢复 persisted session 时，会把 `conversation_id` 一并注册回 `AgentManager` 的 `SessionHandle.mcp_conversation_id`，保证恢复/重启后的 agent 仍使用当前 conversation 的 `--source-thread-id` MCP context。

### 发现机制

写入 `daemon-rpc.json` 到 `$MINOS_HOME/run/`，Desktop 可读取该文件发现 WS 地址。托管启动时也会通过 `DaemonHandle::local_rpc_url()` 直接拿到 binder 返回的 URL，避免再读 discovery 文件时撞上陈旧端口。

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
        ├── LocalRpcImpl (local_rpc.rs) — Desktop JSON-RPC 服务器
        │     └── Host Link RPC（需 RelayClient）:
        │           minos_local_host_prepare_link
        │           minos_local_host_sign_link_proof
        │           minos_local_host_apply_link_token
        ├── Subscription (subscription.rs) — local RPC / Desktop observer 桥接
        ├── device_secret_store.rs — Host 令牌持久化（`hit_*`）
        ├── host_bootstrap_key_store.rs — Ed25519 密钥持久化
        ├── local_state.rs — DeviceId + PeerRecord JSON
        ├── relay_pairing.rs — PeerRecord（linked viewer snapshot）
        └── jsonl_recover.rs — Codex JSONL 恢复
```

### Host Link local RPC（D02）

Desktop 在登录后调用：

1. `minos_local_host_prepare_link` — 返回 `installation_id` + `public_key` + backend nonce
2. Desktop 用 account bearer 调 `POST /v1/hosts/link`（签名可由 `minos_local_host_sign_link_proof` 生成）
3. `minos_local_host_apply_link_token` — 持久化 `hit_*` 并 `secret_notify` 唤醒 relay 拨号 `/ws/host`
