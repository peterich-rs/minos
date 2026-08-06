# Harden Backend IM Delivery & Agent Orchestration

| Field | Value |
|-------|--------|
| Status | **Normative target**（2026-08-03） |
| Date | 2026-08-03 |
| Scope | `minos-backend` 投递层、通知、Agent dispatch、TurnCompletion、Session 生命周期；删除 stub / 死代码 / 错误所有权 |
| Program | [2026-08-03-im-reliability-program](2026-08-03-im-reliability-program/README.md) |
| Pair | [客户端 Sync Engine](2026-08-03-client-im-sync-engine.md) |
| Related | [architecture-messaging.md](../../architecture-messaging.md)；[Hub SSOT 2026-08-02](2026-08-02-hub-collaboration-message-ssot.md) |
| Non-goals | E2EE；客户端 UI；为旧实例保留 wire 兼容；「先 stub 再补」 |

> **一句话目标**：持久化正确之后，**扇出、Push、Host 命令、Agent 派发与完成投影** 也具备清晰状态机与幂等边界；删除 UserOnline 死分支、空 resolve、单 slot watch、no-op sweeper、用时间/lease 碰运气的编排。

**规划约束**：遵守 [AGENTS.md Final-Architecture Planning Rule](../../../Agents.md) — 只设计终态代码结构，不写短期补丁路线。

---

## Breaking Change Notice

1. Outbox **分车道**（social durable vs host_command）；claim lease / worker / deadline 语义按车道定义。
2. Push 以 `event_id` 幂等；`DecisionReason::UserOnline` **接入真实决策路径或删除枚举**（禁止死分支）。
3. `ApprovalRequested` / `AgentSessionEnded` 必须有可解析的 target accounts，或从 decision 表删除对应分支。
4. `CompletionWatch` 键 = **origin user message / dispatch id**，禁止 per-session 单 slot 覆盖。
5. Agent 最终气泡幂等键 = `agent-result:{conversation_id}:{session_id}:{origin_message_id}`（与客户端 / daemon 统一；废止裸 `raw last_seq` 作 turn_write_id）。
6. `try_agent_dispatch` 不再阻塞 HTTP 发送成功路径；dispatch 进入 **AgentDispatch 队列**（持久化）。
7. `stale_session_sweeper` 必须真实终结超时 session 并驱动失败投影；禁止 COUNT-only stub。

---

## Feasibility Assessment

存储与 claim 原语已存在：`outbox_events` FOR UPDATE SKIP LOCKED、`client_message_id` 幂等 insert、`message_seq` 同事务分配、reaction schema、TurnCompletionProjector 文本探针。缺口集中在 **编排状态机与所有权**，不是「从零造 IM」。

**Fully feasible** under latest-only：直接改契约与删除错误路径。

---

## Problem Inventory（已复核）

| ID | 区域 | 严重度 | 现状 | 终态要求 |
|----|------|--------|------|----------|
| B1 | Outbox re-publish | P1 | publish 非幂等；push 按 `msg:{conv}` cooldown | fanout at-least-once + **consumer 幂等**（WS event_id / push event_id） |
| B2 | Multi-instance reclaim | P1 | 每实例 5s recover；host_command 阻塞 claim | 分车道；lease ≥ 处理上界；host_command 异步 ack |
| B3 | Job tick_deadline | P1 | 10s deadline vs 64×250ms host wait | tick 不串行阻塞等 ack；或 deadline 匹配车道 |
| B4 | Host command 过期 | P2 | deadline 过返回 Ok → 仍 ack | dead-letter + 指标，禁止假成功 |
| B5 | message_seq | — | 正确 | 保持 |
| B6 | mark_read | — | 良性 TOCTOU | 保持 |
| B7 | Reaction fanout | P2 | 仅 conversation topic | 产品锁定：不进 account 或补摘要事件 + 文档 |
| B8 | Reaction event_id UUID | P2 | ensure 无逻辑幂等 | 确定性 `event_id` + client_op_id |
| B9 | Sync dispatch | P1 | HTTP 等 host RPC | 消息落库立即返回；dispatch 异步队列 |
| B10 | Host offline dispatch | P1 | 错误气泡、不重试 | 持久 pending_dispatch，host 上线 drain |
| B11 | UserOnline | P0 | 枚举存在，caller 永不 Skip | 决策管线接入 presence **或删除** |
| B12 | Approval/Session push targets | P0 | `resolve_target_accounts` 恒空 | 真实解析或删除 decision 分支 |
| B13 | Session sweeper | P1 | COUNT stub | 终结超时 session + 通知 watch |
| B14 | Completion single slot | P0 | `arm` 覆盖 session watch | 多 watch / 按 origin_message |
| B15 | trigger_seq = last_seq | P0 | 连发碰撞 | origin_message_id 为 turn 键 |
| B16 | Watch 无 TTL | P1 | 泄漏 + 无失败闭环 | TTL → 错误气泡 → remove |

---

## Design

### 1. Target architecture

```
                    ┌─────────────────────────────────────┐
                    │  HTTP / Ingest / Conversation UC      │
                    │  commit message + durable+outbox tx    │
                    │  enqueue AgentDispatch (async)        │
                    └───────────────┬───────────────────────┘
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          ▼                         ▼                         ▼
┌──────────────────┐    ┌──────────────────┐    ┌──────────────────────┐
│ Outbox lane:     │    │ Outbox lane:     │    │ AgentDispatchQueue   │
│ social_durable   │    │ host_command     │    │ (pending|inflight|   │
│ claim / publish  │    │ claim / deliver  │    │  done|failed)        │
│ / ack            │    │ / host ack       │    │ host online drain    │
└────────┬─────────┘    └────────┬─────────┘    └──────────┬───────────┘
         │                       │                         │
         ▼                       ▼                         ▼
   Bus + WS fanout          Host WS only            send_input / start
   + PushPipeline           (幂等 command_id)       arm CompletionWatch
         │                                              │
         ▼                                              ▼
   event_id 幂等 push                            TurnCompletionProjector
   presence 在决策输入内                          key=origin_message_id
                                                SessionLifecycleJob
```

### 2. Key design decisions

1. **Outbox 分车道（lane）**  
   - `social_durable`：聊天 / account / reaction 等；publish 后即可 ack；push 异步且 event_id 幂等。  
   - `host_command`：单独 claim、更长 lease、不与 64 条社交消息同批串行 `wait_ack`。  
   - 拒绝：单队列混跑 + 全局 30s lease 碰运气。

2. **At-least-once publish + 端到端幂等**  
   - Durable 行 `event_id` 稳定；re-publish 允许。  
   - WS：连接级 seen **或** 客户端 event_id 去重（双端约定）。  
   - Push：表 `push_dispatches(event_id, account_id)` UNIQUE，成功后永不重发。  
   - 拒绝：仅靠 `msg:{conversation_id}` 30s cooldown 冒充幂等。

3. **Push 决策是完整纯函数 + 显式副作用边界**  
   ```
   DecisionInput {
     event, prefs, now_ms,
     presence: AccountPresence,      // 由 dispatch 层注入，不是「以后再说」
     already_pushed_event: bool,     // event_id 幂等
   }
   ```  
   - `UserOnline`：账户存在 **任意 live client WS** 且策略为 suppress_push_when_online 时 Skip。  
   - 竞态（刚断线仍 online）：接受 **push-skip 窗口偏短漏推** 或 **online 宽限期 N 秒**（终态选一种写死：默认 **断线后 grace 内仍 Skip，grace 后允许 push**；客户端前台再 suppress 重复）。  
   - 禁止：枚举存在但无 caller；禁止注释甩锅。

4. **可通知事件必须有 target 解析**  
   - `AccountConversationMessageAppended` → account_id（已有）。  
   - `ApprovalRequested` → session owner + 可审批成员（查库，终态实现）。  
   - `AgentSessionEnded` → session owner account。  
   - Conversation-topic 事件不推（避免双推）；若某事件只有 conversation 形态，先映射到 account 或明确 NotNotifiable。

5. **Agent dispatch 与消息提交解耦**  
   - `send_message`：事务提交 + social outbox enqueue → **立即 HTTP 200**（消息体）。  
   - `AgentDispatchQueue` 行：`dispatch_id`, `origin_message_id`, `conversation_id`, `account_id`, `agent_id`, `session_id?`, `payload`, `status`, `attempts`, `next_attempt_at`.  
   - Worker：有 live host 则 `send_input`/`start`；无 host 则 pending + presence 边沿唤醒。  
   - 失败：有限重试 → 终态 failed + **用户可见** agent_error 气泡（与今日 offline 可见性一致，但是可恢复队列）。  
   - 拒绝：HTTP 同步 RPC；拒绝 offline 永久丢意图。

6. **CompletionWatch 一对一 origin message**  
   - 键：`origin_message_id`（或 `dispatch_id`），registry 允许多 session 多 watch。  
   - 幂等气泡 id：`agent-result:{conv}:{session}:{origin_message_id}`。  
   - Daemon Desktop-native 与 Hub projector **同一公式**（客户端方案 Phase 2 对齐）。  
   - 删除：`arm(session_id)` 覆盖；删除 `turn_write_id = raw last_seq`。

7. **Session 生命周期是真实 Job**  
   - Host 失联 / heartbeat 超时 / 超过 STALE：session → failed/ended。  
   - 触发：未完成 watch → `DoneWithoutText` 或 **timeout error bubble** + remove watch。  
   - 拒绝：COUNT(*) 冒充 DidWork。

8. **Reaction 扇出策略写死一种**  
   - **默认终态**：reaction 仅 conversation topic（避免 account 风暴）；inbox digest **不**因 reaction 变；多端仅在打开会话或冷拉见更新。  
   - 文档与测试锁定。若产品要 inbox 反应提示，则增加 **可选** `AccountConversationReactionUpdated` 摘要事件（非每 actor 全量），二选一实现，不留「好像有又没有」。

9. **确定性 reaction event_id**  
   - `social-reaction-{conversation_id}-{message_id}-{emoji}-{actor_key}-{action}-{client_op_id|logical_version}`  
   - toggle 事务内聚合版本单调；禁止 `Uuid::new_v4()` 进 event_id。

### 3. Concrete types / schema

#### 3.1 Outbox lane（逻辑；可用列 `lane` 或分表）

```sql
-- outbox_events 增加
ALTER TABLE outbox_events ADD COLUMN lane TEXT NOT NULL DEFAULT 'social_durable';
-- lane IN ('social_durable', 'host_command')
-- claim_available(lane, limit, claimed_by)
-- host_command: lease_ms 更长；batch 更小；不在 publish 路径同步 wait 阻塞整批
```

#### 3.2 Push 幂等

```sql
CREATE TABLE push_dispatch_log (
  event_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  sent_at_ms BIGINT NOT NULL,
  PRIMARY KEY (event_id, account_id)
);
```

#### 3.3 AgentDispatchQueue

```sql
CREATE TABLE agent_dispatch_queue (
  dispatch_id TEXT PRIMARY KEY,
  origin_message_id TEXT NOT NULL UNIQUE,
  conversation_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  session_id TEXT,
  forwarded_text TEXT NOT NULL,
  status TEXT NOT NULL, -- pending|inflight|succeeded|failed_terminal
  attempts INT NOT NULL DEFAULT 0,
  next_attempt_at_ms BIGINT NOT NULL,
  last_error TEXT,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL
);
```

#### 3.4 CompletionWatch

```rust
// completion_watch.rs 终态
pub struct CompletionWatch {
    pub dispatch_id: String,
    pub origin_message_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub agent: AgentRow,
    /// Only raw events with seq > this count toward this turn.
    pub raw_seq_floor: u64,
    pub armed_at_ms: i64,
    pub deadline_at_ms: i64,
    pub mention_account_id: Option<String>,
    pub mention_minos_id: Option<String>,
}

// Registry: HashMap<origin_message_id, CompletionWatch>
// Secondary index: session_id -> Vec<origin_message_id> for ingest fan-in
```

#### 3.5 Agent result id

```rust
TurnCompletionProjector::agent_result_client_message_id(
    conversation_id,
    session_id,
    origin_message_id, // was trigger_seq
)
// → "agent-result:{conversation_id}:{session_id}:{origin_message_id}"
```

#### 3.6 Notification decide（终态签名）

```rust
pub fn decide(input: &DecisionInput) -> Decision {
    // quiet hours, prefs, self-sender (when known),
    // already_pushed → Skip Idempotent,
    // presence.online_with_grace → Skip UserOnline,
    // else Send { cooldown for UX rate limit only, not correctness }
}
```

### 4. Delete list

| 删除 | 位置 |
|------|------|
| `DecisionReason::UserOnline` 无 caller 的注释甩锅 | `decision.rs` + 接好或删枚举 |
| `resolve_target_accounts` 对 Approval/SessionEnded 返回空却保留 decide 分支 | `use_case.rs` |
| `stale_session_sweeper` COUNT-only | `jobs/stale_session_sweeper.rs` |
| `completion_watches.arm` 按 session 覆盖 | `completion_watch.rs` |
| `watcher_from_seq = last_seq` 作为 turn 主键 | `social.rs` forward_agent_dispatch |
| HTTP `send_message_inner` 内同步 `try_agent_dispatch` | `conversations.rs` |
| reaction `event_id` 中 `Uuid::new_v4()` | `delivery.rs` |
| host_command 过期 `Ok(())` 后当成功 ack | `realtime.rs` publish_durable_row |
| 用 `msg:{conv}` cooldown 当唯一防重 | push 路径 |

---

## Phased Implementation

每个 Phase 是**终态子系统可合并切片**（AGENTS.md），不是临时行为。

### Phase B0: 文档与契约冻结

- 本文 + program README / TASKS。  
- `architecture-messaging.md`：投递/Push/Completion/Dispatch 终态段落；删除「presence 由 caller 处理」谎言。  
- `2026-08-02` Hub SSOT：链到 program；reaction / agent id 与本文一致。  
- 固化 agent-result id 公式与 Outbox lane 名称。

**验收**：文档无「短期 / stub / 以后再接」。

### Phase B1: Push 正确性（幂等 + 目标 + presence）

**目标**：在线策略与审批通知符合产品；重放不双推。

**Files**
- `notifications/decision.rs` — `DecisionInput`；UserOnline 真实分支或删除。  
- `notifications/use_case.rs` — 注入 presence、event_id 幂等、Approval/Session 目标查询。  
- `store/notification_cooldowns.rs` / 新 `push_dispatch_log` — event 级幂等。  
- `realtime.rs` `spawn_push_dispatch` — 传入 event_id；失败可重试但不双成功发送。  
- `runtime.rs` — NotificationService 持有 registry 或 presence 端口。  
- tests：online skip；offline send；同一 event_id 第二次 dispatch 不发送；Approval 有 target。

**删除**：空 `resolve` + 有 `decide` 的死代码路径。

### Phase B2: Outbox 车道与 host_command 生命周期

**目标**：社交 fanout 与 host 命令互不拖死；无假成功 ack。

**Files**
- migrations：`outbox_events.lane`（或分表）。  
- `store/outbox_events.rs` — `claim_available(lane, …)`；requeue 按 lane。  
- `realtime.rs` — 分 worker 或同 job 分 claim；host_command **异步** ack 等待（不阻塞 social batch）。  
- `jobs/outbox_dispatcher.rs` — deadline / interval 匹配车道最坏情况；或拆 `HostCommandOutboxJob`。  
- 过期 host_command → dead_letter + metric。  
- tests：双车道并发；crash after publish re-publish 一次；push log 幂等。

### Phase B3: AgentDispatchQueue + HTTP 解耦

**目标**：发消息延迟与 host RPC 无关；offline 可恢复。

**Files**
- migration `agent_dispatch_queue`。  
- `http/v1/conversations.rs` — send 成功路径只 enqueue dispatch。  
- `http/v1/social.rs` — `try_agent_dispatch` 拆为 plan + enqueue；worker 执行 forward。  
- 新 `jobs/agent_dispatch_worker.rs` — Connected host drain；指数退避。  
- presence / host online 事件 → notify worker。  
- 失败终态 → 现有 `notify_agent_dispatch_failure` 路径。  
- tests：无 host 时消息 200 + queue pending；host 上线后 dispatch；幂等 origin_message_id。

### Phase B4: CompletionWatch 多 turn + id 统一

**目标**：连发 @agent 每条 user 消息对应至多一条 agent 气泡；无覆盖丢失。

**Files**
- `completion_watch.rs` — 多键 registry + session 二级索引 + deadline。  
- `turn_completion.rs` — id 用 `origin_message_id`；探针按 raw_seq_floor。  
- `social.rs` arm / try_project / post — 全链路 origin_message_id。  
- daemon conversation_completion — **同一 id 公式**（跨 crate 契约，与客户端 Phase 2 同步）。  
- tests：同 session 两连发 → 两气泡两 id；重复 project 幂等。

**删除**：session 单 slot arm；last_seq 作 turn_write_id。

### Phase B5: SessionLifecycleJob + watch TTL

**目标**：死 host 有终态；watch 不泄漏。

**Files**
- `jobs/stale_session_sweeper.rs` → 重写为 `SessionLifecycleJob`：按 host last_seen / connection 终结 session。  
- watch TTL 扫描（可同 job）：超时 → 错误可见 + remove。  
- tests：host 消失后 session 非 running；watch 清空。

### Phase B6: Reaction 契约收口

**目标**：扇出策略与 event_id 确定性。

**Files**
- `delivery.rs` — 确定性 event_id；文档锁定 conversation-only。  
- tests 更新。  
- 若产品选 account 摘要：单独 PR 实现完整事件，不留半拉。

### Phase B7: 验证矩阵

| 场景 | 期望 |
|------|------|
| Outbox crash after publish | 重发；WS/push 不双成功可见 |
| 64 host_commands 高峰 | 不拖死 social outbox |
| 用户在线聊天 | 不收 message push（按终态 presence 策略） |
| 用户离线 | 收 push |
| Approval | 有 token 的审批人收到 push |
| 无 host 发 @agent | 消息成功；dispatch pending；上线后执行 |
| 同 session 两连发 | 两 agent 气泡，reply_to 各正确 |
| Host 死亡 | session 终结；用户见失败/超时；watch 清空 |
| Reaction toggle 重试 | 单逻辑结果；event_id 稳定 |

自动化：backend unit + 现有 e2e/ws 扩展。

---

## Architectural Notes

- **不改变** Hub 消息 SSOT、Transactional Outbox 先写后推的总原则。  
- **改变** 的是：dispatch 同步性、completion 主键、push 完备性、session 生命周期诚实性。  
- 与客户端方案：agent-result id 必须 **跨 backend/daemon/desktop** 同一字符串；客户端禁止 soft-dedupe。  
- Multi-instance：claim 继续 SKIP LOCKED；recover 只碰 lease 过期；lane 隔离降低误 reclaim。  
- Semver：仅 monorepo；latest-only 允许破坏旧 agent-result 后缀语义（无生产兼容包袱）。

---

## File Change Summary

- `docs/superpowers/specs/2026-08-03-backend-im-delivery-orchestration.md` -- 本文  
- `docs/superpowers/specs/2026-08-03-im-reliability-program/*` -- 总计划与 TASKS  
- `docs/architecture-messaging.md` -- 投递/Push/Dispatch/Completion 终态  
- `docs/architecture-backend.md` -- workers / outbox lanes  
- `crates/minos-backend/migrations/**` -- lane、push_dispatch_log、agent_dispatch_queue  
- `crates/minos-backend/src/store/outbox_events.rs` -- 分车道 claim  
- `crates/minos-backend/src/store/social/delivery.rs` -- reaction event_id  
- `crates/minos-backend/src/realtime.rs` -- publish/ack/push/host_command  
- `crates/minos-backend/src/jobs/outbox_dispatcher.rs` -- 车道与 deadline  
- `crates/minos-backend/src/jobs/stale_session_sweeper.rs` -- 真生命周期  
- `crates/minos-backend/src/jobs/agent_dispatch_worker.rs` -- 新  
- `crates/minos-backend/src/notifications/**` -- DecisionInput、targets、幂等  
- `crates/minos-backend/src/completion_watch.rs` -- 多 watch  
- `crates/minos-backend/src/turn_completion.rs` -- origin_message_id  
- `crates/minos-backend/src/http/v1/conversations.rs` -- 异步 dispatch  
- `crates/minos-backend/src/http/v1/social.rs` -- enqueue / arm  
- `crates/minos-daemon/**` -- agent-result id 对齐  
- tests under `crates/minos-backend/tests/**`

---

## Success Definition

1. 任意 durable `event_id` 重放不导致双 push 成功记录。  
2. 在线策略与审批 push 行为与文档一致（无死分支）。  
3. 发消息 HTTP 不绑定 host RPC 延迟；offline dispatch 可恢复。  
4. 同 session N 次 @agent → N 条幂等 agent 气泡（origin 1:1）。  
5. 死 host → session 与 watch 有终态；无永久 running + 泄漏 map。  
6. Outbox 车道互不饿死；无过期 host_command 假 ack。  
7. Delete list 清零。
