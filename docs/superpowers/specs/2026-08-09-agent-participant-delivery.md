# Agent as Bot Participant — Delivery Model (Normative)

| Field | Value |
|-------|--------|
| Status | **Normative target**（2026-08-09） |
| Date | 2026-08-09 |
| ADR | [0021-agent-as-conversation-bot-participant](../../adr/0021-agent-as-conversation-bot-participant.md) |
| Global bot identity | [global-bot-identity-design](global-bot-identity-design.md) — 全局唯一 bot 用户、数字肉身、membership vs session |
| Bot mailbox + WS IM | [bot-mailbox-ws-im-bus-design](bot-mailbox-ws-im-bus-design.md) — 严格 IM 写路径 + Bot 逻辑邮箱 + Host 执行口 |
| SSOT product framing | [architecture-messaging.md](../../architecture-messaging.md) |
| Hub bubble ownership | [2026-08-02-hub-collaboration-message-ssot](2026-08-02-hub-collaboration-message-ssot.md)（写者/幂等保留；触发语义以本文为准） |
| Backend jobs | [2026-08-03-backend-im-delivery-orchestration](2026-08-03-backend-im-delivery-orchestration.md)（`AgentDispatchQueue` = Agent inbox 物理表） |
| Client sync | [2026-08-03-client-im-sync-engine](2026-08-03-client-im-sync-engine.md) |
| Non-goals | E2EE；云上跑 CLI；Agent 登录为 Account；为旧 wire 双写兼容 |

> **一句话**：协作只认 Conversation 消息；Agent 是 **全局 bot 用户** 被拉入 conversation 后的成员；`@agent` 与 `@人` 同属 mention → participant delivery；HostCommand/CLI 是 runtime 适配器，不是第二套协作协议。  
> **身份不变量**：bot `agent_id` 跨 conversation 复用；进群只写 membership；每个 conversation 为该 bot 维护独立 session。细节见 [global-bot-identity-design](global-bot-identity-design.md)。

**规划约束**：遵守 AGENTS.md Final-Architecture Planning Rule — 只设计终态；实现可分 Phase，但不得再把「命令式 @agent」写成产品主轴。

---

## 1. Glossary

| Term | Meaning |
|------|---------|
| **Bot identity** | 全局唯一 `agent_id` + 数字肉身（Hub `agents` SSOT）；可被多个 conversation 引用。见 [global-bot-identity-design](global-bot-identity-design.md) |
| **Human participant** | `account_id` 在 `conversation_members`；经 `/ws/client` + Push 收消息 |
| **Agent participant (bot)** | 全局 `agent_id` 的 **membership** 行在 `conversation_agent_members`；**不是** Account；经 bound Host runtime 消费投递 |
| **Participant** | Human ∪ Agent 的统一读模型 |
| **Session** | bot 在某 conversation 内的执行上下文（`agent_sessions`）；**不是** bot 身份；默认 per `(conversation_id, agent_id)` |
| **Mention** | 消息上的结构化目标：`target_kind ∈ {account, agent}` + `target_id`（agent 目标 = 全局 `agent_id`） |
| **Participant delivery** | 消息 commit 后，按 mention/房规把意图投递给参与者的过程 |
| **Agent inbox** | Agent 侧投递队列（今日表名 `agent_dispatch_queue`；领域名 inbox）；幂等键含全局 `agent_id` |
| **Runtime port** | Host 上执行 CLI 的通道（今日 `HostCommand` / `/ws/host`） |
| **Agent reply** | `sender_type=agent` 的 Hub 时间线消息（作者身份 = 全局 bot） |
| **Account sync live** | 人类 IM 可发可收（产品主 Online） |
| **Agent available** | bot 成员可执行：roster 在 + bot status active + bound Host `/ws/host` live |

**禁止混用**：

- 「远程协作」≠「下发 HostCommand」  
- 「Online」≠「仅 live `/ws/host`」  
- 「dual rails」= Account/Host **传输**分轨；≠ 聊天气泡 dual-write 权威  
- bot identity ≠ session ≠ runtime bin 名（`codex`）≠ 端侧本地 profile 行  
- 进群 ≠ 新建 bot

---

## 2. Product invariants

1. **Conversation 时间线是唯一协作 SSOT**（人读气泡、@、未读、recall、reaction）。  
2. **唯一协作原语**：Message（+ reaction / read / recall / mention）。  
3. **@ 语义统一**：只能 @ 当前 conversation 的 participants；人与 bot 都进 mention SSOT。  
4. **Agent 无人类登录态**：无 Supabase、无 access/refresh、不以人类 `account_id` 作为 bot 身份。  
5. **Cloud 不跑 Agent**；执行只在 Host；Hub 负责 IM + 投递编排 +（必要时）完成投影。  
6. **发送成功** = 用户消息落库 + 社交 durable/outbox；**不**等待 Host RPC。  
7. **`message_source ∈ {host_projection, system}` 永不**再投递 agent（防环）。  
8. **Agent 回复幂等键**：`agent-result:{conversation_id}:{session_id}:{origin_message_id}`。  
9. **同一 Account 多端**直接写 Hub；禁止「命令 Desktop 代发用户消息」。  
10. **Host offline**：inbox pending，上线 drain；人类仍可互聊；bot 显示 unavailable。

---

## 3. Target flow

### 3.1 @human（不变，作为对照）

```
send_message (Account)
  → TX: chat_messages + mention(account) + durable + outbox
  → fanout conversation:* / account:* digests
  → unread / Push for targets
```

### 3.2 @agent（终态）

```
send_message (Account, message_source=client_live)
  → TX: chat_messages
       + mentions (account* and/or agent*)
       + social durable + outbox
       + Agent inbox rows for each agent delivery target
  → HTTP/WS 200 (user bubble committed)
  → social fanout (all human devices)
  → Agent inbox worker
       → Runtime port: start | send_input  (HostCommand 实现细节)
       → arm completion watch (origin × session) when needed
  → Host CLI / ingest
  → Agent reply message (sender_type=agent, idempotent client_message_id)
  → social fanout (all ends see bot bubble)
```

### 3.3 Delivery selection rules（房规）

| Priority | Condition | Deliver to |
|----------|-----------|------------|
| 1 | `reply_to` 指向 agent 消息且可解析 agent | 该 agent（复用 session 若有） |
| 2 | 正文结构化/解析出的 agent mentions | 每个唯一 agent（multi-@ fan-out） |
| 3 | group、恰 1 human + 1 agent、无显式 @ | 该 sole agent（auto-route） |
| else | — | 不 enqueue agent inbox |

未匹配的「像 agent 的 @」且无成员：用户可见失败（不静默）。

**Membership-first（已落地）**：无 silent auto-attach。`@codex` 等 host_runtime token 若会话 roster 无对应 bot 成员 → 用户可见失败（`no_agents_in_conversation` / `agent_not_in_conversation`），不暗挂。显式成员变更走 participants / agents/add；`POST /agents/ensure-host-runtime` 只 ensure agent registry 行，不自动 join 会话。

### 3.4 Desktop-native vs client_live

| Mode | User bubble | Who runs CLI | Agent bubble to Hub | Re-deliver? |
|------|-------------|--------------|---------------------|-------------|
| **client_live**（Mobile / 纯 Account 端） | Hub first | Hub inbox → Host | Hub projector 或显式 agent reply | 仅 client_live |
| **Desktop-native Linked** | 可本地先写工作台 | 本机 daemon | `host_projection` 上行同 id | **否** |

领域事件对齐：都是「bot 成员被投递 / 自行消费后回消息」。  
`message_source` 只做 **provenance + 防环**，不是「Desktop 魔法开关」的长期产品名；双跑抑制靠 inbox 幂等 / consumer lease，逐步弱化对 source 的依赖。

---

## 4. Data model (target)

### 4.1 Participants + global bot directory

保留双表 membership（Phase A），但 **bot 身份全局唯一**：

- `agents` — **全局 bot 目录 + 数字肉身**（name / model / reasoning_effort / system_prompt / runtime / status …）；同一 `agent_id` 可加入多个 conversation  
- `conversation_members`（human membership）  
- `conversation_agent_members`（bot membership only — **不** clone bot）  
- `agent_sessions` — per `(conversation_id, agent_id)` 执行上下文  

统一 API：

```http
GET/POST …/conversations/{id}/participants
→ { humans: UserSummary[], agents: AgentSummary[] }

POST …/agents                     # 创建全局 bot（肉身）
POST …/conversations/{id}/agents/add  # 仅 membership：已有 agent_id
```

（现有 list members + list agents 可先聚合；新客户端只用 participants。）  
身份字段与迁移见 [global-bot-identity-design](global-bot-identity-design.md)。

### 4.2 Mentions（Phase 1）

今日：`chat_message_mentions(message_id, mentioned_account_id)` 仅人。

终态二选一（latest-only，推荐 B）：

**A.** 加 `mentioned_agent_id` 可空 + check 恰一非空  
**B.** 替换为：

```text
message_mention_targets(
  message_id,
  target_kind TEXT CHECK IN ('account','agent'),
  target_id   TEXT,
  PRIMARY KEY (message_id, target_kind, target_id)
)
```

Wire：`ChatMessageSummary` 增加 `mentioned_agent_ids` 或统一：

```json
"mentions": [{ "kind": "account"|"agent", "id": "…" }]
```

### 4.3 Agent inbox（物理表可暂名 dispatch）

今日 `agent_dispatch_queue`：

- 幂等 `(origin_message_id, agent_id)`  
- status: pending | inflight | succeeded | failed_terminal  
- worker: `jobs/agent_dispatch_worker.rs`  

领域命名：

| 旧（实现） | 新（文档/日志/API 文案） |
|------------|-------------------------|
| AgentDispatchQueue | Agent inbox |
| try_agent_dispatch | plan_agent_deliveries / enqueue_agent_inbox |
| dispatch_id | delivery_id（可别名同一列） |
| HostCommand | runtime port command |

### 4.4 Author

- DB：`sender_type='agent'`, `sender_agent_id`, 过渡期 `sender_account_id=owner` 可保留作审计 FK  
- Wire 终态：`sender` 用 `SenderRef` 风格，停止长期 `UserSummary.account_id = agent_id` 滥用  
- 回复 id：`agent-result:{conv}:{session}:{origin_message_id}`

---

## 5. Runtime port (Host) — private adapter

Hub → Host 仍可通过：

- `agent_session.start` / `agent_session.send_input`（今日）  
或未来单一 `consume_message`  

Daemon：

- 收 runtime 指令 → 喂 CLI → ingest raw  
- **不**把 raw 当多端聊天气泡 SSOT  
- 最终人读文本：Hub agent message（projector 或 host_projection 上行）

`/ws/host` 职责：**机器执行面**；不承载人类 IM 写权威。

---

## 6. Online & availability

| Signal | Definition | UI |
|--------|------------|-----|
| Account sync | `/ws/client` live + 可 refresh | 主 Online / Reconnecting / Auth required |
| Host online | installation `/ws/host` live | 「This Mac / Host ready」 |
| Agent available | agent member + bound host online | bot 成员状态；@ 后可 pending |

**禁止**：仅 Host online 且 Account 401 时显示可发送的完整 Online。

---

## 7. API sketch (incremental)

| Endpoint | Role |
|----------|------|
| `POST /v1/conversations/{id}/messages` | 人类发送；commit + mentions + inbox enqueue |
| `…/participants` | 统一成员 |
| `…/agents/add|remove` | bot roster（可保留） |
| `…/agents/message` | agent/host_projection 写气泡；**禁止** client_live 再投递 |
| Host ticket + `/ws/host` | runtime only |

发送路径代码收敛：

- `conversations/use_case` 与 `http/v1/social` **一份** mention 解析  
- 去掉发送成功后散落的第二套「业务 if @agent」心智；enqueue 与消息同事务或明确 outbox 次序

---

## 8. Phased implementation map

| Phase | Scope |
|-------|--------|
| **0** | 本文 + ADR 0021 + architecture SSOT 改写（文档） |
| **1** | polymorphic mentions + participants API + 统一 extract |
| **2** | inbox 语义收口（单一 plan 入口）；worker/runtime 不变可先 |
| **3** | Desktop/Mobile composer participants；Online 组合状态 |
| **4** | **membership-first live**；删 silent auto-attach；收敛 Desktop-native 与 client_live 领域事件 |

### Phase 4 notes (current / live backend)

- **Auto-attach removed**: no `ensure_host_runtime_agents_for_mentions` on send. Delivery targets = reply-to-agent → structured `mentioned_agent_ids` (appearance order) → sole-agent room rule. Text is never a delivery target; `#session_short` is only a session hint for agents already in structured mentions.
- **Unmatched `@codex`**: user-visible failure bubble / `agent_error` when token looks agentish but roster has no match.
- **Mentions / mailbox schema**: polymorphic mentions + `bot_message_deliveries` + digital body + revisions/deployments all live in single latest-only `migrations/*/0001_initial.sql` (wipe volumes; no 0002+ ALTER chain, no post-migrate legacy rebuild).
- **Desktop collab**: Account live → WS `AppendMessage` only; bot activation is Hub mailbox (no Composer local fan-out dual path).
- **Online UI**: Desktop `accountSyncStatus` (`/ws/client`) is primary Online; `cloudStatus`/`hubOnline` is Host readiness only.

---

## 9. Acceptance invariants

1. `@人` 与 `@agent` 均产生**结构化 mention**（人进 account 目标，bot 进 agent 目标）。  
2. 无 agent 投递规则时，**零** runtime HostCommand。  
3. `(origin_message_id, agent_id)` 重放 ≤ 一次 inbox 意图。  
4. Agent 回复多端经 Hub DurableEvent 可见。  
5. `host_projection` / `system` 不触发 agent inbox。  
6. 撤销 host link → agent unavailable；人类消息仍同步。  
7. 文档不再把「远程协作」定义为 host command 总线。  
8. Account 失效不得仅因 Host 在线显示可发送。

---

## 10. Relationship to existing specs

| Spec | Keep | Change |
|------|------|--------|
| Hub SSOT 2026-08-02 | 气泡写者、幂等 id、禁止 UI 扫表 dual-write | 触发：dispatch → participant delivery |
| Backend delivery 2026-08-03 | outbox lanes、Push、watch 键 | AgentDispatchQueue 章节 = Agent inbox |
| Client sync 2026-08-03 | Outbox/cursor | Online 以 Account 为主 |
| Realtime surface | T0–T4 | mention 目标含 agent；bot availability |
| ADR 0020 | Account vs Host 人机分权 | 另见 0021 bots |
| [global-bot-identity-design](global-bot-identity-design.md) | 全局 bot 身份、数字肉身、membership vs session | 本文管投递；身份/肉身 SSOT 以该文为准 |

---

## 11. Implementation anchors (current code)

| Concern | Path |
|---------|------|
| Send + post-hoc delivery | `http/v1/conversations.rs` → `try_agent_dispatch` (gated by `message_source.allows_agent_dispatch`; pipeline errors → `notify_agent_dispatch_pipeline_error`) |
| Plan / forward | `http/v1/social.rs` `plan_agent_deliveries` (reply → structured `mentioned_agent_ids` appearance order → sole-agent) |
| Inbox table | 物理表 `bot_message_deliveries`；模块 `store/agent_dispatch_queue.rs`（领域：Agent inbox / Bot mailbox） |
| Worker | `jobs/agent_dispatch_worker.rs` |
| Mentions (human + agent) | `conversations/use_case.rs` `extract_participant_mentions` (single extract path; Vec+seen appearance order) |
| Participants API | `…/conversations/{id}/participants` |
| Agent bubble insert | `store/social/agents.rs` |
| Projector | `turn_completion.rs`, `completion_watch.rs` |
| Schema | latest-only `migrations/{sqlite,postgres}/0001_initial.sql` only |

演进原则：**reuse 队列与幂等，替换产品语义与统一 mention/入口**；schema 不保留增量兼容链。
