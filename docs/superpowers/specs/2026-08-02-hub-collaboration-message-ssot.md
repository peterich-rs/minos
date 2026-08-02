# Hub 协作消息 SSOT 收敛方案

| Field | Value |
|-------|--------|
| Status | **Normative target**（2026-08-02） |
| Date | 2026-08-02 |
| Scope | 协作聊天气泡多端可见性；Desktop dual-write 退役；Agent 最终气泡单一写者；Sync Engine |
| Related | [architecture-messaging.md](../../architecture-messaging.md) §1.2 #3、§7.4；[05-projection-sync.md](2026-07-30-program/05-projection-sync.md)；本分支 `im-cloud-sync*` / `hub-realtime` / Mobile Messages Tab |
| Non-goals | 改 Grok/Codex 协议本身；E2EE；大群百万写扩散；保留历史 dual-write 兼容层（latest-only） |

> **一句话目标**：多端可见的用户/Agent **聊天气泡只认 Hub**；Host SQLite 只权威 **Agent 原始事件与本地工作台**；端上是 **带水位的 Sync + 可选 Outbox**，禁止 UI 层 best-effort 双向镜像当终态。

---

## 0. 背景：当前桥接 vs 文档不变量

### 0.1 文档已锁定的不变量

来自 [architecture-messaging.md](../../architecture-messaging.md)：

1. **对话是 SSOT 容器**（列表、未读、@、reaction 以 conversation 为单元）。  
2. **Cloud 不跑 Agent**（Hub = IM + 投影 + 协调；执行在 Host）。  
3. **Hub 是多端协作 SSOT（已投影部分）**；**Host SQLite 是 Agent 原始事件的长期 SSOT**。  
4. **写路径先事务后推送**（Transactional Outbox）。  
5. **最新架构优先**，不做历史 wire 兼容双写。

正确读法：

| 数据 | SSOT | 端上角色 |
|------|------|----------|
| 人读的聊天气泡（user / agent 最终文本、@、未读、recall） | **Hub** `conversation_messages` | cache / 投影 / optimistic |
| Agent 原始事件（tool / stream / git / raw / approval 过程） | **Host** daemon SQLite | 深度历史 + 执行面 |
| Session 热投影（streaming ui_event） | Hub `raw_events` + Stream fanout | 非 chat_messages 写扩散 |

### 0.2 本分支实际形态（Phase 0–5 后）

```
Desktop Linked 发消息 ──► Outbox ──► Hub POST client_live
Hub conversation_messages（协作气泡 SSOT）
       │
       ├── Durable conversation:{id} + account:* ──► Desktop hub-realtime → messagesByConversation
       ├── TurnCompletionProjector ──► agent 最终气泡
       └── Mobile HTTP + WS（只认 Hub）
Host daemon SQLite = Agent 原始事件 / tool / git / 本地工作台
```

| 层 | 现状 | 评价 |
|----|------|------|
| Mobile UI Messages 主 Tab | conversation-first | ✅ 保留 |
| Backend upsert / `client_message_id` / ensure-host-runtime | IM 原语 | ✅ |
| Desktop 用户写 | Hub-first + Outbox；无 dual-write 全量 project | ✅ Phase 1–3 |
| Hub → Desktop 读 | Sync 投影 store；**不** append daemon | ✅ Phase 3–4 |
| Sync | 状态机 + per-topic cursor + conversation Subscribe + SnapshotRequired + before_ts 上翻 | ✅ Phase 4 |
| Agent 最终气泡 | Hub TurnCompletionProjector 单写者 | ✅ Phase 2 |
| Reaction | 无云端 API → Desktop **local-only** 文档隔离 | ⚠️ Phase 5.2 缺口 |

### 0.3 已观测故障（与桥接同根）

| 症状 | 直接因 | 架构因 |
|------|--------|--------|
| Mobile `@grok` 执行完 conversation 无回复 | Hub `find_completed_agent_reply` 对 Grok 等 `Ok(None)` | Agent 气泡无单一可靠写者 |
| Desktop conversation 偶发无 agent-result | Grok 多工具 turn 边界丢 writeback；dual-write 静默失败 | 双 SSOT + best-effort |
| Mobile 底栏「循环播最后一句」 | 缺 `MessageCompleted` 不收起 + marquee `repeat` | 热投影生命周期与 turn 边界未对齐 |
| 静默丢多端消息 | dual-write 失败不挡本地 UX、无 Outbox | 协作消息当本地权威 |

**结论**：产品方向对，后端原语有用；必须 **收敛 dual-write**，不能在镜像桥上继续堆功能。

---

## 1. 目标架构（终态）

### 1.1 两平面 + 投影合并（UI 层）

```
┌─────────────────────────────────────────────────────────────────┐
│  Presentation（Desktop / Mobile / Web）                         │
│  Projection merge: HubChatBubble + LocalAgentCards + StreamLive │
└────────────────────────────┬────────────────────────────────────┘
                             │
        ┌────────────────────┴────────────────────┐
        ▼                                         ▼
┌───────────────────────┐               ┌─────────────────────────┐
│ Plane A · 协作消息 IM │               │ Plane B · Agent 执行面  │
│ SSOT = Hub            │               │ SSOT = Host daemon      │
│ conversation_messages │               │ raw/transcript/tool/git │
│ mentions / reads /    │               │ conversation_completion │
│ recall / (reactions)  │               │   → 只产出「最终文本」   │
│                       │               │   上行或中心投影到 Hub   │
│ 写: HTTP + 幂等       │               │                         │
│ 热: Durable topic     │               │ 热多端: Stream ui_event │
│ 冷: messages/query +  │               │ 冷: read-turns / ingest │
│     resume_after      │               │                         │
└───────────────────────┘               └─────────────────────────┘
```

**禁止**：把 Plane A 的气泡做成 daemon SQLite 与 Hub 的对等权威镜像。

### 1.2 Desktop 终态读写路径

#### Linked + 已登录（多端 IM 会话）

**写（用户消息）**

```
Composer send
  → optimistic UI（pending + client_message_id）
  → POST Hub /v1/conversations/:id/messages
       { text, client_message_id, reply_to?,
         message_source: "client_live" }   // 显式，见 §3.1
  → 200：confirm optimistic（message_id / hub 序）
  → 可选：写入 Desktop 本地 cache 表（origin=hub, hub_seq）—— 非 SSOT
  → @agent 执行：并行或随后走 Host session start/send_input
       （与「先写本地 chat_messages 再 dual-write」脱钩）
```

**写（Agent 最终气泡）—— 单一写者（见 §2）**

```
Host turn boundary (conversation_completion 语义)
  → 唯一路径：Hub 落 conversation_messages（role=agent）
  → Durable fanout → 所有端（含 Desktop Account 时间线）
  → Desktop 不再 projectTimelineMessagesToCloud 扫本地气泡
```

**读**

```
打开 conversation
  → Subscribe conversation:{id} + resume_after(topic_seq)
  → 冷拉 gap（after_seq / before_seq），禁止「固定 100 条全量 merge 当 Sync」
  → account:* 只刷新 inbox 摘要（未读/preview）
  → UI merge:
       hubBubbles (Plane A)
     + localAgentCards (Plane B: tool/git/system，仅本机)
     + liveStream (可选 session 侧栏 / 底栏 ticker)
```

#### Local-only / 未登录

```
只写 daemon；UI 诚实标注「仅本机」
不对 Mobile 谎称可见（honesty UX，D05）
```

### 1.3 Mobile 终态（基本已对齐，harden）

- Inbox / chat：**只** Hub。  
- @agent：`try_agent_dispatch` → Host 执行 → **Hub 必须**收到 agent 最终气泡（§2）。  
- 底栏：仅 live session 热投影；turn idle / MessageCompleted / host offline → 收起。  
- 不依赖 Desktop dual-write 运气。

### 1.4 Host / Daemon 终态职责

| 职责 | 做 | 不做 |
|------|----|------|
| Agent 进程 / transcript / tool / git | ✅ 本地 SSOT | 不把 raw 写扩散到 chat_messages 当多端 IM |
| turn 最终文本 | ✅ 算出 last segment（已有 conversation_completion 语义） | 不单独长期当「多端气泡 SSOT」 |
| 上行 Hub | ✅ 可靠投递最终 agent 气泡 或 由 Hub 中心投影 | 不经 React loadTimeline 扫表 |
| 协作消息 cache（可选） | origin + hub_seq 的只读缓存 | 不与 Hub 对等权威 |

---

## 2. Agent 最终气泡：单一写者（P2 核心，P0 先止血）

### 2.1 选定策略：**Hub 中心投影（推荐）**

理由：与「Cloud 不跑 Agent、但负责 IM 投影」一致；所有端（Mobile 无 Desktop 时）都能收到；避免 Desktop UI 写者。

```
Host ingest (raw_events 已有)
  + formal agent_session / turn 状态
  → Backend TurnCompletionProjector（替代脆弱的 group completion watcher）
  → insert_agent_message_with_session（client_message_id 幂等）
  → Durable ConversationMessageAppended + Account*
```

**幂等键（稳定）**：

```text
client_message_id = "agent-result:{conversation_id}:{session_id}:{turn_write_id}"
```

与今日 daemon 本地 id 同形，便于迁移与对账；**Hub 行是权威**，本地 id 仅作 projection key。

### 2.2 语义对齐 Daemon `conversation_completion`

必须复用同一 turn 边界语义（见 architecture-daemon / conversation_completion 注释）：

- 仅 **top-level** session（非 subagent）。  
- 工具 / subagent 打断后的 **last non-interrupted assistant 文本段**。  
- Idle/Closed + ingest 竞态：pending_boundary 或等价 latch。  
- 无干净最终文本 → **不写**进度垃圾。  
- 全 runtime：**Codex / Claude / Gemini / OpenCode / Grok** 同一套 translator 路径，禁止 `Ok(None)` 空实现。

### 2.3 删除 / 退役的写者

| 写者 | 终态 |
|------|------|
| `social.rs` `find_completed_agent_reply` 仅 Codex + 永久 poll | **替换**为 TurnCompletionProjector |
| Desktop `projectTimelineMessagesToCloud` 扫 agent-result | **删除** agent 路径 |
| Desktop `syncAgentMessageToCloud` | **删除**（或仅迁移期 Outbox 消费后删除） |
| Daemon 本地 `agent-result` 作为多端真相 | **降级**为 local-only 时间线 / 可选 mirror |

### 2.4 可选并行：Daemon Outbox 上行（若不做中心翻译）

若短期不想在 Hub 重做 Grok 翻译：

```
conversation_completion 成功
  → host_im_outbox(pending)
  → Host 带 account 凭证或 host 专用 API:
       POST /v1/conversations/:id/agents/message
       { message_source: "host_projection", client_message_id, text, session_id }
  → ack / retry
```

仍须 **显式 source**，且 **唯一** 向 Hub 写 agent 气泡的路径（Desktop UI 不写）。  
长期仍建议中心投影：Host 离线时 turn 完成后上线补投更简单、且与 ingest 同源。

**推荐落地顺序**：  
P0 先把 watcher 补全到「全 agent + 正确 last-segment」（止血）；P2 收敛为正式 TurnCompletionProjector + 删 Desktop agent dual-write。

---

## 3. 协议与 API 硬化

### 3.1 显式 `message_source`（替代「有 client_message_id ⇒ 跳过 dispatch」）

```jsonc
// SendChatMessageRequest / SendAgentMessageRequest 增量
{
  "text": "...",
  "client_message_id": "uuid-or-stable-key",  // 仅幂等，不承载语义
  "message_source": "client_live" | "host_projection" | "system",
  "reply_to_message_id": null,
  // 删除或忽略客户端 created_at_ms 作为权威序；Hub 分配 server_created_at_ms
}
```

| message_source | 谁可设 | Agent dispatch |
|----------------|--------|----------------|
| `client_live`（默认） | 人类客户端 | 按 mention / 规则 dispatch |
| `host_projection` | Host 可信链路 / 服务端内部 | **永不** dispatch |
| `system` | 服务端 | 永不 |

迁移：legacy 仅 `client_message_id` 且无 source 的调用 → 默认 `client_live`（**恢复**被误跳过的行为需审计 Desktop dual-write 调用点，改为显式 `host_projection` 后删除）。

### 3.2 服务端时间与序

- **权威时间**：`created_at_ms` = server clock。  
- 客户端可传 `client_sent_at_ms` 仅展示/调试，**不**参与排序主键。  
- 排序：`message_seq`（或现有 conversation 内单调序）+ server time 展示。  
- `reply_to`：目标不在本 conversation → **400**（client_live）；host_projection 可允许 null 降级但 **打 metrics**，不静默吞关键错误。

### 3.3 host-runtime agent 身份

废弃 `description = "minos:host-runtime"` 作为唯一标记。

```sql
-- agents 表（示意）
source TEXT NOT NULL DEFAULT 'user',  -- user | host_runtime | system
runtime_agent TEXT,                  -- codex|claude|...
UNIQUE (owner_account_id, source, runtime_agent) WHERE source = 'host_runtime'
```

`ensure-host-runtime` 只写 `source=host_runtime`。  
映射永不依赖 description 文案。

### 3.4 Conversation upsert（保留）

- client-owned `conversation_id` upsert：✅ 跨端同 ID。  
- roster：cloud `agent_id` only（经 ensure-host-runtime）。  
- Linked 后创建本地 work conversation → **先/只** Hub upsert，再本地 cache（若需要）。

---

## 4. Sync Engine（成熟 IM 形态）

### 4.1 状态机（Desktop Account + Mobile 已部分具备）

```
Disconnected
  → Connecting (ticket)
  → Authenticating / Hello
  → Syncing (resume_after per topic)
  → Live
  → (gap too large) SnapshotRequired → 冷拉 → Live
```

### 4.2 Topic 职责

| Topic | 用途 |
|-------|------|
| `account:{id}` | Inbox 摘要、未读、列表预览（轻） |
| `conversation:{id}` | **打开会话后** 订阅：气泡 append/recall/reaction |
| `agent_session:{id}` | 热 ui_event / 审批（非 chat 气泡） |
| `host:{id}` | 命令与 ingest 控制面 |

今日缺口：Desktop `hub-realtime` 几乎只听 `AccountConversationMessageAppended`，无 `conversation:*` 订阅、无 `resume_after`、无 SnapshotRequired。

### 4.3 冷路径

```
GET/POST messages/query
  { conversation_id, after_seq?, before_seq?, limit }
```

- 打开：`after_seq = local_cursor` 拉 gap。  
- 上翻：`before_seq`。  
- **禁止**以 `limit:100` 全量覆盖当唯一同步策略。  
- SnapshotRequired：清空 conversation 投影窗口 → 全量快照页 → 重置 cursor。

### 4.4 客户端 Outbox（仅用于「端 → Hub 写」可靠，不是双 SSOT）

适用：Mobile/Desktop **用户发送**、Host **host_projection** agent 气泡（若选 Daemon 上行方案）。

```
outbox(
  id, kind, conversation_id, client_message_id,
  payload_json, status: pending|inflight|acked|failed_terminal,
  attempts, next_attempt_at, last_error
)
```

- 写本地 optimistic / Host 完成 → insert pending。  
- Worker：backoff POST；幂等靠 `client_message_id`。  
- UI：失败可见「未同步到云端」，**禁止**静默成功本地、云端永久无。  
- **删除**「每次 loadTimeline 全量 project」作为同步机制。

Desktop 若终态「Composer 直接 POST Hub」，Outbox 仍建议有（弱网）；本地 daemon chat_messages **不再**是发送主路径。

---

## 5. Desktop 领域边界：cache vs SSOT

### 5.1 推荐：**A. Linked 会话时间线以 Hub 为主**

| 数据 | 来源 |
|------|------|
| user / agent 聊天气泡 | Hub Sync 投影（内存 + 可选 disk cache） |
| tool / git / system 卡 | 仅 Host 本地，Projection 层插入时间线空隙或 Session 侧栏 |
| Session transcript | Host 本地 + 可选 Hub stream 订阅 |

实现要点：

- `messagesByConversation` 对 Linked 会话：**Hub 行**为主键。  
- 去掉 `applyHubChatMessageToLocal` → `daemon_append_conversation_message` 作为主路径。  
- 若需离线展示：独立 `hub_message_cache` 表（`hub_message_id`, `topic_seq`, `origin=hub`），**daemon 业务表不混写**。

### 5.2 不推荐继续：**半吊子 merge 进 daemon chat_messages**

若短期必须 mirror，则 **B. 显式 cache 契约**（仍非对等 SSOT）：

```sql
chat_messages 增加:
  origin TEXT NOT NULL,          -- local_agent | hub_cache
  hub_message_id TEXT,
  hub_topic_seq INTEGER,
  sync_generation INTEGER
```

- Hub 行 `origin=hub_cache`，冲突以 Hub 为准。  
- 本地 agent-result 在 Linked 模式 **停止** 作为多端写源。  
- Sync worker 在 daemon 或独立 crate，**不在** React `pull+append`。

**本方案默认选 A**；B 仅作迁移垫片且有明确删除里程碑。

### 5.3 Local-only 会话

- 无 Hub conversation 行；UI badge「仅本机」。  
- 不触发 Outbox；不出现在 Mobile。  
- Link 后是否 backfill：产品可选；默认 **仅新消息**（与 D05 open question 对齐），历史 backfill 另 PR。

---

## 6. Mobile 底栏与热投影（与本次联调 bug 对齐）

| 规则 | 行为 |
|------|------|
| 显示条件 | 存在 **running** turn 的 live session 且 host online |
| 隐藏条件 | session status ∈ idle/ended/… **或** 最后 assistant `MessageCompleted` **或** host offline |
| 文案 | 当前 tool/reasoning/text preview；禁止 turn 结束后仍 `生成回复中` |
| marquee | 仅 overflow 时滚动；turn 结束后组件卸载（不要永久 `repeat` 挂着） |
| 数据 | `agent_session` Stream only；**不是** conversation 消息 |

`conversationAgentActivityProvider`：`idle` **不得**视为 runnable（与 session 列表「可再 @ 续聊」分离：续聊不等于「正在执行」）。

---

## 7. 分阶段落地（可拆 PR，方向不可逆）

> 原则：**先止血多端丢消息与 Grok 回写，再拆双 SSOT，再 Sync 引擎化**。每阶段可独立合并；不做长期 dual-path 兼容。

### Phase 0 — 止血（1–3 PR，可并行）

| ID | 内容 | 主要文件 / 位置 | 验收 |
|----|------|-----------------|------|
| **0.1** ✅ | Hub completion：全 agent last-segment；删 Grok `Ok(None)` | `turn_completion` + `social.rs` watcher | Mobile `@grok` 完成后 Hub 有 agent 行；watcher 不再无限 poll |
| **0.2** ✅ | `message_source` 字段 + dispatch 门控 | `minos-protocol` Send*Request；`conversations.rs` / `social.rs` | 仅 `host_projection` 跳过 dispatch；纯 `client_message_id` 不再隐式跳过 |
| **0.3** ✅ | Mobile 底栏 idle 收起 | `agent_activity_provider.dart`；social_chat ticker | turn 结束后横条消失 |
| **0.4** ✅ | Desktop：去掉 quiet `loadTimeline` **全量** `projectTimelineMessagesToCloud` | `timeline.ts`；改为「仅 outbox pending / 刚发送 id」 | 打开会话不再刷屏 POST |
| **0.5** ✅ | Desktop 用户消息：发送成功路径保证 Hub 一次写 + 失败可见 | Outbox + toast；`im-cloud-sync` / `im-outbox` | dual-write 失败用户可感知 |

**Phase 0 不要求**拆完双 SSOT，但必须让 **Mobile@Agent → Hub 气泡** 不依赖 Desktop 是否打开。

### Phase 1 — 可靠写路径 + 身份

| ID | 内容 | 验收 |
|----|------|------|
| **1.1** ✅ | 持久 Outbox（Desktop localStorage）用户消息 | 杀进程可重试；acked 后不再 project |
| **1.2** ✅ | host-runtime `source` 列；ensure API 改写 | description 改名不丢映射 |
| **1.3** ✅ | 服务端 `created_at_ms` 权威；弱化客户端时钟 | 序稳定 |
| **1.4** ✅ | reply_to 校验策略按 source 分支 | client_live 硬失败 |

### Phase 2 — Agent 气泡中心化 + 删 agent dual-write

| ID | 内容 | 验收 |
|----|------|------|
| **2.1** ✅ | `TurnCompletionProjector`（服务端）正式化；与 0.1 合并演进 | 单测：Grok 多工具 turn 只落最终段 |
| **2.2** ✅ | 删除 `syncAgentMessageToCloud` / timeline agent project | 无 Desktop→Hub agent 路径 |
| **2.3** ✅ | Daemon `conversation_completion`：Linked 时 local-only 文档；多端气泡归 Hub projector | 文档与 honesty badge 一致 |
| **2.4** ✅ | `POST agents/message` 拒绝 `client_live`；默认 `host_projection` 永不 dispatch | 鉴权：account JWT + 拥有 agent |

### Phase 3 — Desktop 读路径改 Hub 主时间线

| ID | 内容 | 验收 |
|----|------|------|
| **3.1** ✅ | Linked conversation timeline store 以 Hub 消息为 SSOT 投影 | 关 daemon chat 列表仍可见 Mobile 气泡 |
| **3.2** ✅ | 删除主路径 `im-cloud-inbound` → daemon append | 无云端 IM 污染 Host SSOT |
| **3.3** ✅ | UI merge：Hub 气泡 + 本地 tool/git 卡 | Session 侧栏仍本地 |
| **3.4** ✅ | Composer Linked 模式默认 POST Hub `client_live`，不再 `append local first` | 与 Mobile 同序 |

### Phase 4 — Sync Engine

| ID | 内容 | 验收 |
|----|------|------|
| **4.1** ✅ | Desktop hub-realtime：状态机 + per-topic cursor 持久化 | `Disconnected→Connecting→Syncing→Live`；`hub-cursors` localStorage |
| **4.2** ✅ | 打开会话 `Subscribe conversation:{id}` + resume_after | Backend fanout `ConversationMessage*` + Desktop subscribe on focus |
| **4.3** ✅ | SnapshotRequired 处理 | 清投影 + `messages/query` 冷拉；cursor reset |
| **4.4** ✅ | messages/query gap API 对齐 Mobile | Linked `loadOlder` 用 `before_ts_ms`；`after_seq` 仍未上线 |

### Phase 5 — 协作能力只在 Hub

| ID | 内容 | 验收 |
|----|------|------|
| **5.1** ✅ | recall 多端 | Durable `*Recalled` 删时间线；`recallMessageOnHub` / Hub API |
| **5.2** ⚠️ | reaction：无云端 API → **明确 local-only**；禁止双 SSOT 伪装 | `reaction-store` 文档隔离；云端 API 后续 |
| **5.3** ✅ | 未读 / mark-read 走 Hub | Linked `markConversationRead` → `POST …/read` |

---

## 8. 按子系统的改造清单（文件级）

### 8.1 Backend（`crates/minos-backend`）

| 区域 | 动作 |
|------|------|
| `http/v1/social.rs` | 重写 completion → TurnCompletionProjector；删除永久空分支 |
| `http/v1/conversations.rs` | `message_source`；dispatch 门控；server time |
| `conversations/use_case.rs` | reply 策略；insert 幂等保持 |
| `store/social/agents.rs` | `source` / host_runtime 唯一约束 |
| `realtime/gateway.rs` | formal session 挂 agent 保持；确保 conversation topic fanout |
| `ingest/*` + `agent_turns` | projector 输入：turn complete / summary_text / raw 翻译 |
| tests | `ws_gateway` / social：Grok/Codex 完成写气泡；dispatch source 矩阵 |

### 8.2 Protocol（`crates/minos-protocol`）

| 区域 | 动作 |
|------|------|
| `messages.rs` | `MessageSource` enum；Send*Request 字段；deprecate 客户端权威 `created_at_ms` |
| `realtime.rs` | 确认 conversation topic 事件足够 Desktop 气泡同步 |

### 8.3 Daemon（`crates/minos-daemon`）

| 区域 | 动作 |
|------|------|
| `conversation_completion.rs` | 保持 last-segment 语义；Linked 路径改为触发 Hub 投影或 host_projection Outbox |
| 本地 `chat_messages` | 明确 origin；或 Linked 会话不再作为多端写源 |
| 新 `im_outbox`（若选 Host 上行） | 持久 pending/ack |

### 8.4 Desktop（`apps/desktop`）

| 区域 | 动作 |
|------|------|
| `shared/lib/im-cloud-sync.ts` | Phase0 收缩 → Phase2 删除 agent 与全量 project |
| `shared/lib/im-cloud-inbound.ts` | Phase3 删除主路径 |
| `shared/lib/hub-realtime.ts` | Phase4 Sync 状态机 + conversation 订阅 |
| `shared/lib/im-hub-bridge.ts` | 改为 Sync 入口，非 append-daemon |
| `store/workspace/timeline.ts` | Linked：Hub hydrate；去 pullHub→daemon |
| `store/workspace/use-cases.ts` | 发送改 Hub-first |
| `store/workspace/live-ingress.ts` | agent-result 刷新仅本地卡；不依赖 dual-write |
| UI badge | local-only / sync-failed |

### 8.5 Mobile（`apps/mobile`）

| 区域 | 动作 |
|------|------|
| Messages Tab / social chat | 保持 Hub-only |
| `agent_activity_provider.dart` | idle 非 runnable；完成收起 |
| realtime | 已有 cursor 则对齐 conversation 订阅与 Desktop |

### 8.6 文档

| 文档 | 动作 |
|------|------|
| `architecture-messaging.md` §7.4 | 标注 dual-write **transitional**，链到本文；终态读写表 |
| `architecture-desktop.md` | 时间线 SSOT 改为 Hub（Linked） |
| `architecture-daemon.md` | agent-result 与 Hub 投影关系 |
| `05-projection-sync.md` | 补充「协作气泡 ≠ session ingest 投影」 |
| 本文 | 执行跟踪（checkbox 可后续改 tasks） |

---

## 9. 数据流对照（Before / After）

### 9.1 Mobile @agent → 多端看见回复

**Before（脆）**

```
Mobile POST → Hub user msg → dispatch Host
  → stream ok（底栏有字）
  → Hub watcher Grok=None ──✗── 无 agent 气泡
  → Desktop 若打开且 local completion 成功 → dual-write 碰运气
```

**After**

```
Mobile POST → Hub user msg → dispatch Host
  → stream ok（底栏 live）
  → TurnCompletionProjector → Hub agent msg（幂等 agent-result:…）
  → Durable → Mobile + Desktop + Web
  → 底栏 idle 收起
```

### 9.2 Desktop 用户发消息

**Before**

```
local SQLite → UI → best-effort Hub（失败静默）
Hub → 再写回 local（镜像）
```

**After（Linked）**

```
optimistic → Hub POST (Outbox) → Durable → 各端
本地仅 cache 或纯投影；tool 卡仍 local merge
```

### 9.3 Desktop 看 Session

不变：**Session transcript = Host SSOT**（+ 可选 stream）。  
Conversation 主舞台气泡 = Hub。  
两者在 UI 分栏/分层，不共用一个「对等 SQLite 真相」。

---

## 10. 测试与验收矩阵

| 场景 | 期望 |
|------|------|
| Mobile `@grok` 无工具短回复 | Hub + Desktop + Mobile 均有 agent 气泡；底栏消失 |
| Mobile `@grok` 多工具长回合 | 仅最终段气泡；无过程刷屏；无 watcher 空转日志 |
| Desktop 发消息后杀进程 | Outbox 重试后 Mobile 可见；或明确 failed UI |
| Desktop quiet 刷新时间线 | **零** 额外 project POST（无 pending 时） |
| 未 Link / 未登录 | 本地可聊；Mobile 不可见；无谎称 |
| `message_source=host_projection` 伪造自普通 client | 401/403 |
| 普通 `client_message_id` 发送 | **仍**可 @ dispatch |
| 重连 resume_after | 不丢气泡；不重复（幂等） |
| SnapshotRequired | 冷拉后 cursor 连续 |

自动化建议：

- Backend：translator + projector unit（Grok fixture raw_events → 一条 text）。  
- Backend：dispatch source 矩阵。  
- Desktop：Outbox reducer unit（无网络）。  
- Mobile：activity provider idle 单测。  
- 可选 e2e：已有 ws_gateway 扩展 account 收到 agent append。

---

## 11. 风险与决策记录

| 风险 | 缓解 |
|------|------|
| Hub 重做 Grok last-segment 与 daemon 不一致 | 共享 `minos-ui-protocol` 翻译；同一套 interrupt 规则；黄金 fixture |
| Desktop 本地时间线短暂变空 | Phase 3 前保持 cache；切换 feature 一次切 Linked 会话 |
| Outbox 与乐观 UI 复杂 | 先 toast + 内存队列（0.5），再持久化（1.1） |
| 中心投影延迟 | Stream 底栏已覆盖「进行中」；气泡允许 turn 结束后数百 ms |
| latest-only 破坏旧 dual-write 客户端 | 与 AGENTS.md 一致；本仓库第一方客户端同 PR 改 |

**已锁定决策**：

1. 协作聊天气泡 **Hub SSOT**（非双权威）。  
2. Agent 最终气泡 **单一写者**（优先 Hub 中心投影）。  
3. `client_message_id` **只做幂等**；dispatch 看 `message_source`。  
4. Desktop Linked 读路径最终 **Hub-first**；daemon 不镜像当 SSOT。  
5. 不保留长期 dual-write 兼容层。

---

## 12. 与现有 Program 的关系

| 文档 | 关系 |
|------|------|
| D05 Projection & Sync | 原焦点 session ingest 可见性；**本文补「协作气泡平面」**，避免用 dual-write 冒充 D05 完成 |
| L0 cloud-identity | Host Link / 登录是 Linked 路径前提；本文不改 IdP |
| architecture-messaging | 本文是 §1.2 #3 的 **可执行收敛**；§7.4 dual-write 标为 transitional |

---

## 13. 立即执行顺序（给实现 Agent）

不进入空泛 plan mode 时的 **推荐动手序**：

1. **0.1** Hub 全 agent turn completion 写气泡（修你刚踩的 Mobile@Grok）。  
2. **0.3** Mobile 底栏 idle 收起。  
3. **0.2** `message_source` 解耦 dispatch。  
4. **0.4–0.5** 砍 timeline 全量 project + 用户消息失败可见。  
5. 再开 Phase 1–2 PR 拆双 SSOT。  
6. Phase 3–4 改 Desktop 读模型与 Sync。  
7. Phase 5 协作能力只留 Hub。

---

## 14. 一句话

**保留** Mobile conversation-first、backend 幂等/upsert/host-runtime 原语；  
**删除**「Desktop SQLite ↔ Hub 对等 dual-write」终态幻想；  
**建成** Hub = 协作气泡 SSOT、Host = 执行与原始事件 SSOT、端 = Sync + Outbox + UI 投影合并。  
当前桥接能联调，但必须按 Phase 0→5 **收敛**，否则每加一个多端功能都会再分叉一套真相。
