# Desktop 状态管理规格：按消费范围拆分

> Status: draft design  
> Date: 2026-07-21  
> Scope: `apps/desktop` 前端状态形状与刷新契约（不绑定具体 Zustand API）  
> Related: [architecture-desktop.md](../../architecture-desktop.md), [2026-07-18-desktop-product-experience.md](2026-07-18-desktop-product-experience.md)

## 1. 目标与非目标

### 1.1 目标

1. 按 **UI 实际消费范围** 划分状态，而不是一个上帝 `WorkspaceState`。
2. 明确每个状态切片的：**字段、生产者、消费者、加载/刷新触发、与 daemon 的映射**。
3. 跨面板共享的实体（尤其 Session 运行态）有 **单一写入点**，视图只做投影。
4. 保持现有产品不变量：
   - Conversation 主时间线 ≠ Session transcript
   - `needs_approval` 由 UI 从 pending approval 派生，daemon 不直接发该状态
   - 导航只改 id；资源由对应切片声明式加载
5. 逻辑层明确：**何时加载、驻留多久、如何淘汰、回切如何无感**（见 §21）。状态形状服务于消费范围；**缓存生命周期服务于体验与内存**。

### 1.2 非目标

- 本规格不要求立刻重构代码；作为拆分与实现的目标契约。
- 不改变 daemon JSON-RPC 形状（除非另开协议变更）。
- 不覆盖 Attention / Agents / Host 的完整产品设计（仅定义其状态边界）。
- 不引入与历史版本的兼容层。
- §21 的预算数字为 **v1 默认**，可按 profiling 调整；不要求磁盘级二级缓存（除非另开规格）。

---

## 2. 分层总览

```text
L0  NavigationState              指针：我在哪
L1  ConnectionState              启动 / daemon 连接 / live 开关
L2  ProjectIndexState            侧栏项目列表 + ProjectSummary
L3  View slices（按 projectView）
    L3a Conversations view
        · ConversationListState(projectId)
        · TimelineState(conversationId)
        · InspectorState(conversationId)
    L3b Sessions view
        · SessionListState(projectId)
        · TranscriptState(threadId)
        · SessionSummaryView(threadId)   // 可派生，非必须独立 store
    L3c Board view
        · BoardState(projectId)          // 可派生自 ConversationList + SessionEntity
L4  Shared entities
    · SessionEntity(threadId)            // status 真相 + 审批抬升
L5  LiveIngress                      推送单入口 → 写 L4 / 通知 L3
L6  Use-cases                        跨切片编排（sendMessage 等）
```

### 2.1 依赖方向

```text
Navigation ──► 选择 family key（projectId / conversationId / threadId）
Connection ──► 允许/禁止对 daemon 的 load
LiveIngress ─► SessionEntity / List 摘要字段 / Timeline dirty
Use-case ────► 多个 slice 的 load/mutate（唯一允许的跨写编排层）
View ────────► 只 read 自己的 slice + Navigation + 少量 Entity 投影
```

禁止：View 直接改另一个 View 的私有缓存（应走 Use-case 或 LiveIngress）。

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
| daemon thread label / manager | 提供除 `needs_approval` 外的生命周期 |
| pending approval signal | `true` → 强制 `needs_approval` |
| manager `running`/`idle` | **不得**覆盖已有 `needs_approval` |
| pending `false`（高置信） | 允许回到 daemon 状态 |
| pending 未知 + 曾为 needs_approval + daemon 仍 running | **保持** needs_approval |

### 3.3 标识

| 名 | 含义 |
|----|------|
| `projectId` | 项目 |
| `conversationId` | 协作对话 |
| `threadId` / `sessionId` | agent run（同一概念；文中统一 `threadId`） |
| `messageSeq` | 协作消息序（Timeline 排序键） |
| `transcriptSeq` | thread 事件序 |

---

## 4. L0 — NavigationState

### 4.1 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `primaryNav` | `work \| attention \| agents \| host` | 主导航 |
| `projectId` | `string` | 当前项目；空 = 未选 |
| `conversationId` | `string \| null` | 当前对话 |
| `threadId` | `string \| null` | 当前 session（Sessions / inspector 高亮） |
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
| **Consumers** | 全部 shell 路由；各 slice 的 family key |
| **Persist** | `projectId`, `conversationId`, `lastConversationByProject`（草稿可选 persist） |

### 4.3 转移规则

| 动作 | 副作用 |
|------|--------|
| `selectProject(P)` | `projectId=P`；`conversationId=last[P]??null`；`threadId=null`；`primaryNav=work` |
| `selectConversation(C)` | `conversationId=C`；`threadId=null`；`projectView=conversations`；写入 `last[projectId]` |
| `selectThread(T)` | `threadId=T` |
| `openSessionTranscript(T, C?)` | `projectView=sessions`；`threadId=T`；可选设 `conversationId` |
| `openConversation(C)` | `projectView=conversations`；`conversationId=C`；`threadId=null` |
| `setProjectView(v)` | 只改 tab；**不**清 thread（Sessions↔Conversations keep-alive 需要） |

---

## 5. L1 — ConnectionState

### 5.1 字段

| 字段 | 说明 |
|------|------|
| `booting` | 启动页门闸 |
| `bootPhase` / `bootProgress` | 启动文案与进度 |
| `bootEpoch` | 每次成功 boot +1；触发各 slice 重新 init |
| `connection` | `{ connected, endpoint?, error?, source, managed }` |
| `livePush` | 前端 listen 是否已挂上 |
| `error` | **仅** boot/连接级错误 |
| `actionError` | 可选：全局动作错误条（或下沉到 use-case 结果） |

### 5.2 生产者 / 消费者

| | |
|--|--|
| **Producers** | `bootstrap` / `reconnect`；Tauri connect 结果；event bridge start 结果 |
| **Consumers** | BootScreen；Sidebar host presence；Host 页；所有 load（`connected` 门闸） |

### 5.3 刷新

| 触发 | 行为 |
|------|------|
| App mount | single-flight `bootstrap` |
| Host Reconnect | 再 `connect`；成功则 `bootEpoch++` 并清空业务 slice 缓存（或标记 stale） |
| 连接边沿 | ConnectionToasts（冷启动首帧不 toast） |

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
  mentions?: { agent, threadId?, threadShortId? }[]
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
| open / quiet re-list | `minos_local_list_conversation_messages`（tail / before_seq） |
| append user（写路径） | `minos_local_append_user_message`（由 use-case 调） |

排序：`messageSeq ASC`（权威）；前端可二次 sort 防御。

#### 刷新触发

| 触发 | 模式 |
|------|------|
| `conversationId` / `bootEpoch` | hydrate tail |
| Live `conversation` dirty | debounce ~200ms quiet re-list（**保留已加载 older**） |
| sendMessage | 乐观 append → 成功后 quiet re-list |
| 上翻 | `loadOlder(beforeSeq)` |
| `livePush=false` 且有 live session | 降级 interval quiet re-list |

#### 打开对话时的附加编排（属 Use-case，结果写入多 slice）

见 §11 `openConversationDetail`：并行 hydrate Timeline + Inspector，并可能 resume / peek transcript。

---

### 7.3 InspectorState `(key: conversationId)`

#### 字段

```text
InspectorState {
  conversationId
  phase, generation, errorMessage?
  threadIds: string[]            // 顺序：列表展示序
  // 行数据优先读 SessionEntity(threadId)；此处可缓存投影快照
}
```

展示行推荐投影：

```text
InspectorRow = SessionEntity 的摘要视图 {
  threadId, agent, shortId, status, model,
  parentId, summary, lastTsMs, needsContinue
}
```

#### 消费范围

| Consumer | |
|----------|--|
| SessionInspector 右栏 | rows + Navigation.threadId 高亮 |
| Timeline 逻辑（@ 补全 / 复用 session） | thread 列表 + status |

**排除：** 完整 transcript items。

#### 生产者 / RPC

| 操作 | RPC |
|------|-----|
| hydrate | `minos_local_list_conversation_agent_sessions` |
| live | LiveIngress → SessionEntity；Inspector 自动投影 |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| open conversation | 与 Timeline 并行 hydrate |
| quiet conversation re-list | 可并行刷新 sessions |
| manager / ingest | Entity 更新，Inspector 订阅 Entity |

---

## 8. L3b — Sessions 视图

### 8.1 SessionListState `(key: projectId)`

#### 字段

```text
SessionListState {
  projectId
  phase, generation, errorMessage?
  // 存储可选二选一：
  // A) flat threadIds + 分组函数
  // B) groups 结构缓存
  groups: SessionListGroup[]
}

SessionListGroup {
  conversationId: string
  conversationTitle: string
  threadIds: string[]          // top-level 序；subagent 通过 Entity.parentId 挂树
  // 下列可派生：
  // liveCount, attentionCount, lastTsMs
}
```

行展示数据：**一律来自 SessionEntity**，避免第三份 status。

#### 消费范围

| Consumer | |
|----------|--|
| Sessions 左栏 | groups + Entity 投影 + Navigation.threadId |
| 折叠态 | 建议放 Navigation 或局部 UI state（非业务缓存） |

#### 生产者 / RPC

| 操作 | 说明 |
|------|------|
| hydrate | 现实现：list conversations × list sessions；目标可保留或加聚合 RPC |
| live | Entity 更新；group 成员变化时（threadAdded）需补拉或插入 |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| 进入 Sessions tab / projectId / bootEpoch | hydrate |
| `livePush=false` | 可选 quiet poll |
| threadAdded 且 list 已 ready | 插入或 quiet re-list |

---

### 8.2 TranscriptState `(key: threadId)`

#### 字段

```text
TranscriptState {
  threadId
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
| hydrate tail / older / full | `minos_local_read_thread_raw_history` → bridge TranscriptAssembler |
| live | `daemon://ingest` → merge by id/seq |
| 审批回复 | resolve / opencode respond RPCs（use-case） |

#### 刷新触发

| 触发 | 模式 |
|------|------|
| Navigation.threadId 变化且 Sessions 可见 | hydrate |
| ingest for threadId | merge items；更新 pending approval → SessionEntity |
| load older | prepend + scroll restore |
| openConversationDetail peek | **elevate-only** tail 读，可写 Transcript 缓存也可只用于 status（实现可选） |

---

### 8.3 SessionSummaryView `(key: threadId)`

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

Board **不是**独立任务系统。优先：

```text
BoardState = view model over ConversationListState(projectId)
             + SessionEntity 运行态（optional 增强 needs_you）
```

### 9.2 列派生

| 列 | 规则（与现产品一致） |
|----|----------------------|
| `done` | `progress === done` 优先 |
| `needs_you` | 存在 approval/suspended 等需用户处理，且 progress 仍可视为 in_progress |
| `running` | 有 running agents / in_progress 运行态 |
| `backlog` | todo 等默认 |

字段不落库为 column；拖拽写 `progress`（needs_you → in_progress）。

### 9.3 消费 / 刷新

| | |
|--|--|
| Consumer | ProjectBoard |
| 刷新 | 跟随 ConversationList + Entity；无单独 poll |

---

## 10. L4 — SessionEntity `(key: threadId)`

跨面板共享核。

### 10.1 字段

```text
SessionEntity {
  threadId: string
  conversationId: string
  conversationTitle?: string
  agent: string
  shortId: string
  status: SessionStatus          // UI 派生后
  daemonStatus: SessionStatus    // 可选：未抬升前
  model: string
  parentId?: string
  summary: string
  messageCount: number
  firstTsMs: number
  lastTsMs: number
  needsContinue: bool
  hasPendingApproval: bool       // 最近已知
  updatedAtMs: number            // 本地合并时间
}
```

### 10.2 写入点（唯一）

| 来源 | 行为 |
|------|------|
| list_sessions / list_project_sessions 映射 | upsert；status 用 derive(prev, pending?) |
| manager.threadStateChanged | 更新生命周期；不降 needs_approval |
| manager.threadClosed | status=done |
| manager.instanceCrashed | 影响集 suspended |
| manager.threadAdded | 可建占位或触发 list 补全 |
| ingest | merge hasPendingApproval；抬 status |
| resolveApproval 成功 | pending=false 路径或等 ingest 确认 |
| resume/send 路径 | 可选 optimistic running |

### 10.3 消费者（只读投影）

- Inspector rows  
- SessionList rows  
- Attention 队列  
- ConversationListItem.runningCount / approvalCount（聚合时）  
- Timeline session 复用选择  

---

## 11. L5 — LiveIngress

### 11.1 输入事件

| 通道 | 载荷 | 处理 |
|------|------|------|
| `daemon://ingest` | `{ threadId, seq, agent, tsMs, items[], hasPendingApproval }` | Transcript merge；SessionEntity 审批/status；可选 list 摘要 |
| `daemon://manager` | tagged union | 仅 SessionEntity（+ SessionList 成员集） |
| `daemon://conversation` | `{ conversationId, messageSeq }` | 调度 Timeline quiet re-list；可选 List preview 刷新 |

### 11.2 规则

1. UI 组件 **禁止** 直接 listen daemon 事件改本地 useState 业务缓存。  
2. Ingress 是推送唯一入口。  
3. `livePush=true` 时关闭常态 quiet poll；仅降级路径启用。

---

## 12. L6 — Use-cases（跨切片编排）

Use-case 可以写多个 slice；View 只调 use-case，不手写多 store 事务。

### 12.1 `bootstrap`

```text
Connection.booting=true
connect → list_projects → list_clis(可选，属 Agents 域)
start event bridge → livePush
bootEpoch++
booting=false
// 不预拉所有 conversation/timeline
```

### 12.2 `openConversationDetail(conversationId)`

```text
parallel:
  Timeline.hydrate(conversationId)
  Inspector.hydrate(conversationId) → upsert SessionEntities
if !quiet:
  at most one top-level needsContinue → resume(thread, autoContinue=true)
  for active threads (running|suspended|prev needs_approval):
    peek transcript tail (elevate-only) → SessionEntity status
mark read → 更新 ConversationListItem.unread + ProjectSummary
```

### 12.3 `sendMessage(conversationId, body)`

```text
parse @routing
Timeline optimistic user bubble
append_user_message
resolve thread:
  #shortId match | reuse latest non-closed top-level same agent | start_agent_in_conversation(+ profile model/effort/instructions)
resume(thread, false)
send_user_message(prompt)
Timeline quiet re-list
ConversationList re-list(project)
Inspector/Entity 随 list 或 live 更新
```

### 12.4 `selectProject(projectId)`（导航 + 数据）

```text
Navigation.selectProject
ensure ConversationList(projectId)   // View init 也可
// Sessions tab keep-alive 时 SessionList 自载
```

### 12.5 `resolveApproval` / opencode permission|question

```text
RPC
SessionEntity / Transcript 随后由 ingest 收敛
禁止仅本地清 approval 而不等确认（除非 RPC 成功且协议保证）
```

---

## 13. 消费矩阵（View → State）

| View | Navigation | Connection | ProjectIndex | ConvList | Timeline | Inspector | SessionList | Transcript | SessionEntity |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| BootScreen | | R | | | | | | | |
| Sidebar | R/W | R | R | | | | | | |
| WorkView shell | R/W | | R | R(phase) | | | | | |
| ConversationList | R/W | | | **R/W load** | | | | | |
| Timeline | R/W draft | R livePush | | R(meta) | **R/W** | R | | | R(reuse) |
| SessionInspector | R/W | | | | | **R** | | | R |
| SessionsView list | R/W | | | | | | **R/W** | | R |
| Sessions transcript | R | | | | | | | **R/W** | R |
| Sessions summary | | | | | | | | R | |
| ProjectBoard | R/W | | | R | | | | | R(opt) |
| HostView | | R/W | | | | | | | |
| AttentionView | R/W | | | | | | | | R filter |
| AgentsView | | | | | | | | | | （独立 AgentsState，见 §14） |

R = read，W = 触发 load/mutate，R/W = 两者。

---

## 14. 邻接域（边界）

### 14.1 AgentsState（独立）

```text
clis[], profiles[], modelsByRuntime, phase
```

与 Work 主路径弱耦合；`sendMessage` start agent 时可 **读** 最新 profile，不写入 Work slice。

### 14.2 AttentionState

```text
// 可派生：filter SessionEntity where status in (needs_approval, failed, suspended)
// 或独立 list + quiet load 跨 project
```

优先派生；跨 project 未加载 Entity 时才需要独立 hydrate。

### 14.3 Host 诊断

只读 ConnectionState；不进业务 slice。

---

## 15. 刷新优先级

```text
P0 声明式 hydrate（id / bootEpoch 变化）
P1 LiveIngress 增量
P2 Use-case 后的 quiet 对齐（send、dirty conversation）
P3 降级 poll（livePush=false）
```

同字段冲突：

```text
generation 更高的 hydrate 结果 > 旧 hydrate
live 增量 merge 到当前缓存（id 稳定）
quiet re-list 不得丢已加载 older（Timeline/Transcript 契约）
```

---

## 16. 与现状 `workspace-store` 映射（迁移用）

| 现状字段 | 目标 |
|----------|------|
| `booting/bootPhase/bootProgress/bootEpoch/livePush/connection/error` | ConnectionState |
| `projects` | ProjectIndexState |
| `conversations` + `conversationsStatusByProject` | ConversationListState(projectId) |
| `messagesByConversation` + `messageHistory*` + `detailStatus*`（消息部分） | TimelineState |
| `sessionsByConversation` + detail 中 sessions | InspectorState + SessionEntity |
| `projectSessionsByProject` + status | SessionListState + SessionEntity |
| `transcriptsByThread` + history + status | TranscriptState |
| `applyIngest/Manager/Conversation` | LiveIngress |
| `sendMessage/create*/resolve*` | Use-cases |
| `ui-store` | NavigationState（已接近） |
| `clis` / profiles RPC 缓存 | AgentsState |
| `attentionSessions` | Attention 派生或独立 |
| `actionError` | Connection 或 per-use-case result |

---

## 17. 不变量清单（验收）

1. 切换 project 不得显示上一 project 的 transcript（Navigation 清 thread 或 Sessions key 隔离）。  
2. Timeline 排序仅 `messageSeq`。  
3. Timeline 不含 session tool 流水。  
4. `needs_approval` 仅由 pending approval 路径抬升；manager running 不降级。  
5. ConversationList ready 前，header 对话数不以陈旧 summary 伪装。  
6. 同一 `threadId` 的 status 在 Inspector / Sessions / Attention 一致（同读 SessionEntity）。  
7. `livePush=true` 时无常态 2s 盲刷 list。  
8. Quiet re-list 保留 older 窗口。  
9. 导航 persist；业务列表/消息/transcript **不**作为真相持久化。  
10. View 不跨 slice 直接写缓存。

---

## 18. 建议落地顺序

| 阶段 | 内容 | 风险 |
|------|------|------|
| S0 | 本文规格评审 | — |
| S1 | 逻辑拆分：LiveIngress / use-case 模块化（仍可一个 store 后端） | 低 |
| S2 | Connection + ProjectIndex 独立 | 低 |
| S3 | Transcript + SessionEntity 独立 | 中（推送路径） |
| S4 | ConversationList / Timeline / Inspector family 化 | 中 |
| S5 | SessionList / Board 投影清理 | 低 |
| S6 | 删除上帝 Workspace 外观 API | 中 |

S1 可先做且不改 UI 行为，专降认知复杂度。

---

## 19. 术语表

| 术语 | 含义 |
|------|------|
| slice | 按 key 划分的状态单元（family） |
| hydrate | 针对 key 的首载/显式加载 |
| quiet | 不改 phase 为 loading 的后台对齐 |
| entity | 跨视图共享的身份状态 |
| projection | 从 entity/list 派生的只读视图模型 |
| use-case | 跨 slice 写编排 |
| LiveIngress | daemon 推送唯一入口 |

---

## 20. 修订记录

| 日期 | 说明 |
|------|------|
| 2026-07-21 | 初稿：按消费范围定义 L0–L6 与刷新契约 |
| 2026-07-21 | 增补 §21：缓存生命周期、加载策略、内存预算、回切无感 UX |
| 2026-07-21 | 增补 §22：CacheRuntime = CacheBudget + ensureLoaded/evict 一体规格 |

---

## 21. 逻辑层：加载、驻留、淘汰与回切体验

> 状态「按视图拆」解决的是 **谁读什么**。  
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
| **T1** | Focus-hot | 当前焦点钉住 | 当前 project 的 ConversationList；当前 conversation 的 Timeline+Inspector；当前 thread 的 Transcript；相关 SessionEntity |
| **T2** | Warm | 刚离开或 keep-alive 隐藏页，短 TTL | 同 project 最近 N 个 conversation 的 Timeline 窗口；最近 M 个 thread 的 Transcript 窗口；当前 project 的 SessionList |
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
| `Navigation.conversationId === C` | `Timeline(C)`；`Inspector(C)`；C 下全部 `SessionEntity` |
| `Navigation.threadId === T` | `Transcript(T)`；`SessionEntity(T)` |
| session `status ∈ {running, needs_approval}` | 该 `SessionEntity`；若 Transcript 已打开过可 pin 其 tail 窗口 |
| 进行中的 sendMessage / resolveApproval | 相关 Timeline + Entity 直至 use-case 结束 |

**Unpin**：条件不再满足后进入 T2，启动 idle TTL。

### 21.6 加载策略（When to load）

#### 21.6.1 触发模型（与 UI 交互序对齐）

```text
可见 / 选中
  → ensureLoaded(key, { reason })
      if 无数据且 phase!=loading → hydrate (show loading if blank)
      if 有数据且 stale/超龄 → revalidate quiet (keep painting cache)
      if 有数据且新鲜 → no-op（live 继续维护）
```

| UI 事件 | ensureLoaded |
|---------|----------------|
| 冷启动成功 | ProjectIndex；当前/默认 project 的 ConversationList |
| 选中 project | ConversationList(P)；若 projectView=sessions → SessionList(P) |
| 选中 conversation | Timeline(C)+Inspector(C)（openConversationDetail） |
| 选中 thread / 打开 Sessions 详情 | Transcript(T) |
| 切到 Sessions tab | SessionList(P)（若 idle） |
| 切到 Board | 依赖 ConversationList；不另拉 transcript |
| 上翻列表 | loadOlder（仅当前 key） |
| live 事件 | 增量写；不整表 hydrate（conversation dirty 除外） |

#### 21.6.2 窗口加载（永远先 tail）

| 资源 | 首载窗口 | older | 默认上限（单 key 内存） |
|------|----------|-------|------------------------|
| Timeline messages | 最近 **80**（`MESSAGE_PAGE_SIZE`） | prepend 页 80 | 建议硬顶 **~500** 条/对话，超出从 **更旧端** 丢弃并 `hasOlder=true` |
| Transcript items | 最近 **~400** events | prepend | 建议硬顶 **~2000** items/thread 或 **~8–16MB** 估重，超出丢旧端 |
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
| Transcript 窗口 | **最多 3** 个 threadId（+ 所有 running/needs_approval 可额外 pin） | 5 min | LRU |
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
| 用户点 Sessions 但未选 thread | 只 SessionList，不预取全部 Transcript | — |
| 发送消息后 | 该 thread 进入关注；Transcript 若在 Sessions 可预热 | 中 |

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
| Timeline(C) | 选中 C | T1 当 C 当前 | SWR + 保滚动（keep-alive/DOM 或 scroll meta） | LRU 5 + trim 500 | conversation dirty quiet |
| Inspector(C) | 随 Timeline | 同 C | 直接画 Entity | 随 list | manager/ingest |
| SessionList(P) | 进 Sessions | T1/T2 | SWR | LRU | threadAdded / quiet |
| Transcript(T) | 选中 T | T1；active 额外 pin | SWR | LRU 3 + trim | ingest |
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
thread 已 evict transcript 后仍收到 ingest → **仍更新 SessionEntity**；Transcript 数据可忽略直至再次 ensureLoaded。

### 21.14 滚动位置与「内容还在」

数据缓存命中但滚动丢了，仍像「重新加载」。

| 状态 | 存哪 |
|------|------|
| Timeline scroll / follow 模式 | 优先 DOM keep-alive；或 `UIChromeState` per conversationId：`scrollTop` 或 `anchorMessageId` |
| Transcript 同上 | per threadId |
| List 滚动 | per projectId |

淘汰 **数据** 时：可保留廉价的 `scrollAnchor` meta（id + offset），再次 hydrate 后 restore。

### 21.15 与当前实现的差距（迁移提示）

| 现况 | 目标 |
|------|------|
| 单一 workspace 大 map，基本不 LRU | 引入 CacheController 预算 |
| tab keep-alive 已做 | 保留；补数据层 T1/T2 |
| Timeline/Transcript 分页已做 | 补 **硬顶 trim** |
| quiet re-list / bootEpoch 已做 | 形式化为 SWR 规则 |
| 无统一 lastAccess/pin | 增加 meta |
| active session transcript 可能长期堆积 | pin + trim + Entity 分离 |

### 21.16 一句话策略

> **按焦点购买窗口数据；按钉住与 LRU 管理内存；有缓存就先画再对齐；没缓存再 loading；用本机 tail RPC 保证冷启动也够快——用工作集模拟「数据一直在」，而不是把 daemon 全量镜像进前端。**

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
  inspector: {
    // 与 Timeline 同 key 生命周期；不单独 LRU
    followTimelineKey: true,
  },

  // ── SessionList ──
  sessionList: {
    maxProjectKeys: 2,
    unpinnedTtlMs: 15 * 60_000,
    revalidateAfterMs: 30_000,
  },

  // ── Transcript ──
  transcript: {
    maxThreadKeys: 3,                  // 另加 active pin 可超出
    pageEvents: 400,                   // TRANSCRIPT_PAGE_EVENTS
    hardMaxItems: 2000,
    // 可选体积顶：超过则 trim（实现可用粗估）
    hardMaxApproxBytes: 12 * 1024 * 1024,
    unpinnedTtlMs: 5 * 60_000,
    revalidateAfterMs: 10_000,
    peekTailForApproval: 120,          // openConversationDetail elevate peek
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
| active pin 规则 | 覆盖 maxKeys：active transcript 可导致线程数 > `maxThreadKeys` |

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
| `ENSURE_LOADED { key, kind, reason }` | View 可见 / Navigation 选中 |
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
  T = Navigation.threadId
  pin ConversationList(P), SessionList(P) if exists
  pin Timeline(C), Inspector(C) if C
  pin Transcript(T) if T
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

```text
on LIVE_MERGE(key, patch):
  entry = get(key)
  if entry == null || entry.phase == idle || entry.data == null:
    // Transcript 已淘汰：只更新 SessionEntity（Ingress 上层处理）
    return
  entry.data = mergeLive(entry.data, patch)
  entry.lastSyncAtMs = now()
  entry.stale = false
  trimWindow(entry, kind)
  touch(key)                                    // 活跃流延长 TTL
```

conversation dirty：

```text
MARK_STALE(timeline key) 或直接 ensureLoaded(reason=swr)
→ quiet revalidate（ready 路径）
```

---

### 22.9 一体验收清单

| # | 用例 | Budget 点 | 状态机点 |
|---|------|-----------|----------|
| 1 | 打开对话 | pageSize=80 | idle→loading→ready |
| 2 | 快切对话 | — | 旧 gen 丢弃 |
| 3 | 来回切已访问对话 | maxKeys 内 hit | ready 无闪白 |
| 4 | 打开第 6 个对话 | maxKeys=5 | 最久未用非 pin 被 evict |
| 5 | 超长上翻 | hardMax | trim 最旧，仍 ready |
| 6 | running agent 离开 transcript | maxThreadKeys | Entity pin；transcript 可按策略 pin/trim |
| 7 | 断线重连 | — | DROP_ALL 或 MARK_STALE + bootEpoch ensure |
| 8 | quiet 失败 | — | 仍显示旧 data |
| 9 | 双 ensure 同 key | — | single-flight |
| 10 | 调小 Budget 单测 | 注入 maxKeys=1 | 第二次打开挤出第一次 |

---

### 22.10 实现落点建议

| 单元 | 职责 |
|------|------|
| `cache-budget.ts` | 仅常量 `CacheBudget` |
| `cache-runtime.ts` | ensureLoaded / onResult / evict / trim / pins |
| 各 slice store | 存 Entry；委托 Runtime；不写私有 LRU |
| View | 只 `ensureLoaded` + 读 entry 展示 |
| 测试 | Runtime 单测不挂 React：注入 fake clock + budget + fake rpc |

---

### 22.11 一句话

> **CacheRuntime = CacheBudget（能留多大）× 状态机（怎么进出与防写花）**；  
> 二者同一模块交付，同一套验收，缺一不可。
