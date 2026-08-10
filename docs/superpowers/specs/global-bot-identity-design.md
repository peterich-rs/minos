# Unify Agent as Global Bot Identity

| Field | Value |
|-------|--------|
| Status | **Normative target** |
| Scope | Bot 身份模型：全局唯一 bot 用户、跨 conversation membership、per-conversation session、数字肉身（模型/推理/系统提示词等）归属 |
| Supersedes (partial) | 把 agent profile 当作端侧私货、把 host_runtime 槽位当产品 bot 身份、为每个 conversation 新建 agent 的产品路径 |
| Related | [ADR 0021](../../adr/0021-agent-as-conversation-bot-participant.md) · [agent-participant-delivery](2026-08-09-agent-participant-delivery.md) · [architecture-messaging.md](../../architecture-messaging.md) · [hub-collaboration-message-ssot](2026-08-02-hub-collaboration-message-ssot.md) |
| Non-goals | E2EE；Cloud 跑 CLI；Agent 登录为 Account JWT；Discord slash/webhook 交互平台完整复刻；大群百万写扩散 |

> **一句话**：Bot 是全局唯一的“数字人”身份；conversation 只持有 membership；每个 conversation 为该 bot 维护独立 session。模型 / 推理等级 / 系统提示词等构成 bot 的数字肉身，属于身份层，不属于“进群时新建的会话专属 agent”。

**规划约束**：遵守 AGENTS.md Final-Architecture Planning Rule — 只设计终态；Phase 是终态切片，不是临时兼容层。

---

## Breaking Change Notice

本改造对 monorepo 内全部一等客户端（Desktop / Mobile / Web / daemon local RPC）为 **latest-only** 破坏性收口（`minos-tui` 已移除）：

1. **Bot 身份唯一权威在 Hub `agents`**。Daemon `agent_profiles` 与 Mobile 本地 `agent_profiles.json` 不再是 bot 身份 SSOT；可短暂作为编辑缓存，但创建/更新/拉人必须以 Hub bot 为准。
2. **进群只写 membership**。`POST …/agents/add` / participants 不得再隐含 “create agent in this conversation”。
3. **@ 只解析 conversation participants 中的 bot 身份**（`agent_id` / bot name），不得用裸 `codex`/`claude` runtime 名或未入群 profile 名作为投递目标。
4. **Wire 作者**从 `UserSummary.account_id = agent_id` 迁到 `SenderRef` / bot card；`ChatMessageSummary.sender` 对 agent 消息不再伪装成人。
5. **Session 启动**必须带真实 `agent_id`（全局 bot）；`agent_codex` 等虚拟 alias 仅允许内部种子/迁移，不得作为产品主路径。
6. **数字肉身字段**（model / reasoning_effort / instructions / tools policy）上收 Hub；start session 的 override 是 session 层，不改写 bot 身份。

下游：同步改 protocol、backend schema/API、daemon profile 解析、Desktop Agents UI、Mobile profile store、文档与测试；无外部 semver 消费者。

---

## Feasibility Assessment

表关系骨架**已经是**全局 bot 模型：

- Hub `agents` 全局行 + `conversation_agent_members(conversation_id, agent_id)` 多对多 membership（`crates/minos-backend/migrations/*/0001_initial.sql`）
- `agent_sessions` 挂 `conversation_id` + `agent_id`，执行上下文与身份分离
- ADR 0021 + participant-delivery 已规定：bot 是 participant，不是 Account；membership-first；inbox 按 `agent_id` 投递
- Desktop 已有 `agent_profiles`（daemon SQLite：name/model/reasoning_effort/instructions）与 Agents UI；字段面接近“数字肉身”，缺的是 **Hub 权威与跨端共享**
- Mobile 有本地 profile store，但明确 **CLIENT-LOCAL**——这是要拆除的契约，不是不可迁移的数据

主要工作是 **身份归属上收 + 产品路径清口**，不是重做 IM 总线。  
**Fully feasible** under final-architecture-only planning. Caveat：Desktop/Mobile 现有本地 profile 需一次性导入 Hub bot，latest-only 允许丢弃未导入的本地私货。

---

## Current Surface Inventory

### Hub identity / membership

- `crates/minos-backend/migrations/{sqlite,postgres}/0001_initial.sql` — `agents`, `conversation_agent_members`, `agent_sessions`
- `crates/minos-backend/src/store/social/agents.rs` — `register_agent`, `ensure_host_runtime_agent`, add/remove membership, agent message insert
- `crates/minos-backend/src/http/v1/social.rs` — participants API, `plan_agent_deliveries`, `try_agent_dispatch`, agent CRUD
- `crates/minos-backend/src/conversations/use_case.rs` — `extract_participant_mentions`（roster 内 human∪agent）
- `crates/minos-backend/src/agent_sessions/use_case.rs` — `resolve_agent`（真实 agent 行 + `agent_codex` 等虚拟 alias ensure）
- `crates/minos-backend/src/store/agent_dispatch_queue.rs` — inbox 幂等 `(origin_message_id, agent_id)`
- `crates/minos-backend/src/turn_completion.rs` / `completion_watch.rs` — 最终 bot 气泡

### Protocol

- `crates/minos-protocol/src/messages.rs` — `AgentSummary`, `RegisterAgentRequest`, `ConversationParticipantsResponse`, `SenderType`, `MessageSource`, `StartAgentRequest.profile_id`, `AgentProfileSummary*`
- `crates/minos-protocol/src/realtime.rs` — `SenderRef`（Durable 已有；Chat HTTP 摘要仍偏 `UserSummary`）
- `crates/minos-protocol/src/local_rpc.rs` — daemon `list/create/update/delete_agent_profiles`

### Host / daemon profiles（本地肉身，非 Hub）

- `crates/minos-daemon/migrations/0001_initial.sql` — `agent_profiles`
- `crates/minos-daemon/src/store/mod.rs` — profile CRUD
- `crates/minos-daemon/src/agent.rs` — start 时 `profile_id` → model/effort/instructions
- `crates/minos-prompt-runtime/` — session prompt 编译（bundle/digest）；与 bot 身份尚未绑死

### Desktop

- `apps/desktop/src/features/agents/*` — Agents 配置 UI
- `apps/desktop/src/store/workspace/resolve-dispatch-targets.ts` — @ 解析混 profile/runtime
- `apps/desktop/src/store/workspace/use-cases.ts` / `send-dispatch.ts` — 本地 bot 执行 + host_projection
- `apps/desktop/src/features/chat/Composer.tsx` — participants / mention picker
- `apps/desktop/src/features/work/lib/create-conversation-form.ts` — 创建会话时 roster 默认 brief 来自本地 profile

### Mobile

- `apps/mobile/lib/infrastructure/agent_profile_store.dart` — **CLIENT-LOCAL only** 契约
- `apps/mobile/lib/domain/agent_profile.dart` / profile services
- `apps/mobile/lib/infrastructure/minos_core.dart` — `registerAgent` / `addAgentToConversation`
- mention extract + participants roster（本分支已部分落地）

### Docs

- `docs/adr/0021-agent-as-conversation-bot-participant.md`
- `docs/superpowers/specs/2026-08-09-agent-participant-delivery.md`
- `docs/architecture-messaging.md` §1 / §3.4

---

## Design

### 1. Target architecture

```text
┌──────────────────────────────────────────────────────────────────────────┐
│  Bot Directory (Hub SSOT)                                                 │
│  agents: 全局唯一 bot 用户 = 数字肉身                                      │
│    agent_id, name, avatar, runtime, model, effort, system_prompt, …     │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │ join / leave (membership only)
                                ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  Conversation                                                             │
│  conversation_members (humans)                                            │
│  conversation_agent_members (bots)  ← 同一 agent_id 可出现在多 conversation │
│  chat_messages + polymorphic mentions                                     │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │ on delivery / @bot
                                ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  Agent Session (per conversation × bot)                                   │
│  agent_sessions: 执行上下文 / transcript / approvals                      │
│  默认每 (conversation_id, agent_id) 一条 active session                   │
│  可显式“新开 session”，仍是同一 bot 身份                                   │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │ runtime port only
                                ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  Host daemon + CLI                                                        │
│  消费 inbox intent；用 bot 肉身 + session override 启动 CLI                 │
│  本地 profile 表降级为 cache / offline 缓冲，不再是身份权威                    │
└──────────────────────────────────────────────────────────────────────────┘
```

### 2. Glossary（钉死）

| Term | Meaning |
|------|---------|
| **Bot identity** | 全局唯一参与者实体 `agent_id`；类比 Discord bot user / 企微应用号 |
| **Digital body（数字肉身）** | 构成 bot 行为与呈现的配置：name/avatar、runtime、default model、default reasoning_effort、system prompt / prompt bundle、tools/skills policy、default workspace（可选） |
| **Membership** | `(conversation_id, agent_id)`；拉入/移出，**不复制** bot |
| **Session** | bot 在某 conversation 内的执行线程；上下文隔离的单位 |
| **Owner** | `owner_account_id`：谁拥有/管理该 bot；**不是** bot 发言身份 |
| **Runtime port** | HostCommand / daemon CLI；私有适配，不是协作协议 |

**禁止混用：**

- bot identity ≠ session  
- bot identity ≠ runtime bin 名（`codex`）  
- bot identity ≠ 本地 profile 行（除非已同步为同一 `agent_id`）  
- “在会话里创建一个 agent” ≠ “创建一个全局 bot 再拉入会话”

### 3. Key design decisions

1. **Bot 全局唯一，跨 conversation 复用**  
   - 选择：一个 `agent_id` 可 membership 到 N 个 conversation。  
   - 拒绝：每个 conversation create 新 agent 行当“会话专属 bot”。  
   - 理由：与人相同——人进多个群不会变成多个人。

2. **数字肉身挂在 bot identity，不挂在 conversation / session 主身份上**  
   - 选择：model / reasoning_effort / instructions / runtime 等默认值存 `agents`（或 `agents` + `agent_prompt_revisions`）。  
   - 拒绝：Mobile “profiles never reach backend”；Desktop profile 仅 daemon SQLite 权威。  
   - Session 可 override（这次用别的 model），但 override 不改 bot 本体。

3. **Membership-only join**  
   - 选择：add-to-conversation = insert `conversation_agent_members`。  
   - 拒绝：add 时 clone bot、或 ensure-host-runtime 并自动 join。  
   - `ensure_host_runtime` 最多 seed registry 槽位，**永不** join。

4. **Per-(conversation, bot) session isolation**  
   - 选择：默认每个 `(conversation_id, agent_id)` 一条 active session；继续 @ 则 `send_input`；用户可 “New session” 开新上下文。  
   - 拒绝：跨 conversation 默认共享同一 CLI session 上下文（除非未来显式 shared-memory 产品）。  
   - 拒绝：新 session 产生新 `agent_id`。

5. **@ 与投递只认 bot identity**  
   - 选择：mention target = `agent_id`；解析仅限本 conversation participants。  
   - 拒绝：裸 runtime 名、未入群 profile 名、search 结果作为投递目标。  
   - Inbox 幂等键保持 `(origin_message_id, agent_id)`。

6. **Sender 一等 bot 卡**  
   - 选择：HTTP/WS 消息作者用 `SenderRef::Agent { agent_id, name, … }` 或等价 bot card。  
   - 拒绝：长期 `UserSummary.account_id = agent_id`。  
   - `sender_account_id = owner` 仅 DB 审计 FK（可选），不进产品身份。

7. **host_runtime 降级为能力种子，不是产品 bot 目录**  
   - 选择：用户可见 bot 目录以 **user_configured**（及导入后的）bots 为主。  
   - `source=host_runtime` 行仅表示 “该 owner 在 Host 上可用的 runtime 槽位/默认种子”，可被 “创建 bot” 向导引用，但 UI 主列表是具名 bot 用户。  
   - 拒绝：`(owner, runtime_agent)` 唯一槽位冒充用户配置的多个 bot 人格。

8. **本地 profile → Hub bot 的迁移方向**  
   - 选择：导入/创建时分配稳定 `agent_id`；daemon 可缓存同 id 投影以降低启动延迟。  
   - 拒绝：继续双 SSOT（Hub agents vs daemon profiles 对等权威）。

### 4. Data model (target)

#### 4.1 `agents` — Bot identity + digital body

```sql
-- conceptual target (latest-only evolution of existing agents table)
agents (
  agent_id              TEXT PRIMARY KEY,
  owner_account_id      TEXT NOT NULL REFERENCES accounts(account_id),
  name                  TEXT NOT NULL,          -- unique per owner (normalized)
  display_name          TEXT NOT NULL,
  description           TEXT NOT NULL DEFAULT '',
  avatar_url            TEXT,
  source                TEXT NOT NULL,          -- user_configured | host_runtime_seed | system
  status                TEXT NOT NULL DEFAULT 'active',  -- active | disabled
  runtime_agent         TEXT NOT NULL,          -- codex|claude|gemini|opencode|grok
  default_model         TEXT NOT NULL DEFAULT '',
  default_reasoning_effort TEXT NOT NULL DEFAULT '',
  system_prompt         TEXT NOT NULL DEFAULT '',
  prompt_bundle_id      TEXT,                   -- optional structured bundle
  tools_policy_json     TEXT,                   -- optional
  default_workspace_path TEXT,                  -- optional hint only
  created_at_ms         INTEGER NOT NULL,
  updated_at_ms         INTEGER NOT NULL
);

-- 同一 owner 下 active bot 名唯一（case-insensitive；@ 解析与索引一致）
-- 实现：migrations/*/0004_agent_digital_body.sql → idx_agents_owner_name_active
UNIQUE (owner_account_id, lower(name)) WHERE status = 'active';

-- host_runtime_seed 仍可 per (owner, runtime) 唯一；user_configured 不受此限
```

**字段归属：**

| 字段 | 层 | 说明 |
|------|----|------|
| name / display_name / avatar / description | identity 呈现 | 全 conversation 同一张脸 |
| runtime_agent / default_model / default_reasoning_effort | 数字肉身 | 默认启动参数 |
| system_prompt / prompt_bundle_id | 数字肉身 | 人设 / 系统提示 |
| tools_policy_json | 数字肉身 | 工具/技能边界 |
| default_workspace_path | 可选 hint | **真正 workspace 以 project/session 为准** |
| owner_account_id | 管理关系 | 谁可编辑/删除 bot；非发言身份 |

#### 4.2 Membership（不变语义，收紧用法）

```sql
conversation_agent_members (
  conversation_id,
  agent_id,                 -- FK → agents.agent_id（全局 bot）
  added_by_account_id,
  joined_at_ms,
  PRIMARY KEY (conversation_id, agent_id)
);
```

- 拉入 = INSERT membership  
- 移出 = DELETE membership（不删 `agents`）  
- 同一 `agent_id` 出现在 A/B 两个 conversation = 同一个 bot

#### 4.3 Session（per conversation）

```sql
agent_sessions (
  session_id PK,
  conversation_id NOT NULL,
  agent_id NOT NULL,           -- 全局 bot
  host_installation_id,
  status,
  -- optional session overrides (do not mutate agents row):
  model_override,
  reasoning_effort_override,
  workspace_path,
  ...
);
```

**默认策略：**

- 投递时：若存在该 `(conversation_id, agent_id)` 的 active/reusable session → `send_input`  
- 否则 → `start` 新 session（仍同一 `agent_id`）  
- “New session” 显式 API：归档旧 active，开新 session，**agent_id 不变**

#### 4.4 Mentions / author（与 participant-delivery 对齐）

- mention SSOT：`target_kind=agent` + `target_id=agent_id` + ordinal  
- 消息作者：`SenderRef::Agent { agent_id, display_name, … }`  
- 回复 id：`agent-result:{conversation_id}:{session_id}:{origin_message_id}`  
  - 注意：id 含 session 是为幂等与多 session；**展示身份仍是 agent_id**

### 5. API surface (target)

#### 5.1 Bot directory（全局）

| Endpoint | Role |
|----------|------|
| `GET /v1/agents` | 列出我拥有/可用的全局 bots |
| `POST /v1/agents` | 创建全局 bot（数字肉身） |
| `PATCH /v1/agents/{agent_id}` | 更新数字肉身 |
| `POST /v1/agents/{agent_id}/disable` | 停用（不删历史消息） |
| `POST /v1/agents/ensure-host-runtime` | **仅** seed runtime 槽位；不 join；不替代 create bot |

#### 5.2 Conversation membership

| Endpoint | Role |
|----------|------|
| `GET/POST /v1/conversations/{id}/participants` | 唯一成员读写面（humans ∪ bots） |
| `POST …/participants/bots` 或保留 `…/agents/add` | body: `{ agent_id }` — **必须已存在** |
| `POST …/participants/bots/remove` | leave membership only |

创建 conversation：

```json
{
  "title": "Review lane",
  "member_account_ids": ["acc_…"],
  "agent_ids": ["agent_…"]   // 已有全局 bot，不是 inline create
}
```

#### 5.3 Messaging / delivery（不变主链，身份收紧）

```text
POST /v1/conversations/{id}/messages
  → mentions include agent_id of global bots in roster
  → agent inbox rows keyed by agent_id
  → worker resolves session for (conversation_id, agent_id)
  → runtime start/send_input with bot body (+ session override)
  → agent reply as SenderRef::Agent{agent_id}
```

#### 5.4 Wire types（示意）

```rust
// conceptual — evolve minos-protocol
pub enum SenderRef {
    User { account_id: String, minos_id: String, display_name: String },
    Agent { agent_id: String, name: String, display_name: String },
    System,
}

pub struct BotSummary {
    pub agent_id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub runtime_agent: String,
    pub default_model: String,
    pub default_reasoning_effort: String,
    pub status: String,
    pub owner_account_id: String,
    // system_prompt: 列表可省略；详情接口返回
}

pub struct ConversationParticipantsResponse {
    pub humans: Vec<UserSummary>,
    pub bots: Vec<BotSummary>,  // rename from agents when clients ready
}
```

### 6. Runtime resolution

```text
inbox row (origin_message_id, conversation_id, agent_id)
  → load Bot identity (digital body)
  → find reusable session for (conversation_id, agent_id)?
       yes → send_input(session_id, text, attachments)
       no  → start_session(
              agent_id,
              conversation_id,
              model = override ?? bot.default_model,
              effort = override ?? bot.default_reasoning_effort,
              instructions = bot.system_prompt (+ prompt runtime bundle),
              workspace = session/project path ?? bot.default_workspace_path
            )
  → CLI on bound Host
  → project final bubble as agent_id (not session-as-identity)
```

Daemon 侧：

- 若本地有同 `agent_id` 的 cache 行可加速；**冲突以 Hub bot 为准**  
- `profile_id` 本地 RPC 参数：迁移期 map 到 `agent_id`；终态 API 直接 `agent_id`

### 7. Product UX invariants

1. **Agents 页 / Bot 目录**：管理全局 bot 用户（创建、编辑肉身、停用）。  
2. **Conversation 成员**：从 bot 目录多选拉入；显示同一 bot card。  
3. **Composer @**：只列出本 conversation participants（人 + 已拉入 bot）。  
4. **同一 bot 在多群**：改 display name / 系统提示词后，各群看到同一身份更新（进行中 session 策略见下）。  
5. **Sessions 侧栏**：按 conversation 展示该 bot 的 session 树；标题是 bot 名，不是新身份。  
6. **不可用状态**：bot disabled 或 bound Host offline → membership 仍在，availability=unavailable；人类仍可互聊。
   - **实现不变量**：`list_conversation_agents`（participants）可含 disabled；`list_conversation_agents_active` 用于 @ 解析与 mailbox delivery。`UpdateAgentRequest` 省略的 digital-body 字段（status / avatar_url / system_prompt / default_reasoning_effort）**merge 保留**，禁止全量默认值擦写。

**进行中 session 与肉身变更：**

- 默认：已 active session **不热改** system prompt/model（避免半局混乱）  
- 下次 start 使用新肉身  
- UI 可提示 “配置已更新，新 session 生效”

### 8. Delivery rules（身份视角重述）

与 participant-delivery 一致，仅强调 target 是全局 bot：

| Priority | Condition | Deliver to |
|----------|-----------|------------|
| 1 | reply_to agent message | 该消息的 `sender_agent_id`（全局 bot） |
| 2 | structured agent mentions | 每个唯一 `agent_id`（外观序） |
| 3 | sole human + sole bot membership，裸文本 | 该 sole `agent_id` |
| else | — | 不 enqueue |

未入群 bot 名 / runtime 名 → 用户可见失败；**不** create；**不** sole-route 到错误 bot。

### 9. Migration from local profiles

| Source | Action |
|--------|--------|
| Daemon `agent_profiles` | 导入为 Hub `agents`（`source=user_configured`）；保留映射 `local_profile_id → agent_id` 直至 UI 切完 |
| Mobile `agent_profiles.json` | 同导入；删除 “never POST to backend” 契约 |
| `host_runtime` rows | 保留为 seed/capability；向导 “用 Claude 创建一个 bot” 预填 runtime |
| 会话内仅有 runtime 成员无具名 bot | 升级为显式 bot 或绑定已有 bot；禁止继续用 bin 名当成员身份 |

Latest-only：未导入的本地 profile 可丢弃；不双写长期兼容。

### 10. Delete list（本专项）

1. 为每个 conversation **新建** agent 行当作进群  
2. Mobile/Desktop **本地 profile 作为 bot 身份 SSOT**  
3. `@codex` 未入群 silent ensure+join  
4. 产品主路径使用 `agent_codex` 虚拟 id  
5. `UserSummary` 伪装 bot 作者  
6. 把 `session_id` 当用户可见 bot 身份  
7. 把 `workspace_path` 当 bot 永恒身份的核心（降为 default/session）  
8. Agents UI 与 IM roster 使用两套不可关联的 id  
9. 文档/注释 “profiles must never reach backend”  
10. host_runtime 唯一槽位冒充多人格 bot 目录  

### 11. Acceptance invariants

1. 同一 `agent_id` 可同时 membership 于 ≥2 conversations；两边 roster 与消息作者为同一 bot。  
2. 创建 bot 一次；拉入 A、拉入 B **零** 新 `agents` 行。  
3. 从 A 移除 bot 不删除 `agents`，B 不受影响。  
4. A 与 B 的 session 上下文隔离；bot 肉身配置相同。  
5. 更新 bot system_prompt/default_model 写 Hub；其他端 bot 详情可见；新 session 使用新肉身。  
6. @ 只命中本 conversation 已拉入 bot；未入群失败可见。  
7. Inbox / 回复 / 未读全端以 `agent_id` 为 bot 主键。  
8. 无路径再把本地-only profile id 当作多端协作身份。

### 12. Relationship to existing specs

| Spec | Keep | Change under this design |
|------|------|---------------------------|
| ADR 0021 | bot participant；无 Account 登录 | 加硬：全局唯一身份 + 数字肉身归属 |
| participant-delivery | inbox、房规、membership-first | target 明确为全局 agent_id；session per conversation |
| Hub SSOT | 气泡写者/幂等 | 作者卡改为 Bot identity |
| Client sync | 多端消息同步 | bot 目录与 roster 也需多端一致（Hub） |
| Desktop profiles / Mobile local profiles | 字段经验 | 权威上收 Hub |

---

## Phased Implementation

## Phase 0: Freeze identity invariants in docs — **DONE**

**File: `docs/adr/0021-agent-as-conversation-bot-participant.md`**

- 增加 “Global bot identity” 条款：跨 conversation 复用；session 非身份；数字肉身在 agents。  
- 指向本文为身份/肉身规范。

**File: `docs/superpowers/specs/2026-08-09-agent-participant-delivery.md`**

- Glossary 增加 Bot identity vs Session。  
- §4.1 agents 表扩展为数字肉身字段说明。  
- 明确 join = membership only。

**File: `docs/architecture-messaging.md`**

- §1 用 “全局 bot 用户” 表述；权威阶梯纳入本文。

**File: `docs/architecture-overview.md` / `architecture-business-flow.md` / `architecture-daemon.md` / `architecture-desktop.md` / `backend-formal-development.md` / cloud-identity long-term**

- 清理 `agent_codex` slug 目录 / 本地 profile SSOT 过期叙述；交叉引用本文。

Rationale：先锁语义，避免实现期回流本地 profile SSOT。

## Phase 1: Hub schema + protocol for digital body — **DONE (core)**

**File: `crates/minos-backend/migrations/*/0004_agent_digital_body.sql`**

- 扩展 `agents`：`display_name`, `avatar_url`, `status`, `default_reasoning_effort`, `system_prompt`。  
- `name` 在 owner 下 active 唯一索引。  
- `prompt_bundle_id` / `tools_policy_json` 仍可后续加列。  
- `workspace_path` 保留为 default hint only。

**File: `crates/minos-protocol/src/messages.rs`**

- `AgentSummary` 扩数字肉身字段（仍名 AgentSummary；BotSummary rename 可后置）。  
- `RegisterAgentRequest` / `UpdateAgentRequest` 纳入 display/avatar/effort/system_prompt/status。  
- `SenderRef` 作者卡与 `ConversationParticipantsResponse.bots` rename 仍属后续 Phase。

**File: `crates/minos-backend/src/store/social/agents.rs`**

- `register_agent_full` / `update_agent_full` 读写新字段；旧 `register_agent`/`update_agent` 保留兼容封装。  
- membership API 仍只 insert `conversation_agent_members`（不 create bot）。

**File: `crates/minos-backend/src/http/v1/social.rs`**

- create/update/list 返回完整肉身字段。  
- Desktop/Mobile/FRB 类型已对齐 wire。

**Verification**

- `cargo check -p minos-protocol -p minos-backend -p minos-mobile -p minos-ffi-frb`  
- `cargo test -p minos-backend --lib store::social::` + `--test v1_social`

## Phase 2: Session binding by global agent_id — **DONE (core)**

**File: `crates/minos-backend/src/agent_sessions/use_case.rs`**

- `resolve_agent`：主路径只接受真实 `agents.agent_id`。  
- 虚拟 `agent_codex` 等：仅 `cfg(test)` 或 `MINOS_ALLOW_VIRTUAL_AGENT_ALIASES=1`。  
- start 时加载 bot 肉身 → 填 model/effort/instructions；request override 仅 session 层。  
- 查找 reusable session：`(conversation_id, agent_id)`。

**File: `crates/minos-backend/src/http/v1/social.rs` (`plan_agent_deliveries` / forward)**

- 投递与 session 复用一律按全局 `agent_id`。  
- 日志字段：`agent_id` 为主，`session_id` 为执行上下文。

**File: `crates/minos-backend/src/turn_completion.rs`**

- 投影气泡 `sender_agent_id = agent_id`（全局）；wire bot card 同源。

**File: `crates/minos-daemon/src/agent.rs` + `rpc_server.rs`**

- start/send_input 接受 Hub 下发的 bot body 快照。  
- `profile_id`：若仍存在，必须 resolve 到同一 `agent_id` 的 cache；冲突以命令内嵌 body 为准。

## Phase 3: Desktop — bot directory + roster join — **DONE (core)**

**File: `apps/desktop/src/features/agents/*`**

- Agents 页改为 **Hub bot 目录** CRUD（数字肉身编辑）。  
- 创建 bot = `POST /v1/agents`，不再只写 daemon profile。

**File: `apps/desktop/src/features/work/lib/create-conversation-form.ts` + conversation create UI**

- Roster 选择 **已有 agent_id 列表**。  
- 默认 brief 来自 Hub bot description，不是本地 profile 旁路。

**File: `apps/desktop/src/features/chat/Composer.tsx`**

- @ picker = participants（人 + 已拉入 bot）。  
- 展示 bot display_name；identity key = agent_id。

**File: `apps/desktop/src/store/workspace/resolve-dispatch-targets.ts`**

- 删除/降级：用未入群 profile 名或裸 runtime 当投递目标。  
- 与 Hub 房规对齐：只 route roster `agent_id`。

**File: `apps/desktop/src/store/workspace/use-cases.ts` / `send-dispatch.ts`**

- **Account live**：`client_live` 仅上行；Hub mailbox 投递；**不**再 Composer 本地 `start_agent` 主链。  
- **Offline workbench**：无 Account 时仍可本地 fan-out（非多端协作路径）。

**File: daemon profile store usage in Desktop**

- 可选：登录后 pull Hub bots → 写本地 cache 表（id 对齐）。  
- UI 禁止“仅本地 profile、无 agent_id”的协作发送。

## Phase 4: Mobile / Web — same bot directory — **DONE (core)**

**File: `apps/mobile/lib/infrastructure/agent_profile_store.dart` + domain**

- 废除 “never POST profiles” 作为架构契约。  
- 本地文件最多 cache；CRUD 走 Hub agents API。

**File: `apps/mobile/lib/infrastructure/minos_core.dart` + repositories/providers**

- register/update/list agents；addAgentToConversation(agent_id only)。  
- 成员页 / @ picker 用 participants bots。

**File: `apps/web` bot surfaces**

- 与 Desktop 同 API：bot 目录 + conversation 拉人；无 Host 执行。

**File: FRB / `crates/minos-mobile` / `crates/minos-ffi-frb`**

- 暴露完整 bot 字段与 participants；生成物同步。

**Also shipped**

- Wire `MessageSender` (Account | Bot) through Desktop/Mobile timeline; author labels use `display_name` + `bot_id`.  
- Desktop Account-live send is `client_live` only (Hub mailbox); offline workbench retains local fan-out.

## Phase 5: Import path + delete old identity paths — **DONE (core)**

**File: Desktop/Mobile migration one-shot**

- Desktop AgentsView：online 时 create/update 写 Hub；daemon profile 仅 offline cache（name-matched mirror best-effort）。  
- Mobile：register/update 走 Hub；本地 JSON 降级 cache。  
- 未做一次性 bulk-import 脚本：latest-only 允许用户在 Hub 重建 bot（未导入本地私货可丢）。

**File: `crates/minos-backend/src/agent_sessions/use_case.rs`**

- 关闭生产路径虚拟 agent alias（仅 `MINOS_ALLOW_VIRTUAL_AGENT_ALIASES=1` 或 crate unit `cfg(test)`）。

**File: docs + `scripts` rg gates**

- 文档与 AGENTS.md 已去掉 CLIENT-LOCAL SSOT 契约；membership-first 禁止 silent ensure+join。  
- Wire 作者使用 `MessageSender::Bot`；`sender_agent_id` 为 bot 主键（不再用 owner account_id 回退伪装 agent_id）。

**File: tests (backend + Desktop + Mobile)**

- v1_social：membership / multi-@ / sole-agent / disabled bot / digital-body merge。  
- Desktop：resolveDispatchTargets roster-only；hub timeline bot card。  
- Mobile：MessageSender grouping by bot_id。

## Phase 6: Verification

- `cargo test -p minos-backend`（含 v1_social 身份/membership 用例）  
- Desktop unit：resolve-dispatch-targets 仅 roster agent_id  
- Mobile unit：mention/participants 使用 agent_id  
- 手工矩阵：  
  1. 创建 bot → 拉入 A、B  
  2. A/B 分别 @ → 两 session，同一 bot 脸  
  3. 更新 system_prompt → 新 session 生效  
  4. 从 A 移除 → B 仍可 @  
  5. Mobile 与 Desktop 看到同一 bot 目录与作者卡  

---

## Architectural Notes

- **Semver / 兼容**：monorepo latest-only；无外部 crate 消费者承诺。  
- **与 ADR 0020 正交**：bot 仍不是 Account；owner 是人类账户；Host 仍是执行身体。  
- **与 participant-delivery 正交且加强**：delivery 仍 message-driven；本设计只钉 “被投递的是谁”。  
- **Prompt runtime**：`minos-prompt-runtime` 继续编译 session 提示；输入源改为 Hub bot body + session/conversation context，而不是无 id 本地 profile。  
- **Workspace**：项目/会话工作目录不是 bot 身份；bot.default_workspace 仅 hint。  
- **Availability**：bound Host online + bot status active；不等于 Account Online。  
- **不改变**：Hub 气泡 SSOT、inbox 表物理名可暂留、Cloud 不跑 CLI、人类多端 Sync Engine 主链。  
- **明确不做（本专项）**：Discord slash command 平台、bot OAuth、跨 conversation 自动共享记忆（可另开）。  
- **Side effects**：改 bot 名影响 @ 解析与历史展示名策略（历史消息可存当时 display snapshot 或现读 bot 表——推荐消息行存 display snapshot，点击进现卡）。  

### Message display snapshot（建议）

- 写入 agent 消息时拷贝 `display_name` 到消息侧（或 sender 快照），避免改名导致历史错乱。  
- roster / profile 页读 live bot 行。

---

## File Change Summary

- `apps/desktop/src/features/agents/*` -- Agents 页改为 Hub 全局 bot 目录与肉身编辑  
- `apps/desktop/src/features/chat/Composer.tsx` -- @ 仅 participants bot identity  
- `apps/desktop/src/features/work/lib/create-conversation-form.ts` -- 拉入已有 agent_id  
- `apps/desktop/src/store/workspace/resolve-dispatch-targets.ts` -- 去掉未入群 profile/runtime 投递  
- `apps/desktop/src/store/workspace/send-dispatch.ts` -- 本地执行绑定全局 agent_id  
- `apps/desktop/src/store/workspace/use-cases.ts` -- 发送/上行不 mint 会话专属 bot  
- `apps/mobile/lib/domain/*` + `infrastructure/agent_profile_store.dart` -- 废除本地-only 身份契约；cache 可选  
- `apps/mobile/lib/infrastructure/minos_core.dart` + repositories/providers -- Hub bot CRUD + membership  
- `apps/web/src/**` -- bot 目录与拉人（无 runtime）  
- `crates/minos-backend/migrations/**` -- agents 数字肉身字段与唯一约束  
- `crates/minos-backend/src/agent_sessions/use_case.rs` -- 真实 agent_id；肉身加载；session 复用  
- `crates/minos-backend/src/http/v1/social.rs` -- bot API + participants + delivery 身份  
- `crates/minos-backend/src/store/social/agents.rs` -- CRUD/membership 收紧  
- `crates/minos-backend/src/turn_completion.rs` -- 气泡作者全局 agent_id  
- `crates/minos-backend/tests/v1_social.rs` -- 跨 conversation 同 bot 与失败路径  
- `crates/minos-daemon/src/agent.rs` / `store/mod.rs` / `rpc_server.rs` -- profile 降级 cache；执行吃 bot body  
- `crates/minos-ffi-frb/**` + `crates/minos-mobile/**` -- bot 字段与 API 贯通  
- `crates/minos-protocol/src/messages.rs` + `realtime.rs` -- BotSummary / SenderRef / 请求字段  
- `crates/minos-prompt-runtime/**` -- 从 bot body 编译 session prompt（接入点）  
- `docs/adr/0021-agent-as-conversation-bot-participant.md` -- 全局身份条款  
- `docs/architecture-messaging.md` / `architecture-overview.md` / `architecture-business-flow.md` -- 叙事对齐  
- `docs/superpowers/specs/2026-08-09-agent-participant-delivery.md` -- identity vs session  
- `docs/superpowers/specs/global-bot-identity-design.md` -- 本文  
- `scripts/*` 或 CI gates -- Delete list rg 门禁  

---

## Appendix A: Mental model cheat sheet

```text
人：  account_id  ──membership──►  conversations  ──messages──►
Bot： agent_id    ──membership──►  conversations  ──messages──►
                      │
                      └──session──► 仅 bot 在该 conversation 的执行上下文

数字肉身 ∈ agent 行
进群 ≠ 克隆人
新 session ≠ 新 bot
```

## Appendix B: Example flows

### B1. 创建并拉入两个群

```text
POST /v1/agents { name: "CodeReviewer", runtime: claude, model, effort, system_prompt }
  → agent_id = bot_cr_1

POST /v1/conversations/A/agents/add { agent_id: bot_cr_1 }
POST /v1/conversations/B/agents/add { agent_id: bot_cr_1 }

agents 表仍 1 行；membership 2 行
```

### B2. 两群分别对话

```text
A: "@CodeReviewer fix flaky test"
  → inbox (origin_a, bot_cr_1) → session_A1 → reply as bot_cr_1

B: "@CodeReviewer summarize PR"
  → inbox (origin_b, bot_cr_1) → session_B1 → reply as bot_cr_1

session_A1 与 session_B1 隔离；作者都是 bot_cr_1
```

### B3. 改肉身

```text
PATCH /v1/agents/bot_cr_1 { system_prompt: "更严格的 review 标准" }
  → A/B roster 详情更新
  → 已在跑的 session_A1 可不热更新
  → 下一 start 使用新 prompt
```
