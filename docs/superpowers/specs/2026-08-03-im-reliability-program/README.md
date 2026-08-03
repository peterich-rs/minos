# IM Reliability Program（客户端 Sync + 后端投递/编排）

| Field | Value |
|-------|--------|
| Status | **Normative program**（2026-08-03） |
| Date | 2026-08-03 |
| Rule | [AGENTS.md Final-Architecture Planning Rule](../../../../Agents.md) — **只按终态结构规划与实现** |
| Specs | [Client Sync Engine](../2026-08-03-client-im-sync-engine.md) · [Backend Delivery & Orchestration](../2026-08-03-backend-im-delivery-orchestration.md) |
| Tasks | [TASKS.md](TASKS.md) |
| Next track | [B6 / C5 / C6 终态细则](next-track-b6-c5-c6.md)（功能已 APPROVE） |
| Closeout | [收口验收 + 三层 backlog](closeout-and-backlog.md)（**V** 验收 · **P** 产品债 · **R** 实时面） |
| Supersedes (tracking) | [2026-08-02 Hub SSOT](../2026-08-02-hub-collaboration-message-ssot.md) 中客户端半成品与 reaction 文档漂移 |

---

## 1. 一句话

让 Minos 协作消息在 **Hub 持久化正确** 之后，**扇出、Push、Agent 派发/完成投影、以及 Desktop/Mobile 同步引擎** 都达到可推理的终态，而不是靠轮询、软去重、死代码分支和「先短期再还债」。

---

## 2. 两半必须一起完成

```
┌─────────────────────────────────────────────────────────────────┐
│  Client Sync Engine（Desktop + Mobile）                           │
│  Outbox · TimelineSync · InboxSync · Connection lifecycle         │
│  Spec: 2026-08-03-client-im-sync-engine.md                        │
└────────────────────────────┬────────────────────────────────────┘
                             │ HTTPS + WSS contracts
┌────────────────────────────▼────────────────────────────────────┐
│  Backend Delivery & Agent Orchestration                           │
│  Outbox lanes · Push · AgentDispatchQueue · CompletionWatch       │
│  SessionLifecycle                                                 │
│  Spec: 2026-08-03-backend-im-delivery-orchestration.md            │
└─────────────────────────────────────────────────────────────────┘
```

| 只做一半的后果 |
|----------------|
| 只做客户端 | 连发 @agent 仍丢气泡；在线仍推；dispatch 仍同步阻塞 |
| 只做后端 | Mobile 仍无幂等发送；每消息全量 invalidate；Desktop inflight 黑洞 |

**共享硬契约（跨半边冻结）：**

1. `client_message_id` 幂等用户消息。  
2. Agent 气泡 id：`agent-result:{conversation_id}:{session_id}:{origin_message_id}`。  
3. 排序主键：`message_seq`；禁止伪 0。  
4. Durable `event_id` 重放：客户端与 Push 均可幂等消化。  
5. SnapshotRequired / resume_after / before_seq / after_seq 全接通。

---

## 3. 终态成功标准（Program Definition of Done）

> 证据索引：[EVIDENCE.md](EVIDENCE.md)（Layer V 2026-08-03）。**仅勾有证明的项**；多端 live / Mobile 套件未跑项保持未勾。

### 3.1 写路径

- [ ] 任意端发送意图：断网 / 杀进程 / 重试 → **Hub 至多一条** 对应消息。（Hub 幂等单测已绿；设备杀进程矩阵 **NOT_RUN** — EVIDENCE V3#1）  
- [x] Desktop outbox：无永久 inflight；agent_result / reaction 同队列语义。（`im-outbox.test.ts` reclaim + kinds — EVIDENCE V2）  
- [ ] Mobile：FFI 透传 `client_message_id`；SQLite outbox；无「仅手动重试」。（代码透传已检；flutter unit **NOT_RUN** — EVIDENCE V2/V3）  
- [x] 发消息 HTTP **不**等待 host agent RPC；offline dispatch 可恢复。（enqueue-only + `agent_dispatch_queues_when_host_offline` / drains — EVIDENCE V1#4）

### 3.2 读与同步

- [x] 入站事件 O(1) inbox patch；禁止热路径全量 conversations wipe。（ConversationsController 热路径 patch — EVIDENCE V3#3）  
- [ ] unread 本地镜像正确（后台 +1，聚焦清零，own 不涨）。（代码路径 + debounce 单测；双端观察 **NOT_RUN** — EVIDENCE V3#4）  
- [x] 两端 loadOlder（before_seq）；Snapshot 定向 gap（after_seq）。（Desktop timeline-sync 单测 + 两端代码路径 — EVIDENCE V3#5）  
- [x] focused ≠ hasWindow；无 quiet load 偷焦点。（timeline hydrate-only + mark-read 契约测 — EVIDENCE V3）

### 3.3 Agent 气泡

- [x] 同 session N 次 dispatch → N 条 agent 气泡（origin 1:1）。（`two_rapid_dispatches_project_two_agent_bubbles` + completion_watch 双 origin — EVIDENCE V1#5）  
- [x] 无 body 软去重、无 0/400/1200 与 2s completion 轮询。（G2 gates + hub-timeline / Timeline 检视 — EVIDENCE V1 G2）  
- [x] Desktop-native 与 Hub projector 同一 id；CompletionWatch 无单 slot 覆盖。（`projector_idempotency_key_is_stable` + `isCanonicalAgentResultId` — EVIDENCE V1#5 / V2）

### 3.4 投递与通知

- [x] Push：`event_id` 幂等；UserOnline 策略实现且无死枚举。（decision + use_case + push_dispatch_log 单测 — EVIDENCE V1#9）  
- [x] Approval / SessionEnded：有真实 target 或从 decision 删除。（`approval_targets_conversation_members` — EVIDENCE V1#9）  
- [x] Outbox 分车道；host_command 不饿死 social；无过期假 ack。（lane isolation + expire dead_letter 单测 — EVIDENCE V1#2–3）

### 3.5 生命周期与卫生

- [x] SessionLifecycleJob 终结死 host session；watch TTL 失败闭环。（lifecycle + drain_expired 单测 — EVIDENCE V1#6）  
- [x] 各 spec Delete list 清零；文档与代码一致。（`scripts/im-reliability-gates.sh` + 检视表 — EVIDENCE V1 B7.2）  
- [x] 无 stub job、无「caller 应该检查 presence」类甩锅注释。（G2 presence lies gate — EVIDENCE V1 G2）

---

## 4. Phase 总序（可多 Agent 并行）

依赖示意：

```
B0/C0 文档冻结 ─────────────────────────────┐
     │                                        │
     ├─► B1 Push 正确性                       │
     ├─► B2 Outbox 车道                       │
     ├─► C1 客户端写路径 Outbox（可与 B1 并行）  │
     │                                        │
     ├─► B3 AgentDispatchQueue ──► B4 CompletionWatch + id
     │         │                      │
     │         └──────────┬───────────┘
     │                    ▼
     │              C2 客户端 agent id / 删 poll soft-dedupe
     │
     ├─► B5 SessionLifecycle
     ├─► C3 TimelineSync + Snapshot
     ├─► C4 InboxSync
     ├─► B6 Reaction 契约 · C5 Reaction/Approval Intent
     └─► C6 Connection lifecycle · B7/C7 验收矩阵
```

| 代号 | Spec Phase | 内容 |
|------|------------|------|
| C0 | Client 0 | 文档归零 |
| C1 | Client 1 | Mobile/Desktop 写路径 + Outbox |
| C2 | Client 2 | Agent id 对齐 + 删补丁 |
| C3 | Client 3 | TimelineSync / 分页 / Snapshot |
| C4 | Client 4 | InboxSync / unread |
| C5 | Client 5 | Reaction + Approval intent outbox |
| C6 | Client 6–7 | Visibility + 清扫 + 验收 |
| B0 | Backend 0 | 文档与契约 |
| B1 | Backend 1 | Push |
| B2 | Backend 2 | Outbox lanes |
| B3 | Backend 3 | AgentDispatchQueue |
| B4 | Backend 4 | CompletionWatch |
| B5 | Backend 5 | SessionLifecycle |
| B6 | Backend 6 | Reaction |
| B7 | Backend 7 | 验收 |

详细勾选见 [TASKS.md](TASKS.md)。

---

## 5. 多 Agent 所有权

| Agent 角色 | 拥有 | 不得擅自改 |
|------------|------|------------|
| Docs | C0/B0、architecture-* 同步 | 实现代码 |
| Backend Push | B1 | client |
| Backend Outbox | B2 | agent id 公式（与 B4 协商） |
| Backend Dispatch | B3 | completion 键（与 B4） |
| Backend Completion | B4 + daemon id | client merge 细节 |
| Backend Lifecycle | B5 | — |
| Mobile Write | C1 mobile | desktop outbox 文件 |
| Desktop Write | C1 desktop | mobile |
| Client Agent cleanup | C2 | backend 公式未冻结前禁止猜 id |
| Client Timeline | C3 | inbox |
| Client Inbox | C4 | timeline |
| Intent / Reaction | C5 + B6 协调 | — |

**冲突热点（串行或锁文件）：**  
`turn_completion.rs` id 公式、`completion_watch.rs`、`im-outbox.ts`、`social_providers.dart`、`client.rs` send、`hub-timeline.ts`、`notifications/use_case.rs`。

---

## 6. 明确不做

- 短期 presence if 而不改 DecisionInput。  
- 保留 soft-dedupe「兼容旧 dual id」。  
- sweeper 继续 COUNT。  
- feature flag 保留旧同步路径。  
- 以改造人天裁剪终态模块。

---

## 7. 文档维护

实现任一 Phase 合并后：

1. 更新 [TASKS.md](TASKS.md) 勾选。  
2. 若行为与 architecture-messaging / desktop / mobile / backend 文档不一致 → **同 PR 改文档**。  
3. 删除 list 中的路径必须在 PR 中消失，不得「留着备用」。
