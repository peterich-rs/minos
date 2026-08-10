# Establish Bot Mailbox and WS-Native IM Bus

| Field | Value |
|-------|--------|
| Status | **Normative target** |
| Scope | 严格 IM 消息驱动：Account WS 写消息、Bot 逻辑邮箱、Host 共享执行连接、Bot-to-Bot 受控 mention |
| Supersedes (partial) | REST 作为协作消息主写路径；`ClientFrame` 混 Account/Host；`host_projection` 一刀切禁投递；Composer 本地 `start_agent` 当协作主路径 |
| Related | [ADR 0021](../../adr/0021-agent-as-conversation-bot-participant.md) · [global-bot-identity-design](global-bot-identity-design.md) · [agent-participant-delivery](2026-08-09-agent-participant-delivery.md) · [architecture-messaging.md](../../architecture-messaging.md) · [bot-identity-session-separation](2026-08-10-bot-identity-session-separation.md) |
| Non-goals | E2EE；Cloud 跑 CLI；Bot Account 登录；每 Bot 一条 WebSocket；长期双写兼容层 |

> **一句话**：协作平面只有 Message；Bot 有独立逻辑收件箱与发送身份，但**没有**独立 WebSocket。Hub 是邮局与调度器；每台 Host 用**一条** `/ws/host` 消费其全部 Bot 的 delivery 并上送回信。

---

## 0. Review of the external proposal

另一 agent 方案（Bot 逻辑信箱 + WS 原生 IM 总线）**方向正确，应采纳为终态主干**。以下为审阅结论。

### 0.1 Keep（强同意）

| 主张 | 理由 |
|------|------|
| Bot 有逻辑邮箱，无独立 WS | 符合 bot≠Account；一 Host 多 bot 可扩展 |
| Account `/ws/client` 上 `AppendMessage` 写协作消息 | 严格 IM：发的是消息帧，不是 REST 业务命令叙事 |
| Hub 事务：message + mentions + bot_deliveries 同提交 | 投递与消息原子，避免“库有消息无投递” |
| Host 专用协议，不再混 `ClientFrame` | principal 分型是正确边界 |
| `agent_session.start/send_input` 退出公开协作路径 | runtime 适配私有化 |
| 端到端只认 `bot_id` | 与 [global-bot-identity](global-bot-identity-design.md) 一致 |
| 用 hop/budget 替代 `host_projection` 绝对阻断 | 允许受控 Bot-to-Bot |
| Host 握手可 Bearer `hit_*` 直连 | 减 ticket 故障面；daemon 原生客户端适合 |

### 0.2 Modify（必须改）

| 原方案点 | 问题 | 修正 |
|----------|------|------|
| **立刻删除 REST `POST …/messages` 主写** | 一刀切会阻塞 Mobile/Desktop 未迁完 WS 写时的可用性；且媒体/冷路径仍需 HTTPS | **终态** WS 写为主；**实现 Phase** 允许短窗 `commit_conversation_message` 双入口（WS + REST 同 use case），客户端迁完后删 REST 写协作消息。**禁止**两套业务语义 |
| **删除 sole-agent / 裸 runtime 全部 fallback** | sole-agent 房规是合法 IM 产品（1 人+1 bot 裸文本），不是 auto-attach | **保留** sole-agent **仅当** membership 恰 1 bot 且无未匹配 agentish token；**删除** silent auto-attach 与裸 `@codex` 无 membership 猜路由 |
| **“人类客户端不再通过 REST 写协作消息”** 写进 Breaking 过硬 | ticket、上传、历史仍 HTTPS | Breaking 改为：**协作消息主写路径变为 Account WS `AppendMessage`；REST 写协作消息退役（同 commit 函数过渡）** |
| **bot_revisions / deployments 一次全上** | 范围过大，阻塞邮箱总线 | **分片**：Phase A 邮箱+WS 帧+lease；Phase B revision/deployment 固化；可用现 `agents` 数字肉身 + host_links 过渡 |
| **Host 删除全部 ticket** | 需网关鉴权改造与运维配合 | 列为 **Host auth 简化** 独立切片；邮箱总线不阻塞于 ticket 删除 |
| **物理表立刻 rename 一切** | 迁移噪声 | 领域名 `bot_message_deliveries`；物理可先演进 `agent_dispatch_queue` 加 lease 列，后 rename |

### 0.3 Add（原方案不足）

1. **与现有 Outbox/Durable 对齐**：Account `AppendMessage` 必须复用现有 `send_message` 事务模板（mentions、message_seq、topic_seq、outbox），禁止第二套写路径。  
2. **ChatSendAck 语义**：仅 commit 后 ACK；`client_operation_id` = 现 `client_message_id` 语义。  
3. **人侧离线**：Account 客户端本地 Outbox 在 WS 恢复后重放 `AppendMessage`（Desktop/Mobile 已有 outbox 骨架）。  
4. **Bot 回复 ingress**：`AppendBotMessage` 必须带 `delivery_id`（可选 root）+ `operation_id`；Hub 校验 Host 是否持有该 bot 的 lease/session。  
5. **Stream 热投影**：CLI token/tool 仍可走 Stream；**最终人读气泡**必须是 Durable bot message。  
6. **Desktop dual-role**：Account UI 只走 `/ws/client` 发消息；本地 daemon 不得再 Composer 直 start；本机 Host 仅消费 mailbox。  
7. **过渡期 REST**：文档明确“同 domain commit，非双 SSOT”。  
8. **观测**：delivery_id / message_id / bot_id / session_id / host_id 全链路 tracing。

### 0.4 Verdict

**合理，作为终态架构采纳。**  
实现上按修正后的切片推进，避免一次改协议+删 REST+删 ticket+全表 rename 导致不可合并。

---

## Breaking Change Notice

latest-only，面向 monorepo 全端：

1. **协作消息主写**：Account 客户端 → `/ws/client` `AppendMessage`（`client_operation_id` 幂等）。REST `POST …/messages` 过渡期调用同一 commit，随后删除公开写协作入口。  
2. **协议分型**：Account 与 Host 帧分离（见 §2）；Host 不再塞进泛化 `ClientFrame` 业务语义。  
3. **公开 `agent_session.start/send_input` 不作聊天路径**；仅 mailbox consumer / 内部适配。  
4. **路由只认 `bot_id`**（全局 agent_id）；禁止 runtime 名 / profile id / session short 当身份。  
5. **防环**：从 `host_projection` 绝对禁投递 → **结构化 mention + hop/budget + self-mention 禁**（§5）。  
6. **Bot 回信**：Host `AppendBotMessage` 经 Hub 校验后成为普通 conversation 消息。

---

## Feasibility Assessment

Hub 已有：conversation SSOT、polymorphic mentions、`agent_dispatch_queue`、Host relay、per-conversation sessions、Durable/Outbox。  
缺口：Account WS 写消息、Bot mailbox lease 语义、principal 分型帧、公开路径去命令化。  

关键冲突点（采纳提案原文）：

- `social.rs` post-send 仍 `start/send_input` 味的 forward  
- `realtime.rs` `ClientFrame` 混 Account 订阅与 Host ingest/command  

**Fully feasible** with phased slices below.

---

## Design

### 1. Three planes

| Plane | Actors | Responsibility |
|-------|--------|----------------|
| **Collaboration** | Account ↔ Hub | Message, mention, reaction, read, timeline |
| **Bot mailbox** | Hub | Per-bot logical inbox, lease, retry, order, audit |
| **Execution** | Host ↔ Hub | Inject delivered message into local session; upload result / run state |

Collaboration semantics contain **only Message** (+ reaction/read/recall).  
HostCommand / CLI start is **execution private detail**.

```text
Mobile / Desktop Account
  └─ WSS /ws/client: AppendMessage(@BotA, @BotB)
        │
        ▼
Hub transaction
  chat_messages + structured_mentions + durable
  + bot_deliveries(M, BotA) + bot_deliveries(M, BotB)
        │ lease                    │ lease
        ▼                          ▼
Host A /ws/host              Host B /ws/host
  session(C, BotA)             session(C, BotB)
        │                          │
        └──── AppendBotMessage ────┘
                    │
                    ▼
            Hub commits bot-authored message
                    │
                    ▼
        all Account clients receive DurableEvent
```

Same runtime binary (two Codex) with **different `bot_id`** = two digital persons, two mailboxes, two sessions.

### 2. Protocol by principal

Target shapes（名称可微调，语义固定）：

```rust
// Account → Hub
pub enum AccountClientFrame {
    Subscribe { topics: Vec<String>, resume_after: Option<HashMap<String, i64>>, client_request_id: Option<String> },
    Unsubscribe { topics: Vec<String> },
    Ping { ts: i64 },
    AppendMessage {
        client_operation_id: String, // == client_message_id
        conversation_id: String,
        text: String,
        mentions: Vec<MentionTarget>, // optional structured; server still validates against participants
        reply_to_message_id: Option<String>,
        attachment_ids: Vec<String>,
    },
    MarkRead { conversation_id: String, read_up_to_message_seq: i64 },
}

// Hub → Account (extends today's ServerFrame durable/stream)
// ChatSendAck { client_operation_id, message_id, message_seq, conversation_id }
// DurableEvent / StreamEvent / SnapshotRequired / …

// Hub → Host
pub enum HubToHostFrame {
    BotInboxDelivery {
        delivery_id: String,
        conversation_id: String,
        message: ChatMessageSummary, // origin human/bot message
        bot_id: String,
        bot_launch: BotLaunchSnapshot, // revision snapshot: runtime/model/effort/prompt refs
        session: SessionBinding,       // existing session_id or create intent
        lease_expires_at_ms: i64,
    },
    CancelDelivery { delivery_id: String },
    // plus existing pull/force-close as needed
}

// Host → Hub
pub enum HostClientFrame {
    DeliveryAccepted { delivery_id: String },
    DeliveryRejected { delivery_id: String, code: String, detail: String },
    AppendBotMessage {
        delivery_id: String,           // causation
        operation_id: String,          // idempotent bot bubble key material
        conversation_id: String,
        bot_id: String,
        text: String,
        mentions: Vec<MentionTarget>,
        reply_to_message_id: Option<String>,
    },
    // ingest live batch / gap / command ack may remain as host-only variants
    HostIngestLiveBatch { batch: HostIngestLiveBatch },
    HostGapManifest { manifest: HostGapManifest },
    HostIngestPullResponse { response: HostIngestPullResponse },
    Ping { ts: i64 },
}
```

**ACK 规则**：`AppendMessage` / `AppendBotMessage` 仅在 Hub **commit 成功后** ACK。  
断线重发同一 `client_operation_id` / `operation_id` → 同一结果，无双泡。

**AppendBotMessage 与 CompletionWatch**：mailbox 成功路径在 `persist_agent_message_from_host` 成功后必须 `remove_by_dispatch_id` + `mark_projected`（与 TurnCompletionProjector 成功同级）。否则 ingest gap 后 `expire_completion_watches` 会误贴 `completion_timeout`。

**HTTPS 保留**：登录、ticket（Account）、媒体上传、历史 query、非实时控制。  
**HTTPS 不保留（终态）**：协作消息主写。

### 3. Bot mailbox model

Logical tables（物理命名可演进）：

```text
bots / agents              -- global identity + digital body (see global-bot-identity)
bot_revisions              -- optional immutable body snapshot (Phase B)
bot_deployments            -- bot × host capability (Phase B; interim: host_links + ensure)
conversation_bot_members   -- membership (today conversation_agent_members)
bot_sessions               -- active (conversation_id, bot_id) → one session (Phase B unique)
bot_message_deliveries     -- mailbox: UNIQUE(message_id, bot_id)
chat_messages
message_mentions
```

**`bot_message_deliveries` 状态机：**

```text
pending → leased → accepted → succeeded
                 ↘ failed_retryable → pending
                 ↘ failed_terminal
                 ↘ cancelled
```

- PK / unique：`(origin_message_id, bot_id)`  
- `delivery_id`、`lease_owner_host_id`、`lease_expires_at_ms`  
- per `(conversation_id, bot_id)` **顺序 drain**（同 bot 同会话 FIFO）  
- 网络至少一次；daemon 按 `delivery_id` 去重；气泡按 `operation_id` / `agent-result:…` 幂等  

**收件规则（严格 IM）：**

1. 结构化 `mention bot_id`（appearance order，multi-@ fan-out）  
2. `reply_to` agent 消息 → 该 bot_id  
3. **sole-agent 房规**（可选产品开关）：恰 1 human membership + 1 bot membership + 无 agentish 未匹配 token + 裸文本 → 该 bot  
4. 否则不投递  

**禁止**：无 membership 的 `@codex` silent join；用 runtime 名当 bot_id。

### 4. Multi-host scheduling

1. 若 `(conversation, bot)` 已有 active session → **粘性**投到该 session 的 Host。  
2. 若无 session → 从 bot 可用 deployment / linked hosts 选一（workspace、online、capacity）并在 session 创建时固化。  
3. 同一 `delivery_id` 仅一个 Host 持 lease。  
4. Host 下线：未绑定 session 的 pending 可改派；**已有 session 不静默迁移**（CLI/fs 上下文不可假定可搬）。  
5. 用户显式 “在 Host B 新开 session” → 归档旧 session，新建绑定 B。

### 5. Bot-to-Bot loop control（替换 host_projection 一刀切）

分离字段：

| Field | Meaning |
|-------|---------|
| `sender` | Account \| Bot \| System |
| `ingress` | account_ws \| host_ws \| system \| http_legacy（过渡） |
| `causation_message_id` / `delivery_id` | 因果 |
| `automation_hop` | 从根人类消息起的 bot 跳数 |
| `automation_budget` | 剩余允许自动投递次数 |

规则：

1. 无结构化 bot mention → 不投递 bot  
2. Bot 消息 **可以** mention 其他 bot  
3. **禁止 self-mention** 投递  
4. hop / budget 耗尽 → 不投递，可打 metrics / 可见系统提示  
5. `(message_id, bot_id)` 去重  
6. 移除 membership → cancel pending + CancelDelivery + 拒绝后续 AppendBotMessage  

过渡期：`message_source=host_projection` 仍可映射为 `ingress=host_ws`；**新路径不再用 source 做唯一防环门闩**。

### 6. Host auth simplification（**正式 Phase，非可选项**）

现状 Host 重连链（问题：重复验 token + ticket 误伤）：

```text
hit_ → POST /host/installations/self → POST /host/realtime/ws-ticket
    → GET /ws/host?ticket=… → JWT + Redis GETDEL + role + device revalidate
```

| Principal | Handshake（终态） |
|-----------|-------------------|
| Account `/ws/client` | **保留** 短期 ticket（浏览器 WebSocket 难设 Authorization） |
| Host `/ws/host` | **`Authorization: Bearer hit_*` 直连 Upgrade**；Hub 验 hash/revocation/role/installation；**无** Host ticket |

**必须保留的强身份：**

- Link：nonce + Ed25519 possession proof  
- 持久 opaque `hit_*`（库只存 hash，可旋转/吊销）  
- Upgrade 时校验 Host token、AgentHost role、installation  
- unlink/revoke 立即踢已有 `/ws/host`  
- 心跳 + 同 installation 最新连接胜出  

**必须删除 / 修正：**

- `POST /v1/host/installations/self` 作为**连接前鉴权预检**（健康检查可另留，但不挡 WS）  
- `POST /v1/host/realtime/ws-ticket` 与 Host 用 JWT ticket / single-use registry  
- 「WS 任意 401 就清掉 `hit_`」——仅 **token 明确 invalid/revoked** 才清；ticket 过期/重放/upgrade race **不得**要求重新 Link  

**安全说明：** 攻击者若已持有长期 `hit_`，本来就能换 ticket；ticket 不增加机器持有证明。Host 是 native daemon，应用 Authorization header，避免 token/ticket 进 URL/access log。

> 若不做本 Phase：邮箱/WS IM 做完后，**Host ticket 重复链与误清 hit_ 问题仍然存在**。

### 7. Client behavior

**Desktop**

- Composer → Account WS `AppendMessage` only  
- 删除/停用 `send-dispatch` 本地 start 主链  
- Timeline：Hub durable 为主；daemon 仅 tool/git enrich  
- @ picker：participant `bot_id` only  

**Mobile / Web**

- 同构 Outbox + WS 重放  
- 无“给 agent 发命令”协作 API  

**Daemon**

- 一 Host 一连接消费 `BotInboxDelivery`  
- `delivery_id` 去重 → inject session → `AppendBotMessage`  
- roster/session 键 = `bot_id`  

---

## Phased Implementation

### Phase 0 — Docs（本文 + 交叉引用）

- ADR 0021 / participant-delivery / architecture-messaging 指向本文  
- 通道表改为：Account IM 总线 + Bot mailbox + Host execution port  

### Phase 1 — Protocol types + mailbox store foundation **(start now)**

- `minos-protocol`：引入 Account/Host 帧变体（可并行旧 `ClientFrame` 映射，latest 客户端切新）  
- `AppendMessage` / `ChatSendAck` / `BotInboxDelivery` / `AppendBotMessage` 类型  
- store：delivery lease 字段与 API（演进 `agent_dispatch_queue` 或新表）  
- domain：`commit_conversation_message` 单入口草图（HTTP 暂调同一函数）  

### Phase 2 — Hub gateway commit + scheduler

- gateway 处理 Account `AppendMessage` → **同一** `send_message` 事务 + fanout + agent inbox + `ChatSendAck`/`Nack`  
- HTTP `POST …/messages` 过渡期调用同一路径（非双 SSOT）  
- worker：lease delivery → `BotInboxDelivery` 下发（替换直接 HTTP start/send_input 叙事）  
- 删除 auto-attach；sole-agent 规则收紧  

### Phase 2.5 — Host Bearer auth（正式，与 mailbox 同 program）

- Gateway `/ws/host`：接受 `Authorization: Bearer hit_*`（及过渡期 ticket）  
- Daemon `relay_client`：去掉 self 预检 + host ws-ticket；直连 Bearer  
- 删除/废弃 `POST /v1/host/realtime/ws-ticket`  
- 修正 401 处理：仅 token 吊销/未知才清本地 `hit_`  
- Account `/ws/client` ticket **不变**  

### Phase 3 — Daemon mailbox consumer

- relay：消费 `BotInboxDelivery`；`AppendBotMessage` 回写  
- 依赖 2.5 的稳定 Host 连接（推荐 2.5 先于或并行 3）  

### Phase 4 — Clients WS-native send

- Desktop/Mobile Outbox → WS AppendMessage  
- 删除 Composer 本地 agent start 主路径  
- REST 写协作消息下线  

### Phase 5 — Bot-to-Bot + revisions/deployments

- hop/budget 全开  
- bot_revisions / deployments / 活跃 session 唯一约束  

### Phase 6 — Verification matrix

见 §Acceptance。

---

## Acceptance

1. `@BotA @BotB` → 恰 2 条 delivery；重连不双执行。  
2. 同 runtime 不同 `bot_id` 在同 conversation 独立收/跑/回。  
3. BotA @ BotB 跨 Host 协作；无 mention 的 bot 输出不触发任何 bot。  
4. 移出 bot → pending cancel；Host 无法再 AppendBotMessage。  
5. Account 断线于 ACK 前，同 `client_operation_id` 重发不双泡。  
6. Host revoke 后立即不可消费 inbox / 写 bot 消息。  
7. Account 主 Online 仍看 `/ws/client`，不看 Host。  
8. 公开路径无 Composer→`start_agent` 协作。  

---

## Architectural Notes

- 不需要每 Bot 一条 WSS。  
- 需要每 Bot 一份逻辑邮箱；Host WS 是共享 transport。  
- Hub 不存 API key 明文；revision 可存 secret ref。  
- Bot 删除 → disabled/retain 历史。  
- 与 [global-bot-identity-design](global-bot-identity-design.md) 正交：身份/肉身 vs 邮箱/总线。  
- 与 [agent-participant-delivery](2026-08-09-agent-participant-delivery.md)：投递房规保留并收紧；实现从 dispatch 命令味改为 mailbox。  

---

## File Change Summary (program)

- `crates/minos-protocol/src/realtime.rs` — principal-split frames; AppendMessage / BotInboxDelivery  
- `crates/minos-protocol/src/messages.rs` — Bot sender / mention targets / operation ids  
- `crates/minos-backend/src/realtime/gateway.rs` — Account append + Host bot append  
- `crates/minos-backend/src/http/v1/social.rs` — demote command dispatch; shared commit  
- `crates/minos-backend/src/store/agent_dispatch_queue.rs` → deliveries + lease  
- `crates/minos-daemon/src/relay_client.rs` — mailbox consumer; host auth simplify  
- `apps/desktop/src/shared/lib/hub-realtime.ts` — WS send outbox  
- `apps/desktop/src/store/workspace/send-dispatch.ts` / `use-cases.ts` — remove local start main path  
- `apps/mobile/...` — WS replay outbox  
- `docs/architecture-messaging.md` — bus model  
- `docs/superpowers/specs/bot-mailbox-ws-im-bus-design.md` — this file  

---

## Implementation status

| Phase | Status |
|-------|--------|
| 0 Docs | **done** — this file + ADR 0021 / participant-delivery / architecture-messaging cross-refs |
| 1 Protocol + delivery foundation | **done** — wire types; lease columns; `set_lease`/`clear_lease`/`set_session_id`; lease-aware reclaim |
| 2 Hub AppendMessage commit | **done (core)** — `/ws/client` `AppendMessage` → same `send_message` + fanout + inbox; HostCommand offline adapter remains |
| 2 worker BotInboxDelivery | **done (core) + P0 fixed** — claim → lease → push `BotInboxDelivery`; **does not** `mark_succeeded` on push (stays inflight until result); canonical `session_id` in `SessionBinding` |
| 2.5 Host Bearer `hit_*` | **done** — `/ws/host` Bearer only; ticket endpoint removed; daemon Bearer dial; selective hit_ clear |
| 3 Daemon mailbox consumer | **done (core)** — inject before DeliveryAccepted; honor Hub session_id; pin delivery_id/bot_id; completion emits AppendBotMessage as primary multi-end final-text when mailbox present |
| 4 Clients WS-native send | **done (core)** — Desktop + Mobile **WS-only** collab write (no REST fallback); ChatSendAck/Nack; Desktop offline local fan-out **deleted**; HostCommand only as private runtime-port adapter |
| 5 Bot-to-bot hop/budget | **done (core)** — automation_hop + MAX_AUTOMATION_HOP=3; `bot_revisions` + `bot_deployments` tables; schedule-time revision + deployment upsert on mailbox push |
| 6 Verification matrix | core compile/tests green |

### Review P0 fixes applied (post first review)

1. **No false success on push** — mailbox row stays inflight after enqueue.  
2. **Single session_id** — Hub mints/carries `SessionBinding.session_id`; daemon does not invent a second `mailbox-*` formula.  
3. **Accept after inject** — daemon only `DeliveryAccepted` after local inject succeeds; inject failure clears InFlight so Hub can requeue.  
4. **Lease reclaim** — SQLite/Postgres reclaim uses `lease_expires_at_ms` when set.  
5. **Desktop ChatSendAck** — outbox uses `sendAppendMessage` waiter; nack does not double-send via REST.  
6. **Status gates** — DeliveryRejected/AppendBotMessage require inflight lease owner and non-expired lease.  
7. **Workspace** — `BotLaunchSnapshot.workspace_path` from agent body; daemon start uses it.  

### Known residual (not product-mainline blockers)

1. **Runtime-port adapter** (`runtime_port_inject` / HostCommand start|send_input) only when no live Host WS for mailbox; private, not collaboration path.  
2. ~~Host ticket~~ **deleted**; Bearer `hit_*` only.  
3. Mailbox path emits `AppendBotMessage` when delivery context pinned; projector remains for offline runtime-port path.  
4. ~~Client REST collab write fallback~~ **deleted** (Desktop + Mobile WS-only). Server HTTP `POST …/messages` remains for tests/tools.  
5. ~~Desktop offline Composer fan-out~~ **deleted**.  
6. Mobile local profile cache: Hub-first create/update; offline draft has empty `agentId` until import.  
7. Bot-to-bot hop/budget + `bot_revisions`/`bot_deployments` tables landed (core).  
8. Daemon roster re-keyed to `bot_id`; Desktop exposes `participatingBots` (botId/name/runtime).  
7. Wire `MessageSender` shipped; agent rows use `sender_account_id = NULL`.
