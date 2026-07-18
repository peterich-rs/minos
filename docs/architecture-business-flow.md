# Minos 业务流程文档

> 本文档描述 Minos 系统从用户注册到 Agent 会话管理的完整端到端业务流程。

## 流程总览

```
注册/登录 → 配对 → 建立 WebSocket → 创建项目 → 启动 Agent → 流式交互 → 审批/停止
```

---

## 1. 用户注册

### 参与方

- **移动端** (Flutter) 或 **Web 端** (React) 或 **fake-peer** CLI
- **后端** (minos-backend)

### 流程

1. 客户端调用 `POST /v1/auth/register`，携带 `email` + `password`
2. 后端:
   - Argon2id 哈希密码
   - 创建 `accounts` 行
   - 创建 `devices` 行（role = mobile-client 或 browser-admin）
   - 创建 `refresh_tokens` 行
   - 事务三联写: 领域行 + `durable_event_log` + `outbox_events`
3. 返回: access token (JWT, ≤15min TTL) + refresh token (30 天滑动) + account_id
4. 客户端持久化 token（移动端: iOS Keychain; Web: localStorage）

### 登录流程

类似注册，但验证已有密码而非创建账户。`POST /v1/auth/login`。

---

## 2. Host 安装与配对

### 参与方

- **macOS 应用** (SwiftUI → UniFFI → minos-daemon)
- **移动端** 或 **Web 端**
- **后端**

### Step 1: Host 引导

1. macOS daemon 启动时生成 Ed25519 密钥对（持久化到 `~/.minos/secrets/`）
2. 调用 `POST /v1/host/bootstrap/nonce` 获取 nonce
3. 用 Ed25519 私钥签名请求

### Step 2: Host 请求配对码

1. daemon 调用 `POST /v1/host/pairing/request-code`（签名请求）
2. 后端:
   - 验证 Ed25519 签名（TOFU — 首次信任公钥注册）
   - 创建 `pairing_codes` 行（status=pending）
   - 创建/更新 `devices` 行（role=agent-host）
3. 返回配对码给 daemon
4. macOS 应用将配对码渲染为 QR 码（`QRCodeRenderer`）

### Step 3: 手机确认

1. 用户用手机扫描 QR 码
2. 移动端调用 `POST /v1/pairing/confirm`，携带配对码 + Bearer token
3. 后端:
   - 验证配对码状态和有效期
   - 创建 `account_host_pairings` 行
   - 转换 `pairing_codes.status` → `confirmed`
4. 返回成功

### Step 4: Host 赎回

1. daemon 每 2s 轮询 `POST /v1/host/pairing/redeem`
2. 后端:
   - 验证所有条件（bootstrap proof + 配对码状态 + 有效期）
   - 原子转换 `pairing_codes.status` → `redeemed`
   - 签发 `host_installation_token`（`hit_*` 长期令牌）
3. daemon 持久化令牌到 `~/.minos/secrets/device-secret.json`

### Step 5: 稳态

- daemon 使用 `hit_*` 令牌访问 `/v1/host/*` 端点
- 多个账户可关联同一 host；一个账户可关联多个 host

---

## 3. 实时连接建立

### 客户端（移动端/Web）

1. 调用 `POST /v1/realtime/ws-ticket`（Bearer 认证）→ 获得 60s 一次性 ticket
2. WebSocket 升级: `/ws/client?ticket=<ticket>`
3. 接收 `Hello` 帧，自动订阅 `account:<account_id>` topic
4. 显式订阅额外 topic: `conversation:<id>`, `agent_session:<id>`
5. 恢复能力: `subscribe` 帧携带 `resume_after = { topic: last_durable_seq }`

### Host 守护进程

1. 调用 `POST /v1/host/realtime/ws-ticket`（HostInstallationPrincipal）→ 获得 ticket
2. WebSocket 升级: `/ws/host?ticket=<ticket>`
3. 自动订阅 `host:<installation_id>` topic
4. 接收 host 命令（start_agent, approval_decision 等）
5. 回送命令结果: `host_command_ack`, `host_command_result`
6. Agent 输出上行: 在线发送 `HostIngestLiveBatch`；断线重连后先发送 `HostGapManifest`，历史正文由后端通过 `PullIngestRange` 主动拉取。

### 连接策略

同一 `(principal, installation_id)` 只保留最新连接；旧连接收到 4401 关闭码。

---

## 4. 项目与对话设置

1. 账户创建项目: `POST /v1/projects/create` — 创建 `projects` 行
2. 账户创建/查找对话: `POST /v1/conversations/ensure-direct` 或创建群组对话
3. 链接对话到项目: `POST /v1/projects/conversations/link`

---

## 5. Agent 会话生命周期

### 启动 Agent 会话

1. 移动端调用 `POST /v1/agent-sessions/start`，携带 agent_id、conversation_id、可选 project_id、幂等键
2. 后端事务:
   - 验证调用者是对话成员
   - 选择 host_installation_id（显式指定或默认）
   - 创建 `agent_sessions` 行（status=pending）
   - 追加 `DurableEvent::AgentSessionStarted` 到 `durable_event_log`
   - 入队 `outbox_events`
   - 入队 `host_commands`（method=`agent_session.start`）
3. Outbox dispatcher 发布到 `host:<installation_id>` topic
4. Host gateway 接收命令，daemon spawn agent 子进程

### Agent 执行与流式传输

1. daemon 运行 agent，将 `RawIngest` 降为 canonical `IngestChunk`，预分配 host-local `seq`。
2. daemon 同时批量写本地 SQLite，并在 WS 在线时通过 `/ws/host` 发送 `HostIngestLiveBatch`。本地写库不等待 relay outbound queue。
2. 后端:
   - 按 `(host_device_id, thread_id, seq)` 幂等写入 `raw_events`
   - 同 key 同 checksum 视为重复；同 key 不同 checksum 是不变量错误
   - 将 chunk 内 projection 发布为 `StreamEvent`
   - Client gateway 推送到订阅客户端
3. Agent 轮次完成时:
   - daemon 回送 `host_command_result`
   - 后端更新 `agent_turns`，写入 `DurableEvent::AgentTurnAppended`
   - 通过 outbox → 所有订阅客户端

### 审批流程

1. agent 需要用户审批工具调用:
   - daemon 触发: 后端创建 `approval_requests` 行（state=pending, deadline）
   - 发布 `DurableEvent::ApprovalRequested` → 客户端显示审批提示
2. 用户响应:
   - `POST /v1/approvals/respond`（approve/deny）
   - 后端转换审批状态，创建 `host_command`（method=`approval.decision`）通知 host
3. 超时: `ApprovalTimeoutJob` 检测过期 deadline → 自动解析为 Timeout
4. 断连: `StaleSessionSweeperJob` 检测所有账户安装离线 → 解析为 Disconnected

### opencode question 流程

1. opencode 需要用户在多个选项中选择或填写答案时发出 `question.asked`
2. daemon 将该事件作为 `Raw(kind="opencode/question.asked")` projection 上行，TUI/mobile 展示问题与选项
3. 用户提交答案:
   - TUI daemon 模式调用 `minos_local_respond_opencode_question`
   - mobile 调用 `POST /v1/agent-sessions/respond-opencode-question`
4. 后端把 mobile 请求转换为 host command `minos_respond_opencode_question`
5. daemon 调用 `AgentManager.respond_opencode_question()`，opencode driver POST `/question/{requestID}/reply`，body 为 `{ "answers": [[...]] }`

### 读取历史（冷回放）

- 客户端调用 `POST /v1/agent-sessions/read-turns`
- 支持轮次级元数据和轮次内部流式事件切片回放

### 停止 Agent 会话

1. 移动端调用 `POST /v1/agent-sessions/stop`
2. 后端: session → `stopping`，入队 host_command（`agent_session.stop`）
3. daemon 终止 agent 进程
4. 后端: session → `ended`，触发 `DurableEvent::AgentSessionEnded`

---

## 6. 对话与消息

1. 发送消息: `POST /v1/conversations/send-message`
   - 事务: `conversation_messages` + `message_mentions` + `DurableEvent::ConversationMessageAppended`
2. Agent 消息: 由 `AgentSessionService` 内部触发（sender_kind=agent）
3. 消息撤回: 5 分钟窗口内
4. 已读状态: `conversation_reads` 表
5. 推送通知: `PushFanoutJob` 基于在线状态、偏好、免打扰和冷却决策

---

## 7. TUI 交互流程

### 嵌入式模式

TUI 直接管理 agent 子进程（进程内 `AgentManager`），无需后端或 daemon。

### Daemon 模式

TUI / Desktop 通过 JSON-RPC 连接 `minos-daemon`:
1. 读取 `~/.minos/run/tui-daemon-rpc.json` 发现 daemon WS 地址并 `minos_local_health`
2. 失败时进程内托管 `DaemonHandle::start_with_local_rpc`，用 `local_rpc_url()` 直连（不依赖再读 discovery）
3. 连接并订阅 ingest + manager events
4. 所有操作通过 `minos_local_*` JSON-RPC 方法

### Host-local agent session resume（TUI / Desktop）

与云端 `agent_sessions` 无关：host 本地 SQLite `threads` 行在 managed daemon 退出后仍可复用。

| 场景 | 行为 |
|------|------|
| 退出时 idle | 落库 `suspended` + `needs_continue=0`；下次 `resume_thread` reattach，用户下一条消息续历史 |
| 退出时 running | 落库 `suspended` + `needs_continue=1`；打开 conversation 时 **最多一个** top-level session `resume_thread(auto_continue=true)` 注入 CONTINUE |
| 用户立即发消息 | send 路径 `resume_thread(auto_continue=false)` + `send_user_message`；`take_needs_continue` 清 flag，**不**注入 CONTINUE |
| 用户显式 close | `Closed`，不可 resume / 默认复用 |

常驻 detached daemon + Soft/Hard quit 见 `docs/superpowers/specs/2026-06-23-daemon-lifecycle-and-agent-mention-scope-design.md`（后续）；本路径保证 process-death recovery 正确。

### 群聊协调

TUI 支持多 agent 在群聊中协作:
1. Agent 通过 MCP 工具 `minos_chat.request_agent_help` 请求其他 agent 协助
2. TUI tick 泵处理 MCP 命令 → 启动目标 agent → 转发 prompt
3. Agent 结果自动回传群聊

---

## 8. 重连与恢复

### 客户端重连

1. 重新请求 WS ticket，升级新 WS 连接（旧连接收到 4401）
2. 订阅时携带 `resume_after = { topic: last_durable_seq }`
3. Gateway 从 `durable_event_log` 回放缺失事件
4. 如 cursor 超出保留窗口: gateway 返回 `snapshot_required` → 客户端通过 read API 重建

### Host daemon 重连

- 指数退避（1s → 30s 封顶）
- WS 断线期间 Agent 继续输出，本地 SQLite 继续写入；上传通道不保留正文 payload backlog，只记录 backend ack 水位和本地 dirty range。
- WS 重连并订阅 `host:<id>` 后，host 先发 `HostGapManifest` metadata：
  - 每个 thread 的 `backend_acked_seq`、`local_from_seq..local_to_seq`
  - bytes、event_count、first/last timestamp、running 状态
- 后端记录“history partial, available_on_host=true”，并立即按 manifest range 通过同一条 host WS 发送 `PullIngestRange`。
- host 从本地 SQLite 读取 range，回 `HostIngestPullResponse`；后端持久化后只按连续 raw event 前缀发送 `PullAck`。

---

## 9. 端到端数据流示例

### 用户从手机发送消息到 Mac Agent

```
[手机 Flutter UI]
  用户在 ThreadViewPage 输入 "修复这个 bug"
  → ActiveSessionController.send()
    → ThreadRepository.sendUserMessage()
      → MobileClient.send_user_message() [Rust, FRB]
        → MobileHttpClient POST /v1/agent-sessions/send-input

[后端]
  接收请求 → 创建 host_command（agent_session.send_input）
  → 入队 outbox_events
  → Outbox Dispatcher 发布到 host:<id> topic
  → Host Gateway 推送到 daemon WS

[Mac daemon]
  WS 接收 host_command → AgentGlue → AgentManager
  → 写入 stdin / JSON-RPC 到 agent 子进程
  → Agent 输出流 → RawIngest(raw bytes / artifact ref)
  → IngestCoalescer 分配 seq、生成 projection/checksum/IngestChunk
  → EventWriter 批量写 SQLite events(body metadata + projection_json)
  → LiveUploadWorker 在线发送 ClientFrame::HostIngestLiveBatch
  → 断线时只记录 gap，重连发送 HostGapManifest

[后端]
  接收 HostIngestLiveBatch
  → 幂等 INSERT raw_events
  → 发布 projection StreamEvent 到 agent_session:<id> topic
  → Client Gateway 推送到手机 WS
  → 需要历史缺口时发送 PullIngestRange，host 回 HostIngestPullResponse

[手机 Flutter UI]
  UiEventFrame (TextDelta/ToolCallPlaced/etc. with DisplayPayload)
  → ThreadEventsProvider 追加事件
  → ThreadViewPage 渲染 preview 并重建
  → 用户看到 agent 回复流式输出
```
