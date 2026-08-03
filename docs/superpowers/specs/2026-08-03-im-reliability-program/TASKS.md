# IM Reliability — TASKS

Normative program: [README.md](README.md)  
Client spec: [../2026-08-03-client-im-sync-engine.md](../2026-08-03-client-im-sync-engine.md)  
Backend spec: [../2026-08-03-backend-im-delivery-orchestration.md](../2026-08-03-backend-im-delivery-orchestration.md)

规则：每个任务交付 **终态切片**（可测、可删旧路径），禁止「临时行为 + 后续还债」。

---

## Track 0 — Docs & contracts

- [x] **C0/B0.1** 冻结 program + 两份 spec 为 SSOT；Agents.md 含 Final-Architecture Planning Rule
- [x] **C0/B0.2** `architecture-messaging.md`：Client Sync 不变量 + Backend 投递/Push/Dispatch/Completion 终态；**代码现实**：client_live projector vs Desktop `host_projection` uplink；B3/B4 异步 dispatch + per-origin watch；C3 `after_seq` 已接通；删除「同步 RPC + 单 slot」/ 错误 dual-write 叙述
- [x] **C0/B0.3** `architecture-desktop.md` / `architecture-daemon.md` / `architecture-backend.md` 对齐代码现实（Desktop 仍可 host_projection agent-result uplink；daemon collab origin hard contract）
- [x] **C0/B0.4** `2026-08-02-hub-collaboration-message-ssot.md`：Phase 4.4 after_seq 客户端必接；Phase 5.2 reaction 云端已实现；链到本 program
- [x] **C0/B0.5** 冻结共享契约字符串：
  - agent-result id = `agent-result:{conv}:{session}:{origin_message_id}`
  - outbox lanes = `social_durable` | `host_command`
  - push 幂等键 = `(event_id, account_id)`

---

## Track B — Backend

### B1 Push 正确性

- [x] **B1.1** `DecisionInput`（event, prefs, now, presence, already_pushed）；实现或删除 `UserOnline`
- [x] **B1.2** 表 `push_dispatch_log`（或等价）；`dispatch_for_event` 成功后写入；重放不双发
- [x] **B1.3** `ApprovalRequested` / `AgentSessionEnded` 真实 target 解析；或删除 decide 分支与测试谎言
- [x] **B1.4** NotificationService 注入 presence 端口（registry / 连接表）
- [x] **B1.5** 单测：online skip、offline send、event_id 幂等、approval target 非空

### B2 Outbox 车道

- [x] **B2.1** migration：`outbox_events.lane`（sqlite + postgres `0001_initial`；DEFAULT `social_durable`）
- [x] **B2.2** `claim_available(lane, …)`；social 与 host_command 分离 claim；requeue 按 lane
- [x] **B2.3** host_command 异步 ack（`HostCommandOutboxJob` 独立 claim；无 serial wait_ack；social job 10s deadline 仅 social）
- [x] **B2.4** 过期 host_command → dead_letter + `outbox_host_command_expired_total`（禁止 Ok 后假 ack）；observation 不含 backend timeout `finished_at_ms`；`wait_for_terminal_response` / `HostCommandTimeoutJob` / expire 共用 dead_letter→mark 顺序
- [x] **B2.5** 测试：车道隔离、lane-scoped requeue、expired → dead 非 acked、timeout finished 不 unlock ack/ack_pending

### B3 AgentDispatchQueue

- [x] **B3.1** migration `agent_dispatch_queue`（sqlite + postgres `0001_initial`）
- [x] **B3.2** `send_message_inner`：落库 + social fanout enqueue 后立即返回；dispatch 只入队
- [x] **B3.3** `AgentDispatchWorker`：pending drain、host online **force-due** + 唤醒、退避、终态 failed + 用户可见错误
- [x] **B3.4** 测试：无 host 消息 200 + queue 行；上线后 force-due 再执行；`origin_message_id` 唯一

### B4 CompletionWatch + id

- [x] **B4.1** registry 键 = `origin_message_id`；session 二级索引；禁止覆盖未完成 watch
- [x] **B4.2** `agent_result_client_message_id(conv, session, origin_message_id)`；删除 last_seq 作 turn 主键
- [x] **B4.3** arm / project / post 全链路改 origin_message_id
- [x] **B4.4** daemon 同一 id：`origin_message_id` 贯通 host `agent_session.start` / `send_input` + Desktop `sendUserMessage`；completion 优先 origin 不回退 message_key
- [x] **B4.5** 测试：同 session 两连发 → 两气泡；重复 project 幂等；host command params 含 origin

### B5 SessionLifecycle

- [x] **B5.1** 重写 sweeper → `SessionLifecycleJob`：host 无 live WS 且 last_seen 过期 → open session `failed` + durable `AgentSessionEnded`
- [x] **B5.2** watch TTL：`deadline_at_ms` 扫描 → 用户可见失败气泡 + remove
- [x] **B5.3** 删除 COUNT-only stub 与假 DidWork
- [x] **B5.4** 测试：死 host → 非 running；watch drain_expired；live host skip

### B6 Reaction 契约

> 终态细则：[next-track-b6-c5-c6.md](next-track-b6-c5-c6.md) §1  
> `event_id = social-reaction-{conv}-{msg}-{emoji}-{actor}-{action}-{client_op_id}`（禁 UUID / at_ms）

- [x] **B6.1** 确定性 reaction `event_id`；请求体 `client_op_id`；去掉 `Uuid::new_v4()`
- [x] **B6.2** 文档锁定 **conversation-only** fanout（代码已测锁；只改 docs，不半拉 account 事件）
- [x] **B6.3** 测试：同 client_op_id 幂等；不同 client_op_id 可区分

### B7 Backend 验收

> **统一收口规划**：[closeout-and-backlog.md](closeout-and-backlog.md) **Layer V**（B7+G2+G3+C6.4/5+G4 一并编排，勿拆成互不相关的零散活）  
> **证据**：[EVIDENCE.md](EVIDENCE.md) V1

- [x] **B7.1** 跑通 backend spec Success Definition 表 → **V1**（9 场景映射单测/集成；`cargo test -p minos-backend --lib` 267 ok + v1_social 关键 3 测 ok — EVIDENCE V1）
- [x] **B7.2** Delete list 清零（grep 无 stub / 甩锅注释）→ **V1**（与 G2 合并；`./scripts/im-reliability-gates.sh` exit 0 + 代码检视表 — EVIDENCE V1）

---

## Track C — Client

### C1 写路径 Outbox

- [x] **C1.1** `minos-mobile` send 透传 `client_message_id`；删除 `None` 硬编码
- [x] **C1.2** FRB 再生；Dart repository / protocol 贯通
- [x] **C1.3** Mobile SQLite `im_outbox` + worker + sending reconcile
- [x] **C1.4** Mobile send/retry 复用同一 client_message_id；enqueue 非同步唯一路径
- [x] **C1.5** Desktop outbox：stale inflight reclaim；listDue 含 reclaim
- [x] **C1.6** Desktop `agent_result` / 后续 reaction kind 入同一 outbox 状态机
- [x] **C1.7** 单测 + 杀进程/断网手工矩阵（Desktop node:test + Mobile `im_outbox_store_test`；device 手工矩阵仍建议 QA）
- Residual (not C1): `projectedMessageIds` soft-session skip in `projectMissingLocalAgentResultsToHub` still C2 territory if any remain.

### C2 Agent id + 删补丁

- [x] **C2.1** 依赖 B4 公式冻结后：Desktop/daemon 只产生规范 id（`sendUserMessage(…, originMessageId)`）；uplink 仅 `isCanonicalAgentResultId`
- [x] **C2.2** 删除 `hub-timeline` body+120s soft-dedupe 与模糊 session 键（仅 id 相等 + 未 ack 乐观）
- [x] **C2.3** 删除 live-ingress 0/400/1200 burst
- [x] **C2.4** 删除 Timeline 2s completion trail poll（仅保留 phase=error 且无 livePush 的 one-shot quiet refresh）
- [x] **C2.5** `messageSeq ?? 0` → 缺失为 undefined；timeline-order 仅 seq 跨源 + optimistic 尾部
- [x] **C2.6** 测试更新（hub-timeline / timeline-order / im-outbox / isCanonicalAgentResultId）
- Residual: **TUI** still passes `origin_message_id: None` on local daemon RPC（local workbench only；not Hub collab path）— acceptable until TUI Linked; collab host/daemon path requires origin hard contract (skip non-canonical).

### C3 TimelineSync

- [x] **C3.1** Desktop：focused 与 hasWindow 分离；loadTimeline hydrate-only（不写 focused、不 mark-read）；focus/mark-read 在 Timeline 打开路径
- [x] **C3.2** Desktop SnapshotRequired：after_seq/before_seq range reconcile（禁止只清空当唯一策略）
- [x] **C3.3** Mobile：min/max seq、loadOlder、Scroll 触发
- [x] **C3.4** Mobile SnapshotRequired 消费：`ref.exists` 才 reconcile；关闭会话不 cold-start autoDispose / 不 mark-read
- [x] **C3.5** `parse_chat_message` 失败不入空壳
- [x] **C3.6** 排序与索引：禁止 COALESCE(seq, ms)

### C4 InboxSync

- [x] **C4.1** 删除 ConversationsController 每事件 `invalidateSelf` 主路径
- [x] **C4.2** 单行 upsert + unread bump；全量 REST 仅 hydrate/校准/snapshot
- [x] **C4.3** markRead debounce（Mobile 入站 + Desktop focused 入站 400ms）；禁止每条 inbound 立即 REST
- [x] **C4.4** Desktop digest：focused 修正后校准策略
- [x] **C4.5** conversations sort tie-break 语义化

### C5 Intent（reaction / approval）

> 终态细则：[next-track-b6-c5-c6.md](next-track-b6-c5-c6.md) §2  
> 契约：`outbox.entry.id` = `client_op_id` = B6 event_id 末段（依赖 B6 协议字段）

- [x] **C5.1** Desktop reaction → 单路径 `syncReactionToggleToCloud`（enqueue + flush；无 inline 双 POST）；due 含 reaction；worker 同路径
- [x] **C5.2** Mobile：reactions UI + SQLite `reactions_json` + outbox toggle + 入站聚合
- [x] **C5.3** Approval：Hub `client_request_id` 顶层 wire（Mobile 必传 / 后端幂等）；Desktop local daemon outbox **不** 注入 decision JSON；store 测同 client_request_id 幂等

### C6 Lifecycle & 验收

> 终态细则：[next-track-b6-c5-c6.md](next-track-b6-c5-c6.md) §3–§4

- [x] **C6.1** Desktop：`forceReconnect` + visibility/online/focus；隐藏暂停 ping、显示强制恢复
- [x] **C6.2** Mobile：App 根 bootstrap outbox worker（修 cold-start / 死 Provider）
- [x] **C6.3** 客户端 Delete list + 状态债：Messages badge 已挂 `socialUnreadCountProvider`；**Hub digest 单轨未读 / 订阅 LRU 见 Honest residuals**（范围大，未本轨强并）
- [ ] **C6.4** Client Success Definition 9 条全绿 → **V3**（[EVIDENCE](EVIDENCE.md) V3：自动化/Mobile outbox **升级**；设备杀进程/双端观察仍 **MANUAL_REQUIRED** runbook）
- [ ] **C6.5** 与 Backend B7 联调：多端发送、@agent 连发、在线不推、Snapshot、离线 reaction、休眠恢复 → **V3**（后端策略已测；多端 live 矩阵 **NOT_RUN** — EVIDENCE runbook）

---

## Integration gates（合并主线前）

> 与 B7 / C6.4–C6.5 一并规划与执行：[closeout-and-backlog.md](closeout-and-backlog.md) Layer V1–V4

- [x] **G1** 文档交叉链接无矛盾；messaging/daemon/desktop 描述 client_live projector + host_projection uplink（非「仅单写者已删 dual」谎言）
- [x] **G2** `rg` 门禁：无 `client_message_id: None`；无 soft 120_000 dedupe；无 stale_session COUNT-only 逻辑 → **V1**（`scripts/im-reliability-gates.sh` ALL PASS — EVIDENCE V1）
- [x] **G3** CI：backend 相关测 + desktop im/timeline 单测 + mobile unit → **V2**（backend 270 + Desktop IM 78 + Mobile 17 **PASS**；pub.dev TLS 时用 `PUB_HOSTED_URL` mirror — EVIDENCE V2）
- [ ] **G4** 手工：Program README §3 DoD 全部勾选 + EVIDENCE 索引 → **V4**（DoD 仅勾有证据项；设备矩阵仍阻塞全勾 — EVIDENCE V4）

### Layer R — Realtime Surface（独立 program 切片）

> 规范：[realtime-surface-model.md](../2026-08-03-realtime-surface-model.md) · 审计：[realtime-surface-audit.md](realtime-surface-audit.md)

- [x] **R0** DurableEvent × HTTP 写审计表
- [x] **R1** HostLinked/HostUnlinked 全链路（backend same-tx + Mobile/Desktop arm + 单测）
- [x] **R2** FriendRequestUpdated T2 发射 + Mobile refresh arm
- [x] **R3** Account thin digest — **DONE**（R3a Mobile conversation subscribe FRB + R3b wire + R3c clients + R3d delivery/push）
- [x] **R4** Desktop conversation subscription LRU (16) + Mobile SubscriptionLimitExceeded 非静默丢弃

### Honest residuals（deferrals）

| 项 | 状态 |
|----|------|
| Hub digest 单轨未读（C6.3 状态债） | Desktop 仍可能存在 local `readMessageCountById` vs Hub digest 双轨；本轨只交付 badge + forceReconnect，未强并 digest 单轨 |
| 订阅 LRU（C6.3 / R4） | **R4 DONE** — Desktop `conversation-sub-lru` + `subscribeConversation` 驱逐；**Mobile R3a** conversation subscribe/unsubscribe on open chat |
| Desktop approval Hub HTTP | Desktop resolve 仍走 **local daemon**（reachability outbox）；Hub `/v1/approvals/respond` + `client_request_id` 由 **Mobile** 全链路使用 |
| Multi-instance CompletionWatch / presence | in-memory registry；单实例假设 |
| TUI origin_message_id | local-only None；非 collab |
| Public HTTP `/v1/agent-sessions/*` | **origin_message_id 已透传**（可选字段）；无 origin 时仍为非 collab 调用方责任 |
| Desktop 会话列表 Hub SSOT | 仍以 daemon 为主 |
| C6.4 / C6.5 多端 live | 自动化见 EVIDENCE V3；设备联调矩阵 **NOT_RUN**（runbook） |
| R3 account thin digest | **DONE** — audit §3 + EVIDENCE R3 |
| G4 full DoD | 依赖 C6.4/C6.5 设备证据；后端 B7 + G3 已 PASS；**Layer R complete** |
