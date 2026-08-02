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
| ADR 0004 / 0009 / 0011 | JSON-RPC、Broker 拓扑、Envelope 历史决策 |

线类型源码：`crates/minos-protocol/src/realtime.rs`。

---

## 1. 定位：协作 Conversation IM 为主轴，Agent 是对话内能力

### 1.1 产品主语

Minos 的**产品核心是对话协作**（Project → Conversation → Timeline），而不是「远程 Agent 运维台」。

- **主场景**：多人 / 多 Agent 在同一 Conversation 里聊天协作（发消息、@、回复、撤回、reaction、未读与通知）。  
- **Agent 能力**：挂在对话里的可执行参与者（@agent 启动/输入、session 流式投影、审批、结果回写时间线）。  
- **Host / Daemon**：对话里 Agent 落地的算力与工具边界；对用户是能力面，不是每天打开的主隐喻。  

Desktop 已按此落地：主舞台是 **Timeline + Composer**；Session / Approval 是侧栏或 Attention 入口，不是独立产品。

成熟 IM（Slack / Discord / Telegram / 微信）解决的基础能力，**全部是一等公民**：

- 长连接、可靠投递、多端水位、会话成员、热推 + 冷拉  
- **@mention、未读、推送偏好、撤回、reaction、列表摘要**  
- 读扩散 / 写扩散的投递模型（随群规模演进）

Minos **额外**叠加 Agent 执行与 Host 控制，但不应用「远程 Agent」心智吞掉 IM 主轴。

### 1.2 三轴模型（按产品优先级）

| 优先级 | 轴 | IM 类比 | Minos 实体 |
|--------|----|---------|------------|
| **P0 主轴** | A. 协作消息 | 用户/机器人聊天消息 + 群协作 | `conversation` + `conversation_members` + `conversation_messages` + mentions/reads/recall（+ 规划中的 reactions） |
| **P1 嵌入** | B. Agent 热投影 | 直播流 / typing / 工具过程卡片 | Host ingest → `StreamEvent(ui_event)` + session transcript；关键状态落 Durable |
| **P1 嵌入** | C. Attention / 特殊 @ | @人、@here、待办、审批 | `message_mentions`、`ApprovalRequested`、列表 `unread_*` / `needs_attention`、Push |
| **P2 能力** | D. Host 控制与上行 | 设备信令 + 边缘日志 | `host_commands`、ingest 幂等、Gap Pull |

**核心原则（产品与投递不变量）**：

1. **对话是 SSOT 容器**。Agent session 从属于 `conversation_id`；列表、未读、@、reaction 都以 conversation 为协作单元。  
2. **Cloud 不跑 Agent**。云端是 **IM + 投影 + 协调中枢**；执行在用户 Host。  
3. **Hub 是多端协作 SSOT（已投影部分）**；Host 本地 SQLite 是 **Agent 原始事件的长期 SSOT**。  
4. **写路径先事务后推送**（Transactional Outbox），禁止「只推不存」。  
5. **Attention 统一建模**（见 §3.4）：人 @、审批、失败 session 等共享「谁需要被打扰 / 进 Attention 列表」的逻辑，而不是各做一套通知。  
6. **最新架构优先**，不做历史 wire 兼容双写（见 AGENTS.md Development-State Policy）。

---

## 2. 总体架构（标准 IM 分层映射）

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  端侧 Presentation / Projection                                              │
│  Mobile (Flutter+FRB) · Web (React) · Desktop Account UI · TUI              │
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
│  │ /ws/client   │  │ 权限/幂等/状态  │  │ SessionRegistry │                  │
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
| Host 守护进程 | `/ws/host` | `Host { host_installation_id }` | `host:{host_installation_id}` |

同一 `(principal, installation_id)` **只保留最新连接**；旧连接 `4401` 强制关闭（成熟 IM 的「单设备踢旧」/ 多 tab 最新胜出策略）。

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
| **Online（IM 语义）** | **账号在线** = 该 account 至少一条 Mobile `/ws/client` live（`mobile_client_session_count`）；**设备在线** = 该 Host installation 有 live `/ws/host`（`SessionRegistry.get`） |
| **last_seen** | 耐久：`device_installations.last_seen_at_ms`；HTTP 列表返回；连接 open/close 强制 touch（冷路径展示，不冒充 online） |
| **Presence 推送** |  ephemeral `StreamEvent{kind:presence}`：Host 上下线 → 各 linked `account:{id}`（Mobile 改设备 online）；Account client 上下线 → 各 linked `host:{id}`（Host peer list） |
| **吊销踢连接** | 同 installation 重连 `session_superseded`（4401）；auth/unlink `auth_revoked`（4401）；Host 可先收 `HostForceClose` 再 close |
| **离线 Push** | 与 presence 正交：账户无活跃 WS 时由 `NotificationService` 决策（见 Plane P） |

```
Client/Host
  │  Bearer / hit_* installation token
  ▼
POST /v1/realtime/ws-ticket  (或 host 变体)
  │  60s 一次性 ticket
  ▼
GET /ws/client?ticket=…  或  /ws/host?ticket=…
  │  校验 + 消费 ticket + 角色匹配
  ▼
ServerFrame::Hello { conn_id, server_time_ms, heartbeat_interval_ms }
  │  registry insert（踢旧）+ presence online + 默认 topic
  ▼
ClientFrame::Subscribe { topics, resume_after? }
  │
  ▼
SubscribeAck / SubscriptionDenied / SnapshotRequired
  │
  ├─ 热路径：DurableEvent / StreamEvent（含 presence）
  ├─ 心跳：server WS Ping ↔ client Pong；或 ClientFrame::Ping ↔ Pong
  └─ 退出：remove registry（若仍是 current）→ presence offline
```

客户端断线后指数退避重连（典型 1s→30s 封顶）；冷路径 HTTP 列表用 `online` + `last_seen_at_ms` 纠偏。

代码：`realtime/gateway.rs`、`realtime/liveness.rs`、`realtime/presence.rs`；线类型 `PresencePayload` / `PRESENCE_STREAM_KIND`（`minos-protocol`）。

### 3.3 Topic 模型（频道 / 会话分区）

线格式：`{kind}:{partition_key}`。

| TopicKind | 示例 | 承载内容 |
|-----------|------|----------|
| `account` | `account:{id}` | 账户级通知：host 链接、会话列表级消息摘要、账户事件 |
| `conversation` | `conversation:{id}` | 对话消息 append / recall /（规划）reaction；时间线热路径 |
| `project` | `project:{id}` | 项目与对话关联、归档 |
| `agent_session` | `agent_session:{id}` | 会话生命周期、turn、**Approval Attention**、**UI 流式投影** |
| `host` | `host:{installation_id}` | Host 命令下发、强制关闭、链接状态 |

权限：订阅时服务端校验 membership / ownership；拒绝则 `SubscriptionDenied`。

### 3.4 协作交互模型（@ / Attention / 撤回 / Reaction）

本节是 **conversation-first 的产品交互 SSOT**：把常见 IM 能力与 Agent 场景统一到同一套概念，避免「聊天一套、Agent 又一套」。

#### 3.4.1 统一 Attention（「谁需要被打扰」）

| Attention 类型 | 触发源 | 目标 | 现有落点 | 产品语义 |
|----------------|--------|------|----------|----------|
| **人 @mention** | 消息正文解析 `@display` / 成员匹配 | `mentioned_account_id` | 表 `message_mentions`；列表 `unread_mention_count` | 经典 IM 艾特 |
| **Agent @mention** | Composer `@codex` 等 | Agent 成员 / 启动 session | Desktop 本地 `ConversationMention`；云端成员 `conversation_agent_members` | 「在对话里叫某个工人」 |
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

#### 3.4.2 @Mention（人）

**写路径（已实现骨架）**：

```
POST send-message
  → extract_mentioned_account_ids(text, members)  // 仅成员可被 @
  → insert conversation_messages + message_mentions
  → Durable ConversationMessageAppended (topic conversation:*)
  → 对成员写 AccountConversationMessageAppended (topic account:*)  // 列表/inbox
  → 未读：相对 conversation_reads 计算 unread_count / unread_mention_count
  → Push：偏好 direct_message / group_mention；在线则可不推
```

不变量：

- **只能 @ 当前 conversation 成员**（防泄漏）。  
- 发送者对自己的 @ 不计入自己的 mention 未读。  
- 撤回消息后，mention 行随 `message_id` CASCADE 删除或消息带 `recalled_at_ms` 后客户端隐藏正文；未读是否回滚以实现为准，规划上 **recall 应修正未读/mention 计数**。  
- 后续：`@everyone` / 角色 @、Agent 作为 mention target 的云端结构化字段（不仅文本解析）。

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
| Desktop 本地 | **已实现** | daemon SQLite + `toggle_conversation_message_reaction`；live `reactionToggled`；聚合 `LocalReactionGroup` |
| Cloud / 多端 | **未接** | backend 无 `message_reactions` 表；无 Durable reaction 事件 |

**目标形态（Discord / Slack 式，可扩展）**：

```text
message_reactions (
  message_id, emoji, account_id,  -- 人
  -- 或 actor: (actor_kind, actor_id) 支持 agent 将来点 reaction
  created_at_ms,
  PRIMARY KEY (message_id, emoji, actor_kind, actor_id)
)
```

写路径模板（与发消息同构）：

```
POST .../messages/:id/reactions/toggle { emoji }
  → TX: upsert/delete reaction 行 + 可选聚合缓存
  → Durable ConversationMessageReactionUpdated {
        conversation_id, message_id, emoji, action: add|remove,
        actor, reactions: [ { emoji, count, me, sample_actors } ]
     }
  → topic conversation:* （打开时间线的端）
  → 可选：轻量 account 摘要（一般不进 Push，避免噪声）
```

不变量：

- 仅 conversation 成员可 reaction。  
- **幂等 toggle**（同 actor 同 emoji 再点取消）。  
- 撤回消息：reaction 只读或随消息隐藏。  
- 聚合下发优先（全量 groups），避免端上自己算全集。  
- Desktop 本地 reaction 在 Host Link / 云端消息对齐后，应 **迁移或双写切断为仅云端**（latest-only，不做长期双 SSOT）。

#### 3.4.6 通知与降噪（Plane P 细化）

`NotificationService` + `decide()` 已覆盖类别：`message` / `approval` / `session_ended`；偏好含 `direct_message`、`group_mention`、`approval_required`、quiet hours。

规划补齐：

| 场景 | 推送策略 |
|------|----------|
| 普通群消息 | 默认可关；依赖未读角标 |
| @我 | 默认开；计入 `unread_mention_count` |
| Approval | 默认开；**绕过 quiet hours**；短 cooldown |
| Reaction 到我的消息 | 可选、默认关或折叠 |
| Agent 流式 token | **永不 Push**（Stream only） |
| 任意事件 | 账户任一 installation **在线 WS** → 可 Skip Push（已有 UserOnline 语义） |

Push **只唤醒**；点进后靠 Durable cursor / HTTP 拉一致状态。

---

## 4. 消息平面（Planes）：IM 双通道 + Host 控制面

Minos 明确区分 **可靠性语义不同** 的消息平面——这是成熟系统避免「一切塞进一个队列」的关键做法。

### 4.1 Plane D — Durable Event（可靠、可回放、有序）

**类比**：IM 的「离线消息 + 会话增量日志」。

- 线帧：`ServerFrame::DurableEvent { topic, topic_seq, kind, payload, event_id }`
- 载荷：`DurableEvent`（typed，见 `realtime.rs`）
- 语义：
  - 业务事务内写入 `durable_event_log`（**每 topic 单调递增 `topic_seq`**）
  - 同事务入队 `outbox_events`
  - Outbox dispatcher → 在线订阅者推送；多实例经 Redis bus
  - 客户端用 `resume_after: { topic → last_topic_seq }` 续传
  - 水位落后保留窗口 → `SnapshotRequired`，走 HTTP 全量/分页重建

**典型事件**：

- 社交：`ConversationMessageAppended` / `Recalled`、`AccountConversationMessage*`
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
  → Outbox → topic host:{installation_id}
  → Daemon 执行
  → ClientFrame::HostCommandAck
  → ClientFrame::HostCommandResult
```

超时：`HostCommandTimeoutJob`。Host 离线：命令持久化，上线后经 Durable 回放/outbox 再投递。

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
| 发社交消息 | `POST /v1/conversations/send-message` |
| Agent start/stop/input | `POST /v1/agent-sessions/*` |
| 审批 | `POST /v1/approvals/respond` |
| 读历史 turns | `POST /v1/agent-sessions/read-turns` |
| 列表/成员/好友 | `/v1/conversations/*`、`/v1/friends/*`… |

**读模型**：列表与历史走 HTTP；打开会话后 WS 订阅补增量。

### 4.6 Plane P — 离线推送（Push）

当客户端无活跃 WS 时，`PushFanoutJob` 根据在线态、偏好、免打扰与冷却决策 APNs/FCM 类推送。  
Push **只负责唤醒**；正文一致性仍靠 Durable + HTTP 同步。

---

## 5. 服务端投递内核（Transactional Outbox + Fanout）

### 5.1 写路径黄金模板

任何关键业务写（发消息、开会话、下发命令…）遵循：

```
BEGIN
  校验权限 / 幂等键
  写业务表 (conversation_messages | agent_sessions | host_commands | …)
  APPEND durable_event_log (topic, topic_seq++)
  ENQUEUE outbox_events
COMMIT
→ OutboxDispatcher 认领 → RealtimeFanout → 本地会话 / Redis bus
```

这就是业界 **Transactional Outbox** 标准解：业务与投递原子一致，避免「库已写但推丢」或「推了库没写」。

### 5.2 Fanout 引擎

`RealtimeFanout` 持有：

- `SessionRegistry`：conn → principal / installation  
- `SubscriptionManager`：topic → 在线订阅者  
- `StoreHandle`：补拉 / 认领  
- `MessageBusBackend`：`Inline`（单机）| `Redis`（多实例）

能力摘要：

| 方法 | 用途 |
|------|------|
| `fanout_ui_event` | Stream 平面，定向设备/会话 |
| `fanout_social_message` | 社交消息到关联账户客户端 |
| `dispatch_outbox_batch` | Durable 出站批处理 |

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
| Host 命令 | **定向写** | `host:{installation_id}` 单目标（不是群 fanout） |

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
| `installation_id` / `device_id` | 安装维度连接与踢旧 |
| `conversation_id` | 社交会话 |
| `message_id` | 单条聊天消息 |
| `mentioned_account_id` | 消息 @ 目标（`message_mentions`） |
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
| Dart providers | `threadEvents` / `activeSession` 投影时间线 |

典型上行：`sendUserMessage` → `POST .../send-input`。  
典型下行：`StreamEvent{ui_event}` → FRB → timeline 原地更新 streaming row。

### 7.3 Web

`RelaySocket` + Zustand：处理 `ui_event_message` / `social_message` 等，与 Mobile 共享同一 backend topic 语义。

### 7.4 Desktop（Tauri Host Console）

双栈：

1. **本地**：JSON-RPC → daemon → 本地 SQLite / agent（低延迟主路径）。  
2. **云端**：Account 登录 + Host Link 后，daemon `/ws/host` 上行 ingest；Account UI 可走 `/ws/client` 与其它端一致看投影。

Local-only session 未在 hub 注册时 **不** 自动出现在 Mobile/Web（设计如此）。

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
