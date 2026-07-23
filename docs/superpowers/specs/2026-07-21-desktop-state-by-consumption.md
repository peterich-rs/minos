# Desktop 状态管理规格：按消费范围拆分

> Status: **P0–P5 residual closed**（2026-07-22）  
> Date: 2026-07-21（修订 2026-07-22 residual）  
> Scope: `apps/desktop` 前端状态形状与刷新契约（不绑定具体 Zustand API）  
> Related: [architecture-desktop.md](../../architecture-desktop.md), [2026-07-18-desktop-product-experience.md](2026-07-18-desktop-product-experience.md)  
> **编码入口：** §18 落地顺序；**P0–P5 done**（含 store 拆分、livePush 断线、badge quiet hydrate、Entity 投影同步）  
> **Review：** [2026-07-22-desktop-state-p0-p4-review.md](../reviews/2026-07-22-desktop-state-p0-p4-review.md)

## 1. 目标与非目标

### 1.1 目标

1. 按 **UI 实际消费范围** 划分状态，而不是一个上帝 `WorkspaceState`。
2. 明确每个状态切片的：**字段、生产者、消费者、加载/刷新触发、与 daemon 的映射**。
3. **加载由消费者可见性驱动**（见 §2.2）：View 可见才 `ensureLoaded`；不可见则不发对应 RPC。导航只改 id，不打包「打开某屏必拉的数据袋」。
4. 跨面板共享的实体（尤其 Session 运行态）有 **单一写入点**，视图只做投影。
5. 保持现有产品不变量：
   - Conversation 主时间线 ≠ Session transcript
   - `needs_approval` 由 UI 从 pending approval 派生，daemon 不直接发该状态
   - 导航只改 id；资源由对应切片声明式加载
6. 逻辑层明确：**何时加载、驻留多久、如何淘汰、回切如何无感**（见 §21）。状态形状服务于消费范围；**缓存生命周期服务于体验与内存**。

### 1.2 非目标

- 本规格不要求立刻重构代码；作为拆分与实现的目标契约。
- 不改变 daemon JSON-RPC 形状（除非另开协议变更）。
- 不覆盖 Attention / Agents / Host 的完整产品设计（仅定义其状态边界）。
- 不引入与历史版本的兼容层。
- §21 的预算数字为 **v1 默认**，可按 profiling 调整；不要求磁盘级二级缓存（除非另开规格）。
- **不**把「同屏布局」误当成「同生命周期数据」：Timeline 与 Inspector 同属 Conversations 布局，但加载彼此独立。

---

## 2. 分层总览

```text
L0  NavigationState              指针 + chrome 可见性
L1  ConnectionState              启动 / daemon 连接 / livePush（订阅是否接通）
L2  ProjectIndexState            侧栏项目列表 + ProjectSummary
L3  View slices（按 key 的工作集缓存，CacheRuntime 管）
    L3a Conversations view
        · ConversationListState(projectId)
        · TimelineState(conversationId)
        · InspectorState(conversationId)
    L3b Sessions view
        · SessionListState(projectId)
        · TranscriptState(sessionId)
        · SessionSummaryView(sessionId)   // 可派生，非必须独立 store
    L3c Board view
        · BoardState(projectId)          // 可派生自 ConversationList + SessionEntity
L4  Shared entities
    · SessionEntity(sessionId)            // 轻量真相：status / hasPendingApproval（全局可写）
L5  LiveIngress（全局单例，薄）
    · 推送唯一入口 → 写 L4 + Dirty 标记 + 条件通知 L3
    · 不镜像全站 messages/transcript
L6  Use-cases                        跨切片写编排（sendMessage 等）
```

### 2.1 依赖方向

```text
                    ┌──────────────────────────┐
                    │  LiveIngress（全局薄源）   │
                    │  Entity + Dirty + notify │
                    └────────────┬─────────────┘
                                 │ 仅当 L3 entry 存在/可见
                                 ▼
Navigation ──► family key + 可见性 ──► View ensureLoaded ──► L3 窗口缓存
Connection ──► livePush / connected 门闸
Use-case ────► 跨 slice **写**（非首载打包）
View ────────► 可见时 ensure；只读自己的 slice + Entity 投影
```

禁止：

- View 直接改另一个 View 的私有缓存（应走 Use-case 或 LiveIngress）。
- Use-case / Navigation 在「选中 conversation」时**强制** hydrate 所有同屏可能用到的 slice（反模式：现状 `loadConversationDetail`）。
- LiveIngress 在无 L3 消费者时为每个 dirty conversation **无条件** `list_messages` / `loadConversationDetail`。
- 把推送层做成「全量 UI 数据镜像」（全局堆全部 messages / 全部 transcript）。

### 2.2 双层数据模型：全局 Ingress + 按 key 二次分发

> **推送是进程级事件源；project / conversation / session 是按 key 的工作集；View 只消费工作集。**

| 层 | 职责 | 存什么 | 不存什么 |
|----|------|--------|----------|
| **LiveIngress + Connection** | 单例订阅、livePush 语义、事件路由 | 连接态；**SessionEntity**；**Dirty 标记**（如 `timelineDirty[C]=lastSeq`） | messages 全文、transcript 全文、各 View 的 phase |
| **L3 CacheRuntime slices** | 按消费的窗口缓存 | Timeline / Inspector / List / Transcript 的 Entry | 不直接 listen daemon |
| **View** | 可见性 → ensureLoaded；渲染 | — | 不 setInterval（live 健康时）；不 listen |

**Ingress 处理三条通道（薄写）：**

```text
ingest 帧
  → 永远：SessionEntity.upsert(sessionId, { hasPendingApproval, lastSeq, … })；抬 needs_approval
  → 仅当 Transcript(S) **map key 已存在**（含 `[]`，见 hasTranscriptWorkingSet）：merge items（重窗）
  → 否则：丢弃 items 正文（禁止 `?? []` 新建 key）

manager 事件
  → 永远：SessionEntity 生命周期（不降级 needs_approval 规则见 §3.2 / §10）
  → 可选：若 SessionList/Inspector entry 存在，更新成员投影；不强制 re-list

conversation dirty { conversationId, messageSeq }
  → 永远：markDirty(Timeline, C) / 更新 lastKnownMessageSeq
  → 仅当 Timeline(C) entry 已存在 且 (可见 ∨ pin ∨ phase==ready 且仍在 cache)：
        scheduleQuietRevalidate(Timeline, C, debounce≈200ms)
  → 否则：只留 dirty；下次 View ensureLoaded(Timeline,C) 时拉尾并清 dirty
```

**Quiet re-list 门闸（拍板）：**

```text
shouldQuietRevalidate(kind, key) =
  entry exists
  && entry.phase ∈ { ready, error }   // 已有或曾有工作集
  && (entry.pinned || viewVisible(kind, key) || entry.data != null)
// 禁止：entry 不存在时因 dirty 创建 entry 并 RPC
```

### 2.3 首要原则：按消费加载（Consumption-driven load）

拆分状态的初衷不是「把一个大 store 切成多个文件」，而是：

> **谁在展示，谁负责加载；不展示，就不为它付 RPC / 内存预算。**  
> **全局只信薄推送（Entity + dirty）；大窗只为工作集付费。**

| 规则 | 含义 |
|------|------|
| **1. 消费者拥有 hydrate** | 每个 heavy slice 的**首载**由**正在消费它的 View**（mount 且可见）调用 `ensureLoaded`。 |
| **2. 导航只改指针** | `selectConversation` 等只更新 Navigation；**不**隐式 `Promise.all` 拉 Timeline+Inspector+… |
| **3. 同屏 ≠ 同绑定** | 中栏 Timeline 与右栏 Inspector 无数据依赖。两栏都开着 → 两个 View 各自 ensure → **自然并行**。右栏收起（`detailsOpen=false`）→ **不发** `list_sessions`。 |
| **4. phase 独立** | 每个 keyed slice 自有 `phase`；Timeline loading 不得拖住 Inspector，反之亦然。 |
| **5. 可选消费再加载** | 例如 Composer `@` 补全需要 session 列表时，在**打开补全面板时** `ensureLoaded(Inspector)`，而不是绑在 Timeline 首载上。 |
| **6. 跨切 use-case 只做「写」** | send / resume / mark-read / resolveApproval 等需要事务语义时才走 L6；**首载不属于 use-case 打包袋**。 |
| **7. Ingress 薄、Cache 重** | 推送层只写 Entity + dirty；messages/transcript 窗口只在 L3 entry 上 merge / re-list（§2.2）。 |
| **8. dirty 不创造消费者** | conversation dirty **不得**在无 Timeline entry 时发起 list_messages。 |

**反模式（须迁移删除）：**

```text
// BAD — 动作打包：打开对话永远双 RPC，与右栏是否可见无关
loadConversationDetail(C) {
  detailStatus = loading
  Promise.all([listMessages(C), listSessions(C)])
  commit both + one phase
}
```

**目标模式：**

```text
// GOOD — 消费驱动首载 + 薄 Ingress
TimelineView  visible && conversationId=C  →  ensureLoaded(Timeline, C)
InspectorView visible && conversationId=C  →  ensureLoaded(Inspector, C)
// detailsOpen=false → InspectorView 不挂载 / 不可见 → 无 listSessions

// GOOD — dirty 二次分发
onConversationDirty(C):
  markDirty(C)
  if hasTimelineEntry(C): quietRevalidate(Timeline, C)  // 仅 messages
  // 绝不 loadConversationDetail 双 RPC
```

---

## 3. 公共类型

### 3.1 ResourcePhase

| phase | 含义 |
|-------|------|
| `idle` | 尚未为该 key 发起加载 |
| `loading` | 首载中（允许空数据 + spinner） |
| `ready` | 至少成功完成一次 hydrate |
| `error` | 最近一次非 quiet 加载失败；保留 `errorMessage` |

每个 **keyed slice** 自带：

```text
phase: ResourcePhase
generation: number          // 防陈旧响应
errorMessage?: string
```

Quiet 刷新：`phase` 保持 `ready`，不闪 loading。

### 3.2 SessionStatus（UI）

```text
idle | running | needs_approval | suspended | failed | done
```

| 来源 | 规则 |
|------|------|
| daemon session label / manager | 提供除 `needs_approval` 外的生命周期 |
| pending approval signal | `true` → 强制 `needs_approval`；并写 **`SessionEntity.hasPendingApproval=true`** |
| manager `running`/`idle` | **不得**覆盖已有 `needs_approval` |
| pending `false`（高置信：resolve 成功或 list/ingest 明确无 pending） | `hasPendingApproval=false`；允许回到 daemon 状态 |
| pending 未知 + 曾为 needs_approval + daemon 仍 running | **保持** needs_approval |
| **Transcript 已 evict / 从未加载** | **禁止**因「扫不到 transcript」而降级；以 **`SessionEntity.hasPendingApproval` 为 fallback 真相**（最近一次 list/ingest/resolve 快照） |

`status === needs_approval` 的展示优先级：`hasPendingApproval` 或等价 pending 信号 > manager 的 running 文案。

### 3.3 标识

| 名 | 含义 |
|----|------|
| `projectId` | 项目 |
| `conversationId` | 协作对话 |
| `sessionId` | agent session / agent run（Minos 全栈主键；DB/RPC/ingest/UI 统一；**禁止**再用 `threadId`） |
| `selectedSessionId` | Navigation 焦点指针，值为某个 `sessionId`（可 null） |
| `messageSeq` | 协作消息序（Timeline 排序键） |
| `transcriptSeq` | session transcript 事件序 |

> **命名硬约定（2026-07 全栈清理）**  
> Minos 自有标识一律 `session_id` / `sessionId`。历史上的 `thread_id` / `threadId` / 表 `threads` 已移除。  
> 上游 Codex app-server 协议仍使用 `thread/start`、`threadId`——**仅**在 `minos-codex-protocol` 与 agent-runtime 的 Codex adapter 边界出现，并映射为 Minos `session_id`。  
> `provider_session_id` 是 CLI/runtime 侧会话 id（如 Codex jsonl 文件名），与 Minos `session_id` 不同，禁止合并。

---

## 4. L0 — NavigationState

### 4.1 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `primaryNav` | `work \| attention \| agents \| host` | 主导航 |
| `projectId` | `string` | 当前项目；空 = 未选 |
| `conversationId` | `string \| null` | 当前对话 |
| `selectedSessionId` | `string \| null` | 当前高亮的 agent session（Sessions / inspector）；值为 `sessionId` |
| `projectView` | `conversations \| sessions \| board` | Work 内 tab |
| `draftByConversationId` | `Map<conversationId, string>` | 输入草稿 |
| `lastConversationByProject` | `Map<projectId, conversationId>` | 恢复用 |
| `conversationListCollapsed` | `bool` | UI chrome |
| `sessionsListCollapsed` | `bool` | UI chrome |
| `detailsOpen` | `bool` | inspector 开合 |
| `commandPaletteOpen` | `bool` | ⌘K |

### 4.2 生产者 / 消费者

| 角色 | 内容 |
|------|------|
| **Producers** | 用户点击；命令面板；深链 `openSessionTranscript` / `openConversation`；冷启动恢复 |
| **Consumers** | 全部 shell 路由；各 View 根据指针 + chrome 决定是否 `ensureLoaded` |
| **Persist** | `projectId`, `conversationId`, `lastConversationByProject`（草稿可选 persist；`detailsOpen` 可选） |

### 4.3 转移规则

| 动作 | 副作用 |
|------|--------|
| `selectProject(P)` | `projectId=P`；`conversationId=last[P]??null`；`selectedSessionId=null`；`primaryNav=work`。**不**预拉 Timeline/Inspector。 |
| `selectConversation(C)` | `conversationId=C`；`selectedSessionId=null`；`projectView=conversations`；写入 `last[projectId]`。**仅改指针**；Timeline/Inspector 由各自 View 见 §2.2。 |
| `selectSession(S)` | `selectedSessionId=S` |
| `openSessionTranscript(S, C?)` | `projectView=sessions`；`selectedSessionId=S`；可选设 `conversationId` |
| `openConversation(C)` | `projectView=conversations`；`conversationId=C`；`selectedSessionId=null` |
| `setProjectView(v)` | 只改 tab；**不**清 selectedSessionId（Sessions↔Conversations keep-alive 需要） |
| `setDetailsOpen(bool)` | 只改 chrome；`false` 时 Inspector **停止消费**（不 ensure、不 pin 为 T1） |

**禁止：** 在 `selectConversation` / `openConversation` 内调用打包式 `loadConversationDetail`。

---

## 5. L1 — ConnectionState

### 5.1 字段

| 字段 | 说明 |
|------|------|
| `booting` | 启动页门闸 |
| `bootPhase` / `bootProgress` | 启动文案与进度 |
| `bootEpoch` | 每次成功 boot +1；触发各 slice 重新 init |
| `connection` | `{ connected, endpoint?, error?, source, managed }` |
| `livePush` | **推送订阅是否已接通**（非「agent 正在输出」）。true = LiveIngress 可收事件；false = 降级 poll（§15 P3） |
| `error` | **仅** boot/连接级错误 |
| `actionError` | 可选：全局动作错误条（或下沉到 use-case 结果） |

**`livePush` 语义：**

| 值 | 含义 | UI 策略 |
|----|------|---------|
| `true` | bootstrap 后 event bridge arm 成功 | **禁止**常态 interval 盲刷；靠 Ingress + 条件 quiet revalidate |
| `false` | 未 arm / arm 失败 / 非 Tauri / mock | 可见 slice 才允许 interval quiet ensureLoaded |
| （目标）断线 | pump 断开应把 `livePush→false` 或 `bootEpoch++` 重订 | 现状若未回落，属实现缺口，须补 |

### 5.2 生产者 / 消费者

| | |
|--|--|
| **Producers** | `bootstrap` / `reconnect`；Tauri connect 结果；LiveIngress bridge start/stop |
| **Consumers** | BootScreen；Sidebar host presence；Host 页；所有 load（`connected` 门闸）；poll 门闸读 `livePush` |

### 5.3 刷新

| 触发 | 行为 |
|------|------|
| App mount | single-flight `bootstrap` |
| Host Reconnect | 再 `connect`；重 arm bridge；成功则 `bootEpoch++` 并清空/stale 业务 slice |
| 连接边沿 | ConnectionToasts（冷启动首帧不 toast） |
| bridge 失败 | `livePush=false`；不假装有 live |

---

## 6. L2 — ProjectIndexState

### 6.1 字段

```text
ProjectIndexState {
  phase
  projects: ProjectSummary[]
}

ProjectSummary {
  id: string
  name: string
  workspacePath: string
  conversationCount: number        // 列表 ready 后以 list 长度为准
  unreadTotal: number              // 派生/回写
  approvalTotal: number            // 派生/回写
  runningAgentTotal: number        // 派生/回写
  needsAttention: number           // 建议 = f(unread, approval, …)
  hasUnread: bool
  lastAttentionMs: number
  updatedAtMs: number
  hostName?: string                // plane C；默认 This Mac
}
```

### 6.2 消费范围

| Consumer | 用到的字段 |
|----------|------------|
| Sidebar 项目列表 | name, needsAttention, hasUnread, updatedAtMs, hostName |
| Project header | name, hostName, conversationCount（list ready 后） |
| 排序 | attention-first + time |
| 空项目引导 | `projects.length === 0` |

**不包含：** conversation 详情、preview 全文、sessions、transcript。

### 6.3 生产者与刷新

| 触发 | 写入 |
|------|------|
| bootstrap `list_projects` | 骨架列表；聚合字段可先 0 |
| `create_project` | 重拉 index |
| ConversationList hydrate/quiet 成功 | **回写** 该 project 的 count/unread/approval/running/attention |
| SessionEntity 审批/running 变化（已加载 list 时） | 增量修正 approval/running 聚合 |
| `markConversationRead` | unreadTotal 下降 |

### 6.4 聚合权威

```text
权威输入：ConversationListItem 的 unread / approvalCount / runningCount
输出：ProjectSummary 聚合字段
禁止：用未加载 list 时的陈旧 project.conversationCount 掩盖空列表
```

### 6.5 红点 / needsAttention 产品承诺（拍板：方案 A + quiet 全项目 index）

侧栏 **Attention 入口 badge** 与 **各 project 行上的 needsAttention** 同源：

```text
project.needsAttention ≈ Σ (unread + approvalCount) over ConversationList(P) 中已加载行
Sidebar Attention badge = Σ project.needsAttention
```

| 承诺 | 说明 |
|------|------|
| **保证** | 对 **已知 project 索引**（bootstrap / `refreshProjects` / `createProject` 后）：经 **quiet** `loadConversations`（有界并发 3–4）覆盖全部 project，badge 使用 daemon ConversationList 的 `approvalCount` + unread 聚合，**不要求**打开 Attention 页 |
| **不保证** | 在 quiet ConversationList 完成前，或单 project list RPC 失败时，该 project 数字可偏小；**不**为 badge 常驻全站 SessionList / Attention 队列 |
| **不依赖** | Attention **详情列表** 是否打开；badge **不**由 `attentionSessions[]` 计算 |

更新路径：

```text
bootstrap / refreshProjects / createProject → quietHydrateAllConversationLists（全 known projects）
ConversationList hydrate/quiet → patchProjectAggregates（含 DTO approvalCount）
ingest / Entity 抬 needs_approval → 回写相关 conversation.approvalCount → 再聚合
markConversationRead → unread 降 → 再聚合
```

与 §14.2 分工：**红点 = 轻量 ConversationList 聚合**；**Attention 页列表 = 打开再拉重队列**（仍不常驻）。

---

## 7. L3a — Conversations 视图

### 7.1 ConversationListState `(key: projectId)`

#### 字段

```text
ConversationListState {
  projectId
  phase, generation, errorMessage?
  items: ConversationListItem[]
}

ConversationListItem {
  id: string
  projectId: string
  title: string
  preview: string
  updatedAtMs: number
  messageCount: number
  unreadCount: number
  priority?: high|medium|low
  progress: todo|in_progress|in_review|done
  branch?: string
  worktree?: string
  agentSessionCount: number
  runningCount: number
  approvalCount: number
  participatingAgents: string[]
}
```

Board 列 **不存死字段**；由 `progress + running/approval` 派生（见 §9）。

#### 消费范围

| Consumer | 字段 |
|----------|------|
| ConversationList 行 | title, preview, unread, priority, progress, time |
| WorkView auto-select | ids + phase |
| Board 卡片 | item + 派生 column |
| ProjectSummary 回写 | unread/approval/running/count |

#### 生产者 / RPC

| 操作 | RPC | 结果形态 |
|------|-----|----------|
| hydrate / quiet | `minos_local_list_conversations` | `ConversationListItem[]` |
| create | `minos_local_create_conversation` | 后 re-list |
| update meta | `minos_local_update_conversation` | patch item 或 re-list |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| `projectId` 变化 / `bootEpoch++` | 非 quiet hydrate |
| sendMessage 成功 | re-list 该 project |
| conversation meta 变更 | patch 或 re-list |
| Live：强依赖预览时 | 可选 debounce re-list；否则仅改 preview 字段若事件足够 |

---

### 7.2 TimelineState `(key: conversationId)`

#### 字段

```text
TimelineState {
  conversationId
  phase, generation, errorMessage?
  messages: TimelineMessage[]
  history: {
    hasOlder: bool
    loadingOlder: bool
    firstLoadedSeq: number | null
  }
}

TimelineMessage {
  id: string
  messageSeq?: number          // 服务端行必有；optimistic 可缺
  role: user|agent|system
  agent?: string
  sessionId?: string
  body: string
  createdAtMs?: number
  kind?: text|tool_summary|approval   // UI 展示用；禁止用 body 文本猜 approval
  replyToMessageId?: string
  delegationId?: string
  mentions?: { agent, sessionId?, sessionShortId? }[]
  pending?: bool               // 乐观气泡
}
```

#### 消费范围

| Consumer | |
|----------|--|
| Timeline 中栏 | messages + history + phase |
| stick-to-bottom / load-older | history + messageSeq |
| 引用条 | replyToMessageId |

**排除：** tool 流水、reasoning、完整审批 transcript（属 Transcript）。

#### 生产者 / RPC

| 操作 | RPC |
|------|-----|
| hydrate / quiet re-list | `minos_local_list_conversation_messages`（tail / before_seq） |
| append user（写路径） | `minos_local_append_user_message`（由 use-case 调） |

排序：`messageSeq ASC`（权威）；前端可二次 sort 防御。

**与 Inspector 的关系：** **无**。不共享 `phase`，不共享 hydrate 函数，不因「同屏」捆绑 RPC。

#### 加载 / 刷新触发（消费驱动）

| 触发 | 模式 |
|------|------|
| **Timeline 中栏可见** 且 `conversationId=C`（含选中 C、从其他 tab 回到 conversations 且中栏在展示） | `ensureLoaded(Timeline, C)` → hydrate tail |
| Live `conversation` dirty | Ingress 只 `markDirty`；**仅** `shouldQuietRevalidate(Timeline,C)` 时 debounce ~200ms quiet re-list messages（**保留 older**）；**不** list_sessions |
| sendMessage 成功 | 乐观 append → 成功后 quiet re-list（仅 Timeline slice） |
| 上翻 | `loadOlder(beforeSeq)` |
| `livePush=false` 且 Timeline 仍可见 | 降级 interval quiet re-list（仅 messages） |
| `bootEpoch++` | 可见则重新 ensure；不可见仅 mark stale；清 dirty |

**不**在「选中 conversation」的导航 handler 里加载 Timeline——由 **Timeline View** 在可见时调用。

可选副作用（仍非 Inspector 捆绑）：对话变为焦点且 Timeline 可见时，可用独立 use-case 做 mark-read / needs_continue resume（见 §12.2），**不**要求 Inspector 已 hydrate。

---

### 7.3 InspectorState `(key: conversationId)`

#### 字段

```text
InspectorState {
  conversationId
  phase, generation, errorMessage?   // 独立于 TimelineState.phase
  sessionIds: string[]               // 顺序：列表展示序
  // 行数据优先读 SessionEntity(sessionId)；此处可缓存投影快照
}
```

展示行推荐投影：

```text
InspectorRow = SessionEntity 的摘要视图 {
  sessionId, agent, shortId, status, model,
  parentId, summary, lastTsMs, needsContinue
}
```

#### 消费范围

| Consumer | 条件 |
|----------|------|
| SessionInspector 右栏 | **仅当** `detailsOpen=true` 且 conversations 布局展示右栏 |
| Composer `@` 补全 / 选 session 路由 | **仅当**用户打开补全 UI 时；此时才 `ensureLoaded(Inspector)`（可与右栏共用同一 key 缓存） |

**排除：** 完整 transcript items；Timeline 消息。

#### 生产者 / RPC

| 操作 | RPC |
|------|-----|
| hydrate / quiet | `minos_local_list_conversation_agent_sessions` |
| live | LiveIngress → SessionEntity；若 Inspector entry 存在则投影刷新 `sessionIds` 序可选 |

Hydrate 成功时 **upsert** 对应 `SessionEntity`（L4），但 Entity 也可由 SessionList / Attention / live 写入——Inspector 不是 Entity 的唯一来源。

#### 加载 / 刷新触发（消费驱动）

| 触发 | 模式 |
|------|------|
| **Inspector 右栏可见**（`detailsOpen && conversationId=C`） | `ensureLoaded(Inspector, C)` |
| 打开 `@` 补全且需要 session 列表 | 同上 ensure（可 hit 已有缓存） |
| 右栏收起 `detailsOpen=false` | **不** hydrate；不发起 list_sessions；已有缓存可留 T2 或按预算淘汰 |
| manager / ingest | 只更新 SessionEntity；Inspector UI 若挂载则重投影，**不**因此强制 re-list |
| 可选 quiet revalidate | 仅当 Inspector entry 仍 pin/可见且超 `revalidateAfterMs` |

**禁止：** 与 Timeline 共用 `detailStatus` / 单一 `loadConversationDetail`；禁止「选中对话 ⇒ 无条件 list_sessions」。

---

## 8. L3b — Sessions 视图

### 8.1 SessionListState `(key: projectId)`

#### 字段

```text
SessionListState {
  projectId
  phase, generation, errorMessage?
  sessionIds: string[]              // 权威序（或由 groups 派生）
  groups: SessionListGroup[]        // 可缓存投影；行数据读 SessionEntity
}

SessionListGroup {
  conversationId: string
  conversationTitle: string
  sessionIds: string[]              // top-level；subagent 经 Entity.parentId 挂树
}
```

行展示数据：**一律来自 SessionEntity**，禁止第三份 status 缓存。

#### 消费范围

| Consumer | |
|----------|--|
| Sessions 左栏 | groups + Entity 投影 + Navigation.selectedSessionId |
| 折叠态 | Navigation 或局部 UI state（非业务缓存） |

#### 生产者 / RPC（拍板）

| 操作 | RPC / 行为 |
|------|------------|
| **hydrate / quiet** | **唯一：** desktop `list_project_sessions(projectId)`（底层可对 conversation 聚合；对 Desktop 只暴露这一入口） |
| **禁止** | UI 层 `list_conversations` × 每个 `list_sessions` 双循环当主路径 |
| live | SessionEntity 更新；`sessionAdded` → 插入 id 或 quiet re-list **本 project** |
| hydrate 成功 | **upsert 全部**返回行到 SessionEntity |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| `projectView===sessions` 且可见 / projectId / bootEpoch | ensureLoaded |
| `livePush=false` 且 Sessions 可见 | P3 quiet poll |
| sessionAdded 且 list ready | 插入或 quiet re-list |

**遗留：** `projectSessions` 全局镜像 → **删除**（见 §16 / S5）；只保留 `SessionListState(projectId)` + Entity。

---

### 8.2 TranscriptState `(key: sessionId)`

#### 字段

```text
TranscriptState {
  sessionId
  phase, generation, errorMessage?
  items: TranscriptItem[]
  history: {
    hasOlder: bool
    loadingOlder: bool
    // next/from seq 游标按现实现
  }
}

TranscriptItem {
  id: string
  kind: user|assistant|tool|tool_result|tool_error|reasoning|status|error|approval|question|…
  role?: string
  text: string
  detail?: string
  title?: string
  tsMs: number
  seq: number
  messageId?: string
  requestId?: string
  approvalMethod?: string
  options?: { label, description? }[]
  approveResponse?: string
  declineResponse?: string
}
```

#### 消费范围

| Consumer | |
|----------|--|
| Sessions 中栏 | items + history |
| 审批 modal | kind=approval/question 且有 requestId |
| SessionSummary 派生 | edit tools / diff 统计 |

#### 生产者 / RPC

| 操作 | RPC / 事件 |
|------|------------|
| hydrate tail / older / full | `minos_local_read_session_raw_history` → bridge TranscriptAssembler |
| live | `daemon://ingest` → merge by id/seq |
| 审批回复 | resolve / opencode respond RPCs（use-case） |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| Navigation.selectedSessionId 变化且 Sessions 可见 | hydrate |
| ingest for sessionId | merge items；更新 pending approval → SessionEntity |
| load older | prepend + scroll restore |
| `onConversationFocused` elevate peek | **elevate-only** tail 读，可写 Transcript 缓存也可只用于 status（实现可选）；不绑定 Inspector 可见性 |

---

### 8.3 SessionSummaryView `(key: sessionId)`

非必须独立 store。

```text
输入：TranscriptState.items
输出：{ editedPaths, diffPlus, diffMinus, … }
策略：纯 selector；计算变重再 memo 缓存
```

Consumer：Sessions 右栏。

---

## 9. L3c — BoardState `(key: projectId)`

### 9.1 定义

Board **不是**独立任务系统：

```text
BoardState = 纯派生视图 over ConversationListState(projectId)
```

**不**单独 store 重数据；无单独 poll。

### 9.2 列派生（读 List 聚合字段）

| 列 | 规则 |
|----|------|
| `done` | `progress === done` 优先 |
| `needs_you` | **`approvalCount > 0`**（或等价「需用户」聚合）且 progress 仍可 in_progress；与现 `deriveBoardColumn` 一致 |
| `running` | `runningCount > 0` / in_progress 运行态 |
| `backlog` | todo 等默认 |

**拍板：** Board **读 ConversationListItem 的 `approvalCount` / `runningCount` / `progress`**，不直接扫全部 SessionEntity 当主路径。  
Entity 变化 → **回写** List 行聚合（hydrate/quiet list 或 Ingress 抬审批时 `patchLocalConversation`）→ Board 自然更新。

字段不落库为 column；拖拽写 `progress`（needs_you → in_progress）。

### 9.3 消费 / 刷新

| | |
|--|--|
| Consumer | ProjectBoard |
| 刷新 | 跟随 ConversationList（及聚合回写）；无单独 poll |

---

## 10. L4 — SessionEntity `(key: sessionId)`

跨面板共享核。**全局轻量真相**（与 L3 重窗分离）：LiveIngress **永远**可写 Entity；Transcript / List 窗可淘汰而不丢 `status` / `hasPendingApproval`。

### 10.1 字段

```text
SessionEntity {
  sessionId: string
  conversationId: string
  conversationTitle?: string
  agent: string
  shortId: string
  status: SessionStatus          // UI 派生后（含 needs_approval 抬升）
  daemonStatus: SessionStatus    // 可选：未抬升前 daemon 标签
  model: string
  parentId?: string
  summary: string
  messageCount: number
  firstTsMs: number
  lastTsMs: number
  needsContinue: bool
  hasPendingApproval: bool       // ★ transcript 淘汰后的审批 fallback 真相
  updatedAtMs: number            // 本地合并时间
}
```

### 10.2 写入点（唯一 API）

所有路径必须经 **`upsertSessionEntity` / `patchSessionEntity`**（名称实现自定），禁止在 UI 里散落 6 处 `.map` 改 status。

| 来源 | 行为 |
|------|------|
| `list_sessions` / **`list_project_sessions`** | upsert；status = derive(daemon, hasPendingApproval) |
| manager.sessionStateChanged | 更新生命周期；**不**降 needs_approval（§3.2） |
| manager.sessionClosed | status=done |
| manager.instanceCrashed | 影响集 suspended |
| manager.sessionAdded | 占位 Entity 或标 SessionList dirty |
| ingest | `hasPendingApproval` 从帧更新；true → status=needs_approval；**无 Transcript key 不写 items** |
| resolveApproval 成功 | 可 optimistic `hasPendingApproval=false`；以 quiet re-list / ingest 收敛 |
| resume/send | 可选 optimistic running |

**迁移步骤（编码顺序）：** ① 抽出读写 API ② 替换 mapSessionStatusInLists / loadTranscript / resolveApproval / apply* 等路径 ③ 删除 `projectSessions` 镜像双写。

### 10.3 消费者（只读投影）

- Inspector rows  
- SessionList rows  
- Attention 队列  
- ConversationListItem.runningCount / approvalCount（聚合时）  
- Timeline session 复用选择  

---

## 11. L5 — LiveIngress（全局薄事件源）

> 进程级单例。职责：**订阅 + 薄写 + 条件通知**。  
> **不是**全站 UI 数据镜像，也 **不是** 每个 conversation 一条连接。

### 11.1 定位

| 是 | 否 |
|----|-----|
| 全局唯一 daemon 推送入口 | 每个 View 各自 listen |
| 写 SessionEntity + Dirty | 无脑 append 全站 messages |
| 在 L3 entry 存在时 merge / 调度 quiet | dirty 时强制 `loadConversationDetail` |
| 表达 livePush 接通语义 | 表示「agent 正在打字」 |

### 11.2 输入事件与处理

| 通道 | 载荷 | **永远**做 | **仅当** L3 条件满足 |
|------|------|------------|----------------------|
| `daemon://ingest` | `{ sessionId, seq, agent, tsMs, items[], hasPendingApproval }` | `SessionEntity` upsert（含 `hasPendingApproval`；抬 `needs_approval`） | **仅当** `Transcript(S)` 工作集 key **已存在**（含空数组，表示已 ensure）→ merge items + trim；**禁止** `transcripts[id] ?? []` 隐式创建窗；无 key → **丢弃 items 正文** |
| `daemon://manager` | sessionStateChanged / closed / crashed / added… | 更新 `SessionEntity` 生命周期（§3.2：不覆盖 needs_approval） | Inspector/SessionList entry 存在 → 投影刷新；`sessionAdded` 可插 id 或标 SessionList dirty |
| `daemon://conversation` | `{ conversationId, messageSeq }` | `markDirty(Timeline, C)`，记录 `lastKnownMessageSeq` | **`shouldQuietRevalidate(Timeline,C)`** → debounce ~200ms **仅** `list_messages` quiet；可选 patch ConversationList preview **若** List entry 存在 |

### 11.3 Quiet revalidate 门闸（硬规则）

```text
shouldQuietRevalidate(kind, key):
  entry = cache.get(kind, key)
  if entry == null || entry.phase == idle: return false   // 不创造消费者
  if entry.data == null && entry.phase != ready: return false
  return entry.pinned || isViewVisible(kind, key) || entry.data != null
```

| 场景 | 行为 |
|------|------|
| 用户从未打开过 C，后台 agent 写了 chat_messages | 只 dirty；**零** list_messages |
| 用户打开过 C，Timeline 仍在 cache（含 keep-alive） | dirty → quiet re-list messages |
| Timeline 已被 LRU evict | 只 dirty；再次打开 ensure 时拉 tail |
| Transcript 已 evict，仍有 ingest | 只更新 Entity；不重建 transcript 窗 |

### 11.4 规则

1. UI 组件 **禁止** 直接 listen daemon 事件改业务缓存。  
2. Ingress 是推送唯一入口（bootstrap 挂一次）。  
3. `livePush=true`：**禁止**常态 interval 盲刷；仅 §11.3 条件 quiet 与 use-case 后对齐。  
4. `livePush=false`：可见 slice 才 P3 interval（§15）。  
5. conversation dirty 路径 **禁止** 调用打包式 `loadConversationDetail`（不得顺带 list_sessions）。  
6. Ingress **禁止** 为 dirty key 自动 `ensureLoaded` 创建 entry（那是 View 的职责）。  
7. 断线 / 订阅结束：应 `livePush=false` 或触发重连 + `bootEpoch`（实现须补齐）。

### 11.5 与「流式 UI」的关系

| UI | 路径 | 频率 |
|----|------|------|
| Session Transcript | ingest →（有 entry 时）merge | 有帧就推，无固定 Hz |
| Session status pill | manager / ingest → Entity | 状态变化才推 |
| Conversation Timeline | dirty →（有 entry 时）debounce quiet list_messages | 脏通知合并 ~200ms；**非** token 流 |
| 无消费者的 key | 仅 Entity / dirty | **不**拉正文 |

### 11.6 实现注记（Desktop，与代码对齐）

| 项 | 约定 |
|----|------|
| Transcript 工作集判定 | `hasTranscriptWorkingSet(map, sessionId)` ≡ `Object.hasOwn(map, sessionId)`（**含** `[]` 空数组 key） |
| `applyIngestEvent` | 无 key → **不写** `transcriptsBySession`；仅抬列表/`SessionEntity` 的 `needs_approval`（用帧上 `hasPendingApproval`） |
| 打开 session | `loadTranscript` / ensure 写入 key 后，后续 ingest 才 merge items |
| 全局订阅 | **保留** 进程级 `subscribe_ingest`；不改为 per-session listen（后台审批仍靠 Entity） |

### 11.7 两种 `?? []` 禁忌（勿混）

| 场景 | 风险 | 正确做法 |
|------|------|----------|
| **Zustand selector** 里 `s.map[id] ?? []` | 每次 getSnapshot **新数组引用** → 无限 re-render / max update depth | 模块级 `EMPTY_TRANSCRIPT` / `EMPTY_*` 常量 |
| **Ingest / set 写路径** 里 `prev = map[id] ?? []; map[id]=merge(prev,…)` | **无消费者也创建工作集** → 内存堆后台 session | `hasTranscriptWorkingSet` 门闸；无 key 不写 map |

Selector 规则与工作集规则**都要守**，解决的问题不同。

---

## 12. L6 — Use-cases（跨切片编排）

Use-case 可以**写**多个 slice；View 只调 use-case 做跨切事务，**不**把手写多 store 事务散落在组件里。

**边界：** Use-case **不负责**「屏幕上可能用到的数据的首载」。首载 = 可见 View → `ensureLoaded`（§2.2）。

### 12.1 `bootstrap`

```text
Connection.booting=true
connect → list_projects → list_clis(可选，属 Agents 域)
start event bridge → livePush
bootEpoch++
booting=false
// 不预拉 conversation list 以外的 Timeline / Inspector / Transcript
// ConversationList 由侧栏/Work 可见时 ensure
```

### 12.2 `onConversationFocused(conversationId)`（可选；**不是**双 hydrate）

当 Navigation 的 `conversationId` 变为 C，且产品需要「焦点副作用」时调用。  
**禁止**在此函数内 `Promise.all(Timeline, Inspector)`。

```text
// 仅跨切写 / 副作用；数据首载仍归 View
mark read → ConversationListItem.unread + ProjectSummary（若 List 已有该行）
if !quiet:
  // 需要 session 元数据时：可用已有 SessionEntity，或轻量 list_sessions 一次写入 Entity
  // 不得假设 Inspector View 已加载
  at most one top-level needsContinue → resume(session, autoContinue=true)
  for active sessions needing elevate:
    peek transcript tail (elevate-only) → SessionEntity.hasPendingApproval / status
```

> 迁移说明：删除 `loadConversationDetail`。Timeline/Inspector 分别实现 `hydrate`/`ensureLoaded`；组件层按可见性调用。

### 12.3 `sendMessage(conversationId, body)`

```text
parse @routing
// 若路由需要 session 列表且尚未加载：ensureLoaded(Inspector, C) 或 ensure SessionEntity 索引
Timeline optimistic user bubble
append_user_message
resolve session:
  #shortId match | reuse latest non-closed top-level same agent | start_agent_in_conversation(+ profile model/effort/instructions)
resume(session, false)
send_user_message(prompt)
Timeline quiet re-list          // 只动 Timeline
ConversationList re-list(project)
// Inspector：仅当其 entry 存在或右栏可见时 quiet revalidate；Entity 随 listSessions 或 live 更新
```

### 12.4 `selectProject(projectId)`（导航）

```text
Navigation.selectProject   // 只改指针
// ConversationList / SessionList 由对应 View 可见时 ensureLoaded
```

### 12.5 `resolveApproval` / opencode permission|question

```text
RPC
SessionEntity / Transcript 随后由 ingest 收敛
禁止仅本地清 approval 而不等确认（除非 RPC 成功且协议保证）
```

### 12.6 `retryFailedMessage`（与 send 共享解析）

与 `sendMessage` **必须共享** session 解析 / `startNewAgentSession`（禁止复制 150 行）；同样只 quiet re-list Timeline（及可选 List），**不**强制 Inspector hydrate。

### 12.7 mock 模式（`source === "mock"`）

| 规则 | |
|------|--|
| 不 arm LiveIngress；`livePush` 保持 false |
| **不**走 CacheRuntime 真 RPC；bootstrap 直接注入 fixture → 各 slice **伪 ready** |
| View ensureLoaded 对 mock：**no-op 或读已注入数据** |
| 禁止 mock 分支再实现一套与 daemon 不同的 status 语义 |

### 12.8 ReadReceipt（`readMessageCountById`）

| 规则 | |
|------|--|
| 归属 | **ReadReceiptState**（可挂在 Navigation 旁或独立小 slice）；**不是** ConversationList 业务缓存 |
| 用途 | unread = max(0, messageCount - baseline) |
| Persist | **允许** localStorage（与 §4 Navigation 类似） |
| bootEpoch | 不因 boot 清空用户已读基线；换设备可丢 |
| 禁止 | 把整份 messages 当 persist 真相 |

### 12.9 进程内 in-flight（替代 `window.__minos*`）

| 现状 | 目标 |
|------|------|
| `window.__minosResumedInterrupted` / `__minosResumeInFlight` / `__minosConvRefreshTimers` | **LiveIngress 或 Use-case 模块私有 Map**；禁止挂 `window` |
| 语义不变 | resume 去重、conversation dirty debounce 定时器 |

---

## 13. 消费矩阵（View → State）

| View | Navigation | Connection | ProjectIndex | ConvList | Timeline | Inspector | SessionList | Transcript | SessionEntity |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| BootScreen | | R | | | | | | | |
| Sidebar | R/W | R | R | | | | | | |
| WorkView shell | R/W | | R | R(phase) | | | | | |
| ConversationList | R/W | | | **ensure** | | | | | |
| Timeline 中栏 | R/W draft | R livePush | | R(meta) | **ensure** | | | | R(reuse) |
| SessionInspector 右栏 | R | | | | | **ensure iff detailsOpen** | | | R |
| Composer `@` 补全 | R | | | | | **ensure on open** | | | R |
| SessionsView list | R/W | | | | | | **ensure** | | R |
| Sessions transcript | R | | | | | | | **ensure** | R |
| Sessions summary | | | | | | | | R | |
| ProjectBoard | R/W | | | R | | | | | R(opt) |
| HostView | | R/W | | | | | | | |
| AttentionView | R/W | | | | | | | | R filter |
| AgentsView | | | | | | | | | | （独立 AgentsState，见 §14） |

**ensure** = 可见时 `ensureLoaded` 该列 slice。Timeline 与 Inspector **无**互为 ensure 的前置条件。

---

## 14. 邻接域（边界）

### 14.1 AgentsState（独立）

```text
clis[], profiles[], modelsByRuntime, phase
```

与 Work 主路径弱耦合；`sendMessage` start agent 时可 **读** 最新 profile，不写入 Work slice。

### 14.2 Attention（红点 vs 详情列表）

#### 14.2.1 侧栏红点（轻）

见 **§6.5**：`ProjectSummary.needsAttention` 聚合；**方案 A + quiet 全项目 ConversationList**——bootstrap 后 quiet hydrate 所有 known project（非全站 session scan）；Attention 队列仍打开再拉。

#### 14.2.2 Attention 页详情队列（重）

```text
AttentionQueueState {
  phase, generation
  // 跨 project 的 session 摘要行（打开页才 hydrate）
  items: SessionEntity 投影[]   // needs_approval | failed | suspended 等
}
```

| 规则 | |
|------|--|
| **何时 hydrate** | `primaryNav === attention` 且页可见 → `ensureLoaded(AttentionQueue)` |
| **如何拉** | 跨 project `list_project_sessions`（或等价聚合）后 filter；结果 **upsert SessionEntity** |
| **不常驻** | 离开 Attention 后队列可 LRU/T2 淘汰；**不影响** §6.5 红点数字 |
| **禁止** | 用「纯 filter 已懒加载 Entity」假装队列完整；未打开页时不保证队列在内存 |
| **禁止** | 用 `attentionSessions` 大数组驱动侧栏 badge |

#### 14.2.3 用户感知

| 行为 | 期望 |
|------|------|
| 在 Work 里用过的 project 出现审批 | 侧栏红点 / project 数字应升（live 或 list 回写） |
| 从未点过的 project 后台有审批 | quiet ConversationList 完成后应亮（DTO `approvalCount`）；在 quiet 完成前可暂为 0 |
| 点进 Attention | 拉全站 session 队列，列表尽量完整；可与红点范围不完全一致（列表可更全） |

> 仍 **不** 用常驻 Attention 队列或全站 `list_project_sessions` 驱动 badge。

### 14.3 Host 诊断

只读 ConnectionState；不进业务 slice。

---

## 15. 刷新优先级

```text
P0 View 可见 → ensureLoaded（首载 / bootEpoch）
P1 LiveIngress 薄写 Entity + 条件 merge / 条件 quiet（§11）
P2 Use-case 后 quiet 对齐（send 后 Timeline re-list 等）
P3 降级 poll —— 仅 livePush=false 且 slice 可见
```

同字段冲突：

```text
generation 更高的 hydrate 结果 > 旧 hydrate
live 增量 merge 到**已有** entry（id 稳定）
quiet re-list 不得丢已加载 older（Timeline/Transcript 契约）
无 entry 的 live 数据不得「创造」重窗
```

**反模式：** `livePush=true` 时 Timeline/Sessions 仍 `setInterval` 全量 quiet re-list（现状 6s/8s）。

---

## 16. 与现状 `workspace-store` 映射（迁移用）

| 现状字段 / API | 目标 |
|----------------|------|
| `booting/bootPhase/bootProgress/bootEpoch/livePush/connection/error` | ConnectionState |
| `projects` | ProjectIndexState |
| `conversations` + `conversationsStatusByProject` | ConversationListState(projectId) |
| `messagesByConversation` + `messageHistory*` | TimelineState |
| **`detailStatusByConversation`（单一 phase）** | **删除**；拆为 Timeline.phase + Inspector.phase |
| **`loadConversationDetail`（双 RPC 打包）** | **删除**；→ Timeline.ensureLoaded + Inspector.ensureLoaded（各自 View） |
| `sessionsByConversation` | InspectorState.sessionIds + SessionEntity upsert |
| `projectSessionsByProject` + status | SessionListState + SessionEntity |
| `projectSessions`（@deprecated 镜像） | → SessionListState；S6 删除 |
| `transcriptsBySession` + history + status | TranscriptState |
| `applyIngestEvent`（旧：无脑 merge 建窗） | **已落地：** 无 Transcript key 不建窗；有 key 才 merge；`hasPendingApproval` 仍抬 Entity |
| `applyManagerEvent` | LiveIngress → Entity（`applyManagerLifecycleToEntity` / `commitSessionEntity`） |
| **`applyConversationEvent`** | **已落地：** markDirty + 条件 quiet **仅** `loadTimeline`（list_messages）；无 entry 零 RPC |
| `sendMessage` / `retryFailedMessage` / `create*` / `resolve*` | Use-cases（写路径） |
| `ui-store`（含 `detailsOpen`） | NavigationState |
| `clis` / profiles RPC 缓存 | AgentsState |
| `attentionSessions` | AttentionQueue 打开时 hydrate；**不**驱动侧栏 badge（badge 用 project.needsAttention，§6.5） |
| `actionError` | Connection 或 per-use-case result |
| `readMessageCountById` | ReadReceipt / unread baseline（可 persist；非业务列表） |
| `window.__minos*` in-flight | Use-case / CacheRuntime 进程内 Map |

---

## 17. 不变量清单（验收）

1. 切换 project 不得显示上一 project 的 transcript（Navigation 清 selectedSessionId 或 Sessions key 隔离）。  
2. Timeline 排序仅 `messageSeq`。  
3. Timeline 不含 session tool 流水。  
4. `needs_approval` 仅由 pending approval 路径抬升；manager running 不降级。  
5. ConversationList ready 前，header 对话数不以陈旧 summary 伪装。  
6. 同一 `sessionId` 的 status 在 Inspector / Sessions / Attention 一致（同读 SessionEntity）。  
7. `livePush=true` 时无常态盲刷 list。  
8. Quiet re-list 保留 older 窗口。  
9. 导航 persist；业务列表/消息/transcript **不**作为真相持久化。  
10. View 不跨 slice 直接写缓存。  
11. **按消费加载：** `detailsOpen=false` 时选中对话 **不得** 发起 `list_sessions`（除非 `@` 补全等显式消费）。  
12. Timeline 与 Inspector **独立 phase**；一侧 error/loading 不得阻塞另一侧展示已有数据。  
13. `selectConversation` **不得**隐式双 hydrate。  
14. **Ingress 薄写：** conversation dirty 时无 Timeline entry → **零** list_messages。  
15. **条件 quiet：** 仅 entry 已存在（且可见/pin/有 data）才 quiet re-list。  
16. **livePush=true** 无 interval 盲刷；Transcript 不靠 poll（与现状 Sessions transcript 路径一致）。  
17. **Ingest 工作集门闸：** 无 `transcriptsBySession` key 时不得因 ingest 创建该 key；审批只靠 Entity/`hasPendingApproval`。  
18. **Selector 稳定空值：** Zustand selector 禁止 `?? []` / `?? {}` 临时对象；用模块级 `EMPTY_*`。  
19. **Attention 红点（方案 A）：** badge = Σ 已加载 project 的 `needsAttention`；不保证从未 hydrate ConversationList 的 project；详情列表打开再拉，不驱动 badge。  
20. **SessionList hydrate 唯一 RPC：** `list_project_sessions(projectId)`。  
21. **SessionEntity.hasPendingApproval** 为 transcript 不可用时的审批 fallback 真相。  
22. **Board** 读 List 的 `approvalCount`/`runningCount`；Entity 经回写聚合影响 Board。  
23. **无 `projectSessions` 镜像**（迁移期后删除）。  
24. **mock 伪 ready**；**ReadReceipt 可 persist**；**禁止 window.__minos* 业务 in-flight**。  
25. **ensureLoaded per-key single-flight**；Timeline/Transcript **hardMax trim**。

---

## 18. 编码落地顺序（可执行）

> S0 规格已定稿到可编码。下列顺序 **有依赖**：P0 先于 P1，Entity API 先于删镜像。

| 阶段 | 要改什么（人话） | 验收 | 状态 |
|------|------------------|------|------|
| **Done** | ingest 无 Transcript key 不建窗；`hasTranscriptWorkingSet` | 后台 session 不堆 items；审批仍可抬 | **已做** |
| **P0** | ① 删 Timeline 6s / Sessions list 8s **live 下** interval ② `applyConversationEvent` → markDirty + 条件 quiet **仅** `listMessages`（禁止 `loadConversationDetail`）③ livePush 断线尽量回落 | live 开着网络面板无周期双 RPC；脏对话仅 messages re-list | **已做**（含 residual：pump 结束 → `daemon://push-status` → `livePush=false`） |
| **P1** | 拆 `loadConversationDetail`：`loadTimeline` + `loadInspector`；独立 phase；View 按可见性调用；`detailsOpen` 门闸 | 收起右栏切对话 **无** list_sessions | **已做** |
| **P2** | `SessionEntity` map + `upsertSessionEntity`；`hasPendingApproval` fallback；替换多处 status 写入 | 同 session status 一致；evict transcript 不假降审批 | **已做**（`lib/session-entity.ts` + `sessionsById`；list 投影自 Entity） |
| **P3** | SessionList **只** `list_project_sessions`；Attention 打开 hydrate；删 `projectSessions` 镜像；Board 只读 List 聚合 | 无双写镜像；Attention 列表与红点分工正确 | **已做**（删 `projectSessions`；Attention upsert Entity；badge 用 quiet ConversationList 覆盖 known projects） |
| **P4** | ensureLoaded single-flight；hardMax trim；mock 伪 ready；ReadReceipt；`window.__minos*` → 模块 Map；retry 共享 send 解析 | 无重复 RPC 风暴；无 window 全局；mock 不走 daemon 路径 | **已做**（`desktop-inflight.ts` singleFlight + resume Sets；hardMax 500/2000；mock no-op loads；ReadReceipt 注释+persist；retry 已用 `startNewAgentSession` 提取，send 路径仍可再收敛） |
| **P5** | 反模式扫尾 + **多文件 store 拆分** + tsc/测试 + 文档 | 无 live 盲刷 / 无 `loadConversationDetail` / 无 `window.__minos*`；`import { useWorkspaceStore } from "@/store/workspace-store"` 不变 | **已做**（`store/workspace/*` L0–L6 模块 + 薄 `workspace-store.ts`） |

### 18.1 P0 编码检查清单（建议第一刀）

```text
[x] Timeline.tsx：livePush=true 时移除 setInterval quiet loadConversationDetail
[x] SessionsView.tsx：livePush=true 时移除 setInterval loadProjectSessions
[x] applyConversationEvent：debounce 后仅 listMessages + 写 Timeline；不 listSessions
[x] shouldQuietRevalidate(Timeline,C)：无 entry 则只 dirty、零 RPC
[x] pump 结束 / 订阅失败 → emit `daemon://push-status` `{live:false}`；前端 `livePush=false` 恢复降级 poll
```

### 18.2 P1 编码检查清单

```text
[x] loadTimeline(conversationId) / loadInspector(conversationId) 替代 loadConversationDetail
[x] detailStatusByConversation → timelineStatus + inspectorStatus（或 keyed phase）
[x] TimelineView mount → loadTimeline；Inspector 仅 detailsOpen
[x] 打开 @ 补全 → ensure Inspector
[x] 删除/停用打包函数
```

---

## 19. 术语表

| 术语 | 含义 |
|------|------|
| slice | 按 key 划分的状态单元（family） |
| hydrate | 针对 key 的首载/显式加载 |
| ensureLoaded | 可见消费者请求某 key 处于可用（含 SWR）；CacheRuntime 入口 |
| quiet | 不改 phase 为 loading 的后台对齐 |
| entity | 跨视图共享的身份状态 |
| projection | 从 entity/list 派生的只读视图模型 |
| use-case | 跨 slice **写**编排（非首载打包袋） |
| LiveIngress | daemon 推送唯一入口 |
| 消费驱动 | 仅当 View 可见（或显式子 UI 打开）才为该 slice 付 RPC |

---

## 20. 修订记录

| 日期 | 说明 |
|------|------|
| 2026-07-21 | 初稿：按消费范围定义 L0–L6 与刷新契约 |
| 2026-07-21 | 增补 §21：缓存生命周期、加载策略、内存预算、回切无感 UX |
| 2026-07-21 | 增补 §22：CacheRuntime = CacheBudget + ensureLoaded/evict 一体规格 |
| 2026-07-22 | 标识统一 `sessionId` / `selectedSessionId`（全栈 thread→session） |
| 2026-07-22 | **澄清按消费加载：** Timeline∥Inspector 独立；删 `openConversationDetail` 双 hydrate；`detailsOpen` 控制 Inspector 消费；反模式 `loadConversationDetail` |
| 2026-07-22 | **双层数据模型：** 全局薄 LiveIngress（Entity+dirty）+ 按 key 二次分发；quiet re-list 仅 entry 存在/可见；livePush 语义与禁盲刷 |
| 2026-07-22 | ingest：禁止无 Transcript 工作集时 `?? []` 创建窗；无 key 只抬 Entity/审批 |
| 2026-07-22 | §11.6–11.7：`hasTranscriptWorkingSet` 实现注记；两种 `?? []` 禁忌；§17 不变量 17–18；S3 落地进度 |
| 2026-07-22 | **Attention 红点方案 A：** §6.5 / §14.2——badge 仅保证已加载 project 工作集；详情队列打开 hydrate |
| 2026-07-22 | **编码定稿包：** A3/A4/A5/A6/B1/C* 写入 §3.2/§8.1/§9/§10/§12.7–12.9/§17/§18 可执行清单；准备开写 |
| 2026-07-22 | **P2–P4 落地：** SessionEntity/`sessionsById`；删 `projectSessions`；Attention upsert；single-flight；hardMax；module inflight Maps |
| 2026-07-22 | **P0+P1 落地：** 删 live 下 Timeline/Sessions interval；`applyConversationEvent` 条件 quiet `loadTimeline`；拆 `loadTimeline`/`loadInspector` + 独立 phase；删 `loadConversationDetail` |
| 2026-07-22 | **P5 cleanup + review：** 扫反模式；Timeline tsc 清理；OpenCode respond 走 Entity；§16/§18 同步；[review](../reviews/2026-07-22-desktop-state-p0-p4-review.md) |
| 2026-07-22 | **Residual 关闭：** Entity→list 投影 helper + hydrate 兄弟 list 同步；`daemon://push-status` livePush 回落；§6.5 quiet 全项目 ConversationList；P5 `store/workspace/*` 拆分 |

---

## 21. 逻辑层：加载、驻留、淘汰与回切体验

> 状态「按视图拆」解决的是 **谁读什么**。  
> **按消费加载**（§2.2）解决的是 **谁在展示才付 RPC**。  
> 本节解决的是 **数据何时进内存、待多久、怎么扔、回来时如何不闪 loading**。  
> 这是比 store 文件边界更影响产品手感的部分。

### 21.1 设计目标

| 目标 | 含义 |
|------|------|
| **交互序加载** | 加载触发跟用户焦点/可见性走，不预扫全库 |
| **热数据常驻** | 当前焦点与「很可能会立刻回来」的数据保留 |
| **冷数据可丢** | 远离焦点的大体量窗口可淘汰，防内存膨胀 |
| **回切无感** | 有可用缓存时先画缓存，后台对齐；避免空白闪屏 |
| **失败可恢复** | 无缓存才展示明确 loading/error；有缓存则静默重试 |
| **真相在 daemon** | 内存是工作集缓存，不是持久源 |

### 21.2 核心心智：工作集（Working Set），不是「全量镜像」

```text
daemon SQLite / runtime     = 真相与容量无上限（相对 UI）
前端内存                    = 有界工作集
UI                          = 工作集的投影 + 加载相位
```

禁止目标：

- 启动时拉齐所有 project 的全部 conversation 消息与全部 transcript  
- 无限 prepend older 且永不裁剪  
- 用「永不淘汰」换无感（迟早 OOM / 卡顿）

允许目标：

- 用户感觉「数据一直在」——通过 **SWR + keep-alive + 热集保留 + 小窗口 hydrate**，不是真无限常驻。

### 21.3 每条缓存条目的元数据

每个 keyed slice（及 SessionEntity）除业务数据外统一带：

```text
CacheEntryMeta {
  phase: idle|loading|ready|error
  generation: number
  errorMessage?: string

  // 生命周期
  lastAccessAtMs: number       // 被 View 订阅/展示时刷新
  lastSuccessAtMs?: number     // 最近一次成功 hydrate/quiet
  lastSyncAtMs?: number        // 最近一次与 daemon 对齐（含 live merge）
  pinned: bool                 // 见 21.5 钉住规则
  stale: bool                  // 已知可能过期，但仍可展示
  partial: bool                // 仅 tail/窗口，非完整历史
  byteEstimate?: number        // 可选：用于预算淘汰
}
```

`lastAccessAtMs` 更新时机：slice 被可见 View 订阅、或 Navigation 选中对应 key。

### 21.4 驻留等级（Residency Tier）

| Tier | 名称 | 含义 | 典型成员 |
|------|------|------|----------|
| **T0** | Process-hot | 进程级常驻，几乎不淘汰 | Connection；ProjectIndex 骨架；Agents clis 列表 |
| **T1** | Focus-hot | 当前焦点钉住 | 当前 project 的 ConversationList；**可见**的 Timeline(C)；**仅当 detailsOpen** 的 Inspector(C)；当前 session 的 Transcript；相关 SessionEntity |
| **T2** | Warm | 刚离开或 keep-alive 隐藏页，短 TTL | 同 project 最近 N 个 conversation 的 Timeline 窗口；收起后的 Inspector 缓存；最近 M 个 session 的 Transcript；当前 project 的 SessionList |
| **T3** | Cold | 可立即淘汰或从不预取 | 其他 project 的 Timeline/Transcript；很久未访问的 Entity 正文附属数据 |
| **T4** | Ephemeral | 用完即弃 | 审批 modal 大 plan 文本的一次性展开、一次性 full dump（若有） |

```text
T0 ──常驻──► 进程结束
T1 ──随焦点移动──► 降为 T2
T2 ──TTL / 预算压力──► 淘汰为 idle（数据释放）
T3 ──默认不占内存
```

### 21.5 钉住（Pin）规则

**Pinned = true** 时禁止淘汰数据体（meta 可更新）：

| 条件 | Pin 对象 |
|------|----------|
| `Navigation.projectId === P` | `ConversationList(P)`；`SessionList(P)`（若 Sessions 曾加载或当前 tab 需要） |
| `Navigation.conversationId === C` 且 Timeline 中栏在展示 | `Timeline(C)` |
| `Navigation.conversationId === C` 且 `detailsOpen`（Inspector 在展示） | `Inspector(C)` |
| 当前 C 下已 upsert 的 SessionEntity | 可 pin 轻量 Entity（不依赖 Inspector 是否打开） |
| `Navigation.selectedSessionId === S` | `Transcript(S)`；`SessionEntity(S)` |
| session `status ∈ {running, needs_approval}` | 该 `SessionEntity`；若 Transcript 已打开过可 pin 其 tail 窗口 |
| 进行中的 sendMessage / resolveApproval | 相关 Timeline + Entity 直至 use-case 结束 |

**Unpin**：条件不再满足后进入 T2，启动 idle TTL。  
例：`detailsOpen=false` → `Inspector(C)` 从 T1 降为 T2（缓存可留，**不再**因焦点强制 revalidate）。

### 21.6 加载策略（When to load）

#### 21.6.1 触发模型（与 UI 交互序对齐）

```text
View 变为可见（mount / 展开 / tab 切到）
  → ensureLoaded(key, { reason: focus })
      if 无数据且 phase!=loading → hydrate (show loading if blank)
      if 有数据且 stale/超龄 → revalidate quiet (keep painting cache)
      if 有数据且新鲜 → no-op（live 继续维护）

Navigation 只改 key
  → 各可见 View 的 effect 因 key 变化再次 ensureLoaded
  → 不可见 View 不跑 ensure
```

| UI 事件 | ensureLoaded |
|---------|----------------|
| 冷启动成功 | ProjectIndex；当前/默认 project 的 ConversationList（侧栏/Work 可见时） |
| 选中 project | 指针变更 → ConversationList View ensure(P)；若 projectView=sessions 且 Sessions 可见 → SessionList(P) |
| 选中 conversation | 指针变更 → **仅 Timeline 中栏可见则** ensure Timeline(C)；**仅 detailsOpen 则** ensure Inspector(C) |
| 展开右栏 `detailsOpen=true` | ensure Inspector(C)（此时才 list_sessions） |
| 收起右栏 | 停止 Inspector 消费；不 cancel 在途 RPC，但结果按 gen 写入缓存即可 |
| 打开 `@` 补全 | ensure Inspector(C)（若需要 session 列表） |
| 选中 session / Sessions 详情 | Transcript(S) |
| 切到 Sessions tab | SessionList(P) |
| 切到 Board | 依赖 ConversationList；不拉 Timeline/Inspector/Transcript |
| 上翻 | loadOlder（仅当前可见 key） |
| live 事件 | 见 §11：Entity 永远更新；重窗仅 entry 存在时 merge/quiet；**不**因 dirty 创建 entry |

#### 21.6.2 窗口加载（永远先 tail）

| 资源 | 首载窗口 | older | 默认上限（单 key 内存） |
|------|----------|-------|------------------------|
| Timeline messages | 最近 **80**（`MESSAGE_PAGE_SIZE`） | prepend 页 80 | 建议硬顶 **~500** 条/对话，超出从 **更旧端** 丢弃并 `hasOlder=true` |
| Transcript items | 最近 **~400** events | prepend | 建议硬顶 **~2000** items/session 或 **~8–16MB** 估重，超出丢旧端 |
| ConversationList | 一页最多 ~100（现 RPC） | 若未来分页再定 | 通常整表可驻留 per project |
| SessionList | project 聚合 | — | Entity 级；list 只存 id 分组 |
| ProjectIndex | 全量项目（通常很小） | — | T0 |

**原则：** 首屏只买「够画一屏 + 一点缓冲」；历史用滚动购买；内存用硬顶回收最旧窗口。

#### 21.6.3 新鲜度（何时 quiet revalidate）

| 资源 | 视为新鲜 | 变 stale | 回切行为 |
|------|----------|----------|----------|
| ConversationList | 成功同步后 + live 维护 | `bootEpoch` 变；显式 mutate；TTL 如 60s 无访问且无 pin | SWR |
| Timeline | live conversation 推送已接；或 lastSync < 15s | dirty 事件；发送后；超龄 | SWR quiet re-list **保留 older** |
| Inspector/Entity | live manager/ingest | list hydrate；bootEpoch | 有实体则直接画 |
| Transcript | ingest 连续；打开中 | 离开后再进且 lastSync 较旧；bootEpoch | SWR：先画 tail 缓存，quiet append/校验 |
| ProjectIndex | boot 后 | create project；TTL 长 | 通常直接画 |

`livePush=true` 且 key 仍 pin 时：**信任推送**，不做定时 blind poll。

### 21.7 回切体验：Stale-While-Revalidate（SWR）

这是「感觉数据一直在」的主机制。

```text
用户从 C1 切到 C2 再切回 C1
        │
        ▼
Timeline(C1) 仍有 ready 缓存？
    │是                              │否
    ▼                              ▼
立刻渲染缓存消息（0 loading 闪白）   显示结构化 loading/骨架
phase 保持 ready                     phase=loading
同时 quiet revalidate（可选）         hydrate tail
live 继续合并
```

#### 21.7.1 UI 相位展示规则

| 数据态 | UI |
|--------|-----|
| `phase=ready` 且有 items | **永远先画数据**；后台同步不闪全屏 Loading |
| `phase=ready` 且 empty | 空态文案（真的没有消息） |
| `phase=loading` 且无 items | 骨架 / 局部 spinner（仅内容区，不卸壳） |
| `phase=loading` 且有 items | **禁止**清空列表；顶/底细条或静默 |
| `phase=error` 且有 items | 画旧数据 + 非阻塞错误条 + Retry |
| `phase=error` 且无 items | 错误空态 + Retry |

#### 21.7.2 Keep-alive 与缓存的关系

| 机制 | 作用 |
|------|------|
| Conversations/Sessions/Board **DOM keep-alive**（hidden+inert） | 保滚动位置、避免 remount 闪烁；**不**等于无限内存 |
| Slice 内存缓存 | 跨 tab/对话仍能秒开 |
| 二者解耦 | 可销毁 DOM 仍留 cache；也可丢 cache 但 keep DOM 壳 |

推荐：

- **当前 projectView 三套面板 DOM keep-alive**（现有行为）  
- **Timeline/Transcript 数据** 按 T1/T2 驻留，不因 tab hidden 卸载  
- 切 **project** 时：旧 project 的 Timeline/Transcript 降 T2，受预算淘汰  

### 21.8 淘汰策略（Eviction）

#### 21.8.1 触发条件（任一）

1. **TTL**：T2 条目 `now - lastAccessAtMs > TTL`  
2. **数量预算**：某类 key 超过上限  
3. **字节预算**（可选）：总工作集超过阈值  
4. **显式**：`bootEpoch++`、logout/reconnect 清空业务层、用户切换导致 project 级 drop  

#### 21.8.2 默认预算（v1）

| 缓存类 | 上限 | TTL（unpin 后） | 淘汰序 |
|--------|------|-----------------|--------|
| ConversationList per project | 当前 project 必留；其他 project **最多 2** 份 list | 15 min | LRU |
| Timeline 窗口 | **最多 5** 个 conversationId | 10 min | LRU；当前 pin 除外 |
| Transcript 窗口 | **最多 3** 个 sessionId（+ 所有 running/needs_approval 可额外 pin） | 5 min | LRU |
| SessionEntity | **最多 ~200** 条；优先保留 pin/active | 30 min | LRU |
| SessionList | 当前 project + LRU **1** 个旧 project | 15 min | LRU |
| older 页累积 | 受 §21.6.2 硬顶裁剪 | — | 丢最旧 |

数字可配置；实现应集中在一处 `CacheBudget` 常量。

#### 21.8.3 淘汰时做什么

```text
evict(key):
  释放 items / messages 大数组
  phase → idle（或保留 meta: lastSuccessAtMs 供「曾加载过」提示，可选）
  保留 SessionEntity 轻量字段更久（status 小）
  不取消 daemon 侧事实
```

**SessionEntity 与 Transcript 分离：**  
可丢 4000 行 transcript，仍保留 status=needs_approval，列表 pill 不瞎。

#### 21.8.4 裁剪 vs 整 key 淘汰

| 操作 | 场景 |
|------|------|
| **Trim oldest** | 单 Timeline/Transcript 超过硬顶但仍 pin | 保持 ready，设 `hasOlder=true`，`partial=true` |
| **Evict key** | 非 pin 且 LRU/TTL | 整窗释放 |

### 21.9 预取（Prefetch）——少而准

预取目标：降低「下一次点击」的空白，而不是猜全世界。

| 时机 | 预取 | 优先级 |
|------|------|--------|
| ConversationList ready | **不**自动拉所有 Timeline | — |
| hover 对话行 > 150ms（可选） | Timeline tail soft prefetch | 低；可取消 |
| 当前对话 Inspector 有 running/needs_approval | 后台 peek transcript tail（已有 elevate 逻辑） | 中 |
| 用户点 Sessions 但未选 session | 只 SessionList，不预取全部 Transcript | — |
| 发送消息后 | 该 session 进入关注；Transcript 若在 Sessions 可预热 | 中 |

禁止：进入 project 后并行 hydrate 全部 conversation 的 messages。

### 21.10 并发与正确性

| 规则 | 说明 |
|------|------|
| 单 key 单飞行 | 同 key hydrate 合并为 in-flight Promise |
| generation | 响应返回时 key/generation 不匹配则丢弃 |
| 切换打断 | 切走不强制 abort RPC（可选 abort）；但结果不得写错 key |
| quiet 不降级 phase | 避免闪 loading |
| live 与 hydrate 竞态 | hydrate 整窗替换时 merge 策略：以较高 seq 为准或 re-list 后 ingest 再追 |

### 21.11 按资源的完整生命周期表

| 资源 | 首次加载 | 驻留 | 回切 | 淘汰 | 后台对齐 |
|------|----------|------|------|------|----------|
| ProjectIndex | bootstrap | T0 | 直接画 | 几乎不 | 手动 refresh / create |
| ConversationList(P) | 选中 P | T1 当 P 当前；否则 T2 | SWR | LRU/TTL | live 可选 + mutate 后 re-list |
| Timeline(C) | 中栏可见 ∧ C | T1 当中栏展示 C | SWR + 保滚动 | LRU 5 + trim 500 | conversation dirty quiet |
| Inspector(C) | 右栏可见 ∧ C（或 @ 补全打开） | T1 仅 detailsOpen；否则 T2/idle | 独立 phase + Entity 投影 | 独立 LRU（可与 Timeline 同 max 量级） | 可见时 SWR；Entity live |
| SessionList(P) | 进 Sessions | T1/T2 | SWR | LRU | sessionAdded / quiet |
| Transcript(S) | 选中 S | T1；active 额外 pin | SWR | LRU 3 + trim | ingest |
| SessionEntity | list/live | 轻量 T2 长 | 无 loading | 数量顶 | manager/ingest |

### 21.12 「无感」的分层标准（验收体验）

| 级别 | 用户感知 | 实现要点 |
|------|----------|----------|
| **L-instant** | 像没离开 | pin 缓存 + DOM keep-alive + 不闪 phase |
| **L-fast** | <100–200ms 有内容 | 内存 hit；最多 quiet 小圆点 |
| **L-ok** | 短骨架 | 冷 hydrate tail（本机 RPC，通常可接受） |
| **L-slow** | 长等待 | 应避免：缺窗口化、同步拉 full history、主线程重 markdown 全量 |

验收用例：

1. A↔B 对话来回切换 10 次：已访问过的一侧 **不得** 全屏 Loading（除非被淘汰且冷加载）。  
2. Conversations↔Sessions tab 来回：滚动位置与已载 tail **保持**。  
3. 打开超长 transcript 只增 older：内存项数不超过硬顶；上翻仍可再 load（若 daemon 有）。  
4. 同时多个 running agent：Entity 全在；Transcript 只保证当前 + pin active 的 tail。  
5. 切换 project 再切回：List 应 L-instant/L-fast；Timeline 视是否在 LRU 5 内。  

### 21.13 逻辑层模块职责（建议）

与 §2 对应，强调「谁做生命周期」：

```text
View
  · 订阅 slice
  · 报告可见性 / focus（或由 Navigation 推导）
  · 不实现 LRU

Slice Store / Family
  · 持有 CacheEntryMeta + 数据
  · ensureLoaded / applyQuiet / applyLive

CacheController（逻辑核）
  · pin/unpin（订阅 Navigation + active sessions）
  · evict/trim 执行预算
  · 提供 getResidency(key)

Use-case
  · 业务编排时 touch lastAccess / 临时 pin

LiveIngress
  · 只写仍存在的 entry；淘汰后的事件可丢或只更新 Entity
```

**Live 与淘汰：**  
session 已 evict transcript 后仍收到 ingest → **仍更新 SessionEntity**；Transcript 数据可忽略直至再次 ensureLoaded。

### 21.14 滚动位置与「内容还在」

数据缓存命中但滚动丢了，仍像「重新加载」。

| 状态 | 存哪 |
|------|------|
| Timeline scroll / follow 模式 | 优先 DOM keep-alive；或 `UIChromeState` per conversationId：`scrollTop` 或 `anchorMessageId` |
| Transcript 同上 | per sessionId |
| List 滚动 | per projectId |

淘汰 **数据** 时：可保留廉价的 `scrollAnchor` meta（id + offset），再次 hydrate 后 restore。

### 21.15 与当前实现的差距（迁移提示）

| 现况 | 目标 |
|------|------|
| 单一 workspace 大 map，基本不 LRU | 引入 CacheRuntime 预算 |
| **`loadConversationDetail` 永远双 RPC** | 按可见性分别 ensure；收起右栏不拉 sessions |
| **`applyConversationEvent` → loadConversationDetail** | markDirty + 条件 quiet **仅 messages** |
| **单一 `detailStatusByConversation`** | Timeline.phase ∥ Inspector.phase |
| applyIngest 无脑写 transcript map | **已改：** `hasTranscriptWorkingSet`；无 key 只抬审批 status |
| tab keep-alive 已做 | 保留；T1/T2 + detailsOpen pin |
| Timeline/Transcript 分页已做 | 补 **硬顶 trim** |
| quiet re-list / bootEpoch 已做 | 形式化为 SWR + §11.3 门闸 |
| 无统一 lastAccess/pin | 增加 meta |
| live 下 Timeline 6s / Sessions 8s 轮询 | livePush=true **删除** interval；P3 仅 false |
| livePush 断线不回落 | pump 结束 → false 或重订 |

### 21.16 一句话策略

> **谁在看谁加载；按钉住与 LRU 管内存；有缓存就先画再对齐；没缓存再 loading；用本机 tail RPC 保证冷启动也够快——用工作集模拟「数据一直在」，而不是把 daemon 全量镜像进前端，也不是按「打开动作」打包双 RPC。**

---

## 22. CacheRuntime（一体规格）：Budget + ensureLoaded / evict

> **结论：二者一体，必须一起做。**  
> - **CacheBudget** = 运行时的约束参数（尺码）  
> - **ensureLoaded / evict 状态机** = 在这些约束下的动作剧本  
> 实现上落成同一模块：`CacheRuntime`（或 `CacheController`），而不是两套互不调用的文档/代码。

### 22.1 一体关系

```text
                    ┌──────────────────┐
                    │   CacheBudget      │  常量 / 可配置表
                    │   maxKeys, TTL,    │
                    │   pageSize, hardCap│
                    └────────┬─────────┘
                             │ 每次 ensure / evict / trim 读取
                             ▼
┌──────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ Navigation   │────►│  CacheRuntime      │────►│ Slice entries    │
│ + active     │事件 │  ensureLoaded      │写入 │ phase/data/gen   │
│ sessions     │     │  onHydrateResult   │     │ pin/stale/…      │
│ + live       │     │  evictIfNeeded     │     └─────────────────┘
└──────────────┘     │  trimWindow        │
                     │  recomputePins     │
                     └──────────────────┘
```

| 没有 Budget | 没有状态机 |
|-------------|------------|
| 状态机不知道踢谁、留多久 | 有上限却到处 if 魔法，竞态不一致 |
| 内存策略无法调参 | 调参了也防不住写错 key |

**验收按一体做：** 改 Budget 数字不应改状态机结构；改竞态规则不应散落改 Budget。

---

### 22.2 CacheBudget 配置表（v1 默认，实现落常量）

实现建议：单文件 `cache-budget.ts`（或同名 Rust 无关，纯前端）。

```text
// 伪常量名 — 实现时用 as const 对象即可
CacheBudget = {
  // ── ConversationList ──
  conversationList: {
    maxProjectKeys: 3,                 // 含当前 project；超出 LRU evict 非 pin
    pageLimit: 100,                    // 对齐现 RPC limit
    unpinnedTtlMs: 15 * 60_000,
    revalidateAfterMs: 60_000,         // ready 且超过此时长可 quiet SWR
  },

  // ── Timeline ──
  timeline: {
    maxConversationKeys: 5,            // pin 不计入可踢集合
    pageSize: 80,                      // MESSAGE_PAGE_SIZE
    hardMaxMessages: 500,              // 单 key 硬顶；超出 trim 最旧
    unpinnedTtlMs: 10 * 60_000,
    revalidateAfterMs: 15_000,
    prefetchHoverMs: 150,              // 可选 hover 预取；0=关闭
  },

  // ── Inspector（会话摘要列表；行数据多在 Entity）──
  // 独立 Entry + 独立 phase；key 仍是 conversationId，但加载/ pin 不跟随 Timeline
  inspector: {
    maxConversationKeys: 5,            // 可与 timeline 同量级；独立 LRU
    unpinnedTtlMs: 10 * 60_000,
    revalidateAfterMs: 30_000,
    // pin 条件：detailsOpen || @补全打开；不 follow Timeline 可见性
    pinWhenDetailsOpen: true,
  },

  // ── SessionList ──
  sessionList: {
    maxProjectKeys: 2,
    unpinnedTtlMs: 15 * 60_000,
    revalidateAfterMs: 30_000,
  },

  // ── Transcript ──
  transcript: {
    maxSessionKeys: 3,                  // 另加 active pin 可超出
    pageEvents: 400,                   // TRANSCRIPT_PAGE_EVENTS
    hardMaxItems: 2000,
    // 可选体积顶：超过则 trim（实现可用粗估）
    hardMaxApproxBytes: 12 * 1024 * 1024,
    unpinnedTtlMs: 5 * 60_000,
    revalidateAfterMs: 10_000,
    peekTailForApproval: 120,          // onConversationFocused elevate peek
  },

  // ── SessionEntity ──
  sessionEntity: {
    maxEntities: 200,
    unpinnedTtlMs: 30 * 60_000,
    // status in running|needs_approval → 强制 pin（见状态机）
  },

  // ── ProjectIndex ──
  projectIndex: {
    // T0：无 maxKeys 淘汰；仅 bootEpoch/reconnect 清空
    revalidateAfterMs: 5 * 60_000,
  },

  // ── 全局 ──
  global: {
    // 可选总字节顶；0=不启用
    maxApproxBytes: 0,
    // evict 扫描间隔（也可纯事件驱动）
    evictionCheckOn: ["ensureLoaded", "unpin", "hydrateSuccess", "interval"],
    evictionIntervalMs: 60_000,
  },
}
```

#### 22.2.1 预算语义（状态机必须遵守）

| 字段 | 状态机如何用 |
|------|----------------|
| `max*Keys` | `evictIfNeeded(kind)`：非 pin 条目按 `lastAccessAtMs` LRU 删到上限内 |
| `hardMax*` | `trimWindow(key)`：pin 中也可裁最旧，保持 `phase=ready`，`partial=true`，`hasOlder=true` |
| `pageSize` / `pageEvents` | hydrate / loadOlder 的请求 limit |
| `unpinnedTtlMs` | `now - lastAccessAtMs > ttl && !pinned` → 可 evict |
| `revalidateAfterMs` | `ensureLoaded`：已有 ready 且超时 → quiet SWR，不进 loading |
| active pin 规则 | 覆盖 maxKeys：active transcript 可导致线程数 > `maxSessionKeys` |

#### 22.2.2 调参原则

1. 先调 `max*Keys` / `hardMax*` 观察内存与回切命中率。  
2. 不在业务代码写死第二套数字。  
3. 测试可注入更小 Budget 验证 LRU。

---

### 22.3 条目状态（所有 heavy slice 共用）

适用于：`ConversationList` / `Timeline` / `SessionList` / `Transcript`。  
`SessionEntity` 用简化版（通常无 large items 数组）。

```text
phase ∈ { idle, loading, ready, error }

Entry {
  key: string
  phase
  generation: number              // 单调；每次「会写满数据」的加载意图 +1
  quietGeneration: number         // quiet 飞行世代；可与 generation 分立
  data: T | null                  // items / messages / …
  errorMessage?: string
  pinned: bool
  stale: bool
  partial: bool                   // 窗口非全量
  lastAccessAtMs: number
  lastSuccessAtMs?: number
  lastSyncAtMs?: number
  inFlight?: Promise<void>        // single-flight
  scrollAnchor?: cheap meta
}
```

**generation 契约：**

- 只有 `result.generation === entry.generation` 的 **非 quiet** hydrate 结果可把 phase 打到 ready/error 并替换窗口策略允许的 data。  
- quiet 使用 `quietGeneration`；过期 quiet 结果丢弃；成功 quiet **不得**把 phase 从 ready 打成 loading。  
- 切 key 不共享 generation。

---

### 22.4 事件（CacheRuntime 输入）

| 事件 | 来源 |
|------|------|
| `ENSURE_LOADED { key, kind, reason }` | **View 可见**（或 `@` 等显式子 UI）；Navigation 变 key 后由可见 View 再发 |
| `LOAD_OLDER { key, kind }` | 上翻 |
| `HYDRATE_OK { key, gen, mode, data, hasMore }` | RPC 成功；mode=full\|quiet\|older |
| `HYDRATE_ERR { key, gen, mode, error }` | RPC 失败 |
| `TOUCH { key }` | 展示/订阅 |
| `RECOMPUTE_PINS` | Navigation 变 / session status 变 / use-case 起止 |
| `LIVE_MERGE { key, kind, patch }` | LiveIngress |
| `MARK_STALE { key }` / `MARK_STALE_KIND { kind }` | bootEpoch、显式 mutate |
| `TRIM_CHECK { key }` | 数据变长后 |
| `EVICT_PASS` | 定时或 ensure 后 |
| `DROP_ALL_BUSINESS` | reconnect / boot 失败清理 |

`reason` 示例：`focus | boot | hover_prefetch | post_mutate | swr`。

---

### 22.5 ensureLoaded 算法（伪代码）

```text
function ensureLoaded(key, kind, reason):
  budget = CacheBudget[kind]
  entry = getOrCreate(key, kind)    // create → phase=idle, gen=0, data=null
  entry.lastAccessAtMs = now()
  recomputePin(entry)

  if entry.phase == loading && entry.inFlight:
    return entry.inFlight            // single-flight 合并

  if entry.phase == ready && entry.data != null:
    fresh = (now() - entry.lastSyncAtMs) < budget.revalidateAfterMs
            && !entry.stale
    if fresh && reason != post_mutate:
      return                         // 信任 live + 近期同步
    else:
      return startQuietRevalidate(entry, kind)   // SWR：保持 ready

  if entry.phase == error && entry.data != null:
    // 有脏缓存：先仍展示；允许 SWR 重试
    return startQuietRevalidate(entry, kind)

  // 无可用数据：真 loading
  return startHydrate(entry, kind, mode=full)

function startHydrate(entry, kind, mode):
  entry.generation += 1
  gen = entry.generation
  if mode == full:
    entry.phase = loading            // 仅无 data 时；若有 data 勿清空
    // 若 data==null：UI 可骨架；若 data!=null 不应走到 full 除非 force
  entry.stale = false
  entry.inFlight = rpcHydrate(kind, entry.key, limit=budget.page*)
    .then(data => dispatch HYDRATE_OK { gen, mode, data })
    .catch(err => dispatch HYDRATE_ERR { gen, mode, err })
    .finally(() => { if entry.generation==gen: entry.inFlight=null })
  evictIfNeeded(kind)                // 新 key 可能顶掉 LRU
  return entry.inFlight

function startQuietRevalidate(entry, kind):
  entry.quietGeneration += 1
  qgen = entry.quietGeneration
  // phase 保持 ready/error-with-data
  return rpcHydrate(...)
    .then(data => dispatch HYDRATE_OK { gen:qgen, mode:quiet, data })
    .catch(err => dispatch HYDRATE_ERR { gen:qgen, mode:quiet, err })
```

#### 22.5.1 HYDRATE_OK / ERR 处理

```text
on HYDRATE_OK(key, gen, mode, data, hasMore):
  entry = mustGet(key)
  if mode == quiet:
    if gen != entry.quietGeneration: return          // 过期 quiet
    entry.data = mergeQuiet(entry.data, data)        // Timeline：保留 older
    entry.lastSuccessAtMs = entry.lastSyncAtMs = now()
    entry.stale = false
    entry.phase = ready
    entry.partial = true                             // 仍是窗口
    trimWindow(entry, kind)
    return

  if mode == full:
    if gen != entry.generation: return               // 过期 full（竞态关键）
    entry.data = data                                // tail 窗口替换
    entry.phase = ready
    entry.lastSuccessAtMs = entry.lastSyncAtMs = now()
    entry.stale = false
    entry.partial = true
    entry.errorMessage = null
    trimWindow(entry, kind)
    return

  if mode == older:
    if gen != entry.generation: return               // older 也绑当前 gen 或独立 olderGen
    entry.data = prependOlder(entry.data, data)
    entry.history.hasOlder = hasMore
    trimWindow(entry, kind)                          // 可能立刻裁最旧以守 hardMax
    return

on HYDRATE_ERR(key, gen, mode, error):
  entry = mustGet(key)
  if mode == quiet && gen != entry.quietGeneration: return
  if mode != quiet && gen != entry.generation: return
  if mode == quiet:
    // 保持原 data 与 phase=ready；可记 lastError 轻提示
    return
  if entry.data != null:
    entry.phase = ready                              // 降级：有缓存不当空白 error 页
    entry.stale = true
    entry.errorMessage = error                       // 非阻塞
  else:
    entry.phase = error
    entry.errorMessage = error
```

#### 22.5.2 竞态场景（必须通过）

| 场景 | 期望 |
|------|------|
| 快切 C1→C2，C1 响应后到 | C1 的 gen 不匹配当前 C1 entry 或 entry 已不在焦点；**不得**写入 C2；若 C1 entry 仍在且 gen 匹配可更新 C1 缓存 |
| C1 loading 中再次 ensureLoaded(C1) | 复用 inFlight，不双发 |
| ready 上 quiet 与 full 并发 | full 提高 generation 后，旧 quiet 丢弃 |
| evict 后晚到响应 | entry 已 idle/不存在 → 丢弃 |
| older 加载中用户切走再回 | 新 gen hydrate tail；旧 older 丢弃 |

**写保护一句话：**  
任何 RPC 回调必须带 `(key, gen, mode)`，写前校验，失败则 no-op。

---

### 22.6 pin / touch / evict 算法

```text
function recomputePins():
  clear all pinned flags
  P = Navigation.projectId
  C = Navigation.conversationId
  S = Navigation.selectedSessionId
  pin ConversationList(P), SessionList(P) if exists
  if C && timelinePaneVisible: pin Timeline(C)
  if C && (Navigation.detailsOpen || mentionPickerOpen): pin Inspector(C)
  pin Transcript(S) if S
  for each SessionEntity e where e.status in {running, needs_approval}:
    pin e
    if Transcript(e.id) exists: pin it          // 可选：仅当曾打开
  pin keys touched by in-flight use-cases

function touch(key):
  entry.lastAccessAtMs = now()

function evictIfNeeded(kind):
  budget = CacheBudget[kind]
  entries = all entries of kind ordered by lastAccessAtMs asc
  victims = entries where !pinned
  while count(entries) > budget.maxKeys:
    v = oldest(victims)
    if v == null: break                         // 全 pin，允许暂时超标
    evictOne(v)

function evictByTtl():
  for entry in all:
    if entry.pinned: continue
    if now - entry.lastAccessAtMs > budget.unpinnedTtlMs:
      evictOne(entry)

function evictOne(entry):
  // 可保留 scrollAnchor 与 lastSuccessAtMs 极简 meta（可选）
  entry.data = null
  entry.phase = idle
  entry.stale = false
  entry.inFlight = null
  // generation 保留或归零皆可；归零更简单
  entry.generation += 1                         // 作废一切 in-flight 写

function trimWindow(entry, kind):
  cap = budget.hardMax*
  if size(entry.data) <= cap: return
  drop oldest until size <= cap
  entry.partial = true
  entry.history.hasOlder = true
```

#### 22.6.1 何时跑 EVICT_PASS

```text
ensureLoaded 末尾
hydrate success 后（key 数/体积变化）
recomputePins 后（有 unpin）
每 evictionIntervalMs（若启用）
DROP_ALL_BUSINESS
```

---

### 22.7 与 UI phase 展示的绑定（状态机输出）

| Entry 条件 | UI |
|------------|-----|
| `phase=loading && data==null` | 内容区骨架 |
| `phase=loading && data!=null` | 禁止；实现错误 |
| `phase=ready` | 渲染 data |
| `phase=ready && quiet in flight` | 渲染 data；可选极淡同步指示 |
| `phase=error && data==null` | 错误空态 + Retry → ENSURE_LOADED |
| `phase=error && data!=null` | 不应持久；见上 ERR 处理降为 ready+stale |
| `phase=idle && data==null` | 未请求；View 应发 ENSURE_LOADED |

---

### 22.8 Live 与 Runtime 的衔接

Ingress 与 Runtime **分工**（与 §2.2 / §11 一致）：

```text
// ── Ingress（永远）──
onIngest(ev):
  SessionEntity.upsert(ev.sessionId, { hasPendingApproval, … })
  LIVE_MERGE_IF_ENTRY(Transcript, ev.sessionId, ev.items)

onManager(ev):
  SessionEntity.applyLifecycle(ev)

onConversationDirty(ev):
  dirty.set(Timeline, ev.conversationId, ev.messageSeq)
  if shouldQuietRevalidate(Timeline, ev.conversationId):
    scheduleQuiet(Timeline, ev.conversationId, 200ms)   // 仅 list_messages
  // else: 不 ensure、不创建 entry

// ── Runtime ──
on LIVE_MERGE_IF_ENTRY(kind, key, patch):
  entry = get(kind, key)
  if entry == null || entry.phase == idle || entry.data == null:
    return                    // 重窗不存在：Ingress 已写 Entity，此处 no-op
  entry.data = mergeLive(entry.data, patch)
  entry.lastSyncAtMs = now()
  entry.stale = false
  trimWindow(entry, kind)
  touch(key)

function shouldQuietRevalidate(kind, key):   // 同 §11.3
  …
```

**禁止：**

```text
// BAD（现状）
applyConversationEvent → loadConversationDetail(C)  // 双 RPC + 无 entry 也拉
```

---

### 22.9 一体验收清单

| # | 用例 | Budget 点 | 状态机点 |
|---|------|-----------|----------|
| 1 | 打开对话（右栏开） | Timeline+Inspector 各 ensure | 两 phase 可先后 ready |
| 1b | 打开对话（右栏关） | 仅 Timeline | **无** list_sessions |
| 1c | 从未打开的 C 后台 dirty | — | **零** list_messages；仅 dirty 位 |
| 1d | 已打开 C 仍 cache 时 dirty | — | 200ms quiet **仅** messages |
| 1e | Transcript evict 后仍 ingest | Entity 仍更新 | **不**重建 transcript 窗 |
| 2 | 快切对话 | — | 旧 gen 丢弃 |
| 3 | 来回切已访问对话 | maxKeys 内 hit | ready 无闪白 |
| 4 | 打开第 6 个对话 | maxKeys=5 | 最久未用非 pin 被 evict |
| 5 | 超长上翻 | hardMax | trim 最旧，仍 ready |
| 6 | running agent 离开 transcript | maxSessionKeys | Entity pin；transcript 可 pin/trim |
| 7 | 断线重连 | — | livePush 回落；bootEpoch ensure |
| 8 | quiet 失败 | — | 仍显示旧 data |
| 9 | 双 ensure 同 key | — | single-flight |
| 10 | livePush=true | — | **无** Timeline/Sessions interval |
| 11 | 调小 Budget 单测 | maxKeys=1 | 第二次打开挤出第一次 |

---

### 22.10 实现落点建议

| 单元 | 职责 |
|------|------|
| `cache-budget.ts` | 仅常量 `CacheBudget` |
| `cache-runtime.ts` | ensureLoaded / onResult / evict / trim / pins / shouldQuietRevalidate |
| `live-ingress.ts` | 单例 listen；Entity + dirty；条件 notify Runtime |
| 各 slice store | 存 Entry；委托 Runtime；不写私有 LRU |
| View | 可见时 `ensureLoaded` + 读 entry；不可见不 ensure；**不** setInterval（live 时） |
| 测试 | Runtime + Ingress 单测：dirty 无 entry 不 RPC；有 entry 才 quiet |

---

### 22.11 一句话

> **CacheRuntime = CacheBudget × 进出状态机**；  
> **LiveIngress = 全局薄源（Entity + dirty）**；  
> **ensureLoaded 只由可见消费者触发**；  
> **quiet re-list 只服务已有工作集**——不替打开动作打包双 RPC，不因推送创造重窗。
