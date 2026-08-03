# Next Track: B6 / C5 / C6（终态规范）

| Field | Value |
|-------|--------|
| Status | **Normative execution SSOT** for remaining client/backend IM reliability |
| Date | 2026-08-03 |
| Program | [README](README.md) · [TASKS](TASKS.md) |
| Rule | [AGENTS.md Final-Architecture Planning Rule](../../../../Agents.md) — 只落地终态结构，禁止短期补丁 |
| Depends on | B1–B5 / C1–C4 **APPROVE** |

> 本文吸收 2026-08-03 深挖评估：问题定位、终态契约、微信级矩阵、状态管理债务。实现按 **B6 → C5 → C6** 串行闭环（实现 → review → 打回 → APPROVE）。

---

## 0. 现状结论（已复核）

| 轨 | 完成度（约） | 硬缺口 |
|----|--------------|--------|
| 消息收发 / 未读 / 重连水位 | ~90% | 局部 drift（见 §4） |
| **B6** Reaction 服务端契约 | ~40% | `event_id` 末尾 `Uuid::new_v4()` 破坏幂等 |
| **C5** Intent Outbox | Desktop reaction ~20%；Mobile reaction ~0%；Approval ~0% | outbox kind 半装载；approval 丢 `client_request_id` |
| **C6** 连接生命周期 | Mobile ~80%；Desktop ~30% | Desktop 无 visibility/sleep；Mobile worker 冷启动 gap |
| 整体相对「微信级」 | **架构对齐，实现约 75%** | 缺 reaction 可靠写 + Desktop 休眠恢复 + approval 持久化 |

---

## 1. B6 — Reaction 契约（后端）

### 1.1 问题

`ensure_reaction_delivery_in_tx` 的 `event_id` 若含 **随机 UUID**：

- 同一逻辑 op 的 retry / outbox re-publish → **新 event_id**
- WS seen-set / `push_dispatch_log` **无法幂等**
- 客户端无法把 durable 帧关联回 outbox `client_op_id`

正解不是「用 UUID 区分并发 same-emoji」——那是把 **不同 op** 与 **同 op 重试** 混为一谈。  
**不同 op** 用 **client_op_id** 区分；**同 op 重试** 用 **同一 event_id** 让 `ensure_one_in_tx` 自然 drop。

### 1.2 终态 event_id

```text
social-reaction-{conversation_id}-{message_id}-{emoji}-{actor_key}-{action}-{client_op_id}
```

| 规则 | 说明 |
|------|------|
| **禁止** `Uuid::new_v4()` 进 event_id | — |
| **禁止** `at_ms` 进 event_id | 同一逻辑 op 时间戳不稳定 |
| **必须** 请求体携带 `client_op_id` | 与 C5 outbox entry id 同一值 |
| 幂等 | `ensure_one_in_tx` 同 event_id 第二次 no-op |

### 1.3 Fanout（B6.2）

- **锁定 conversation-only**（现有测试 `reaction_delivery_is_conversation_only`）。
- **不改 fanout 代码**，只更新 `architecture-messaging.md`：reaction **不** 驱动 account topic / sidebar unread。
- 若未来要 inbox 反应提示：另开 PR 做完整 `AccountConversationReactionUpdated` 摘要事件，禁止半拉。

### 1.4 改动清单

| 区域 | 文件（示意） |
|------|----------------|
| event_id 公式 | `store/social/delivery.rs` |
| toggle API 入参 | `http/v1/conversations.rs`、`conversations/use_case.rs`、protocol 请求体 `client_op_id` |
| 测试 | delivery / reaction 幂等：同 client_op_id 不双 durable；不同 client_op_id 可并发 |
| 文档 | `architecture-messaging.md` §3.4.5 fanout 写死 |

### 1.5 验收

- [x] 同 `client_op_id` 双 POST → 单 durable 行 / 单 outbox 消费语义  
- [x] 不同 `client_op_id` same emoji toggle → 两条逻辑事件（可序列化正确聚合）  
- [x] 文档与测试锁定 conversation-only  

---

## 2. C5 — Intent Outbox（客户端）

**与 B6 契约：**  
`outbox.entry.id` = `client_op_id` = 出现在 B6 `event_id` 末段。

### 2.1 C5.1 Desktop reaction outbox

**现状：** `reaction-store` 乐观 UI + 直连 Hub；`listDuePending` **排除** `reaction_toggle`；flush 为 no-op/skip。

**终态：**

1. `toggleReaction`：生成 `client_op_id` → **enqueue** `reaction_toggle` → 乐观 UI → worker flush  
2. 去掉 `listDuePending` 对 `reaction_toggle` 的排除  
3. `im-cloud-sync` / worker：`reaction_toggle` 真正 POST（body 含 `client_op_id`）  
4. 入站 `ConversationMessageReactionUpdated`：继续 generation-gated apply；与确定性 event_id / op 对齐  

**失败语义：** 网络 transient 不 terminal；业务 4xx 可 terminal；重试用同一 `client_op_id`。

### 2.2 C5.2 Mobile reaction（较大）

**现状：** 无 UI / 无 toggle / 无入站聚合；worker 非 user_message early-return。

**终态：**

- `ConversationMessageRow`：reaction strip + emoji picker  
- Repository：`enqueueReactionToggle` → `im_outbox`  
- Worker：`_flushOne` 处理 `reaction_toggle`  
- 入站：读 `reactions` 更新本地聚合  
- SQLite：`cached_social_messages.reactions_json`（migration 升版）  

### 2.3 C5.3 Approval IntentOutbox

**现状：** 后端 `respond` **丢弃** `client_request_id`；端上直接 RPC。

**终态：**

- **后端：** `approval_requests`（或旁表）幂等键 = `client_request_id`；resolve 前命中则返回已有结果  
- **Desktop（及后续 Mobile）：** `kind=approval_resolve` 入 outbox；daemon 不可达时不丢意图  

### 2.4 验收

- [x] 离线 reaction → 上线单次生效（Desktop） — 代码路径：outbox + 同 client_op_id 重试；手工矩阵见 C6.5  
- [x] Mobile 可见 Desktop reaction（UI + 入站） — 代码路径：reactions UI + reaction_updated 帧  
- [x] 审批在 daemon 短暂不可达后仍可送达（幂等） — Desktop `approval_resolve` outbox + 后端 client_request_id  

---

## 3. C6 — 连接生命周期 + 收口

### 3.1 C6.1 Desktop visibility / sleep（最大可靠性缺口）

**现状：** 无 `visibilitychange` / online / offline / Tauri window 处理；休眠后 TCP 静默死亡，依赖 ping 超时。

**终态：**

| 组件 | 行为 |
|------|------|
| `HubRealtimeSession` | 暴露 `forceReconnect()`：`attempt=0`，关当前 WS，立即 `connect` |
| `im-hub-bridge` | 注册：Tauri focus/blur、`visibilitychange`、`window.online` |
| **隐藏** | **不主动关 WS**（让 TCP 自然超时）；**暂停 ping** 降噪 |
| **显示 / online** | 若 state ≠ `live` → `forceReconnect()` |

禁止：仅靠 25s ping 当唯一活性手段而无前台强制恢复。

### 3.2 C6.2 Mobile Connected edge drain

**现状：** worker 在 Connected 边沿 `flush()`，但依赖 ConversationsController `ensureStarted`；冷启动不进 Messages tab → worker 可能永不启动。

**终态：**

- App 根：`watch(imOutboxBootstrapProvider)` **或** `connectionStateProvider` Connected 边沿启动 worker  
- 删除「死 Provider、零消费者」状态  

### 3.3 C6.3–C6.5

- 客户端 Delete list 清零  
- Client Success Definition 9 条联调  
- 与 Backend B7：多端发送、@agent 连发、在线不推、Snapshot、**离线 reaction**、**休眠恢复**  

---

## 4. 状态管理债务（C5/C6 同轨或紧随 follow-up）

### 4.1 Desktop

| 问题 | 终态 |
|------|------|
| 双轨未读：`readMessageCountById` vs Hub digest | **Hub digest 单轨**；local baseline 仅 daemon-only / 未登录 fallback |
| `unsubscribeConversation` 无调用 | 订阅集 **LRU**（最近 N 个 conversation）；reconnect 不全量重订历史打开集 |

### 4.2 Mobile

| 问题 | 终态 |
|------|------|
| `socialUnreadCountProvider` 零消费者 | 底栏 Messages **badge** `ref.watch` |
| `imOutboxBootstrapProvider` 零消费者 | 见 C6.2 根启动 |

---

## 5. 微信级体验矩阵（目标，非自夸现状）

| 体验 | 目标 | 现状（约） |
|------|------|------------|
| 对话页新消息即时 | ✅ | Desktop/Mobile 已较强 |
| 非对话页 unread++ | ✅ | 已 patch |
| 进入清零 + 多端 last_read | ✅ | debounced mark-read 已有 |
| 断网重连不丢 | ✅ | resume + Snapshot range |
| 发消息可靠 | ✅ | outbox + client_message_id |
| Reaction 可靠 | ✅ 终态 | **未接通** |
| 休眠恢复 | ✅ 终态 | **Desktop 硬伤** |
| 审批意图持久 | ✅ 终态 | **0%** |
| 底栏未读 badge | ✅ | Mobile provider 未挂 UI |

---

## 6. 串行执行顺序

```
B6  (backend event_id + docs fanout)
  → review loop
C5  (Desktop reaction outbox → Mobile reaction → Approval)
  → review loop   // C5.1 依赖 B6 client_op_id 协议字段
C6  (Desktop visibility → Mobile bootstrap → 清扫与联调)
  → review loop
可选同批：§4 双轨未读 / LRU / badge（或 C6.3 清扫项）
B7 + G2–G4
```

**冲突热点（串行）：** `im-outbox.ts`、`im-cloud-sync.ts`、`delivery.rs`、protocol SendReaction、`social_providers.dart`。

---

## 7. 明确不做（本三轨）

- 百万群 reaction account 风暴  
- E2EE  
- 短期「先 toast 再还债」  
- 保留 UUID event_id「兼容旧行」（latest-only：改公式 + 测试）  

---

## 8. Success（三轨合完才算）

1. Reaction：离线 toggle → 上线 **恰好一次** 服务端效果；多端聚合一致。  
2. Approval：daemon 抖动不丢用户已确认意图。  
3. Desktop 休眠/回前台：分钟级内恢复 live，无需手杀进程。  
4. Mobile 冷启动：不进 Messages 也会 drain outbox。  
5. B6 event_id 可 grep 无 `Uuid::new_v4` 于 reaction delivery 路径。  
6. TASKS B6/C5/C6 与代码一致；无半装载 kind 进 due 却 throw。  
