# IM Reliability Full Review（2026-08-09）

| Field | Value |
|-------|--------|
| Status | **Active remediation** |
| Date | 2026-08-09 |
| Scope | Workspace snapshot at HEAD `92e86ad` + local WIP (66 modified files, untracked) |
| Rule | [AGENTS.md Final-Architecture Planning Rule](../../../Agents.md) — target structure only |
| Related | [IM Reliability Program](./2026-08-03-im-reliability-program/README.md) · [Client Sync Engine](./2026-08-03-client-im-sync-engine.md) · [Backend Delivery](./2026-08-03-backend-im-delivery-orchestration.md) · [Realtime Surface](./2026-08-03-realtime-surface-model.md) |
| Evidence base | Code behavior primary; architecture docs as target reference only |

---

## 总体结论

Minos 的 IM 目标架构方向基本正确，但当前实现仍没有达到微信、Discord、Slack 这类成熟 IM 的可靠性水平。

**正确的部分：**

- Conversation-first 产品模型
- Hub 作为协作消息 SSOT
- Durable / Stream 双通道
- Transactional Outbox
- 每 topic cursor、断线 replay、SnapshotRequired
- Host 本地 SQLite ingest log
- Desktop/Mobile optimistic、outbox、分页、snapshot rebuild
- Agent dispatch、completion、approval 已开始按持久状态机建模

**核心缺口：** 基础不变量没有闭合，会导致漏消息、乱序、重复执行、越权读取、幽灵状态。

### 必须闭合的 8 条不变量

1. 全局序号永不回退
2. replay 与 live 无缝衔接
3. cursor 只能在状态持久应用后推进
4. 同 conversation 出站严格 FIFO
5. 消息、附件、事件、outbox 原子提交
6. Agent completion/approval/delegation 可重放
7. 成员权限变化立即撤销实时访问
8. read watermark 必须由客户端明确提交 observed seq

参考：Discord Gateway sequence + Resume；Matrix `/sync` initial snapshot + incremental delta token。Minos 设计形式接近，实现破坏了这些模型赖以成立的关键不变量。

---

## P0：Durable topic sequence 会回退（同步黑洞）

### 现象

- `durable_event_log::record_in_tx` 用 `MAX(topic_seq)+1` 分配序号
- `retention_cleaner` 物理删除 90 天前 durable rows
- 空日志时 gateway 把 `retention_floor` 当成 `0`，不返回 SnapshotRequired

### 场景

1. 客户端保存 `account:x = 50`
2. topic 长期不活跃
3. retention 删除全部历史行
4. 新事件重新得到 `topic_seq=1`
5. 客户端 `resume_after=50` 重连
6. 新事件永远不满足 `topic_seq > 50` → 长期同步黑洞

### 终态结构

独立 `topic_metadata`（或等价 sequence authority）：

| Column | Role |
|--------|------|
| topic / topic_kind | 主键 |
| high_watermark | 已分配最大 seq（永不回退） |
| retention_floor | 已删除 payload 的上界（客户端 resume 低于此则 SnapshotRequired） |
| updated_at | 审计 |

- seq 由该表或专用 sequence authority 分配
- retention 只能删除 payload，不能删除序号权威

### 修复状态

- [ ] sequence authority 表 + 分配路径
- [ ] retention 更新 floor，不销毁 watermark
- [ ] 空日志时正确返回 SnapshotRequired

---

## P1 清单

### 1. replay / live 竞态（同连接内逆序）

**现状：** gateway 先注册 live subscription → SubscribeAck → 再 replay。

可能收到：`live 101` → `replay 91..100`。客户端若 cursor 已到 101，91–100 永久跳过。

**终态：** replay/live barrier

1. 事务读取 high watermark
2. replay 到该 watermark，同时缓冲新 live
3. 按 seq drain 缓冲
4. 再切换到 live

Redis Pub/Sub 只能作唤醒与 fanout，不能替代 barrier。

### 2. 成员移除后仍可接收消息（数据泄露）

**现状：**

- 成员增删无 durable membership event
- 无 account inbox 更新、无撤销已有 subscription、无通知被移除端、无 Push
- 订阅权限只在 Subscribe 时检查
- 任意普通成员可移除其他成员甚至创建者

**终态：** membership state machine

- owner/admin/member
- self-leave / moderator-remove / ownership transfer
- membership version + visibility epoch
- commit 后实时撤销 subscription

### 3. Push 不是可靠投递任务

**现状：** durable publish 后 `tokio::spawn` 发 Push，social outbox 即可 ack；失败只日志不重试；TOCTOU 双发。

**终态：** 独立持久 Push lane

```
event_id, account_id, installation_id, status, attempts, next_attempt_at, provider_message_id
```

### 4. client_message_id 幂等语义不完整

**现状：** 命中相同 ID 只检查 conversation，不检查 sender/text/reply/attachments/source。

**终态：** 幂等键至少 `(sender_account_id, client_message_id)` + request fingerprint；语义冲突返回 conflict。

### 5. 附件提交不原子 + 接收者打不开

**现状：** 先提交 message/durable/outbox，再单独写附件关联；下载只允许 blob owner。

**终态：** 消息、附件引用、mentions、durable、outbox 同事务；下载授权基于 membership + message visibility。

### 6. Agent completion 仍是内存状态

**现状：** Backend `completion_watch` 与 Daemon completion 均不从 persisted events 重建。

**终态：** durable projector

```
origin_message_id, session_id, raw_seq_floor, deadline, status,
projected_message_id, projector_checkpoint
```

启动时 replay，不依赖内存回调。

### 7. Delegation / result / source delivery 不原子

**现状：** 先写结果气泡再结束 delegation；source 先发 provider 再记 delivery → 崩溃可双发。

**终态：** transactional outbox + stable delivery ID + receiver inbox dedupe。

### 8. Daemon 队列满静默丢 provider event

**现状：** write retry 2048 满丢新 frame；parent deferred 满丢最老。

**终态：** durable spool / 反压 / 显式终止 session 写失败终态 — 禁止静默丢弃。

### 9. 生命周期广播 lag 只打日志

**现状：** Manager 广播容量 64；Lagged 后只 log，可丢 Idle/Closed/Crash。

**终态：** versioned lifecycle journal，或 lag 后 manager snapshot + SQLite checkpoint 全量对账。

---

## 客户端状态管理

### Desktop

| Issue | 终态 |
|-------|------|
| Outbox 同 conv 失败后继续 flush 后续 → Hub seq 乱序 | per-conversation send lane 严格 FIFO |
| localStorage outbox：写失败仍成功、超 500 裁剪、无 CAS | Tauri/SQLite 事务 outbox |
| Host seq 与 Hub seq 双排序世界 | Hub `message_seq` 规范序；Host 卡用 `anchor_hub_message_seq` + suborder |
| 首次发送 multi-@ fan-out，重试只解析单 Agent | 首次/重试同一 intent 语义 |

### Mobile

| Issue | 终态 |
|-------|------|
| cursor 在 apply 前 `update_seq` | cursor 仅在 cache transaction/reducer commit 后推进 |
| Subscribe desired/confirmed 混淆 | desired / pending / confirmed 三态 |

### Web

非正式 IM 客户端：mock data、旧 envelope、单槽 latestSocialEvent、无 outbox/resume/SnapshotRequired。全端统一 Sync Engine 对 Web 不成立。

---

## Read / unread

全端 mark-read 未提交 observed seq，服务端标到最新 → 未渲染消息被静默已读。

**终态：**

```
mark_read(conversation_id, read_up_to_message_seq)
```

服务端单调 `MAX`；客户端仅在消息进入可见窗口后推进。

---

## 与成熟 IM 的产品差距（非全部为 bug）

| 领域 | 差距 |
|------|------|
| 统一端侧数据库 | 缺 messages-by-id + window IDs + outbox + read watermark 事务 reducer |
| 多设备同步 | 缺设备级 cursor、read merge、设备滞后管理 |
| 群状态机 | 缺 owner/admin、邀请、离群、解散、ownership transfer、visibility epoch |
| 消息能力 | 缺编辑、置顶、thread、around-seq、首未读锚点 |
| Attention | mention/approval/session failure 多套派生状态 |
| Push | 缺持久队列、retry、badge reconciliation |
| 媒体 | 缺扫描、缩略图、转码、配额、引用计数、GC、授权版本 |
| 搜索 | 缺本地/服务端索引与上下文定位 |
| 大群 | 偏写扩散，缺大小群分级 |
| 安全 | refresh token 在 localStorage；Desktop 应用系统凭据存储 |
| 隐私 | 无 E2EE（若目标微信/Signal 级需完整密钥体系） |

---

## 架构判断：可保留主干

```
HTTP command
  → domain transaction
  → canonical message/state rows
  → durable event
  → transactional outbox
  → ordered topic delivery
  → client transactional reducer
  → advance apply cursor
```

### 需重构的关键 seam

1. **TopicLog** — sequence authority、retention metadata、replay/live barrier
2. **ConversationCommand** — 一次事务写 message/attachments/mentions/membership/durable/outbox
3. **ClientSyncEngine** — 统一 snapshot/delta/apply/cursor
4. **ConversationSendLane** — per-conversation FIFO
5. **AgentTurnProjector** — durable watch/checkpoint/replay
6. **MembershipStateMachine** — 角色、visibility、订阅撤销、媒体授权

---

## 修复优先级（本轮执行）

1. **立即** topic sequence authority + 空日志 retention floor
2. membership revocation 数据泄露
3. replay/live barrier
4. Mobile cursor-before-apply + 全端 per-conversation FIFO
5. 持久化 Push、CompletionWatch、Agent projector
6. 原子化附件与 delegation/source delivery
7. 统一 read watermark
8. 收敛客户端架构与产品能力（后续）

---

## 验证备注（审查时）

IM reliability gate 当时结果：

| Gate | Result |
|------|--------|
| client_message_id production guard | PASS |
| soft-dedupe guards | PASS |
| lifecycle job guard | PASS |
| presence guard | PASS |
| reaction event-id guard | PASS |
| messageSeq ?? 0 | FAIL（`hub-timeline.test.ts:159` 测试代码；门禁白名单漂移） |

---

## Remediation tracking

| # | Item | Status |
|---|------|--------|
| P0 | Topic sequence authority + retention floor | **done** — `topic_metadata` sequence authority; retention advances floor without resetting watermark; empty log SnapshotRequired |
| P1.1 | Replay/live barrier | **done** — arm barrier before live register; buffer live; replay ≤ HW; ordered drain |
| P1.2 | Membership revocation + roles | **done** — owner/admin roles; durable membership events; `revoke_topic_for_account` on remove; self-leave allowed |
| P1.3 | Durable Push lane | **done** — `push_dispatch_queue` + worker claim/backoff; `push_dispatch_log` success ledger; fire-and-forget spawn removed |
| P1.4 | client_message_id fingerprint | **done** — sender/body/reply/**attachments**/`message_source` conflict → `idempotency_conflict` (HTTP 409) |
| P1.5 | Atomic attachments + membership download auth | **done** — link blobs in same TX as message/durable; download auth via membership |
| P1.6 | Durable CompletionWatch / Agent projector | **done** — `completion_watches` table; arm/project/expire persist; startup hydrate |
| P1.7 | Atomic delegation / source delivery | **done** — fail-closed complete after result bubble; source delivery outbox-first stable id |
| P1.8 | Daemon drop / lag (related) | **partial** — ingest queue full → session close (no silent drop); Manager lag still open |
| C1 | Mobile cursor-after-apply | **done** — social frames carry `topic`/`topic_seq`; cursor advances only after Dart `ackDurableApplied` post-SQLite commit; parse/hold on no subscriber |
| C2 | Per-conversation FIFO outbox | **done** — Desktop intent lanes (`message`/`reaction`/`approval`) + per-lane worker (no hot-path flush); Mobile `buildDueOutboxLanes`; Desktop Tauri SQLite never drops unacked |
| C3 | mark_read observed seq | **done** — `MarkConversationReadRequest.read_up_to_message_seq`; Desktop/Mobile pass observed watermark |
| C4 | Subscribe desired/pending/confirmed | **done** — Mobile `SubscriptionManager` tri-state; SubscribeAck handled; failed send re-desires |
| C5 | Hub seq SSOT + host anchors | **done** — Desktop Hub `messageSeq` social order; host cards `anchorHubMessageSeq`+`suborder`; split Hub/Host page cursors |
| C6 | First/retry same intent | **done** — shared `resolveDispatchTargets` + `fanOutAgentTurns`; validate before append |

### Residual (out of this remediation slice)

- Web is not a formal IM client
- Full membership SM: invite, dissolve, ownership transfer UI, visibility epoch clients
- Product gaps in the review table (search, E2EE, large-group sharding, etc.)

### Follow-on (2026-08-09 continued)

| Item | Status |
|------|--------|
| Desktop IM outbox → Tauri SQLite | **done** — `im_outbox.sqlite3` + fail-closed `replace_all`; localStorage v1 one-shot migrate |
| Manager lifecycle lag reconcile | **done** — capacity 512; Lagged → manager snapshot + SQLite active-row reconcile + completion redrive |
