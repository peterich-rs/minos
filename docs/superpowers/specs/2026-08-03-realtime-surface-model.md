# Realtime Surface Model（全局实时面模型）

| Field | Value |
|-------|--------|
| Status | **Normative target**（2026-08-03） |
| Scope | 全产品：哪些状态走 Durable / Stream / 信令+HTTP / 纯 HTTP；带宽优先级；新增功能设计 checklist |
| Related | [architecture-messaging.md](../../architecture-messaging.md)；[IM Reliability Program](2026-08-03-im-reliability-program/README.md) |
| Rule | [AGENTS.md Final-Architecture Planning Rule](../../../Agents.md) — 禁止 case-by-case 补丁式「这个列表也加个 WS」 |

> **一句话**：实时不是「所有 HTTP 写都推完整 payload」。按 **变更类** 选通道与 payload 厚度；**常驻只订 account + 当前打开的少量 topic**；低优先级用 **信令（hint）+ HTTP 冷读**。

Host 设备列表「只能手动刷新」只是 **类 R（名册成员变更）缺 Durable 发射** 的一个实例，不是单独 case。

---

## 1. 问题陈述（全局）

今日实现出现三种病态：

| 病态 | 表现 | 例 |
|------|------|-----|
| **协议死枚举** | `DurableEvent` 有变体，业务写库从不 enqueue | `HostLinked` / `HostUnlinked`；`ProjectArchived` 等需审计 |
| **通道半接** | 后端发了，客户端 unhandled 丢掉 | Mobile `session.rs` 仅 message/reaction/approval |
| **错层厚 payload** | 未打开会话的 account 帧携带完整消息体 | `AccountConversationMessageAppended.message: ChatMessageSummary` |

不能用「每个 UI 列表加一个 WS 订阅」解决。要统一：

1. **变更分类**（什么必须实时）  
2. **通道分级**（Durable / Stream / Hint+HTTP / 纯 HTTP）  
3. **订阅拓扑**（account 常驻 + conversation 按需）  
4. **payload 厚度**（full / digest / invalidation token）  

---

## 2. 订阅拓扑（已实现骨架，冻结）

```
常驻（每连接）：
  account:{accountId}     ← 名册 / inbox meta / 账号级 durable
  （host 连接另有 host 默认 topic）

按需（UI 打开时）：
  conversation:{id}       ← 打开中的会话全文 live（Desktop 主用）
  agent_session:{id}      ← 打开中的执行 transcript / approval 热路径

禁止作为默认：
  为 inbox 上每个 conversation 订阅 conversation:{id} 全文
```

| 端 | 今日倾向 |
|----|----------|
| Desktop | account 常驻 + **当前打开** conversation |
| Mobile | account 常驻 + 按需 agent_session；会话全文多靠 HTTP + account 帧 |

**Topic 上限（128 / 批 32）**：在上述拓扑下设备列表与 inbox **不是**瓶颈。  
真正风险：错误地「打开过的会话永不 unsubscribe」→ LRU（见 IM Reliability residual）。

---

## 3. 通道分级（Realtime Tier）

| Tier | 通道 | 语义 | 带宽 | 可靠性 | 典型用途 |
|------|------|------|------|--------|----------|
| **T0 Critical live** | Stream（可丢） | 毫秒感 | 高（delta） | at-most-once | agent 流式 token、typing、presence 心跳类 |
| **T1 Durable full** | Durable + Outbox | 有序、可 resume | 中–高 | at-least-once + event_id | **打开中**会话的 message append/recall/reaction 全文 |
| **T2 Durable digest** | Durable + Outbox | 有序、可 resume | **低** | at-least-once | inbox：未读 delta、preview、last_at、名册增删、好友请求 meta |
| **T3 Invalidate hint** | Durable **或** Stream 短帧 | 「某资源脏了」 | **极低** | 最好 durable；stream 可补 HTTP | 列表需刷新但不值得推 body：agent 配置改、项目归档、次要 roster |
| **T4 HTTP only** | REST | 用户主动 / 低频 | 0 on WS | 请求-响应 | 搜索、设置页打开、历史分页、profile 编辑后的本机确认 |

### 3.1 选型规则（新增功能强制）

写任何「多端要看见的状态变更」时，**必须**先选 Tier，再写 API：

```
Q1: 用户没打开任何相关页，是否仍必须秒级知道？
  否 → T4 或打开页时 HTTP 拉
  是 → Q2

Q2: 未读/角标/成员是否/会话排序是否依赖？
  是 → T2 digest on account（或 session 的 attention digest）
  否 → Q3

Q3: 是否在「当前打开的 conversation/session」内？
  是 → T1 full 或 T0 stream（流式过程）
  否 → T2 或 T3

Q4: payload 是否 > ~1KB 且接收方未打开详情？
  是 → 禁止 T1 full；降为 T2 digest 或 T3 hint + HTTP
```

**禁止**：未打开会话却推完整 `ChatMessageSummary` 当唯一方案（演进方向：T2 thin；今日可暂用 full 但标记 tech debt）。

**禁止**：仅 HTTP 写库、多端依赖「用户下拉 / 重进页」当产品语义（除非 T4 明确接受）。

---

## 4. 变更类矩阵（全局审计，非 case 列表终点）

对每类变更：**应选 Tier**、**今日状态**、**缺口**。

### 4.1 协作消息（IM 主轴）— Reliability 已覆盖大半

| 变更 | 应选 | 今日 | 缺口 |
|------|------|------|------|
| 打开中会话新消息 | T1 full on `conversation:` | ✅ | — |
| 未打开会话新消息 | T2 digest on `account:` | ⚠️ full body on account | 演进 thin digest |
| 撤回 | T1 + T2 | ✅ | — |
| Reaction | T1 conversation only | ✅（B6） | 故意不进 account（风暴） |
| 已读 | HTTP + 可选 debounced；他人已读若产品要 | 本地 mark-read | 非必须多端实时 |

### 4.2 Agent / Approval

| 变更 | 应选 | 今日 | 缺口 |
|------|------|------|------|
| 流式 token | T0 stream `agent_session:` | ✅ ingest | 仅订阅打开的 session |
| Session 起止 | T1 durable `agent_session:` **或** T2 account attention | 部分 durable | 客户端是否全消费需审计 |
| Approval 请求/解决 | T1 session + Push T2 | ✅ durable + C5 | account 级 attention 是否够 |
| Agent 最终气泡 | T1 conversation + T2 account | ✅ projector / host_projection | — |

### 4.3 名册 / 社交图（Host 列表是此类）

| 变更 | 应选 | 今日 | 缺口 |
|------|------|------|------|
| Host 配对 / 解绑 | **T2** `HostLinked`/`HostUnlinked` on account | ❌ 只写 DB | **发射 + 客户端 arm** |
| Host 在线 | **T0** presence stream | ✅ | 不能代替成员增删 |
| 好友请求 / 接受 | **T2** account digest 或 T3 invalidate | ❌ 多靠 HTTP+refresh | 协议若无则 **先补枚举再发射**（禁止 silent HTTP） |
| 好友删除 | T2/T3 | 待审计 | 同上 |
| Agent 注册/改配置 | T3 invalidate `agents` 或 T2 | 多 HTTP | 低频可用 T3/T4 |
| 群成员加减 | T2 conversation+account 或 T3 | 待审计 | 影响 ACL 时须 durable |

### 4.4 项目 / 工作区

| 变更 | 应选 | 今日 | 缺口 |
|------|------|------|------|
| ProjectConversationLinked | T2/T3 | 枚举有，发射待审计 | 可能死枚举 |
| ProjectArchived | T3 | 同上 | 打开项目列表时 HTTP 也可接受（T4）若产品接受延迟 |
| Host 工作区扫描结果 | T4 或 T3 | 本地 daemon | 非多端 SSOT |

### 4.5 账号 / 安全

| 变更 | 应选 | 今日 | 缺口 |
|------|------|------|------|
| 密码修改 | T2/T3 Force re-auth | 枚举有 | 须强制下线时 T1 host force close 类 |
| HostForceClose | T1 host topic | 有 | 安全关键 |

---

## 5. Payload 厚度（与带宽优先级）

### 5.1 三级 payload

| 厚度 | 内容 | 用在 |
|------|------|------|
| **Full** | 完整领域对象（消息全文、reaction 全聚合） | 仅 **已订阅的 conversation/session** |
| **Digest** | id + preview + counters + flags | account inbox / 名册行 / 好友请求摘要 |
| **Hint** | `resource_kind` + `resource_id` + `revision`/`at_ms` | 「去 HTTP 拉」；禁止塞 body |

### 5.2 优先级（拥塞时）

实现层可逐步做，原则先定：

| 优先级 | 内容 | 策略 |
|--------|------|------|
| P0 | 打开中 conversation 的 T1；approval；force close | 不降级、不合并丢弃有序性 |
| P1 | account T2 digest（未读/preview/名册） | 可合并同一 conversation 的连续 digest（coalesce） |
| P2 | T0 流式（非当前聚焦 session 可丢弃/降采样） | 未订阅 session 本就不推 |
| P3 | T3 hint | 可合并；客户端 debounce HTTP |

**低优先级不要占 T1 full 带宽**：  
未打开会话的完整消息体 → 降为 digest 或 hint+HTTP（微信模型）。

### 5.3 「信令 + HTTP」何时用

适用 **T3 / 部分 T2 低频**：

```
服务端：事务写库 + 发 Hint durable（小）
客户端：收到 hint → 标记 dirty → 合并 debounce → GET 列表/详情
用户点进详情 → 若未 hydrate → 立即 HTTP（可与 hint 竞态，HTTP 为准）
```

**不要**用于：

- 打开中会话的聊天消息（体验要求 live）  
- 未读角标若 hint 延迟导致「有消息无红点」（应用 T2 digest 直接 +1）

Host 名册：`HostLinked` 可以是 **T2 digest 行**（带 display_name），不必 T3；解绑同理。  
Agent 模型列表更新：T3 + 打开 Agents 页 HTTP 足够。

---

## 6. 服务端写路径模板（新增功能强制）

任何多端可见写：

```
BEGIN
  校验 + 幂等键
  写业务表
  按 Tier 选择：
    T1/T2: durable_event_log + outbox_events（确定性 event_id）
    T3:    durable hint 或 outbox 小事件
    T4:    不写 durable
COMMIT
wake_outbox  // T1–T3
```

检查清单（PR 必答）：

- [ ] 变更类（矩阵 §4）  
- [ ] Tier T0–T4  
- [ ] Topic（account / conversation / agent_session / host）  
- [ ] Payload 厚度 full / digest / hint  
- [ ] 确定性 `event_id`  
- [ ] 哪些客户端订阅、哪些 arm 分支  
- [ ] 未打开 UI 时是否仍正确（角标/名册）  
- [ ] SnapshotRequired / resume 后如何校准  

**禁止**：只写 HTTP handler + 注释「客户端会 refresh」。

---

## 7. 客户端 Sync 模板

```
Connection
  └─ account:* 常驻 handler:
       Host* / AccountConversation* / Account* roster / T3 hint
  └─ conversation:{focused} 按需:
       ConversationMessage* / Reaction*
  └─ agent_session:{open} 按需:
       stream + approval durable

Store 分层:
  HostRosterStore     ← HostLinked/Unlinked + presence
  InboxDigestStore    ← account message digest（C4）
  TimelineStore       ← conversation full（C3）
  SessionTranscript   ← agent_session
```

规则：

- **presence 不得 insert 名册**（仅 update online）  
- **full message 不得只靠 account 帧灌进未打开 Timeline**（可丢弃 body，只 patch digest）  
- **hint** → dirty flag → debounce `GET`  
- **SnapshotRequired(account)** → hydrate hosts + conversations 列表  

---

## 8. 与现有程序的切割

| 程序 | 负责 |
|------|------|
| IM Reliability B1–B6 / C1–C6 | 消息/推送/outbox/reaction/连接生命周期 |
| **本模型 Track R0** | 全局矩阵审计 + 死枚举清单 |
| **Track R1** | 名册类：HostLinked/Unlinked + 客户端 arm（原「设备列表」） |
| **Track R2** | 社交图：好友请求等 T2/T3（若产品要实时） |
| **Track R3** | Account payload thin digest（breaking，带宽） |
| **Track R4** | Subscribe 批限 / LRU / hint coalesce（体验） |

不要把 R1–R4 拆成互不相关的 case PR；共用 §3–§7 模板。

---

## 9. Track R0 审计产出物（实现前先做）

脚本/人工表（建议进 `TASKS`）：

对每个 `DurableEvent` 变体：

| event | 有发射点？ | 客户端 arm？ | 实际 Tier | 应用 |
|-------|------------|--------------|-----------|------|
| HostLinked | ❌ | ❌ | 应 T2 | R1 |
| HostUnlinked | ❌ | ❌ | 应 T2 | R1 |
| ProjectArchived | ? | ? | 应 T3 | R0 查 |
| … | | | | |

对每个「多端可见 HTTP 写」：

| API | 有 durable？ | 客户端依赖 refresh？ |
|-----|--------------|----------------------|
| link_host | 否 | 是 → 违规 |
| friend request | ? | ? |
| … | | |

---

## 10. 演进：Account thin digest（R3，非阻塞 R1）

目标态（示意）：

```text
AccountConversationDigestUpdated {
  account_id, conversation_id, message_id,
  preview, at_ms, sender_label,
  unread_delta: i32,   // 或仅 signal + 客户端 +1
  mentioned: bool,
}
```

- 打开会话仍用 T1 full / HTTP  
- 与 C4 InboxSync **天然契合**（本就 patch preview/unread）  
- **latest-only breaking**：协议与客户端一次切齐  

R1 Host 事件 **不依赖** R3。

---

## 11. Success Definition（模型落地）

1. 任意「多端可见」写在矩阵中有 Tier，无「只靠下拉」。  
2. 协议无「永久死枚举」（有变体必有发射+消费或删除变体）。  
3. 默认订阅 topic 数 O(1 + 打开窗口)，不随会话总数增长。  
4. 未打开会话不强制收 full message body（R3 后强制；R3 前标记债务）。  
5. 新功能 PR 无 checklist 不合并。  

---

## 12. 对你问题的直接回答

| 问题 | 答案 |
|------|------|
| 是否只有 host 列表有问题？ | **否**。是一类「名册/图变更缺 T2」+ 一类「厚 account payload」+ 一类「客户端 unhandled」。好友/项目等需 R0 扫表确认。 |
| 新功能怎么设计实时？ | 走 §3 选型 + §6 写路径模板 + §7 客户端 arm，禁止 case 特判。 |
| 低优先级是否信令+HTTP？ | **是（T3）**。角标/未读/打开中聊天仍应用 T2/T1，不要全挤 hint。 |
| WS 带宽？ | 优先级 §5.2；减负靠 **订阅按需 + digest/hint**，不是多开通道。 |

---

## 13. 建议执行顺序

```
R0  矩阵审计表（1 次全局，产出死枚举 + HTTP 盲区清单）
R1  HostLinked/Unlinked 全链路（名册类样板实现）
R2  按 R0 结果批量补「名册/社交图」同类
R3  Account thin digest（带宽）
R4  LRU / subscribe 错误上浮 / hint coalesce
```

R1 作为 **样板**：证明「T2 account + 客户端 store patch」模板，后续好友/群成员复用同一模式，而不是再写一篇 host-only 设计。
