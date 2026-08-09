# Unify Client IM Sync Engine (Desktop + Mobile)

| Field | Value |
|-------|--------|
| Status | **Normative target**（2026-08-03） |
| Date | 2026-08-03 |
| Scope | Desktop + Mobile + minos-mobile FFI 的 **客户端 IM 同步引擎终态**；旧补丁路径清理；与后端投递契约对齐 |
| Program | [2026-08-03-im-reliability-program](2026-08-03-im-reliability-program/README.md) · [TASKS](2026-08-03-im-reliability-program/TASKS.md) |
| Pair | [Backend Delivery & Orchestration](2026-08-03-backend-im-delivery-orchestration.md) |
| Supersedes (partial) | [2026-08-02-hub-collaboration-message-ssot.md](2026-08-02-hub-collaboration-message-ssot.md) Phase 4–5 客户端半成品描述；reaction Phase 5.2 ⚠️ |
| Related | [architecture-messaging.md](../../architecture-messaging.md)；Hub SSOT 方案 2026-08-02 |
| Non-goals | E2EE；百万群写扩散；改 Grok/Codex 协议；为旧客户端保留兼容层（latest-only）；短期补丁路线 |

> **一句话目标**：两端客户端对 Hub 协作消息具备 **同一套** 可靠性原语——幂等写、持久 Outbox、水位 Sync、增量 Inbox、seq 主键排序、定向 gap 恢复；删除一切时间轮询、软去重、全量 invalidate、伪 seq 默认值与文档谎言。

**规划约束**：遵守 [AGENTS.md Final-Architecture Planning Rule](../../../Agents.md) — 只设计终态代码结构；Phase 是终态切片，不是临时行为。

---

## Breaking Change Notice

本改造按 **Development-State Compatibility Policy（latest-only）** 与 **Final-Architecture Planning Rule** 推进：

1. Mobile `sendChatMessage` FRB / Rust API **要求** 发送路径携带 `client_message_id`（实现时直接贯通；禁止长期 `None` 硬编码残留）。
2. Desktop `TimelineMessage.messageSeq` 语义：缺失为 `undefined`/`null`；**禁止** `?? 0` 伪序。
3. Desktop Linked 时间线 **禁止** 以 daemon `agent-result` 与 Hub agent 气泡双权威并存；本地 sibling 仅在 Outbox 未 ack 时乐观展示。
4. `focusedConversationId` 与 `hasTimelineWindow` 语义拆分；任何 `loadTimeline({ quiet: true })` **不得** 改焦点。
5. Spec Phase 5.2「reaction local-only」废止；Hub reaction 为唯一多端路径。
6. Agent 气泡 id 与后端终态一致：`agent-result:{conversationId}:{sessionId}:{originMessageId}`（见 Backend B4；**禁止** 客户端 body 软去重兼容旧后缀）。

下游（仅本 monorepo）：同步改 Desktop / Mobile / FRB 生成物 / 测试 / 文档；无外部 semver 消费者。

---

## Feasibility Assessment

| 后端能力（存储） | 位置 | 客户端缺口 |
|----------|------|------------|
| `client_message_id` 幂等 PK | `store/social/conversation_messages.rs` | Mobile FFI 硬编码 `None` |
| Transactional Outbox + fanout | `store/social/delivery.rs` | Desktop outbox inflight 黑洞；Mobile 无 outbox |
| `before_seq` / `after_seq` keyset | messages/query | after_seq 零调用；Mobile 无 loadOlder |
| `last_read_seq` + unread 派生 | conversations/read | Mobile 本地不 mirror |
| SnapshotRequired | gateway + mobile session | Flutter **未消费**；Desktop 清空粗重建 |
| Reaction toggle + durable | `message_reactions.rs` | Mobile 零渲染；Desktop 无 outbox |

**注意**：后端 **投递/编排层** 并非终态就绪（Push UserOnline 死代码、CompletionWatch 单 slot、同步 dispatch 等）——见 [Backend spec](2026-08-03-backend-im-delivery-orchestration.md)。客户端与后端 **同 program 推进**；agent id 以 Backend B4 冻结为准。

客户端改造边界清晰：Sync Engine 模块化（Write / Timeline / Inbox / Connection）。多 Agent 可按 Track C 并行。

**Fully feasible** under final-architecture-only planning.

---

## Current Surface Inventory

### Mobile write / cache

- `crates/minos-mobile/src/client.rs` — `send_chat_message`（`client_message_id: None`）
- `crates/minos-ffi-frb/src/api/minos.rs` + generated Dart FRB — send API 无幂等字段
- `apps/mobile/lib/data/repositories/social_repository.dart` — send / list / markRead 透传
- `apps/mobile/lib/infrastructure/social_cache_store.dart` — SQLite conversations/messages；无 outbox 表；touch preview 不 bump unread
- `apps/mobile/lib/application/social_providers.dart` — send 同步阻塞；每事件 `invalidateSelf`；无 loadOlder；无 SnapshotRequired
- `apps/mobile/lib/application/conversations_sort.dart` — UUID tie-break
- `apps/mobile/lib/domain/social_message.dart` — delivery_state 三态；无 clientMessageId 字段语义

### Mobile realtime

- `crates/minos-mobile/src/realtime/session.rs` — social fanout；SnapshotRequired → UiEvent raw；`parse_chat_message` 空壳 fallback
- `apps/mobile/lib/application/minos_providers.dart` — 仅处理 `presence` raw，忽略 `snapshot_required`

### Desktop write / sync

- `apps/desktop/src/shared/lib/im-outbox.ts` — Tauri SQLite outbox（intent lanes；见 full-review C2）
- `apps/desktop/src/shared/lib/im-cloud-sync.ts` — user flush；agent uplink fire-and-forget
- `apps/desktop/src/shared/lib/hub-realtime.ts` — WS 状态机；无 visibility
- `apps/desktop/src/shared/lib/im-hub-bridge.ts` — 入站 merge；SnapshotRequired 清空；unread patch
- `apps/desktop/src/shared/lib/hub-digest-cache.ts` — inbox digest 缓存
- `apps/desktop/src/shared/lib/im-cloud-inbound.ts` — afterSeq 参数闲置
- `apps/desktop/src/shared/lib/hub-timeline.ts` — dual agent-result soft dedupe
- `apps/desktop/src/shared/lib/timeline-order.ts` — seq / createdAt 混排
- `apps/desktop/src/shared/lib/minos-cloud.ts` — `messageSeq ?? 0`
- `apps/desktop/src/store/workspace/timeline.ts` — loadTimeline 偷写 focused；quiet poll 入口
- `apps/desktop/src/store/workspace/live-ingress.ts` — 0/400/1200ms burst
- `apps/desktop/src/features/chat/Timeline.tsx` — 2s interval poll
- `apps/desktop/src/features/chat/reaction-store.ts` — Hub toggle 无 outbox
- `apps/desktop/src/store/workspace/use-cases.ts` — approval 直连 daemon

### Backend（只读契约 / 小幅加固）

- `crates/minos-backend/src/store/social/conversation_messages.rs` — 幂等 insert
- `crates/minos-backend/src/turn_completion.rs` — agent-result id 生成
- `crates/minos-backend/src/http/v1/social.rs` / `conversations.rs` — send / reaction / query

### Docs（漂移）

- `docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md` Phase 5.2 / 4.4
- `docs/architecture-messaging.md` §3.4.5 / §7.4
- `docs/architecture-desktop.md` / `architecture-mobile.md`

---

## Design

### 1. Target architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Presentation                                                             │
│  Desktop Timeline / Rail · Mobile Messages / Chat · (Web later)           │
│  只消费 ProjectionStore；禁止直接「事件 → invalidate 全家桶」               │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │ subscribe (selectors)
┌───────────────────────────────▼──────────────────────────────────────────┐
│  ClientSyncEngine（逻辑同构；Desktop TS · Mobile Dart+Rust）                 │
│                                                                            │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │ Outbox      │  │ TimelineSync │  │ InboxSync    │  │ IntentOutbox    │  │
│  │ user/agent/ │  │ window+meta  │  │ digest patch │  │ reaction/       │  │
│  │ reaction*/  │  │ gap fill     │  │ unread mirror│  │ approval*       │  │
│  │ 幂等键      │  │ Snapshot     │  │              │  │                 │  │
│  └──────┬──────┘  └──────┬───────┘  └──────┬──────┘  └────────┬────────┘  │
│         │                │                 │                   │           │
│  ┌──────▼────────────────▼─────────────────▼───────────────────▼────────┐  │
│  │ Connection: ticket · subscribe · resume_after · visibility lifecycle │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────┬────────────────────────────────────────────┘
                                │ HTTPS + WSS
┌───────────────────────────────▼────────────────────────────────────────────┐
│  Hub (SSOT for collaboration bubbles, unread, reaction, recall)             │
│  Host daemon (SSOT for agent raw/tool/git; local workbench cards only)      │
└────────────────────────────────────────────────────────────────────────────┘
```

\* IntentOutbox 与 Message Outbox 可同表不同 `kind`，语义统一：持久化 + 退避 + 服务端幂等。

### 2. Key design decisions

1. **单一客户端契约，两端同语义，实现可不同语言**  
   - 拒绝：Desktop 一套、Mobile 另一套「差不多」的逻辑。  
   - 采用：共享不变量文档 + 同构模块边界（Outbox / TimelineSync / InboxSync / Connection）。  
   - 不共享：运行时语言、UI 框架、Desktop 的 daemon 执行面。

2. **所有用户意图写路径必须带幂等键 + 可持久 Outbox**  
   - 覆盖：user message、agent host_projection 上行、reaction toggle、（可选二期）approval resolve。  
   - 拒绝：fire-and-forget POST；同步 await REST 当唯一路径。  
   - Mobile：`client_message_id` = 本地 pending 的稳定 UUID（即 wire id / 未来 server message_id）。  
   - Desktop 用户消息：保持 `clientMessageId` = 本地 message id。  
   - Agent 上行：`client_message_id` **必须等于** Hub `TurnCompletionProjector` 与 Host 约定的 **同一** `agent-result:{conv}:{session}:{trigger_seq}` 形态（见决策 4）。

3. **Outbox 状态机完整（修 Desktop inflight 空洞）**  
   ```
   pending ──flush──► inflight ──2xx──► acked
                 │              └──err──► pending (backoff) 或 failed_terminal
                 │
   startup / stale_inflight_ttl ──► pending
   ```
   - `listDue` 包含：`pending && nextAttemptAt<=now` **或** `inflight && updatedAtMs < now - STALE_MS`。  
   - 拒绝：仅 `status===pending` 的 listDue。  
   - Desktop：localStorage 可先保留，但补 stale 回收 + 单写锁（BroadcastChannel 或 leader election）；中期迁 daemon SQLite 表可选，不阻塞终态语义。  
   - Mobile：SQLite 表 `im_outbox`（或 messages 行上的 outbox 列：attempts/next_attempt/status）。

4. **Agent 最终气泡：单一 id 空间 + 单一多端写者**  
   - **Mobile client_live 派发**：Hub `TurnCompletionProjector`（后端 B3/B4 终态：异步 dispatch + 多 watch）。  
   - **Desktop-native 执行**：daemon `conversation_completion` 产出最终文本后，**Host 以 projector 同构 id** 经 Outbox `host_projection` 上行；禁止 UI 扫时间线 best-effort 投影当主路径。  
   - **id 规则（强制，与 Backend B4 一致）**：`agent-result:{conversationId}:{sessionId}:{originMessageId}`  
     - `originMessageId` = 触发该 turn 的 user Hub `message_id`（client_message_id）。  
     - 禁止用 `raw last_seq` / 本地 message_key 当跨端主键；**改 backend+daemon 契约**，禁止客户端 soft-dedupe。  
   - **删除**：`hub-timeline` body+120s 软去重；`*:sessionId` 模糊键；Timeline 2s poll 与 live-ingress 0/400/1200。  
   - 本地 tool/git/system 卡仍 merge，永不冒充 Hub 气泡 SSOT。

5. **时间线排序：仅 `message_seq` 跨源主键**  
   - 有 `message_seq` 的行：严格按 seq ASC。  
   - 无 seq 的行：仅允许 **本端乐观行**（sending/failed 且无 server id）；彼此用 `clientSeq` / 本地创建 排序，并 **钉在窗口尾部**（或尾部附近），不与 durable 行交错比较 `createdAtMs`。  
   - **禁止**：`messageSeq ?? 0`；`COALESCE(server_order_key, created_at_ms)` 跨量纲比较。  
   - Hub map：`message_seq` 缺失 → 丢弃行或标 error，不默认 0。

6. **Inbox：增量 patch 为主，全量校准为旁路**  
   - 入站 durable：`patchDigest(conversationId, {preview, lastAt, unreadDelta})`。  
   - 规则：非 focused 且非 own → `unread += 1`；mention → `unreadMention += 1`；focused → 0 并 debounce markRead。  
   - 全量 `GET conversations`：仅 pull-to-refresh、定时校准（如 5–15min）、account SnapshotRequired、登录冷启动。  
   - **禁止**：每条 social event `invalidateSelf` + DELETE 全表。

7. **TimelineSync：窗口 + 水位 + 定向 gap**  
   - 每会话维护：`minLoadedSeq`, `maxLoadedSeq`, `hasOlder`, `loadingOlder`, `dirty`。  
   - 冷开：`limit=PAGE` 最新页。  
   - 上翻：`before_seq = minLoadedSeq`。  
   - SnapshotRequired(conversation)：  
     - **不**无脑清空 UI（可保留窗口做骨架）；  
     - 用 `after_seq=minLoaded-1`… 或 `after_seq=maxLoaded` forward fill + 必要时最新页校准；  
     - cursor 已在连接层清零时，catch-up 后对窗口做 range reconcile。  
   - 入站 append：若 `message_seq == max+1` 直接 append；若 `> max+1` 标 gap → forward fill；若 `< min` 忽略或标 hasOlder。

8. **focused ≠ hasWindow**  
   - `focusedConversationId`：用户当前查看的会话 → 控制 unread 清零与 markRead。  
   - `openTimelineIds` / `hasWindow`：内存中有时间线窗口 → 允许 WS upsert。  
   - `loadTimeline` / quiet refresh **只** 操作 window，不写 focused。

9. **Connection lifecycle**  
   - Desktop：visibility / 系统休眠 → 主动 close WS + `intentReconnect`；回前台 attempt=0 立即连。  
   - Mobile：已有 reconnect/backoff；补 SnapshotRequired Flutter 消费；connection → Connected 时 drain Outbox。  
   - parse 失败：drop + log，**禁止** 空壳 ChatMessageSummary 入 store。

10. **Reaction / Approval 进入 Intent 可靠性面**  
    - Reaction：Hub 为 SSOT；Desktop outbox kind=`reaction_toggle`（messageId+emoji+clientOpId）；Mobile 渲染 `ChatMessageSummary.reactions` + 同语义。  
    - Approval：至少 Desktop IntentOutbox（client_request_id）；服务端幂等若缺则补；Mobile 同步 REST 可二期对齐。

11. **清理优先于堆功能**  
    - 终态落地后删除：时间驱动 completion 轮询、body 软去重、全量 invalidate 主路径、伪 seq、过期注释（`before_ts_ms`）、spec 假状态。

### 3. Concrete interfaces

#### 3.1 共享不变量（逻辑，非跨语言类型）

```text
ClientMessageId = UUID | "agent-result:{conv}:{session}:{triggerSeq}"
OutboxKind      = user_message | agent_result | reaction_toggle | approval_resolve
OutboxStatus    = pending | inflight | acked | failed_terminal

TimelineMeta {
  minLoadedSeq: Option<u64>
  maxLoadedSeq: Option<u64>
  hasOlder: bool
  loadingOlder: bool
}

InboxDigest {
  conversationId
  title, preview, lastMessageAtMs
  unreadCount, unreadMentionCount
}
```

#### 3.2 Mobile Outbox（SQLite）

```sql
-- social_cache.db v4+
CREATE TABLE im_outbox (
  client_op_id TEXT PRIMARY KEY,      -- = client_message_id or reaction op id
  kind TEXT NOT NULL,                 -- user_message | reaction_toggle | ...
  conversation_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  status TEXT NOT NULL,               -- pending|inflight|acked|failed_terminal
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at_ms INTEGER NOT NULL,
  last_error TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_im_outbox_due
  ON im_outbox(status, next_attempt_at_ms);

-- messages: client_message_id 与 wire 对齐（pending 行 server_message_id 可空，
-- 发送成功后 server_message_id = client_message_id）
ALTER TABLE cached_social_messages
  ADD COLUMN client_message_id TEXT;
```

#### 3.3 Desktop Outbox 状态修复（语义）

```typescript
// im-outbox.ts — 目标 API
export function listDuePending(now = Date.now()): ImOutboxEntry[];
// 包含 pending due + stale inflight (updatedAtMs < now - STALE_INFLIGHT_MS)

export function reclaimStaleInflight(now = Date.now()): number;

export type OutboxKind =
  | "user_message"
  | "agent_result"
  | "reaction_toggle";

// enqueue 扩展 kind；agent_result 与 reaction 共用 worker
```

#### 3.4 TimelineSync API（两端同语义）

```typescript
// 逻辑接口
interface TimelineSync {
  open(conversationId: string): Promise<void>;       // cold page
  loadOlder(conversationId: string): Promise<void>;  // before_seq
  applyDurable(msg: HubMessage): void;               // gap-aware
  onSnapshotRequired(topic: string): Promise<void>;// range reconcile
  markFocused(conversationId: string | null): void;
}
```

```dart
// Mobile SocialConversation 目标状态
class SocialConversationState {
  final String? myAccountId;
  final List<SocialChatMessage> messages;
  final int? minLoadedSeq;
  final int? maxLoadedSeq;
  final bool hasOlder;
  final bool loadingOlder;
  final bool isLoading;
  final Object? error;
}
```

#### 3.5 InboxSync API

```typescript
// 禁止每事件 full fetch
interface InboxSync {
  hydrateFromServer(): Promise<void>;           // rare
  patchFromMessage(msg, { focused, myAccountId }): void;
  applyMarkRead(conversationId, lastReadSeq): void;
  onAccountSnapshot(): Promise<void>;
}
```

#### 3.6 发送路径（Mobile 终态）

```dart
// insertPending → enqueue outbox(client_message_id=localId or dedicated UUID)
// → optimistic UI → worker POST with client_message_id
// → markSent on ack (server message_id == client_message_id)
// retryMessage 只把 outbox 行重置为 pending，不新建 key
```

#### 3.7 Agent-result id（跨端强制）

```text
agent-result:{conversation_id}:{session_id}:{origin_message_id}

origin_message_id = 触发该 turn 的 user Hub message_id（= client_message_id 幂等键）
```

Daemon `conversation_completion` 与 `TurnCompletionProjector` **生成同一字符串**；Desktop 本地行 id = Hub id；merge 按 id 相等即可，删除 session soft-dedupe。与 [Backend B4](2026-08-03-backend-im-delivery-orchestration.md) 同步落地。

### 4. Data flows (end state)

#### 4.1 Mobile 发消息

```
UI send
  → cache insert pending (client_message_id=UUID, delivery=sending)
  → outbox enqueue (same id)
  → inbox patch preview (unread 不变)
  → OutboxWorker (on Connected / timer)
       POST /messages { client_message_id, text, ... }
       → acked: markSent, delivery=sent
       → err: backoff / failed_terminal + UI failed
```

#### 4.2 入站消息（两端）

```
Durable ConversationMessage*
  → TimelineSync.apply (if window open / focused)
  → InboxSync.patch (unread delta unless focused/own)
  → NOT full conversations REST
```

#### 4.3 SnapshotRequired

```
Connection layer: clear topic cursor
  → emit SnapshotRequired(topic) to SyncEngine
  → conversation: range fill + meta repair（保留滚动尽量）
  → account: InboxSync.hydrateFromServer
```

#### 4.4 Desktop-native agent 完成

```
daemon conversation_completion (id = agent-result:…:triggerSeq)
  → local workbench shows bubble immediately
  → Outbox kind=agent_result host_projection
  → Hub insert idempotent
  → Mobile sees same message_id via durable
  → no 0/400/1200 poll; no body soft-dedupe
```

### 5. Delete list（旧代码 / 反模式）

| 删除项 | 位置 | 替代 |
|--------|------|------|
| `client_message_id: None` 硬编码 | `minos-mobile/client.rs` | 参数透传 |
| 每事件 `ref.invalidateSelf()` 主路径 | `social_providers ConversationsController` | InboxSync.patch |
| `DELETE FROM cached_social_conversations` 主路径 | `saveConversations` 热路径 | upsert by id；全量仅 hydrate |
| `markConversationRead` 每条 inbound | `_applyRemoteMessage` | debounce / 焦点进入一次 |
| `COALESCE(server_order_key, created_at_ms)` | `social_cache_store` | seq 主键排序 |
| `messageSeq ?? 0` | `minos-cloud.ts` | null 或丢弃 |
| body+120s hub soft dedupe | `hub-timeline.ts` | 统一 agent-result id |
| live-ingress 0/400/1200 burst | `live-ingress.ts` | completion/outbox 事件 |
| Timeline 2s trail poll（completion 用途） | `Timeline.tsx` | 仅保留真·错误恢复可选 |
| `loadTimeline` 写 `focusedConversationId` | `timeline.ts` | 拆 focused setter |
| `syncAgentResultToCloud` 无队列 warn | `im-cloud-sync.ts` | Outbox agent_result |
| `parse_chat_message` 空壳 | `session.rs` | drop+log |
| listDue 仅 pending | `im-outbox.ts` | stale inflight reclaim |
| Spec「reaction local-only」 | SSOT 2026-08-02 §5.2 | 已实现云端 |
| 注释 `before_ts_ms` | `timeline.ts` header | before_seq |

---

## Phased Implementation

各 Phase 可独立合并；**验收以不变量为准**，不以「改了多少文件」为准。多 Agent 建议按 Phase 内模块并行。

### Phase 0: Contract lock + 文档归零

**目标**：冻结终态不变量，消除文档与代码认知冲突。归属 program **C0/B0**。

**File: `docs/superpowers/specs/2026-08-03-im-reliability-program/`**
- README + TASKS 为总执行面。

**File: `docs/superpowers/specs/2026-08-03-client-im-sync-engine.md`**
- 本文作为客户端 Sync 终态 SSOT。

**File: `docs/superpowers/specs/2026-08-03-backend-im-delivery-orchestration.md`**
- 后端投递/编排终态；agent id 与本文 3.7 对齐。

**File: `docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md`**
- Phase 4.4：`after_seq` 标为客户端必接；删除 `before_ts_ms` 表述。
- Phase 5.2：改为 ✅ Hub reaction API 已实现；客户端 Outbox/Mobile UI 归本 program。

**File: `docs/architecture-messaging.md`**
- §3.4.5：与代码一致（云端已实现）。
- 新增 Client Sync + Backend delivery 终态指针。

**File: `docs/architecture-desktop.md` / `architecture-mobile.md`**
- 时间线 / inbox / outbox 描述改为终态。

**Verification**
- 文档交叉链接无矛盾；无「短期补丁」「local-only reaction」「caller 应检查 presence」。

---

### Phase 1: Write-path reliability（幂等 + Outbox 完备）

**目标**：任何「用户已表达的发送意图」在杀进程 / 断网后可恢复且不重复。

#### 1.A Mobile client_message_id 全链路

**File: `crates/minos-protocol`**（若请求体已有字段则 unchanged）
- 确认 `SendChatMessageRequest.client_message_id` 已存在。

**File: `crates/minos-mobile/src/client.rs`**
- `send_chat_message(..., client_message_id: Option<String>, ...)` 透传，删除 `None` 硬编码。
- 同步 `message_source` / `client_sent_at_ms` 若 Mobile 需要 client_live 默认。

**File: `crates/minos-mobile/src/http.rs`**
- HTTP body 序列化包含上述字段。

**File: `crates/minos-ffi-frb` + regenerate FRB**
- Dart API 暴露 `clientMessageId`。

**File: `apps/mobile/lib/infrastructure/social_cache_store.dart`**
- pending 行写入 `client_message_id`（可用 UUID；推荐与将要作为 `message_id` 的值同一）。
- schema v4 migration。
- `markMessageSent`：以 `client_message_id` / `server_message_id` 合并，删除错误 delete-echo 竞态。

**File: `apps/mobile/lib/application/social_providers.dart`**
- `sendMessage` / `retryMessage` 传同一 `clientMessageId`。
- 发送改为 enqueue + 立即返回 UI；网络错误不丢 key。

#### 1.B Mobile Outbox worker

**File: `apps/mobile/lib/infrastructure/im_outbox_store.dart`**（新）
- CRUD + listDue + reclaim stale inflight。

**File: `apps/mobile/lib/application/im_outbox_worker.dart`**（新）
- Riverpod keepAlive：监听 `connectionState == Connected` + 定时 tick。
- 指数退避；MAX_ATTEMPTS；terminal → delivery failed。

**File: `apps/mobile/lib/infrastructure/social_cache_store.dart`**
- App 启动：`UPDATE messages SET delivery_state='failed' WHERE delivery_state='sending'`（或交给 outbox reclaim 统一）。

#### 1.C Desktop Outbox 修洞 + agent_result 入队

**File: `apps/desktop/src/shared/lib/im-outbox.ts`**
- `STALE_INFLIGHT_MS`（如 45s）。
- `reclaimStaleInflight`；`listDuePending` 包含 stale inflight。
- 扩展 `kind: user_message | agent_result | reaction_toggle`。
- 启动 `startImOutboxWorker` 先 reclaim。

**File: `apps/desktop/src/shared/lib/im-cloud-sync.ts`**
- `syncAgentResultToCloud` 改为 enqueue + flush；失败进 backoff，不单 warn。
- 删除「仅 process 内存 projected set 当终态」的路径依赖（acked 以 outbox 为准）。

**File: `apps/desktop/src/shared/lib/im-outbox.test.ts`**
- 覆盖：inflight 杀进程模拟 → reclaim → 再 due；agent_result kind。

**Verification Phase 1**
- 单测：Mobile outbox + Desktop inflight reclaim。
- 手工：断网发送 → 恢复 → 单条；杀进程 mid-flight → 启动后单条（两端）。

---

### Phase 2: Agent bubble id 统一 + 删除补丁合并层

**目标**：多端同一 turn 同一 `message_id`；删除 soft-dedupe 与 completion 轮询。

#### 2.A 契约：origin_message_id 贯穿（与 Backend B4 同一 PR 链）

**File: `crates/minos-backend/src/turn_completion.rs`**
- 固化 `agent_result_client_message_id(conv, session, origin_message_id)`。

**File: daemon conversation_completion 路径**
- 完成写本地气泡时使用 **同一 id 公式**；上下文必须持有 origin user `message_id`。

**File: Desktop `projectMissingLocalAgentResultsToHub` / completion uplink**
- 仅 uplink 已是规范 id 的行；非法 id 不入 outbox。

#### 2.B 删除脏 merge / poll

**File: `apps/desktop/src/shared/lib/hub-timeline.ts`**
- 删除 body+120s soft dedupe；删除 `*:sessionId` 模糊集（保留严格 id 相等与「未 ack 乐观本地」）。
- merge 规则简化为：Hub id 胜；本地仅保留 non-chat cards + 未在 Hub 且 outbox 未 ack 的乐观行。

**File: `apps/desktop/src/store/workspace/live-ingress.ts`**
- 删除 0/400/1200 `loadTimeline` burst。
- turn end：依赖 local completion 事件更新 workbench + outbox drain；可选 **一次** quiet refresh 若 local card 依赖 list。

**File: `apps/desktop/src/features/chat/Timeline.tsx`**
- 删除 trailRefresh 2s 轮询（或收窄为 phase===error 且 !livePush）。

**File: `apps/desktop/src/shared/lib/timeline-order.ts` + tests**
- 实现决策 5 排序；更新回归测。

**File: `apps/desktop/src/shared/lib/minos-cloud.ts`**
- `messageSeq: raw.message_seq` 必填校验；缺则 skip message + warn。

**Verification Phase 2**
- hub-timeline tests：同 session 不同 suffix **不再**靠 body 去重——上游保证同 id。
- 无 2s poll 时 Desktop-native turn 仍在 Mobile 可见（outbox agent_result）。

---

### Phase 3: TimelineSync + SnapshotRequired + 分页

**目标**：窗口水位完备；Snapshot 定向恢复；Mobile 上翻。

#### 3.A Desktop

**File: `apps/desktop/src/shared/lib/message-history.ts`**
- meta 增加 `maxLoadedSeq`（或从 messages 派生 helper）。

**File: `apps/desktop/src/shared/lib/im-hub-bridge.ts`**
- `onSnapshotRequired`：range reconcile 优先于 clear-all。
- 调用 `afterSeq` / `beforeSeq` 填 gap。

**File: `apps/desktop/src/shared/lib/im-cloud-inbound.ts`**
- 暴露并使用 forward gap helper。

**File: `apps/desktop/src/store/workspace/timeline.ts`**
- **移除** quiet/full load 对 `focusedConversationId` 的赋值。
- focus 仅由 UI 导航 / `openConversation` use-case 设置。

**File: `apps/desktop/src/store/workspace/types.ts` + store**
- 区分 `focusedConversationId` 与 timeline window 存在性（已有 messagesByConversation key 即可，修正语义即可）。

#### 3.B Mobile

**File: `crates/minos-mobile/src/realtime/session.rs`**
- `parse_chat_message`：失败 return Option / 不发送空壳。
- SnapshotRequired 可同时发 social-control 事件（或保持 UiEvent raw，但 Flutter 必须处理）。

**File: `apps/mobile/lib/application/minos_providers.dart` 或新 `im_sync_provider.dart`**
- 监听 `snapshot_required`：按 topic 触发 TimelineSync / InboxSync。

**File: `apps/mobile/lib/application/social_providers.dart`**
- `SocialConversationState` 加 min/max/hasOlder/loadingOlder。
- `loadOlder()` 调 `beforeSeq`。
- 入站 apply 增量插入，禁止每次全表 `loadMessages` 若可 merge 单行（至少避免重复全表 sort 放大；可 retained in-memory list + 单行 upsert）。

**File: `apps/mobile/lib/ui/...` chat scroll**
- 顶部阈值触发 loadOlder。

**File: `apps/mobile/lib/infrastructure/social_cache_store.dart`**
- 修正 ORDER BY 与索引：`(conversation_id, server_order_key, client_seq)`。
- 排序逻辑对齐决策 5。

**Verification Phase 3**
- 长会话上翻 >100 条。
- 模拟 SnapshotRequired：不丢已加载上下文（或可恢复），cursor 重建后无重复垃圾行。

---

### Phase 4: InboxSync 增量 + unread 镜像

**目标**：角标实时正确；去掉 O(会话) 热路径。

**File: `apps/mobile/lib/application/social_providers.dart` — ConversationsController**
- 删除 `listen → invalidateSelf` 主路径。
- 改为 `InboxSync`：事件 → patch 单 conversation（内存 state + SQLite upsert 单行）。
- `refresh()` / 定时 / snapshot 才 `_fetchRemoteConversations`。

**File: `apps/mobile/lib/infrastructure/social_cache_store.dart`**
- `upsertConversation` 单行。
- `bumpUnread(conversationId, {mention})`。
- `saveConversations` 仅 hydrate：可用 replace 策略，但 **不在热路径调用**。
- `touchConversationPreview` 扩展 unread 参数或拆 bump。

**File: `apps/desktop/src/shared/lib/im-hub-bridge.ts` + `hub-digest-cache.ts`**
- 保持 patch 模型；校准：定期或 account snapshot 时 `hydrate` 覆盖本地 drift。
- 确保 focused 语义修正后 unread 不再被 quiet load 误清。

**File: `apps/mobile/lib/application/conversations_sort.dart`**
- tie-break 改为 `updatedAt`/`createdAt` 若字段可得，否则 lastMessageAt + title。

**Verification Phase 4**
- 后台会话收消息：角标 +1；打开清零；自己多端 echo 不 +1。
- 活跃群聊 CPU/网络：无每消息全量 conversations HTTP。

---

### Phase 5: IntentOutbox（reaction + approval）+ Mobile reaction UI

**目标**：高价值意图与消息同级可靠；三端 reaction 一致。

**File: `apps/desktop/src/features/chat/reaction-store.ts`**
- toggle → outbox；失败重试；ack 后 applyServerReactions。
- 去掉「仅 toast + 回滚」作为唯一失败处理。

**File: `apps/desktop/src/shared/lib/im-outbox.ts` / worker**
- kind=reaction_toggle payload。

**File: Mobile domain + cache + UI**
- 持久化 / 展示 reactions；toggle API + outbox。
- 入站 Durable reaction 更新聚合。

**File: Desktop approval use-cases**
- resolveApproval 等入 IntentOutbox（client_request_id）；daemon 不可达时持久待发。

**Backend（若缺）**
- approval respond 幂等键；已有则只接客户端。

**Verification Phase 5**
- 离线 reaction → 上线单次生效。
- Mobile 可见 Desktop reaction。

---

### Phase 6: Connection lifecycle + 性能收尾 + 清扫

**File: `apps/desktop/src/shared/lib/hub-realtime.ts`**
- visibilitychange / Tauri 窗口事件：pause WS + intent reconnect。
- 回前台立即 connect(attempt=0)。

**File: Mobile**
- Connected 边沿：outbox drain + 可选 inbox 轻校准。

**File: 全局清扫**
- 删除死代码：`projectMissingLocal…` 若被 outbox 取代；无用 imports；错误注释。
- `broadcast` 慢消费：SyncEngine 处理同步化/有界队列，避免 256 丢帧。

**Verification Phase 6**
- 笔记本睡眠唤醒后 Desktop 自动 live。
- 压测：群聊持续消息，inbox/timeline 无 O(N) 全量刷新。

---

### Phase 7: Verification matrix（强制门禁）

| 场景 | Desktop | Mobile |
|------|---------|--------|
| 发送中断网，恢复后单条 | yes | yes |
| 发送中杀进程，重启后单条（幂等） | yes | yes |
| inflight 中杀进程 | reclaim 后单条 | reclaim 后单条 |
| 后台会话 unread | +1 正确 | +1 正确 |
| 自己多端 echo | 不涨 unread | 不涨 unread |
| 长会话 loadOlder | yes | yes |
| SnapshotRequired | range 恢复 | range 恢复 |
| Desktop-native agent 完成 | Mobile 见同 id | — |
| Mobile @agent 完成 | Desktop 见 projector id | — |
| reaction 离线 | 重试成功 | 重试成功 |
| 无 2s completion poll | agent 仍可见 | — |

自动化：
- Desktop：vitest outbox / timeline-order / hub-timeline / digest。
- Mobile：dart unit outbox + cache sort + inbox patch。
- Backend：已有 client_message_id / reaction 测保持绿。
- 可选 e2e：`minos-backend/tests` 扩展 multi-client 幂等。

---

## Architectural Notes

- **SSOT 不变**：Hub = 协作气泡；Host daemon = agent 原始事件 / 本地工作台卡。本方案修的是 **客户端 Sync**，不把气泡权威搬回 daemon。
- **Latest-only**：不保留 soft-dedupe「兼容旧双 id」开关；统一 id 后旧错误 id 的本地行可在 hydrate 时丢弃。
- **Desktop vs Mobile 实现**：逻辑同构，代码不强制 mono 包共享（可后续抽 `packages/im-sync-spec` 伪代码/测试向量）。
- **Outbox 存储**：Desktop localStorage 可接受为 Phase 1；语义正确优先于迁 daemon。Mobile 必须 SQLite（进程可杀）。
- **markRead**：服务端 last_read_seq 仍是权威；客户端 unread 是 **镜像 + 增量**，定期校准。
- **不做**：E2EE、写扩散改读扩散、Web 端完整对齐（可复用同一不变量后续做）。
- **Agent 多写者风险**：禁止再引入「UI 扫本地 project 到 Hub」主路径；仅 Outbox host_projection + 服务端 projector。
- **性能**：热路径复杂度目标 O(1) per event（单行 upsert + 单 digest patch），禁止 O(conversations) HTTP。

---

## Multi-Agent Execution Map

| Agent | 拥有 Phase | 依赖 |
|-------|------------|------|
| A · Docs | 0 | — |
| B · Mobile Write | 1.A + 1.B | protocol/FRB |
| C · Desktop Outbox | 1.C | — |
| D · Agent Id | 2.A backend+daemon | 与 B/C 并行设计冻结 id |
| E · Desktop Merge cleanup | 2.B | 2.A |
| F · Desktop TimelineSync | 3.A | 2.B 部分 |
| G · Mobile TimelineSync | 3.B | 1.B |
| H · Inbox both | 4 | 3 可并行 inbox 与 timeline |
| I · Reaction/Approval | 5 | 1 outbox kind 扩展 |
| J · Lifecycle + sweep | 6–7 | 集成 |

冲突热点（需串行或锁文件）：`im-outbox.ts`、`social_providers.dart`、`hub-timeline.ts`、`client.rs` send API。

---

## File Change Summary

- `docs/superpowers/specs/2026-08-03-client-im-sync-engine.md` -- 本方案（客户端 Sync 终态 SSOT）
- `docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md` -- 修正 Phase 4.4 / 5.2 与 after_seq
- `docs/architecture-messaging.md` -- Client Sync 不变量 + reaction 状态
- `docs/architecture-desktop.md` -- 时间线/outbox/focus 终态
- `docs/architecture-mobile.md` -- Sync/Outbox/Inbox 终态
- `crates/minos-mobile/src/client.rs` -- send 透传 client_message_id；删除 None
- `crates/minos-mobile/src/http.rs` -- 请求体字段
- `crates/minos-mobile/src/realtime/session.rs` -- 解析失败不入壳；Snapshot 信号可消费
- `crates/minos-ffi-frb/src/api/minos.rs` + generated -- FRB send API
- `crates/minos-daemon/**` (conversation_completion) -- agent-result id = projector 同构
- `crates/minos-backend/src/turn_completion.rs` -- id 公式文档化/测试锁死
- `apps/mobile/lib/infrastructure/social_cache_store.dart` -- schema v4、排序、单行 upsert、unread bump
- `apps/mobile/lib/infrastructure/im_outbox_store.dart` -- 新 Outbox 存储
- `apps/mobile/lib/application/im_outbox_worker.dart` -- 新 worker
- `apps/mobile/lib/application/social_providers.dart` -- 去全量 invalidate；分页；发送走 outbox
- `apps/mobile/lib/application/minos_providers.dart` / im_sync -- SnapshotRequired
- `apps/mobile/lib/domain/social_message.dart` -- clientMessageId
- `apps/mobile/lib/application/conversations_sort.dart` -- tie-break
- `apps/mobile/lib/ui/features/messages/**` -- loadOlder / reactions UI
- `apps/desktop/src/shared/lib/im-outbox.ts` -- stale inflight；多 kind
- `apps/desktop/src/shared/lib/im-cloud-sync.ts` -- agent/reaction 入队
- `apps/desktop/src/shared/lib/im-hub-bridge.ts` -- snapshot range；focus 语义
- `apps/desktop/src/shared/lib/im-cloud-inbound.ts` -- after_seq gap
- `apps/desktop/src/shared/lib/hub-timeline.ts` -- 删除 soft-dedupe
- `apps/desktop/src/shared/lib/timeline-order.ts` -- seq-only 跨源
- `apps/desktop/src/shared/lib/minos-cloud.ts` -- 禁止 messageSeq??0
- `apps/desktop/src/shared/lib/hub-realtime.ts` -- visibility lifecycle
- `apps/desktop/src/shared/lib/hub-digest-cache.ts` -- 校准策略
- `apps/desktop/src/store/workspace/timeline.ts` -- 不写 focused
- `apps/desktop/src/store/workspace/live-ingress.ts` -- 删 burst poll
- `apps/desktop/src/features/chat/Timeline.tsx` -- 删 completion 2s poll
- `apps/desktop/src/features/chat/reaction-store.ts` -- outbox
- `apps/desktop/src/store/workspace/use-cases.ts` -- approval intent outbox
- 对应 `*.test.ts` / dart tests -- 回归门禁

---

## Success Definition（完成目标，非「改完文件」）

1. **At-most-once 用户可见语义，at-least-once 投递**：任意发送意图最多产生一条 Hub 消息。  
2. **进程死亡可恢复**：pending/inflight 不丢、不卡死。  
3. **热路径 O(1)**：单事件不触发全会话列表 REST 与全表 DELETE。  
4. **角标诚实**：后台 +1，聚焦清零，own echo 不涨。  
5. **历史可翻**：两端 before_seq；Snapshot 后可定向补洞。  
6. **排序确定性**：跨端仅 message_seq；无时钟交错。  
7. **Agent 气泡单 id**：无 soft-dedupe、无为撞 completion 的定时器。  
8. **Reaction 多端真实**：Hub SSOT + 客户端可靠写；文档一致。  
9. **无死代码补丁**：上述 Delete list 清零。

达到 1–9 即改造完成。
