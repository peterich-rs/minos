# Minos 消息架构体系

> 本文档是 **Server + 全端** 的消息/实时/投递架构 SSOT。  
> 它把 Minos 映射到成熟 IM（即时通讯）行业标准模型，并锚定当前实现：`minos-backend` 中枢、Host ingest、Durable/Stream 双通道、Transactional Outbox、端侧重连水位。

相关文档：

| 文档 | 关系 |
|------|------|
| [architecture-overview.md](architecture-overview.md) | 顶层拓扑与子系统入口 |
| [architecture-backend.md](architecture-backend.md) | 网关、store、jobs 实现细节 |
| [architecture-daemon.md](architecture-daemon.md) | Host 本地 SSOT、relay 出站 |
| [architecture-mobile.md](architecture-mobile.md) / [architecture-desktop.md](architecture-desktop.md) / [architecture-web.md](architecture-web.md) | 各端 UI 与投影 |
| [architecture-business-flow.md](architecture-business-flow.md) | 端到端业务步骤 |
| [architecture-shared-crates.md](architecture-shared-crates.md) | `minos-protocol` / `minos-ui-protocol` 线类型 |
| [adr/0021-agent-as-conversation-bot-participant.md](adr/0021-agent-as-conversation-bot-participant.md) | 产品决策：消息驱动协作；Host 为 runtime port |
| ADR 0004 / 0009 / 0011 | JSON-RPC、Broker 拓扑等历史决策 |
| ADR 0020 | Account vs Host 人机分权（与 bot participant 正交） |

线类型源码：`crates/minos-protocol/src/realtime.rs`。

**文档权威阶梯（冲突时）**：ADR 0021 决定产品边界；本文定义消息、身份与投递不变量；各端 `architecture-*.md` 定义其实现边界；最终以代码、共享线类型和测试为准。实施计划、review 与阶段任务不构成权威来源。

---

## 1. 定位：协作 Conversation IM 为主轴，Agent 是对话内 bot 成员

### 1.1 产品主语

Minos 的**产品核心是对话协作**（Project → Conversation → Timeline），形态对齐成熟 IM（Slack / 企微 / Discord）：同一账号多端连续聊天；**不是**「远程 Agent 运维台」，也**不是**以 HostCommand 总线当协作主协议。

- **主场景**：人与人、人与 **bot 成员（Agent）** 在同一 Conversation 时间线协作（发消息、@、回复、撤回、reaction、未读与通知）。  
- **Agent / Bot**：Hub 上的 **全局唯一 bot 用户**（稳定 `agent_id` + 数字肉身：模型/推理/系统提示词/runtime 等）——可被拉入多个 conversation（membership），可被 @、可发气泡；**不是**真人 Account（无登录 JWT）。类比 Slack Bot / Discord Bot。每个 conversation 仅为该 bot 维护独立 **session**（执行上下文），进群不新建 bot。
- **协作驱动**：**纯消息**。`@人` 与 `@agent` 都是 mention → participant delivery；bot 的「执行身体」是 Host 上的 daemon/CLI，属于 **runtime 适配**，不是第二套协作模型。  
- **Host / Daemon**：bot 落地的算力与工具边界；对用户是能力面，不是每天打开的主隐喻。`/ws/host` 是 **执行面传输**，不是 IM 主轴。本地 agent profile 可作 cache，**不是**多端 bot 身份 SSOT。

Desktop：主舞台是 **Timeline + Composer**；Session / Approval 是侧栏或 Attention，不是独立产品。

成熟 IM 基础能力 **全部是一等公民**：

- 长连接、可靠投递、多端水位、会话成员、热推 + 冷拉  
- **@mention、未读、推送偏好、撤回、reaction、列表摘要**  
- 读扩散 / 写扩散的投递模型（随群规模演进）

### 1.2 三轴模型（按产品优先级）

| 优先级 | 轴 | IM 类比 | Minos 实体 |
|--------|----|---------|------------|
| **P0 主轴** | A. 协作消息 | 用户 + **bot** 聊天消息 + 群协作 | `conversations` + `conversation_members` + `conversation_agent_members` + `chat_messages` + mentions/reads/recall + `message_reactions` |
| **P1 嵌入** | B. Agent 热投影 | 直播流 / typing / 工具过程卡片 | Host ingest → `StreamEvent(ui_event)` + session transcript；关键状态落 Durable |
| **P1 嵌入** | C. Attention / @ | @人、@bot、@here、待办、审批 | 结构化 mentions、`ApprovalRequested`、列表 `unread_*` / `needs_attention`、Push |
| **P2 能力** | D. Host runtime port | bot 身体的设备信令 + 边缘日志 | `host_commands`（**adapter**）、ingest 幂等、Gap Pull |

**核心原则（产品与投递不变量）**：

1. **对话是 SSOT 容器**。列表、未读、@、reaction、agent session 都从属于 `conversation_id`。  
2. **Cloud 不跑 Agent**。云端是 **IM + participant delivery + 投影中枢**；CLI 只在用户 Host。  
3. **Hub 是多端协作 SSOT（人读气泡、@、未读、recall）**；Host 本地 SQLite 是 **Agent 原始事件 / 本地工作台** SSOT。禁止聊天气泡对等双权威。
4. **写路径先事务后推送**（Transactional Outbox）；端上静默 dual-write **不算**满足。  
5. **Attention 统一建模**（§3.4）：人 @、bot @、审批、失败 session 共享「谁需要被打扰」。  
6. **最新架构优先**（AGENTS.md Development-State Policy）。  
7. **Agent = bot participant**（[ADR 0021](adr/0021-agent-as-conversation-bot-participant.md)）：
   - 发送成功 = 用户消息落库；bot 投递进 **Agent inbox**（异步）。  
   - Runtime 调用（今日 HostCommand）是 inbox consumer 的私有适配，**不是**产品层「命令式协作」。  
8. **Agent 最终气泡写者（路径不同，Hub 权威唯一）**：  
   - **client_live**（Desktop/Mobile Account）：Hub mailbox 路径 `AppendBotMessage` 或 `TurnCompletionProjector`（CompletionWatch per `origin_message_id`）。  
   - **host_projection**：仅 Host 已执行结果的上行 provenance（如 offline runtime-port 适配后的 agent-result）；**永不**再投递 Agent inbox。  
   - 禁止 UI 扫本地时间线无 id 投影；禁止 body 软去重。  
9. **协作消息主写 = Account WS `AppendMessage`**。服务端 HTTP `POST …/messages` 可与 WS 同 domain commit（测试/工具）；**客户端不得 REST 写协作气泡**。  
10. **Bot 激活只经 Hub Agent inbox / Bot mailbox**；禁止 Desktop Composer 本地 `startAgent` fan-out。

### 1.3 命名对照（产品 · 模块 · 物理表）

协作与 bot 投递在文档/日志/代码中混用历史名。**以下为现行 SSOT 对照**（latest-only；新代码优先右列产品名，物理 rename 可后置）：

| 产品 / 领域名 | 含义 | 代码模块 / 符号（可暂留） | 物理表 / wire |
|---------------|------|---------------------------|---------------|
| **Bot identity** | 全局唯一 bot 用户 + 数字肉身 | `agents` row / `AgentSummary` / `bot_id`≡`agent_id` | `agents` |
| **Bot membership** | 某 conversation 的 bot 成员 | `conversation_agent_members` | 同左 |
| **Session** | bot 在 conversation 内执行上下文 | `agent_sessions` / daemon `sessions` | 同左 |
| **Participant** | human ∪ bot 统一读模型 | `…/participants` API | members + agent_members 聚合 |
| **Agent inbox / Bot mailbox** | bot 侧投递队列（幂等 intent） | `agent_inbox` 规划 + `store/agent_dispatch_queue.rs`（`enqueue_in_tx` 与消息同事务）；`try_agent_dispatch` 仅作已提交消息 re-drive | **`bot_message_deliveries`**（历史名 `agent_dispatch_queue` 已退役） |
| **delivery_id** | inbox 行主键 | `dispatch_id` 列可别名 | `bot_message_deliveries.dispatch_id` |
| **BotInboxDelivery** | Hub→Host 邮箱投递帧 | `ServerFrame::BotInboxDelivery` | `/ws/host` |
| **AppendMessage** | Account 协作消息主写 | `ClientFrame::AppendMessage` → `send_message` use case | `/ws/client` |
| **AppendBotMessage** | Host 上送 bot 最终气泡 | `ClientFrame::AppendBotMessage` | `/ws/host` |
| **Runtime port** | 私有执行适配（非协作协议） | `HostCommand` / `agent_session.start\|send_input` / `runtime_port_inject` | 仅 Host offline 或内部 |
| **host_projection** | 已执行结果上行的 provenance；**不**再投递 inbox | `MessageSource::HostProjection` | `chat_messages.message_source` |
| **client_live** | 真人客户端消息；可触发 inbox | `MessageSource::ClientLive` | 同左 |

**禁止混用**：

- 「远程协作」≠ HostCommand 总线  
- Agent inbox / Bot mailbox = 同一投递队列的产品名；**不要**再写「dispatch 是第二套协作协议」  
- `agent_id` 与 `bot_id` 在 wire 上指同一全局身份（新字段优先 `bot_id` / `MessageSender::Bot`）  
- daemon 本地 `bot_identities` / 历史 `agent_profiles` = **cache**，不是多端身份 SSOT  

---

## 2. 总体架构（标准 IM 分层映射）

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  端侧 Presentation / Projection                                              │
│  Mobile (Flutter+FRB) · Web (React) · Desktop Account UI              │
│  热路径: WS StreamEvent / DurableEvent   冷路径: HTTP list/read-turns/msgs  │
├─────────────────────────────────────────────────────────────────────────────┤
│  端侧 Connection & Sync SDK（成熟 IM 的「长连接 + 同步引擎」）                 │
│  minos-mobile RealtimeSession · Web RelaySocket · daemon relay_client       │
│  ticket · subscribe · resume_after · heartbeat · reconnect backoff          │
├─────────────────────────────── 公网 WSS / HTTPS ────────────────────────────┤
│  Edge: Caddy TLS (prod) / Cloudflare tunnel (可选)                           │
├─────────────────────────────────────────────────────────────────────────────┤
│  minos-backend  =  IM Access + Logic + Storage + Worker                      │
│  ┌──────────────┐  ┌────────────────┐  ┌─────────────────┐                  │
│  │ Access 层    │  │ Logic 层       │  │ Delivery 层     │                  │
│  │ HTTP /v1/*   │  │ Domain UC      │  │ RealtimeFanout  │                  │
│  │ /ws/client   │  │ 权限/幂等/状态  │  │ ConnRegistry    │                  │
│  │ /ws/host     │  │ Host Command   │  │ Subscriptions   │                  │
│  │ WS ticket    │  │ Ingest/Transl. │  │ MessageBus      │                  │
│  └──────┬───────┘  └───────┬────────┘  └────────┬────────┘                  │
│         │                  │                    │                           │
│  ┌──────▼──────────────────▼────────────────────▼────────┐                  │
│  │ Persistence: PostgreSQL (prod) / SQLite (dev)          │                  │
│  │ durable_event_log · outbox_events · raw_events · …     │                  │
│  └──────────────────────────┬────────────────────────────┘                  │
│  ┌──────────────────────────▼────────────────────────────┐                  │
│  │ Redis (prod): cache · ticket · pub/sub bus · nonce     │                  │
│  └───────────────────────────────────────────────────────┘                  │
├─────────────────────────────────────────────────────────────────────────────┤
│  Host 执行平面 (minos-daemon)                                                │
│  Agent Runtime · 本地 chat-store/SQLite · /ws/host ingest · 命令执行        │
└─────────────────────────────────────────────────────────────────────────────┘
```

对应成熟 IM 术语：

| 成熟 IM 层 | Minos 组件 |
|-----------|------------|
| Access / Conn Gateway | `realtime/gateway.rs`：`/ws/client`、`/ws/host` |
| Auth / Session | JWT + 一次性 WS ticket；`ConnectionPrincipal` |
| Logic / Biz Service | `AgentSessionService`、`ConversationService`、`HostCommandService`、`IngestUseCase`… |
| Message Store | `conversation_messages`、`agent_turns`、`raw_events`、`durable_event_log` |
| Seq / Log | 每 topic 单调 `topic_seq`（Durable）；host-local `seq`（Ingest） |
| Push / Fanout | `RealtimeFanout` + Redis MessageBus |
| Offline / Outbox | `outbox_events` + `OutboxDispatcherJob` |
| APNs/FCM 类离线推送 | `NotificationService` + `PushFanoutJob`（设备 token / 偏好 / 冷却） |
| Client Sync Engine | 各端 `resume_after` + HTTP 冷回放 + `SnapshotRequired` |

---

## 3. 角色、连接与会话模型

### 3.1 参与方

| 角色 | 连接入口 | Principal | 默认 Topic |
|------|---------|-----------|------------|
| 人类客户端（Mobile / Web / Desktop Account） | `/ws/client` | `Account { account_id }` | `account:{account_id}` |
| Host 守护进程 | `/ws/host` | `Host { host_device_id }` | `host:{host_device_id}` |

同一 `(rail, DeviceId)` **只保留最新连接**（Account IM 与 Host runtime 可共享一个 DeviceId，互不踢）；旧连接 `4401` 强制关闭（成熟 IM 的「单设备踢旧」/ 多 tab 最新胜出策略）。

### 3.2 IM 稳态：长连接 · 心跳 · Presence · 吊销

产品级稳态（formal gateway `/ws/client` · `/ws/host`）：

```
Client ──长连接──► Gateway
            ▲
            │ heartbeat（连接活着）
            │
Gateway ──推送──► 订阅方（online / offline / last_seen）
吊销 ──踢连接──► Client
```

| 能力 | 实现 |
|------|------|
| **长连接** | Ticket 升级 → `Hello` → 默认 topic 自动订阅 → 业务 Subscribe |
| **心跳** | `Hello.heartbeat_interval_ms`（25s 建议）；服务端每 15s WS `Ping` 探测；**90s 无入站活动** → close `1011 heartbeat_timeout` |
| **活动定义** | 入站 text / WS Ping·Pong / 应用层 `ClientFrame::Ping` 均刷新 liveness；`last_seen_at_ms` 节流写库（≥30s） |
| **Online（分层语义）** | **Account sync（人类 IM）** = 该 account 至少一条 account-client `/ws/client` live（实现上 Mobile 计数见 `mobile_client_session_count`；Desktop/Web 同轨）；**Host online（设备）** = installation 有 live `/ws/host`；**Agent available** = bot 在 roster 且 bound Host online。**产品主 Online（能发能收）必须以 Account sync 为准**，不得仅用 Host 冒充 |
| **last_seen** | 耐久：`devices.last_seen_at_ms`；HTTP 列表返回；连接 open/close 强制 touch（冷路径展示，不冒充 online） |
| **Presence 推送** |  ephemeral `StreamEvent{kind:presence}`：Host 上下线 → 各 linked `account:{id}`（Mobile 改设备 online）；Account client 上下线 → 各 linked `host:{id}`（Host peer list） |
| **吊销踢连接** | 同 installation 重连 `session_superseded`（4401）；auth/unlink `auth_revoked`（4401）；Host 可先收 `HostForceClose` 再 close |
| **离线 Push** | presence + 偏好 + event_id 幂等 决策（见 Plane P / Backend B1）；**非**「与 presence 脱钩」 |

```
Account client (browser / mobile)
  │  Bearer access JWT
  ▼
POST /v1/realtime/ws-ticket     # 浏览器 / WebView JS WebSocket 难设 Authorization
  │  60s 一次性 ticket
  ▼
GET /ws/client?ticket=…

Desktop Account IM (Rust host)
  │  Authorization: Bearer <access JWT>   # 无 ticket
  ▼
GET /ws/client
  │  校验 + 消费 ticket + 角色匹配
  ▼
ServerFrame::Hello { conn_id, server_time_ms, heartbeat_interval_ms }
  │  registry insert（踢旧）+ presence online
  │  **默认 topic 仅 live register**（可选 SubscribeAck）；**不** `replay_topic(0)`
  ▼
ClientFrame::Subscribe { topics, resume_after? }
  │  含 account 默认 topic + 打开的 conversation；catch-up 靠 resume_after
  │  终态还可：ClientFrame::AppendMessage（协作消息主写，见 bot-mailbox-ws-im-bus）
  ▼
SubscribeAck / SubscriptionDenied / SnapshotRequired / ChatSendAck
  │
  ├─ 热路径：DurableEvent / StreamEvent（含 presence）
  ├─ 心跳：server WS Ping ↔ client Pong；或 ClientFrame::Ping ↔ Pong
  └─ 退出：remove registry（若仍是 current）→ presence offline

Host daemon (native) — 终态（bot-mailbox-ws-im-bus-design §6 / Phase 2.5）
  │  Authorization: Bearer hit_*     # 无 host ticket 链
  ▼
GET /ws/host
  │  校验 token hash / revocation / AgentHost / installation
  ▼
ServerFrame::Hello { … }
  │  BotInboxDelivery / CancelDelivery（mailbox）
  │  Host ingest / 既有 host 控制帧
  └─ 退出 / revoke → HostForceClose + presence offline

# Host 无 ticket。Desktop 登录同事务签发 host_token（绑 DeviceId + account_id）。
# WS 401 不得一律清本地 host_token。
```

客户端断线后指数退避重连（典型 1s→30s 封顶）；冷路径 HTTP 列表用 `online` + `last_seen_at_ms` 纠偏。  
**不变量**：Hello 静默（无历史洪水）；`resume_after < retention_floor` → `SnapshotRequired`（冷重建），禁止静默空回放。  
**Host 鉴权不变量**：强身份（Link 证明 + hashed `hit_*` + revoke 踢连接）保留；Host ticket 重复链删除后，邮箱/WS IM 做完也不会再出现 self→ticket→WS 三连验。

代码：`realtime/gateway.rs`、`realtime/liveness.rs`、`realtime/presence.rs`；线类型 `PresencePayload` / `PRESENCE_STREAM_KIND`（`minos-protocol`）。

### 3.3 Topic 模型（频道 / 会话分区）

线格式：`{kind}:{partition_key}`。

| TopicKind | 示例 | 承载内容 |
|-----------|------|----------|
| `account` | `account:{id}` | 账户级通知：host 链接、会话列表级消息摘要、账户事件 |
| `conversation` | `conversation:{id}` | 对话消息 append / recall / reaction aggregate；时间线热路径 |
| `project` | `project:{id}` | 项目与对话关联、归档 |
| `agent_session` | `agent_session:{id}` | 会话生命周期、turn、**Approval Attention**、**UI 流式投影** |
| `host` | `host:{device_id}` | Host 命令下发、强制关闭、链接状态 |

权限：订阅时服务端校验 membership / ownership；拒绝则 `SubscriptionDenied`。

### 3.4 协作交互模型（@ / Attention / 撤回 / Reaction）

本节是 **conversation-first 的产品交互 SSOT**：把常见 IM 能力与 Agent 场景统一到同一套概念，避免「聊天一套、Agent 又一套」。

#### 3.4.1 统一 Attention（「谁需要被打扰」）

| Attention 类型 | 触发源 | 目标 | 现有落点 | 产品语义 |
|----------------|--------|------|----------|----------|
| **人 @mention** | 解析 `@` → **human participant** | `target_kind=account` | 表 `chat_message_mentions`（终态：多态 mention targets）；`unread_mention_count` | 经典 IM 艾特 |
| **Agent @mention（bot）** | 解析 `@` → **agent participant** | `target_kind=agent` + roster | 终态写入 mention SSOT + **Agent inbox** 投递；过渡期仍可能仅文本路由 + `conversation_agent_members` | 与 @人同一 mention 管线；**不是**平行 RPC 产品 |
| **Approval（特殊 @）** | Agent 需要权限 | 可审批的人类成员（会话/对话 ACL） | Durable `ApprovalRequested` / `ApprovalResolved`；Push category `approval`；Desktop Attention | **语义等同高优 @**：必须有人介入，否则任务卡住 |
| **Needs you / 失败** | session 失败、超时、断连 | 相关账户 | 列表 `needs_attention_count`；Attention 视图 | 非消息体的状态 Attention |
| **普通新消息** | 任意 append | 未读成员（除发送者） | `conversation_reads` + `unread_count` | 列表角标；可按偏好降噪 |

**原则**：Approval **不是**独立工单产品，而是 Conversation/Session 上的 **高优先级 Attention**，与 @ 共用：

1. **目标解析**（谁该收到）  
2. **收件箱投影**（列表角标 / Attention 页）  
3. **实时帧**（打开相关 topic 的端）  
4. **离线 Push**（偏好 + 免打扰 + 冷却；审批可绕过 quiet hours）  
5. **已读/消解**（用户打开并处理 / 他人已 resolve）

演进上可收敛为显式 `attention_items`（或等价读模型），字段大致：

```text
attention_id, account_id, kind (mention | approval | session_failed | …),
conversation_id?, message_id?, session_id?, request_id?,
created_at_ms, read_at_ms?, resolved_at_ms?
```

当前实现可继续由 `message_mentions` + approvals + 列表聚合 **派生**；产品与端侧应按统一 Attention 心智渲染。

#### 3.4.2 @Mention（人 + bot，统一管线）

**目标写路径（participant delivery）**：

```
AppendMessage / POST send-message (Account)
  → client sends structured mentions (human ∪ bot membership ids; optional start/length)
  → Hub validates membership only (body never decides delivery targets)
  → TX: chat_messages
       + mention targets (account* and/or agent*)
       + social durable + outbox
       + bot_message_deliveries when bot delivery rules match
  → Durable ConversationMessageAppended (conversation:*)
  → Account digests (account:*) for human members
  → Human: unread / Push
  → Bot: Agent inbox worker → runtime port (Host) → agent reply message
```

**投递选择（正文永不参与）**：

1. `reply_to` 指向 agent 气泡（仅 human 发送者）→ 该 bot  
2. 结构化 `mentioned_agent_ids`（membership 校验后）→ 每个唯一 bot  
3. sole-agent：membership 恰 1 human + 1 active bot，且**无**结构化 agent mentions → 该 bot  
4. 否则不 enqueue bot inbox  

`chat_message_mentions` 为多态权威表（`target_kind ∈ {account,agent}` + `ordinal` 保序）。HostCommand 仅是 Host runtime port 实现细节，不是产品协作原语。

不变量：

- **只能 @ 当前 conversation participants**（人 ∈ `conversation_members`，bot ∈ `conversation_agent_members`）。  
- **正文不决定投递对象**；客户端负责解析并发送结构化 mentions；服务端只做 membership 校验与房规。  
- 发送者对自己的 @ 不计入自己的 mention 未读。  
- 撤回后 mention 随消息处理；规划上 **recall 修正未读/mention 计数**。  
- `message_source=host_projection|system`：**永不**再投递 agent inbox（防环）。  
- 客户端校验未匹配的 agentish `@`；服务端不得用正文 soft-route 到错误 bot。  
- wire：`mentions: [{kind, id, start?, length?}]`。

#### 3.4.3 Approval ≈ 特殊 @

```
Agent tool 需权限
  → ingest / host 上报 approval request
  → 写 approval_requests + Durable ApprovalRequested (topic agent_session:*)
  → 同时应保证「可审批账户」的 inbox / Attention 可见
     （今日：session 订阅 + Push category approval；演进：写入 account topic 或 attention_items）
  → 用户 POST approvals/respond
  → HostCommand + Durable ApprovalResolved
  → Attention 消解
```

与 @ 的同构点：

| 维度 | @mention | Approval |
|------|----------|----------|
| 目标 | mentioned accounts | approvers（对话成员 / host 所有者） |
| 优先级 | 高（mention 未读） | 更高（quiet hours 例外、短 cooldown） |
| 消解 | 读到该消息 / mark read | resolve / timeout / disconnect |
| UI | 时间线高亮 + Attention | 时间线卡片 / Modal + Attention |
| 推送 | group_mention / DM | approval_required |

**不要**为审批单独发明一套与 IM 无关的「工单总线」；保持 Durable + Outbox + 同一通知决策引擎（`notifications/decision.rs`）。

#### 3.4.4 消息撤回（Recall）

**已实现**：

- API：`POST /v1/conversations/:id/messages/:message_id/recall`  
- 规则：仅发送者；窗口 **5 分钟**（`RECALL_WINDOW_MS`）  
- 存储：`conversation_messages.recalled_at_ms`（软撤回，保留行）  
- 实时：`ConversationMessageRecalled` + 成员 `AccountConversationMessageRecalled`

客户端：正文占位「已撤回」；reply 引用若目标已撤回需降级展示。  
规划：撤回后修正 unread/mention；可选「撤回并删除 reaction」；Agent/system 消息默认不可用户撤回（或仅 admin）。

#### 3.4.5 Reaction（表情回应）

| 层 | 状态 | 说明 |
|----|------|------|
| Cloud / 多端 | **已实现（Hub SSOT）** | `message_reactions` + `POST …/reactions/toggle` + Durable `ConversationMessageReactionUpdated`（**conversation topic only**） |
| Desktop UI | Hub authoritative timeline；端侧写入使用 Intent Outbox，并以 Hub ACK 或 durable echo 确认 | 非 local-only |
| Desktop 本地 workbench | daemon `LocalReaction*` | 仅 Host-local message ids；禁止与 Hub 双 SSOT |

```text
message_reactions (
  reaction_id PK,
  message_id, conversation_id, emoji,
  actor_kind CHECK (user|agent), actor_id, display_name, created_at_ms,
  UNIQUE (message_id, emoji, actor_kind, actor_id)
)
```

写路径：

```
POST /v1/conversations/:cid/messages/:mid/reactions/toggle
  body: { emoji, client_op_id }   // client_op_id = Intent Outbox entry id (C5)
  → TX: toggle message_reactions + aggregate + ensure_reaction_delivery_in_tx
  → event_id = social-reaction-{conv}-{msg}-{emoji}-{actor_key}-{action}-{client_op_id}
     （确定性；禁止 Uuid::new_v4 / at_ms；同 client_op_id 重试 → ensure_one no-op）
  → COMMIT → wake_outbox + publish ConversationMessageReactionUpdated
  → topic conversation:* only（不驱动 sidebar unread；无 account fanout）
```

**Fanout 锁定（B6.2）：conversation-only。**  
Reaction **不** 写 account topic、**不** 驱动 rail / sidebar unread。若未来要 inbox 反应提示，须另开完整 `AccountConversationReactionUpdated` 设计，禁止半拉。

不变量：

- 仅 conversation 成员可 reaction；**幂等 toggle**（同 `client_op_id` 幂等；不同 `client_op_id` 可区分并发 op）。  
- **`reactions` 聚合向量是 SSOT**；`action` 仅动画 hint。  
- 客户端 gap 后仍以最新完整 aggregate 全量替换。  
- Hub IM 消息 id → 仅 Hub toggle（Desktop/Mobile Intent Outbox `reaction_toggle`）；local workbench → 仅 daemon `LocalReaction*`（禁止 dual-write）。

#### 3.4.6 通知与降噪（Plane P 细化）

`NotificationService` + `decide()` 目标类别：`message` / `approval` / `session_ended`；偏好含 `direct_message`、`group_mention`、`approval_required`、quiet hours。

**终态策略**（**禁止** 枚举存在但 caller 不接）：

| 场景 | 推送策略 |
|------|----------|
| 普通群消息 | 默认可关；依赖未读角标；`AccountConversationMessageAppended` 触发 |
| @我 | 默认开；计入 `unread_mention_count` |
| Approval | 默认开；**绕过 quiet hours**；target = 可审批账户（须解析，非空） |
| Session ended | 默认按偏好；target = session owner |
| Reaction | 不默认 Push（conversation fanout only） |
| Agent 流式 token | **永不 Push**（Stream only） |
| Presence | 账户有 live client WS（含 grace）→ `UserOnline` Skip；**event_id 幂等** 防 outbox 重放双推 |

Push **只唤醒**；点进后靠 Durable cursor / HTTP 拉一致状态。

---

## 4. 消息平面（Planes）：IM 双通道 + Host 控制面

Minos 明确区分 **可靠性语义不同** 的消息平面——这是成熟系统避免「一切塞进一个队列」的关键做法。

### 4.1 Plane D — Durable Event（可靠、可回放、有序）

**类比**：IM 的「离线消息 + 会话增量日志」。

- 线帧：`ServerFrame::DurableEvent { topic, topic_seq, kind, payload, event_id }`
- 载荷：`DurableEvent`（typed，见 `realtime.rs`）
- 语义：
  - **序号权威**在 `topic_metadata`（`high_watermark` 永不回退；`retention_floor` 为已删 payload 上界）
  - 业务事务内写入 `durable_event_log` payload（**`topic_seq` 由 authority 分配，禁止 `MAX(log)+1`**）
  - retention 只删 payload、推进 floor，**不能**重置 watermark
  - 同事务入队 `outbox_events`
  - Outbox dispatcher → 在线订阅者推送；多实例经 Redis bus
  - Subscribe 路径：**arm replay/live barrier → 注册 live → Ack → replay ≤ HW → 按 seq drain 缓冲**
  - 客户端用 `resume_after: { topic → last_topic_seq }` 续传；`after < retention_floor`（含空日志）→ `SnapshotRequired`
  - cursor **仅在 apply/commit 成功后**推进（Mobile/Desktop）

**投递契约（fail-closed / 有序）**：

1. **Server ordered no-drop**：outbox claim 按 `(topic, topic_seq)` 排序；同批串行 dispatch。durable `try_send` / catch-up barrier 溢出时 **revoke 连接**（`Backpressure`），禁止丢帧后仍当作已投递。出站失败后客户端以 cursor 重连回放 `durable_event_log`；outbox 可在 bus publish 成功后 ack（权威在日志，不在单次 fanout）。
2. **Client hole detection**：若已有正 cursor 且收到 `topic_seq > cursor + 1`，**不得**静默 `max` 推进；清 cursor 并走 `SnapshotRequired` / REST 重建。cursor=0（含 snapshot 后）允许落到高 seq。
3. **Mobile hold 不越过**：`AwaitDartAck` 期间同 topic 禁止 `AdvanceNow`（含未识别 kind）；`ack_durable_applied` 串行化，避免并发 ack 乱序。
4. **单调 outbox_id**：完整 ULID/雪花迁移若未落地，不得依赖 UUID 字典序保证全局序；以 `(topic, topic_seq)` claim 序为准。

**典型事件**：

- 社交：`ConversationMessageAppended` / `Recalled`、`AccountConversationMessage*`
- 成员：`ConversationMemberChanged`、`AccountConversationMembershipChanged`（remove 后立即 revoke live sub）
- 会话：`AgentSessionStarted/Ended`、`AgentTurnAppended`
- 审批：`ApprovalRequested` / `ApprovalResolved`
- 控制：`HostCommandIssued`、`HostForceClose`
- 账号/Host：`HostLinked` / `HostUnlinked`、账户注册等

### 4.2 Plane S — Stream Event（低延迟、可丢、侧重 UI 热更新）

**类比**：IM 的 typing indicator / 直播弹幕 / 流式 token；**不替代** Durable 真相。

- 线帧：`ServerFrame::StreamEvent { topic, kind, seq?, payload }`
- 主用途：`kind = "ui_event"`，载荷为 `UiEventMessage`（`minos-ui-protocol`）
- 来源：Host ingest 成功落库后，**server 侧** `SessionTranslators` 将 raw 翻译为 UI 投影并 fanout
- 语义：
  - 优先延迟，可在断连窗口丢失
  - **恢复靠**：Durable 生命周期事件 + HTTP `read-turns` / 冷回放，而不是无限 Stream 缓冲

### 4.3 Plane C — Host Command（可靠下行信令）

**类比**：IM 的系统信令 / 设备指令通道，带 ACK/结果回执。

```
Client HTTP (start/stop/send-input/approval/…)
  → 事务写 host_commands + DurableEvent::HostCommandIssued
  → Outbox lane=host_command（与 social_durable 分车道 claim）
  → HostCommandOutboxJob publish → topic host:{device_id}
  → Daemon 执行
  → ClientFrame::HostCommandAck / HostCommandResult
  → ack_pending_host_command_events（成功观察后 outbox acked）
```

超时：deadline 过期 → outbox **dead_letter** + metric（禁止假成功 ack）；`poll_timed_out_commands` 同步终结 `host_commands` 行。Host 离线：命令持久化，上线后经 Durable 回放 / host_command outbox 再投递。Social fanout 永不串行等待 host ack。

### 4.4 Plane I — Host Ingest（上行原始事件，幂等、可补洞）

**类比**：边缘设备上行日志 + 中心补齐（IoT/游戏状态同步常见模式）；在 Minos 中是 Agent 输出主路径。

```
Agent CLI
  → daemon IngestCoalescer 分配 host-local seq + checksum
  → 本地 SQLite 批写（SSOT，不等待 WS）
  → 在线: HostIngestLiveBatch
  → 断线重连: HostGapManifest（仅元数据）
  → Backend PullIngestRange → HostIngestPullResponse
  → 幂等写入 raw_events(host_device_id, session_id, seq)
  → HostIngestAck / PullAck
  → 翻译 + StreamEvent fanout（及必要的 formal turn 状态）
```

幂等规则：

| 条件 | 行为 |
|------|------|
| 同 `(host, session, seq)` + 同 checksum | 重复，忽略 |
| 同 key + 不同 checksum | **不变量错误**，不重分配 backend seq |
| `agent_sessions` 无此 session | 丢弃（不自动创建 hub 投影；local-only 会话需先 start/注册） |

出站优先级（daemon）：**control → live → backfill**（避免补历史饿死实时信令）。

### 4.5 Plane H — HTTP 控制面与冷路径

所有「用户意图写」优先 REST（成熟 IM 亦常 REST/短连写 + 长连推）：

| 能力 | 路径族 |
|------|--------|
| 发社交消息 | `POST /v1/conversations/send-message` 或 `POST /v1/conversations/:id/messages`（可选 `client_message_id` 幂等） |
| Upsert 工作会话（Desktop→Hub） | `POST /v1/conversations/upsert`（client-owned `conversation_id` + title） |
| Agent start/stop/input | `POST /v1/agent-sessions/*` |
| 审批 | `POST /v1/approvals/respond` |
| 读历史 turns | `POST /v1/agent-sessions/read-turns` |
| 列表/成员/好友 | `/v1/conversations/*`、`/v1/friends/*`… |

**读模型**：列表与历史走 HTTP；打开会话后 WS 订阅补增量。

### 4.6 Plane P — 离线推送（Push）

当客户端无活跃 WS（或超出 online grace）时，根据 **presence + 偏好 + 免打扰 + event_id 幂等 + UX cooldown** 决策 APNs/FCM。  
Push **只负责唤醒**；正文一致性仍靠 Durable + HTTP 同步。

**不变量**：`UserOnline`、approval target 与 `push_dispatch_log` 都属于后端投递实现的一部分；缺少任一环时不得把推送标记为已完成。

---

### 4.7 可靠性不变量

持久化正确 ≠ 端到端可靠。所有客户端和后端都遵守以下共享契约：`client_message_id`；agent 气泡 id = `agent-result:{conv}:{session}:{origin_message_id}`；`message_seq` 排序；durable `event_id` 可重放可幂等消化。端侧实现见本文件 §7，后端事务、outbox、投递与 completion 生命周期见 §5、§8、§10。

---

## 5. 服务端投递内核（Transactional Outbox + Fanout）

### 5.1 写路径黄金模板

任何关键业务写（发消息、开会话、下发命令…）遵循：

```
BEGIN
  校验权限 / 幂等键
  写业务表 (chat_messages | agent_sessions | host_commands | …)
  APPEND durable_event_log (topic, topic_seq++)
  ENQUEUE outbox_events
  [client_live] ENQUEUE bot_message_deliveries  -- 与用户气泡同事务
COMMIT
→ OutboxDispatcher / agent_dispatch worker → RealtimeFanout / BotInboxDelivery
```

这就是业界 **Transactional Outbox** 标准解：业务与投递原子一致，避免「库已写但推丢」或「推了库没写」。

**社交消息路径（已对齐）**：`DefaultConversationService::send_message` 在同一事务内完成 `chat_messages` + mentions + `durable_event_log` + social `outbox_events` + **`bot_message_deliveries`（当房规命中）**。commit 后：social fanout + wake agent-dispatch worker + `ChatSendAck`。`try_agent_dispatch` 仅用于**已提交**消息的 re-drive（host online force 等）。规划在 `agent_inbox`；`enqueue_in_tx` 保证与气泡原子。Bot 气泡路径同理可 co-commit bot→bot hops。

参考实现：`conversations/use_case.rs`、`agent_inbox.rs`、`store/agent_dispatch_queue.rs`、`http/v1/social.rs`。

### 5.2 Fanout 引擎

`RealtimeFanout` 持有：

- `SubscriptionManager`：topic → 在线订阅者（formal gateway 推送）
- `RealtimeConnectionRegistry`：installation → 当前连接、同设备替换、在线计数 / push grace
- `StoreHandle`：补拉 / 认领  
- `MessageBusBackend`：`Inline`（单机）| `Redis`（多实例）

能力摘要：

| 方法 | 用途 |
|------|------|
| `fanout_stream_event` | 即时 StreamEvent（presence / ui_event / agent_error 等） |
| `dispatch_outbox_batch` | Social Durable 出站批处理 |
| `dispatch_host_command_outbox_batch` | HostCommandIssued Durable 出站 |

### 5.3 多实例

生产：`MINOS_MESSAGE_BUS_BACKEND=redis`。  
任一实例处理 HTTP 写 → outbox → 本机 fanout + Redis pub/sub → 其它实例对本机连接推送。  
WS 会话亲和在单实例内存；跨实例靠 bus，而不是粘会话强依赖（可后续加 sticky，但不作为正确性前提）。

### 5.4 读扩散 vs 写扩散（Fanout 策略）

成熟 IM 在「消息如何到达接收方」上常见两类（可混合）：

| 模式 | 含义 | 写时成本 | 读/在线推成本 | 典型场景 |
|------|------|----------|---------------|----------|
| **写扩散（Write fanout）** | 写路径为每个接收方物化一份 inbox/推送任务 | O(成员数) | 读自己的收件箱 O(1) 页 | 小群、邮箱、站内信 |
| **读扩散（Read fanout）** | 消息只写一份；在线订阅者按 channel 推；离线靠拉频道日志 | O(1) 写消息 | 打开/订阅时拉增量；推送给当前订阅者 | 大群、频道、Discord 式 |

#### 当前 Minos（小群 / 项目协作，混合模型）

```
                    ┌─ 写一份消息体 ──────────────────────────────┐
                    │  conversation_messages                       │
POST send-message ──┤  durable_event_log @ conversation:{id}       │  ← 读扩散主体
                    │  （打开对话且 Subscribe 的端收到）            │
                    └─ 对每个成员（或相关账户）再写 ───────────────┘
                       AccountConversationMessageAppended
                       @ account:{account_id}                      ← 写扩散摘要
                       （列表/inbox/未读提示，无需已订阅该对话）
```

| 内容 | 策略 | Topic / 存储 |
|------|------|----------------|
| 消息正文与序 | **读扩散** | 单行 `conversation_messages` + Durable `conversation:*` |
| 会话列表 / 未读提示 | **轻量写扩散** | 每成员 `account:*` 上的 Account* 事件；读模型用 `conversation_reads` 算 unread |
| @mention 索引 | **写时索引** | `message_mentions`（按消息×被@人），非全文检索 |
| Approval | **读扩散为主** | Durable `agent_session:*`；Push 写扩散到设备 token；演进可补 account Attention |
| Agent Stream UI | **读扩散 / 订阅** | `StreamEvent` → 当前订阅 `agent_session:*` 的连接 |
| Host 命令 | **定向写** | `host:{device_id}` 单目标（不是群 fanout） |

**为什么当前混合合理**：Project 协作群通常 **N 很小**（几人 + 数个 Agent），对 N 做 account 摘要写扩散成本可接受，且保证「没打开对话的端」仍能刷新会话列表——这是 IM 列表体验的刚需。

#### 规模演进（文档规划，非立即实现）

当出现 **大群 / 广播 / 组织级频道** 时：

1. **消息体保持读扩散**（永不按人复制全文）。  
2. **Inbox 写扩散改为异步**：Outbox/worker 按成员分片写 `inbox_cursors` 或只推「有更新」水位，避免请求路径 O(N)。  
3. **在线仍靠 topic 订阅**（读扩散热路径）。  
4. **@ / Approval 保持写时窄播**：只对目标集合建 Attention 行 + Push，不向全员写扩散正文。  
5. **Agent 高频 Stream 永不写扩散、永不进 Durable 全文**（已是现状）。  

决策表（实现前对照）：

| 事件 | 小群（现状默认） | 大群演进 |
|------|------------------|----------|
| 普通消息 append | conv 读扩散 + 成员 account 摘要 | conv 读扩散 + 异步 inbox 水位 |
| @mention | mention 表 + 目标高优 Push | 同左（目标集仍小） |
| Approval | session topic + Push | + account Attention 行 |
| Reaction toggle | conv 读扩散（规划） | 同左；不写 account 噪声 |
| Recall | conv + account 摘要（与 append 对称） | 同 append 策略 |

**禁止**：为「省事」把 Stream token 或完整 transcript 按账户写扩散。

---

## 6. 标识、序、幂等与水位（IM 一致性核心）

### 6.1 ID 体系

| ID | 作用 |
|----|------|
| `account_id` | 人类账户 |
| `device_id` | 设备维度连接与踢旧（Account / Host 分轨） |
| `conversation_id` | 社交会话 |
| `message_id` | 单条聊天消息 |
| `chat_message_mentions.(target_kind,target_id)` | 多态 @ 目标：`account`/`agent` + `ordinal` 外观序 |
| `request_id` | Approval 请求（Attention 子集） |
| `session_id` | Agent 会话（云端 formal；从属 conversation） |
| `turn_id` / `turn_seq` | Agent 轮次 |
| `event_id` | Durable / ingest 事件稳定 ID |
| `command_id` | Host 命令 |
| `client_request_id` | 客户端写幂等键（start/send 等） |
| `conn_id` | 单次 WS 连接 |

### 6.2 序列与水位

| 序列 | 作用域 | 谁分配 | 用途 |
|------|--------|--------|------|
| `topic_seq` | 每个 RealtimeTopic | Backend | Durable 有序回放、`resume_after` |
| host-local `seq` | `(host, session)` | Daemon | Ingest 幂等主键 |
| `turn_seq` | session | Backend formal 模型 | 轮次列表 |
| Stream `seq`（可选） | 热流 | Backend | UI 去重/排序辅助，**非**跨断连 SSOT |

### 6.3 客户端同步状态机（标准 IM Sync）

```
                    ┌─────────────┐
         connect    │ Disconnected│
       ┌────────────┤             │
       │            └──────▲──────┘
       ▼                   │ drop / 4401
┌─────────────┐      Hello │
│ Connecting  │────────────┘
└──────┬──────┘
       │
       ▼
┌─────────────┐  resume_after
│ Syncing     │──────────────────► 推送 gap Durable
└──────┬──────┘
       │ SnapshotRequired?
       ├─ yes → HTTP snapshot / list / read-turns → 重置 cursor
       └─ no
       ▼
┌─────────────┐
│ Live        │◄──── StreamEvent + DurableEvent
└─────────────┘
```

每个端维护：

```text
topic_cursors: Map<topic, last_topic_seq>
```

重连：`Subscribe { resume_after: topic_cursors }`。

---

## 7. 端侧消息架构

### 7.1 共同模式（所有人类客户端）

```
┌──────────────────────────────────────────────────────────┐
│ UI / ViewModel (Riverpod / Zustand / React state)        │
├──────────────────────────────────────────────────────────┤
│ Projection 层                                            │
│  · Inbox 列表投影 (sessions / conversations)             │
│  · Thread timeline (user / assistant / tool / reasoning) │
│  · 本地 optimistic UI（可选）+ 服务端确认纠偏            │
├──────────────────────────────────────────────────────────┤
│ Sync / Realtime SDK                                      │
│  · WS 生命周期 · cursor · frame dispatch                 │
│  · HTTP repository（写意图 + 冷读）                      │
├──────────────────────────────────────────────────────────┤
│ Secure storage: tokens / installation identity           │
└──────────────────────────────────────────────────────────┘
```

**写**：HTTP（可带 `client_request_id` 幂等）。  
**热读**：WS `StreamEvent` / `DurableEvent`。  
**冷读**：HTTP list / messages / `read-turns`。  
**绝不**把未投影的 local-only Desktop 会话谎称为云端可见（honesty UX）。

### 7.2 Mobile（Flutter + minos-mobile）

| 组件 | 职责 |
|------|------|
| `MobileHttpClient` | REST 写/冷读 |
| `RealtimeSession` + `FrameHandler` | WS 帧 |
| `SubscriptionManager` | topic + seq cursor |
| `ReconnectController` | 退避；前后台策略 |
| Dart providers | `socialConversationProvider`（Hub 协作 IM）/ `threadEvents` / `activeSession`（执行 transcript） |
| 协作 IM UI | `SocialChatPage` → `ConversationMessageRow`：**Slack/Buzz 全宽左对齐**（与 Desktop `MessageChrome` 同构）；day divider + 10min 分组；长按引用/复制/撤回 |
| Agent transcript UI | `ThreadViewPage`：stream / tool / reasoning 深链面，**不是**多端协作气泡 SSOT |

典型上行（协作）：`sendMessage` → Hub `POST …/messages`。  
典型上行（Agent）：`sendUserMessage` → `POST .../send-input`。  
典型下行（协作）：Durable / list messages → `SocialChatMessage` 列表。  
典型下行（stream）：`StreamEvent{ui_event}` → FRB → session timeline 原地更新。

### 7.3 Web

`RelaySocket` + Zustand：处理 `ui_event_message` / `social_message` 等，与 Mobile 共享同一 backend topic 语义。

### 7.4 Desktop（Tauri Host Console）

双栈：

1. **本地（Agent 执行面）**：JSON-RPC → daemon → 本地 SQLite（**Agent 原始事件 / tool / git / session transcript** SSOT）。  
2. **云端（协作 IM）**：Account 登录 + Host Link 后，daemon `/ws/host` 上行 ingest；Account UI 走 `/ws/client` 消费 **Hub 协作气泡**。

**SSOT 不变量（与 §1.2 #3 一致）**：

| 数据 | SSOT | Desktop 角色 |
|------|------|----------------|
| 人读的聊天气泡（user / agent 最终文本、@、未读、recall） | **Hub** | 投影 / cache / optimistic；**不是**与 Hub 对等权威库 |
| Agent 原始事件与本地工作台 | **Host daemon SQLite** | 主权威 |
| Session 热流 | Host ingest → Hub Stream | 侧栏 / 底栏，不写扩散进 chat 气泡 |

**目标读路径（Linked）**：打开 conversation → `Subscribe conversation:{id}` + `resume_after` → 冷拉 gap（`before_seq` / `after_seq`）；UI merge「Hub 气泡 + 本地 tool/git 卡」（**同 id 相等**；禁止 body 软去重）。  
**目标写路径（Linked）**：Composer → Account WS **`AppendMessage`**（`client_message_id` 幂等 + `client_live`）→ 客户端 Outbox 可重试；bot 激活经 Hub Agent inbox；Agent 最终气泡由 **AppendBotMessage** / TurnCompletionProjector；id = `agent-result:{conv}:{session}:{origin_message_id}`。

Hub 协作消息、客户端同步与后端编排均以本文件的 SSOT、Outbox、cursor 和 participant-delivery 不变量为准。

#### 7.4.1 当前实现（Phase 2–5 已收敛主体）

| 投影 | 触发 | Hub API / 路径 | 状态 |
|------|------|----------------|------|
| Conversation 壳 + agent roster | 创建 / 改标题 / 列表 / 起 session | `POST /v1/conversations/upsert` | ✅ 保留 |
| Host runtime → cloud agent | 首次 resolve runtime | `POST /v1/agents/ensure-host-runtime` + `source=host_runtime` | ✅ |
| 用户气泡（Desktop / Account live） | 本地工作台 append + Account WS **`AppendMessage`**（`client_live`） | 无 REST 协作写；同 domain commit | ✅ bot 激活仅经 Hub Agent inbox / Bot mailbox；**禁止** Composer 本地 fan-out |
| 用户气泡（Mobile / client_live） | Account WS **`AppendMessage`** → **Agent inbox**（表 `bot_message_deliveries`）→ `BotInboxDelivery` / runtime port | 无 REST 协作写 | ✅ 落库后 ACK；投递异步；仍需 **live Host** 才执行 |
| Agent 气泡（client_live） | Hub `TurnCompletionProjector`（CompletionWatch per **origin_message_id** → last-segment） | `insert_agent_message_with_session` 幂等 `agent-result:{conv}:{session}:{origin}` | ✅ B4；禁止 session 单 slot 覆盖 |
| Agent 气泡（Desktop-native） | 本机 daemon 写工作台 `agent-result:…`；Desktop Outbox **`host_projection`** 上行（仅 `isCanonicalAgentResultId`） | `POST …/agents/message` + 同 id | ✅ 可信上行，非 UI 扫时间线；Hub 不二次投递 |
| 跳过二次 bot 投递 | **仅** `message_source=host_projection\|system` | — | ✅ 防环（provenance 门控） |
| Hub → Desktop 读 | **Sync Engine**：`account:*` + 打开会话 `conversation:{id}` + `resume_after` / SnapshotRequired | Durable + `messages/query` | ✅ **不再** `daemon_append` 云端 IM |
| 冷路径 gap | 打开 tail + 上翻 `before_seq`；Snapshot / 前向补洞 **`after_seq`** | `POST …/messages/query` | ✅ **C3 已接通**（Desktop range reconcile + Mobile loadOlder / Snapshot） |
| 撤回 | Hub `POST …/messages/:id/recall` + Durable `*Recalled` | conversation + account topics | ✅ |
| Reaction | Hub toggle + conversation durable | Hub API + Intent Outbox | Desktop/Mobile `reaction_toggle` outbox + B6 `client_op_id` 幂等；conversation-only fanout |
| 未读 / mark-read | Linked 打开会话 → `POST …/read` body `{ read_up_to_message_seq }`（客户端 observed watermark，服务端单调 MAX clamp） | Hub + local count | ✅ **observed-seq**；禁止服务端静默标到最新 |
| Mobile `@agent` → bot 投递 | 消息落库后 **Agent inbox** + CompletionWatch(per origin×session) | participant delivery | 异步 enqueue；**多 @ fan-out**（`UNIQUE(origin, agent_id)`）；watch 键 = `{origin}:{session_id}`。物理表 `bot_message_deliveries` |
| Agent 表情互动 | teamwork MCP `react_to_message` → daemon 本地 reaction | Host workbench | ✅ **硬门禁**：仅允许对 **@ 了该 agent** 的消息；actor_kind=`agent` |
| Session 生命周期 | `session_lifecycle` job：失联 host → session `failed` + durable end；watch TTL → 失败气泡 + remove | Backend **B5** | ✅ 非 COUNT-only |

**Desktop Sync 状态机**（`cloud-realtime.ts`）：`Disconnected → Connecting → Syncing → Live`；per-topic `topic_seq` 持久化 `localStorage`（`minos.cloud.topic_cursors.v1`）；重连 `Subscribe { resume_after }`；`SnapshotRequired` → **range reconcile**（`after_seq=maxLoaded` forward fill + latest page merge，保留已加载窗口；禁止 clear-only）。`focusedConversationId` ≠ timeline `hasWindow`：`loadTimeline` hydrate-only（不写 focus、不 mark-read）；focus/mark-read 在 Timeline 打开路径 + focused 入站 400ms debounce。

**Phase 6.0（已落地）**：

- Postgres 社交 schema 对齐 SQLite：`agents`（`source` / `host_runtime` 唯一索引）、`chat_messages`、`chat_message_mentions`、`conversation_agent_members`、friends、`raw_events` / `sessions`（latest-only wipe 升级）  
- Desktop Hub 重建 / 打开 / loadTimeline **统一** `mergeCloudAndLocalTimeline`（禁止 quiet-tail 把本地 chat 气泡合回）  
- 删除残留 dual-write API：`daemon_append_conversation_message`、timeline 全量 project、agent dual-write no-op、hub→daemon append 路径  

**已收敛（正确性地基）**：

- 社交写路径 Transactional Outbox（`chat_messages` + attachments + durable + social outbox + `bot_message_deliveries` 同事务；commit 后 fanout/wake/Ack）  
- Hub `chat_messages.message_seq` + `messages/query` `before_seq` / `after_seq`  
- `conversation_reads.last_read_seq` 作为未读边界；mark-read 提交 **observed** `read_up_to_message_seq`  
- `topic_metadata` 序号权威 + retention floor + Subscribe replay/live barrier  
- per-conversation FIFO outbox（Desktop/Mobile）  
- 持久 Push lane（`push_dispatch_queue`）+ 持久 `completion_watches`（启动 hydrate）  
- TurnCompletionProjector：dispatch 时 arm watch，host ingest 事件驱动投影（2s settle 复检），不再 100ms 无限轮询  
- 群成员 owner/admin 角色 + remove 后 revoke subscription + durable membership 事件  

Review 与实施追踪属于 PR/issue，不作为仓库架构文档的一部分。

**仍待 / 已知缺口**：

- Desktop Linked **会话列表**仍以 daemon 为主（Hub inbox 列表 SSOT 未完成）  
- Desktop IM outbox：**Tauri SQLite**（`im_outbox.sqlite3`，fail-closed；localStorage 仅一次性迁移）  
- Manager lifecycle lag：**snapshot 对账**（manager live + SQLite mid-flight → reaped；completion redrive）  
- Web 非正式 IM 客户端  

- Account topic 重连 resume（网关 auto-sub 仍可能从 0 回放）  
- Reaction Intent Outbox / Mobile UI / B6 event_id 确定性：**已实现**（见 IM Reliability B6/C5）  

- Desktop 撤回 UI 按钮（API + realtime 已通）  
- Multi-instance presence / CompletionWatch 跨实例（当前 in-memory registry，单 worker 假设）  
- Outbox LISTEN/NOTIFY、durable retention、Redis Streams 等运维增强  



**身份不变量**：本地 bin 名（`codex`/`claude`/…）**不得**当作 cloud `agent_id`；必须经 ensure-host-runtime 映射。  
Local-only session 在 Host **未** Link 时不得对 Mobile 谎称可见（honesty UX）。

### 7.5 Daemon（Host）

| 职责 | 机制 |
|------|------|
| 本地事件 SSOT | SQLite（chat-store / event writer） |
| 在线上传 | `HostIngestLiveBatch` |
| 缺口宣告 | `HostGapManifest` |
| 被拉历史 | `PullIngestRange` → `HostIngestPullResponse` |
| 执行控制 | 消费 `HostCommandIssued`，回 `Ack`/`Result` |
| 出站队列 | control / live / backfill 分车道 |

**本地写库不阻塞于 relay 出站**——成熟边缘设备模式：先落盘再 best-effort 上报。

---

## 8. 端到端链路（主路径）

### 8.1 社交发消息（经典 IM 路径，含 @）

```
Client                Backend                         Peers / 被@者
  │                      │                              │
  │ POST send-message    │                              │
  │  (+ reply_to?)       │                              │
  │─────────────────────►│ TX:                          │
  │                      │  messages + message_mentions │
  │                      │  durable(conversation:*)     │
  │                      │  durable(account:*) × members│
  │                      │  outbox                      │
  │◄──── 200 + message   │                              │
  │                      │ Outbox fanout                │
  │◄════════ WS ═════════╪═════════════════════════════►│
  │  ConversationMessage │  AccountConversationMessage  │
  │  Appended            │  Appended（列表/未读）       │
  │                      │  Push?（@ / 偏好 / 离线）    │
```

撤回：`POST .../recall` → 同构 TX + `*Recalled` 双 topic。  
Reaction（规划）：`POST .../reactions/toggle` → 仅 `conversation:*` 读扩散（默认不 Push）。

### 8.2 对话内启动 Agent + 流式投影（嵌入能力，非独立产品）

```
Mobile/Web                 Backend                      Daemon/Host                 Agent
   │                          │                              │                        │
   │ POST agent-sessions/start│                              │                        │
   │─────────────────────────►│ sessions + host_commands     │                        │
   │                          │ + Durable HostCommandIssued  │                        │
   │                          │──────── WS host topic ──────►│                        │
   │                          │                              │ spawn / attach         │
   │                          │                              │───────────────────────►│
   │                          │                              │◄── stdout / ACP /… ────│
   │                          │                              │ seq++ / SQLite         │
   │                          │◄── HostIngestLiveBatch ──────│                        │
   │                          │ raw_events 幂等              │                        │
   │                          │ translate → UiEvent          │                        │
   │◄══ StreamEvent ui_event ═│                              │                        │
   │   (agent_session topic)  │                              │                        │
   │                          │ turn complete → Durable      │                        │
   │◄══ DurableEvent ═════════│                              │                        │
```

### 8.3 审批 = 高优 Attention（双向信令）

```
Agent 需要权限 → ingest approval/request
  → Backend 写 approval_requests + Durable ApprovalRequested
  → 在线端：agent_session topic / Attention UI（特殊 @）
  → 离线：Push category approval（可绕过 quiet hours）
  → POST /v1/approvals/respond
  → host_command approval.decision
  → Daemon 继续 / 中止
  → Durable ApprovalResolved → Attention 消解
```

超时 / 全端离线：后台 Job 解析为 Timeout / Disconnected。  
产品上与 §3.4.3 同构：目标解析 → 通知 → 消解，而不是独立工单系统。

### 8.4 断线与补洞

**Client 断 WS**：cursor 保留 → 重连 `resume_after` → Durable 补齐；Stream 缺口靠 `read-turns`。  

**Host 断 WS**：本地继续写 SQLite → 重连 `HostGapManifest` → Backend 调度 `PullIngestRange` → 连续前缀 ACK。

---

## 9. 存储与保留模型

| 存储 | 角色 | 保留倾向 |
|------|------|----------|
| Host SQLite | Agent 原始事件长期 SSOT | 长（用户机器） |
| `raw_events` | Hub 侧幂等 ingest + 投影源 | 中（可按 retention job 裁剪） |
| `durable_event_log` | 多端可靠增量 | 中；低于 floor 则 SnapshotRequired |
| `outbox_events` | 投递工作队列 | 短；认领后清理 |
| `conversation_messages` / `message_mentions` / `conversation_reads` | 协作消息读模型 | 长 |
| `message_reactions`（规划） | reaction 聚合源 | 长 |
| `agent_turns` / `approval_requests` | Agent 与 Attention 读模型 | 长 |
| `thread_sync_state` | Host↔Backend 水位与 gap 元数据 | 中 |
| 端内存 cursor | 热同步 | 进程/本地持久化策略由各端定 |

`RetentionCleanerJob` 等后台任务负责过期清理；具体天数以配置/实现为准，产品策略倾向：**Hub 中短期投影 + Host 深度历史**。

---

## 10. 可靠性与失败语义一览

| 场景 | 期望行为 |
|------|----------|
| 客户端重复 POST（同 idempotency key） | 返回同一业务结果，不重复副作用 |
| 重复 LiveBatch chunk | 幂等忽略 |
| Outbox 投递失败 | 重试认领；不丢 durable |
| 订阅落后过多 | `SnapshotRequired`，强制冷同步 |
| Host 离线时下发命令 | 命令持久；上线后投递；超时 Job 失败 |
| 未注册 session 的 ingest | 丢弃；不污染云端列表 |
| 旧连接被踢 | 4401；客户端换新 ticket 重连 |
| 多端同时打开同一 session | 同 topic 广播；各自 cursor |

**明确不做（当前）**：

- 端到端加密消息体（ADR：MVP 无 E2EE；中枢可见业务载荷）  
- 以 Stream 作为跨断连的唯一真相  
- Cloud 执行 Agent  
- 大群百万级写扩散（见 §5.4 演进；当前按小群项目协作设计）  

---

## 11. 与成熟 IM 方案的对照表

| 成熟方案要素 | 业界常见实现 | Minos 对应 |
|--------------|--------------|------------|
| 会话 / 成员 | 群、频道、DM | `conversations` + `conversation_members` + agent members |
| 长连接接入 | TCP/WS/QUIC Gateway | Axum `/ws/*` + ticket |
| 消息有序 | per-channel seq | `topic_seq` / host `seq` |
| 可靠投递 | 落库 + ACK + 重传 | Durable + Outbox + ingest ack |
| 离线消息 | 同步服务 / 漫游 | `resume_after` + durable log + HTTP |
| 多端同步 | 多设备 cursor | 每 installation 独立连接 + 同 topic 广播 |
| @mention | 结构化 mention + 未读 | `message_mentions` + `unread_mention_count` |
| 审批 / 待办 | 可操作通知 | `ApprovalRequested` 作为高优 Attention（特殊 @） |
| 撤回 | soft delete + 广播 | `recalled_at_ms` + `*Recalled` 事件 |
| Reaction | emoji 聚合 | Desktop 本地已有；云端规划见 §3.4.5 |
| 未读 | 每会话游标 | `conversation_reads` + list 聚合 |
| 写扩散/读扩散 | 混合 | §5.4：正文读扩散 + 成员 account 摘要写扩散 |
| 信令 vs 消息 | 分通道 | Plane C/D/S/I/P 分离 |
| 推送 | APNs/FCM + 偏好 | Notification / decision / PushFanoutJob |
| 幂等 | client msg id | `client_request_id` + ingest checksum |
| 网关无状态 + 中心状态 | Conn 层薄、Logic 厚 | Gateway 薄；状态在 PG + Redis |

Minos 相对「纯聊天 IM」的**增量**（仍挂在 Conversation 上）：

1. **Host 作为特殊超级设备**（可执行、可上行高频 raw）。  
2. **UI 协议投影**（`UiEventMessage`）把异构 Agent 协议统一给多端。  
3. **Local-only vs Cloud-visible** 双可见性矩阵（见 projection-sync 域文档）。  
4. **Approval / Agent session** 进入统一 Attention，而不是旁路工单。

---

## 12. 协议帧速查

### Client → Server（`ClientFrame`）

| 帧 | 角色 |
|----|------|
| `Subscribe` / `Unsubscribe` | 频道订阅与水位 |
| `Ping` | 心跳 |
| `HostCommandAck` / `HostCommandResult` | 命令回执（Host） |
| `HostIngestLiveBatch` | 实时上行 |
| `HostGapManifest` | 缺口元数据 |
| `HostIngestPullResponse` | 补洞正文 |
| `HostStreamEvent` | 遗留/次要流（新路径以 LiveBatch 为准） |

### Server → Client（`ServerFrame`）

| 帧 | 角色 |
|----|------|
| `Hello` | 连接参数 |
| `SubscribeAck` / `SubscriptionDenied` / `SubscriptionLimitExceeded` | 订阅结果 |
| `DurableEvent` | 可靠事件 |
| `StreamEvent` | 热流 |
| `SnapshotRequired` | 强制冷同步 |
| `HostForceClose` | 踢 Host |
| `HostIngestAck` / `PullIngestRange` / `PullAck` | Ingest 控制 |
| `Pong` / `Error` | 心跳与错误 |

完整字段与枚举：`crates/minos-protocol/src/realtime.rs`。

---

## 13. 运维与扩展边界

| 维度 | 现状 | 演进方向（非承诺） |
|------|------|-------------------|
| 部署 | 单 VPS monolith + PG + Redis | HttpOnly / WorkerOnly 拆分已支持配置 |
| 水平扩展 | Redis bus | 可加 conn 路由 / sticky |
| 可观测 | tracing、metrics、request-id | 按 topic/session 投递延迟直方图 |
| 保留 | Job 清理 | 显式 TTL 产品策略 |
| E2EE | 无 | 独立 ADR |
| 社交群规模 | 小群/项目向（混合 fanout） | 大群：inbox 异步化，正文保持读扩散（§5.4） |
| Reaction 云端 | 未接（Desktop 本地有） | `message_reactions` + Durable + 端对齐 |
| Attention 统一读模型 | 派生（mention / approval / 列表计数） | 可选 `attention_items` 物化 |

生产强制：`external-sql` + Redis cache + Redis message bus（见 backend / vps-deploy 文档）。

---

## 14. 文档维护规则

- 改 wire 帧、topic、幂等键、outbox 语义时：**先改代码与测试，再改本文**。  
- 子系统实现细节仍落在各自 `architecture-*.md`；本文只保留 **跨端消息体系** 与不变量。  
- 与 L0 云身份 / Host Link / 投影域冲突时，以最新代码 + 本文件「不变量」段落为准，并回写 program 规格。

---

## 15. 一句话架构总结

> **Minos 消息体系 = 以 Conversation 协作为产品主轴的成熟 IM（成员、消息、@、未读、撤回、reaction 规划、统一 Attention、读/写混合 fanout、Topic + Durable + Outbox + 多端 Cursor），在对话内嵌入 Agent 热投影与 Host 执行平面（Ingest / Command），用 Stream 服务实时体验、HTTP 承载写意图与冷历史——而不是「远程 Agent 控制台外挂聊天」。**
