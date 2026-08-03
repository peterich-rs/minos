# IM Reliability：收口验收 + 全局后续规划

| Field | Value |
|-------|--------|
| Status | **Normative planning**（2026-08-03） |
| Parent | [README](README.md) · [TASKS](TASKS.md) |
| Related | [Realtime Surface Model](../2026-08-03-realtime-surface-model.md) · [next-track B6/C5/C6](next-track-b6-c5-c6.md) |
| Rule | Final-Architecture：验收轨 **证明** 已实现终态；不把「没扫过」当「做完」；新能力另开 program |

> **回答「之前没完成的是不是要一并完整规划」：是。**  
> 但要拆成三层，**禁止**把验收、产品债、新实时架构揉成一个无限 PR。

---

## 0. 总览：三层 backlog

```
┌─────────────────────────────────────────────────────────────────────┐
│ Layer V — Verification & Closeout（本 program 收口，几乎不写新架构）   │
│   V1 = B7 + G2（后端证明 + rg 门禁）                                  │
│   V2 = G3（CI/本机测试绿）                                            │
│   V3 = C6.4 + C6.5（客户端场景 + 多端联调）                             │
│   V4 = G4（README §3 DoD 全勾 + 证据索引）                             │
└─────────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────────┐
│ Layer P — Product residuals（已知债，可另开任务，不阻塞「实现轨完成」）  │
│   Hub digest 单轨未读 · 订阅 LRU · Desktop 审批走 Hub HTTP · TUI origin │
└─────────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────────┐
│ Layer R — Realtime Surface program（新能力/全局模型，独立于 IM 收口）   │
│   R0 审计矩阵 · R1 Host 名册 durable · R2 社交图 · R3 thin digest · R4  │
│   见 realtime-surface-model.md                                        │
└─────────────────────────────────────────────────────────────────────┘
```

| 层 | 与 B7/G2–G4 关系 | 完成判据 |
|----|------------------|----------|
| **V** | **就是** B7 + C6.4/5 + G2–G4 的统一编排 | Program README §3 全勾 |
| **P** | 不混进 V；TASKS Honest residuals 跟踪 | 单独 PR / 里程碑 |
| **R** | **不是** IM Reliability 尾项；独立 program | 实时面 Success Definition |

---

## 1. 现状：实现 vs 收口

### 1.1 实现轨（功能已落地，track review APPROVE）

| 轨 | 内容 |
|----|------|
| B1–B6 | Push、lanes、dispatch、completion、lifecycle、reaction event_id |
| C1–C6.3 | Outbox、agent id、timeline、inbox、intent、visibility/bootstrap/badge |
| G1 | 文档主路径不互斥撒谎 |

### 1.2 收口轨（Layer V — 2026-08-03 执行状态）

| 原代号 | 统一进 | 状态 | 证据 |
|--------|--------|------|------|
| B7.1 / B7.2 | **V1** | **PASS** | [EVIDENCE.md](EVIDENCE.md) V1 |
| G2 | **V1** | **PASS** | `scripts/im-reliability-gates.sh` |
| G3 | **V2** | **PASS**（backend 270 + Desktop 78 + Mobile 17） | EVIDENCE V2 |
| C6.4 | **V3** | **PARTIAL**（自动化升级；设备仍 runbook） | EVIDENCE V3 |
| C6.5 | **V3** | **NOT_RUN** live 多端（runbook） | EVIDENCE V3 |
| G4 | **V4** | **PARTIAL** DoD（未全勾） | EVIDENCE V4 + README §3 |
| R0–R4 | Layer R | **PASS**（R3 thin digest shipped） | realtime-surface-audit.md · EVIDENCE R3 |

### 1.3 为何必须「一并规划」

若不统一：

- B7 与 G2 重复扫、结论不一致  
- C6.4 手工场景与 G3 自动化脱节  
- G4 被提前勾选但无证据  
- 同时开 R1 Host 实时 → 验收基线一直在变  

**规划原则：**

1. **先 V 收口 IM Reliability**（基线冻结）  
2. **P 可穿插小 PR**（不改 V 证据语义）  
3. **R 在 V 后或明确并行分支**，不宣称「IM program 未收口却已全部完成」

---

## 2. Layer V — 收口验收详细计划

### 2.1 执行方式

- **一个 Verification agent**（或一人）按 V1→V2→V3→V4 顺序  
- 产出：本目录下 `EVIDENCE.md`（或 PR 描述固定章节）  
- Review：对照证据勾 TASKS / README，禁止无证据勾选  

### 2.2 V1 — Backend 证明 + rg 门禁（= B7 + G2）

**输入文档：**

- Backend Success Definition：`2026-08-03-backend-im-delivery-orchestration.md` §Success  
- Delete list：同文档 §Delete list + client delete list 交叉  

**B7.1 场景表（最低集）**

| # | 场景 | 证明方式 |
|---|------|----------|
| 1 | Outbox crash after publish → re-publish；push 不双成功 | 单测/集成 + push_dispatch_log |
| 2 | social / host_command 车道不互饿死 | 单测 claim isolation + 双 job |
| 3 | host_command 过期 dead_letter；observed 优先于 expire | outbox_events 单测 |
| 4 | 无 host 时 send 200 + dispatch pending；host online force-due | v1_social 等 |
| 5 | 同 session 两连发 → 两 agent 气泡；origin 公式 | completion + v1_social |
| 6 | SessionLifecycle 死 host；watch TTL | job 单测 |
| 7 | Reaction 同 client_op_id 幂等；conversation-only | delivery 单测 |
| 8 | Approval client_request_id 幂等 | approval_requests 单测 |
| 9 | UserOnline skip；event_id push 幂等 | notifications 单测 |

**B7.2 / G2 rg 门禁（脚本化，建议 `scripts/` 或 xtask）**

最低扫描（失败即门禁红）：

```text
# 禁止（生产路径）
client_message_id:\s*None          # Mobile send 硬编码（测试除外需白名单）
120_000|120000                     # body soft-dedupe 窗口（测试断言「不存在」除外）
COUNT-only stale_session           # SessionLifecycle 假 DidWork（确认 job 真终结）
callers should check presence      # 甩锅注释
Uuid::new_v4() in reaction event_id path
```

产出：命令 + 退出码 + 白名单文件列表。

**完成标准：** 场景表每行有「测名/日志路径」；rg 门禁可重复运行。

### 2.3 V2 — CI / 本机测试绿（= G3）

| 套件 | 命令（以仓库实际为准，执行时校正） |
|------|-------------------------------------|
| Backend | `cargo test -p minos-backend --lib` + 关键 integration tests |
| Desktop IM | im-outbox / hub-timeline / timeline-order / reactions 等 node test |
| Mobile | outbox + conversations_sort + 相关 unit（网络允许时） |

**完成标准：** 退出码 0；失败不得靠 skip 伪装。记录于 EVIDENCE。

### 2.4 V3 — 客户端场景 + 多端联调（= C6.4 + C6.5）

**C6.4 Client Success（9 条）** — 来自 client-im-sync-engine Success Definition：

| # | 不变量 | 证明 |
|---|--------|------|
| 1 | 发送 at-most-once 可见 / 幂等 | 断网重试 / 双 POST 同 client_message_id |
| 2 | 杀进程 / inflight reclaim | 冷启动 outbox drain |
| 3 | 热路径 O(1) inbox | 无每消息全量 conversations wipe |
| 4 | unread 后台 +1 / 聚焦清 / own 不涨 | 双端观察 |
| 5 | loadOlder + Snapshot range | 长会话 + snapshot_required |
| 6 | 排序 message_seq | 跨源不交错时钟 |
| 7 | agent 气泡同 id / 无 soft-dedupe | @agent 连发 |
| 8 | reaction 离线可送达 | Desktop/Mobile |
| 9 | Delete list 客户端侧清零 | 与 V1 rg 交叉 |

**C6.5 联调矩阵（多端）**

| 场景 | Desktop | Mobile | Backend 相关 |
|------|---------|--------|--------------|
| 互发文本 | ✅ | ✅ | B7.1#1 |
| @agent 连发两泡 | ✅ | ✅ | B7.1#5 |
| 在线不推 | 可观察 | 可观察 | B7.1#9 |
| SnapshotRequired | ✅ | ✅ | — |
| 离线 reaction | ✅ | ✅ | B7.1#7 |
| 休眠恢复 live | ✅ C6.1 | reconnect | — |
| 审批 intent | daemon outbox | Hub client_request_id | B7.1#8 |

无法自动化的：**手工 checklist + 录屏/日志** 仍算证据。

### 2.5 V4 — Program DoD 全勾（= G4）

仅当 V1–V3 证据齐全：

1. 更新 `README.md` §3 全部 checkbox  
2. 更新 `TASKS.md` B7 / C6.4 / C6.5 / G2–G4  
3. `EVIDENCE.md` 索引：场景 → 证据位置  
4. 全局 re-review（可选轻量）：无新增死枚举、无 TASKS 过勾  

**完成 = IM Reliability Program 正式收口。**

---

## 3. Layer P — Product residuals（完整列表，不混进 V）

| ID | 项 | 建议优先级 | 依赖 |
|----|-----|------------|------|
| P1 | Desktop Hub digest **单轨**未读（去掉/降级 `readMessageCountById` 双轨） | P1 体验 | 无 |
| P2 | Conversation 订阅 **LRU** + `unsubscribeConversation` | P2 | 无 |
| P3 | Desktop 审批 **Hub HTTP** + client_request_id（今日仅 daemon） | P2 | C5.3 residual |
| P4 | TUI `origin_message_id` 贯通 | P3 | collab 若走 TUI |
| P5 | APNs/FCM **真通道**（非 log stub） | 产品 | 运维 |
| P6 | Multi-instance CompletionWatch / presence 共享 | 规模 | 多副本部署 |

P 项 **单独 TASKS 区或 issue**；不阻挡 V4，除非产品强制。

---

## 4. Layer R — Realtime Surface（独立 program）

规范：[2026-08-03-realtime-surface-model.md](../2026-08-03-realtime-surface-model.md)

| 轨 | 内容 | 与 V 关系 |
|----|------|-----------|
| **R0** | DurableEvent × HTTP 写 审计矩阵 | 可与 V **并行**（只读审计） |
| **R1** | HostLinked/Unlinked 全链路（名册样板） | **V 后**或旁路分支；不改 V 证据语义 |
| **R2** | 好友/群成员等 T2/T3 按 R0 批量补 | 依赖 R0/R1 模板 |
| **R3** | Account thin digest（breaking） | **DONE** — audit §3 + EVIDENCE R3 |
| **R4** | Subscribe 批限 / hint coalesce | 体验 |

**规划原则：**  
R 是「全局实时面」；V 是「消息可靠性 program 收口」。  
Host 列表只是 R1 样例，**不**在 V 里 case-by-case 修完再假装 program 结束。

---

## 5. 推荐时间线（逻辑序，非人天）

```
现在
  │
  ├─► V1 (B7+G2) ──► V2 (G3) ──► V3 (C6.4+C6.5) ──► V4 (G4)  = IM Program 收口
  │
  ├─► R0 审计（可并行，只读）
  │
  └─► V4 之后（或明确 fork）:
        R1 Host 名册 → R2 图/成员 → R3 thin digest → R4 订阅打磨
        P1–P6 按产品优先级插入
```

**一个 Verification subagent** 跑完 V1–V4 足够（熟悉成本只付一次）。  
**不要** B7 / G2 / G3 / G4 四个 agent。

---

## 6. 交付物清单

| 产物 | 路径建议 |
|------|----------|
| 本规划 | `closeout-and-backlog.md`（本文） |
| 验收证据 | `EVIDENCE.md`（V 执行时创建） |
| rg 门禁 | `scripts/im-reliability-gates.sh` 或 xtask（V1） |
| R0 审计表 | `realtime-surface-audit.md`（R0） |
| TASKS 勾选 | 仅 V 完成后更新对应框 |

---

## 7. 对你问题的直接回答

| 问题 | 答案 |
|------|------|
| B7 / C6.4 / C6.5 / G2–G4 要不要一并规划？ | **要** → 统一为 **Layer V**，顺序 V1→V4 |
| 它们是不是新架构？ | **不是**；是证明 + 门禁 + 联调 |
| 和 Host 实时 / thin digest 一起做？ | **规划在同一 backlog 文档**；**执行上 V 与 R 分层**，避免验收基线漂移 |
| 何时算 IM Reliability 完？ | **V4**（DoD 全勾 + EVIDENCE） |
| 何时做设备列表实时？ | **R1**（V 后或旁路），按 Realtime Surface 模型，不 case-by-case |

---

## 8. 下一步（调度建议）

1. **开 Layer V**：单 agent 执行 V1→V4，产出 EVIDENCE + 勾选  
2. **并行可选**：R0 只读审计表  
3. **V4 后**：R1 Host 名册样板（全局实时面第一落地）  

说「开 Layer V」即按此收口；说「开 R0/R1」即做实时面，不与 V 混称「IM 还没做完的零散 checkbox」。
