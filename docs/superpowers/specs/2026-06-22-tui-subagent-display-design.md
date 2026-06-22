# TUI Subagent 全量展示设计

> 日期: 2026-06-22
> 状态: 待审核
> 类型: TUI + 协议层 + 运行时层功能增强
> 关联: `2026-06-18-conversation-centric-hierarchy-design.md`（Agent Sessions 列表）

## 1. 背景与动机

### 1.1 问题：Subagent 不可见

当前 TUI 中，当主 agent（codex/opencode）调用 subagent 时，用户只能看到一个普通的 tool call 行（`Tool Task · running task=xxx`），长时间看不到 subagent 的执行情况。Subagent 的完整会话内容无处查看。

### 1.2 根因

1. **协议层**：`UiEventMessage` 没有 subagent 生命周期事件。Codex 的 `CollabAgentToolCall` item 在 `item/started` 时落入 `Raw` 逃逸（`codex.rs:217`），在 `item/completed` 时被**静默丢弃**（`codex.rs:524` 的 `_ => Vec::new()`）。
2. **运行时层**：`AgentManager` 只追踪用户显式启动的顶层线程。Codex app-server 通知流中包含 subagent 线程事件，但 pump 未注册这些线程。Opencode SSE pump 的 `resolve_thread_id` **主动丢弃**所有未注册 sessionID 的事件（`opencode_driver.rs:341`）。
3. **TUI 层**：`ChatItem` 没有 subagent 概念——一切是扁平的 `ToolCall`。Sidebar 没有父子线程层级。

### 1.3 目标

- Subagent 在 sidebar（Agent Sessions 列表）中作为嵌套子条目展示，标记为 subagent，用户可切换查看其完整实时 transcript
- 主 agent 会话记录中，subagent 调用展示为专属卡片（包含 subagent 标记、模型、prompt、状态），取代当前的 `Tool Task` 行
- 全量支持 codex 和 opencode 的 subagent

### 1.4 范围

本设计覆盖：
- `minos-ui-protocol`：新增 subagent 生命周期事件 + codex/opencode 翻译器扩展
- `minos-agent-runtime`：线程父子关系跟踪 + codex subagent 事件路由 + opencode subagent session 自动注册
- `minos-tui`：数据模型、翻译、sidebar 树状渲染、卡片渲染、导航

不覆盖：claude/gemini 的 subagent（架构上预留，但不实现翻译器）。

## 2. 协议层设计 (`minos-ui-protocol`)

### 2.1 新增 `UiEventMessage` 变体

```rust
// message.rs
pub enum UiEventMessage {
    // ... existing variants unchanged ...

    // ── Subagent lifecycle ──────────
    SubagentSpawned {
        parent_thread_id: String,
        sub_thread_id: String,
        tool_call_id: String,       // 关联主 agent transcript 中的卡片
        agent: AgentName,
        model: Option<String>,      // codex: CollabAgentToolCall.model
        prompt: Option<String>,     // codex: CollabAgentToolCall.prompt
        title: Option<String>,      // codex: agent_nickname / agent_role
    },
    SubagentStatusUpdated {
        sub_thread_id: String,
        status: SubagentStatus,
    },
    SubagentClosed {
        sub_thread_id: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}
```

### 2.2 Subagent transcript 事件流

Subagent 自己的会话内容使用**现有** `UiEventMessage` 变体，以 `sub_thread_id` 为 key 流动：
- `ThreadOpened { thread_id: sub_thread_id, agent, ... }`
- `MessageStarted { ... }`, `TextDelta { ... }`, `ToolCallPlaced { ... }` 等
- `ThreadClosed { thread_id: sub_thread_id, ... }`

TUI 的 `ChatState` 系统无需修改即可处理 subagent transcript——每个 subagent 在 `chat_states: HashMap<String, ChatState>` 中有自己的条目。

### 2.3 Codex 翻译器扩展 (`codex.rs`)

**`CodexTranslatorState` 新增字段：**
```rust
pub struct CodexTranslatorState {
    // ... existing ...
    emitted_subagent_ids: HashSet<String>,  // 去重已发射 SubagentSpawned 的 collab item
}
```

**`translate()` 的 `item/started` 分支：**
- 新增 `"collabAgentToolCall"` arm：解析 `receiverThreadIds`、`model`、`prompt`、`tool`、`status`。若 status 为 `inProgress` 且未发射过，对每个 `receiverThreadId` 发射 `SubagentSpawned`。记录到 `emitted_subagent_ids`。

**`translate_item_completed()` 的新增 arm：**
- `"collabAgentToolCall"`：解析最终 `status`。发射 `SubagentStatusUpdated`（+ 若 terminal 则 `SubagentClosed`）。补发 `SubagentSpawned`（若 item/started 未处理到的补丁路径）。

### 2.4 Opencode 翻译器扩展 (`opencode.rs`)

**`OpencodeTranslatorState` 新增字段：**
```rust
pub struct OpencodeTranslatorState {
    // ... existing ...
    /// 跟踪 pending 的 Task 工具调用，用于 parent 归属
    pending_task_tools: HashMap<String, PendingTaskTool>,  // key: parent opencode session id
}

struct PendingTaskTool {
    tool_call_id: String,
    subagent_type: Option<String>,
    prompt: Option<String>,
    parent_thread_id: String,
}
```

当检测到 `session.created` 事件且其 sessionID 不在已知集合中时（由 driver 层自动注册后通知翻译器），发射 `SubagentSpawned`。Model 信息从 opencode session 的 `model` 字段提取（如可用）。

## 3. 运行时层设计 (`minos-agent-runtime`)

### 3.1 线程父子关系跟踪

**`AgentManager` 新增字段：**
```rust
pub struct AgentManager {
    // ... existing ...
    /// sub_thread_id → parent_thread_id
    subagent_parents: Arc<Mutex<HashMap<String, String>>>,
    /// parent_thread_id → Vec<sub_thread_id>（有序）
    subagent_children: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// sub_thread_id → SubagentMeta
    subagent_meta: Arc<Mutex<HashMap<String, SubagentMeta>>>,
}

struct SubagentMeta {
    agent: AgentKind,
    model: Option<String>,
    prompt: Option<String>,
    status: SubagentStatus,
}
```

**新增方法：**
```rust
impl AgentManager {
    pub async fn register_subagent(&self, parent: &str, sub: &str, agent: AgentKind, model: Option<String>, prompt: Option<String>) { ... }
    pub async fn list_subagents(&self, parent_thread_id: &str) -> Vec<SubagentSummary> { ... }
    pub async fn update_subagent_status(&self, sub_thread_id: &str, status: SubagentStatus) { ... }
}
```

### 3.2 `ManagerEvent` 扩展

```rust
pub enum ManagerEvent {
    // ... existing ...
    SubagentThreadAdded {
        parent_thread_id: String,
        sub_thread_id: String,
        agent: AgentKind,
        model: Option<String>,
        prompt: Option<String>,
    },
    SubagentThreadClosed {
        sub_thread_id: String,
    },
}
```

### 3.3 Codex event pump 路由 (`event_pump_loop`)

**当前问题：** pump 使用 `params.threadId` 路由事件到已知线程。Subagent 事件携带 sub_thread_id，但该线程未注册。

**两阶段注册方案：**

**Phase 1 — 父线程收到 CollabAgentToolCall：**
```
pump 收到: item/started, params.threadId = parent_thread_id, item.type = collabAgentToolCall
  → translator 发射 SubagentSpawned { parent_thread_id, sub_thread_id = receiverThreadIds[0], model, prompt }
  → pump 调用 register_subagent()，在 self.threads 注册 ThreadHandle
  → pump 发射 ManagerEvent::SubagentThreadAdded
  → pump 为 parent_thread_id 创建 RawIngest（含 SubagentSpawned）
```

**Phase 2 — Subagent 自己的事件到达：**
```
pump 收到: thread/started, params.threadId = sub_thread_id
  → pump 在 self.threads 找到 sub_thread_id（Phase 1 已注册）
  → translator 正常处理 → ThreadOpened { thread_id: sub_thread_id }
  → pump 为 sub_thread_id 创建 RawIngest
```

**竞态处理（推测性注册）：**

Subagent 的 `thread/started` 可能在父线程的 `CollabAgentToolCall` item/started **之前**到达。处理策略：
- pump 遇到未知 `threadId` 时，**推测性注册**：创建 ThreadHandle，标记为 orphan（parent_thread_id = None）
- Orphan 线程**不发射** `ManagerEvent::SubagentThreadAdded`（因为 parent 未知），也不出现在 sidebar
- 当后续 `CollabAgentToolCall` 到达时，通过 `receiverThreadIds` 匹配 orphan，此时才：
  1. 设置 parent_thread_id
  2. 发射 `ManagerEvent::SubagentThreadAdded`
  3. 在父线程的 ingest 中补发 `SubagentSpawned`
- 若 orphan 线程在 30s 内未被关联到任何 parent，清理它并丢弃已缓冲的事件

### 3.4 Opencode SSE pump 自动注册 (`spawn_sse_pump`)

**当前：** `resolve_thread_id` 对未知 sessionID 返回 `None` → 事件被丢弃。

**修改后：**
```rust
async fn resolve_thread_id_or_register(
    payload: &Value,
    session_map: Arc<Mutex<HashMap<String, String>>>,
    pending_tasks: Arc<Mutex<HashMap<String, PendingTaskTool>>>,
    discovery_tx: &mpsc::Sender<SubagentDiscovery>,
) -> Option<String> {
    let session_id = extract_session_id(payload)?;
    let map = session_map.lock().await;

    // 已知顶层 session → 返回其 thread_id
    if let Some(tid) = map.iter().find_map(|(t, s)| (s == &session_id).then(|| t.clone())) {
        return Some(tid);
    }
    drop(map);

    // 未知 session → 尝试归属到 pending Task 工具调用的 session
    let parent = pending_tasks.lock().await.values()
        .find(|pt| pt.is_active())
        .map(|pt| pt.parent_thread_id.clone())?;

    let sub_thread_id = format!("opencode-sub-{}", &session_id[..8.min(session_id.len())]);
    session_map.lock().await.insert(sub_thread_id.clone(), session_id.clone());

    discovery_tx.send(SubagentDiscovery {
        sub_thread_id,
        parent_thread_id: parent,
        provider_session_id: session_id,
    }).await.ok()?;

    Some(sub_thread_id)
}
```

**Parent 归属启发式：** 追踪每个 workspace 中最近一个处于 `pending`/`running` 状态的 Task 工具调用。当未知 sessionID 出现时，归属到它。

**Pending Task 跟踪：** opencode driver 在 SSE pump 中解析 `message.part.updated` 事件中 `part.type == "tool"` 且 `part.tool == "task"` 的 part，记录其 sessionID 和状态。当状态变为 `completed`/`error` 时清除。

### 3.5 Opencode driver → manager 通知

Opencode driver 通过新的 `mpsc::Sender<SubagentDiscovery>` 将发现的 subagent 通知 manager。Manager 在收到通知后：
1. 调用 `register_subagent()`
2. 创建 `ThreadHandle`（状态 Starting）
3. 发射 `ManagerEvent::SubagentThreadAdded`
4. 后续该 sessionID 的 SSE 事件自动路由到 sub_thread_id

## 4. TUI 层设计 (`minos-tui`)

### 4.1 `ChatItem` 新增变体

```rust
// translation/chat_item.rs
pub enum ChatItem {
    // ... existing ...

    /// 主 agent 会话中的 subagent 调用卡片
    SubagentCall {
        message_id: String,
        tool_call_id: String,         // 关联 SubagentSpawned 事件
        sub_thread_id: String,        // 用于导航跳转
        agent: AgentName,
        model: Option<String>,
        prompt_summary: String,       // 首行 / 前 N 字符
        prompt_detail: Option<String>, // 完整 prompt（展开时）
        status: SubagentStatus,
        is_expanded: bool,
    },
}
```

`message_id()` 和 `set_streaming()` 需覆盖新变体。

### 4.2 `UiState` 新增字段

```rust
// ui/mod.rs
pub struct UiState {
    // ... existing ...
    /// parent_thread_id → Vec<sub_thread_id>
    pub subagent_children: HashMap<String, Vec<String>>,
    /// sub_thread_id → parent_thread_id
    pub subagent_parents: HashMap<String, String>,
    /// sub_thread_id → SubagentInfo（模型、prompt、状态）
    pub subagent_info: HashMap<String, SubagentInfo>,
}

pub struct SubagentInfo {
    pub agent: AgentName,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub status: SubagentStatus,
}
```

### 4.3 `ChatState` 翻译扩展 (`translation/chat_state.rs`)

`apply_ui_event` 新增分支：

- **`SubagentSpawned`：**
  1. 在**父线程**的 ChatState 中创建 `ChatItem::SubagentCall`
  2. 为 `sub_thread_id` 立即创建一个空的 `ChatState`（防止后续 ingest 帧被丢弃）
  3. 更新 `subagent_children` / `subagent_parents` / `subagent_info`

- **`SubagentStatusUpdated`：**
  1. 更新父线程 ChatState 中对应 `SubagentCall` 的 `status`
  2. 更新 `subagent_info`

- **`SubagentClosed`：** 更新 status 为 terminal

- **正常事件 keyed by `sub_thread_id`：** 路由到 sub 的 ChatState（现有逻辑不变，只要 ChatState 已存在）

### 4.4 Ingest 路由调整 (`lifecycle.rs`)

`handle_ingest` 当前逻辑：如果 `chat_states` 中没有该 thread_id 的条目，则丢弃。

**修改：** 保持不变。但确保 `SubagentSpawned` 处理时**先创建** sub_thread_id 的 ChatState，这样后续 ingest 帧不会被丢弃。

ManagerEvent 处理（`handle_manager_event`）新增：
- `SubagentThreadAdded`：在 `ui.threads` 中添加 `ThreadEntry`（标记 is_subagent），更新 `subagent_children`
- `SubagentThreadClosed`：更新 ThreadState 为 Closed

### 4.5 Sidebar 树状渲染 (`ui/conversation_detail.rs`)

`AgentSessionListRenderable` 扩展为展示嵌套结构：

```
 Agent Sessions
 ─────────────────
 > codex #a1b2c3d4
   └ codex #e5f6g7h8 · gpt-4.1 · ⠋
   └ codex #i9j0k1l2 · gpt-4.1 · ✓
   opencode #m3n4o5p6
   └ opencode #q7r8s9t0 · ⠋
```

**渲染逻辑：**
- 遍历 `conversation_agent_sessions`（顶层 agent session）
- 对每个顶层 session，查 `subagent_children` 获取子条目
- 子条目缩进前缀 `  └ `，显示 agent bin name、短 thread id、模型（dimmed）、状态指示器
- 状态指示器：`⠋`（running，可旋转动画）、`✓`（done，绿色）、`✗`（failed，红色）

**导航：** 将树扁平化为一个 `Vec<FlatEntry { thread_id, depth, is_subagent }>`。Up/Down 在扁平列表中移动。现有 `agent_list_key_to_mapping` 只需改为操作扁平列表索引。

### 4.6 Subagent 卡片渲染 (`ui/chat.rs`)

`build_item_lines` 新增 `ChatItem::SubagentCall` 分支：

```
 Subagent · codex · gpt-4.1 · running
 ┐ Prompt: Refactor the parser module to use...
 └ [Press Enter to view subagent session →]
```

**视觉设计：**
- `Subagent` 标签用紫色/品红色（区别于 `Tool` 的默认色）
- Model 名 dimmed
- 状态：`running`（带旋转动画字符）、`done`（绿色 ✓）、`failed`（红色 ✗）
- Prompt 摘要：首行截断到终端宽度
- 展开时：显示完整 prompt
- **Enter 键动作：** 导航到 `NavLevel::AgentDetail { thread_id: sub_thread_id }`

### 4.7 导航

现有 `NavLevel::AgentDetail { thread_id, ... }` 无需修改。新增：
- 从 sidebar 选中 subagent 条目按 Enter → push `AgentDetail { sub_thread_id }`
- 从 transcript 中 SubagentCall 卡片按 Enter → 同上
- Back → pop 回父 agent 的 `AgentDetail`

**ChatState 水合：** 导航进入 subagent 时，调用 `read_thread_raw_history(sub_thread_id, ...)` 拉取历史。

- **Daemon 模式：** 所有 subagent 事件（含 `SubagentSpawned`）作为 `RawIngest` 行持久化到 SQLite，重放时通过翻译器重建 `ChatState`。无需特殊处理。
- **Embedded 模式：** `read_thread_raw_history` 返回空（现有 stub 行为，对顶层线程也一样）。subagent transcript 仅通过实时事件积累，不支持重连恢复。这与顶层线程行为一致。

## 5. Backend Trait 扩展 (`backend/mod.rs`)

### 5.1 新增 trait 方法

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync {
    // ... existing ...

    async fn list_subagents(&self, parent_thread_id: &str) -> Result<Vec<SubagentSummary>>;
}

#[derive(Debug, Clone)]
pub struct SubagentSummary {
    pub sub_thread_id: String,
    pub parent_thread_id: String,
    pub agent: AgentName,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub status: SubagentStatus,
}
```

`EmbeddedBackend` 和 `DaemonBackend` 均通过 `AgentManager::list_subagents()` 实现。

### 5.2 Daemon JSON-RPC

新增 RPC 方法 `minos_local_list_subagents`，参数为 `parent_thread_id`，返回 `Vec<SubagentSummary>`。

## 6. 文件变更清单

### `minos-ui-protocol`
| 文件 | 变更 |
|------|------|
| `src/message.rs` | 新增 `SubagentSpawned`、`SubagentStatusUpdated`、`SubagentClosed` 变体 + `SubagentStatus` enum |
| `src/codex.rs` | `CodexTranslatorState` 加 `emitted_subagent_ids`；`translate()` 加 `collabAgentToolCall` arm；`translate_item_completed()` 加 `collabAgentToolCall` arm |
| `src/opencode.rs` | `OpencodeTranslatorState` 加 pending task 跟踪；subagent session 检测逻辑 |

### `minos-agent-runtime`
| 文件 | 变更 |
|------|------|
| `src/manager.rs` | `AgentManager` 加 `subagent_parents`/`subagent_children`/`subagent_meta` 字段 + `register_subagent`/`list_subagents`/`update_subagent_status` 方法；`event_pump_loop` 加 subagent 路由 + 推测性注册 |
| `src/manager_event.rs` | 新增 `SubagentThreadAdded`、`SubagentThreadClosed` 变体 |
| `src/opencode_driver.rs` | `resolve_thread_id` → `resolve_thread_id_or_register`；新增 `SubagentDiscovery` channel + pending task 跟踪 |
| `src/store_facing.rs` | `ThreadSnapshot` 可选加 `is_subagent`/`parent_thread_id` 字段 |

### `minos-tui`
| 文件 | 变更 |
|------|------|
| `src/translation/chat_item.rs` | 新增 `SubagentCall` 变体；`message_id()`/`set_streaming()` 覆盖新变体 |
| `src/translation/chat_state.rs` | `apply_ui_event` 加 `SubagentSpawned`/`StatusUpdated`/`Closed` 分支 |
| `src/translation/tool_summary.rs` | 可选：为 SubagentCall 提供 prompt 摘要辅助函数 |
| `src/ui/mod.rs` | `UiState` 加 `subagent_children`/`subagent_parents`/`subagent_info`；`ThreadEntry` 加 `is_subagent` 标记 |
| `src/ui/conversation_detail.rs` | `AgentSessionListRenderable` 支持树状渲染 + 扁平化导航 |
| `src/ui/chat.rs` | `build_item_lines` 加 `SubagentCall` 渲染分支 |
| `src/app/lifecycle.rs` | `handle_manager_event` 加 `SubagentThreadAdded`/`Closed` 分支；ingest 路由验证 |
| `src/app/event_mapping.rs` | `agent_list_key_to_mapping` 改为扁平化索引 |
| `src/app/event_loop.rs` | `execute_effect` 处理 subagent 导航 Effect |
| `src/backend/mod.rs` | trait 加 `list_subagents`；新增 `SubagentSummary` |
| `src/backend/embedded.rs` | 实现 `list_subagents` |
| `src/backend/daemon.rs` | 实现 `list_subagents` via RPC |

### `minos-protocol`（如有 daemon RPC schema）
| 文件 | 变更 |
|------|------|
| daemon RPC 定义 | 新增 `minos_local_list_subagents` 方法 |

## 7. 实施阶段

### Phase 1: 协议层（无运行时依赖）
1. `UiEventMessage` 新增变体 + `SubagentStatus` enum
2. Codex 翻译器：`collabAgentToolCall` arm
3. 单元测试：翻译器输入/输出验证

### Phase 2: 运行时层（依赖 Phase 1）
1. `AgentManager` 父子关系跟踪 + `ManagerEvent` 扩展
2. Codex event pump subagent 路由 + 推测性注册
3. Opencode SSE pump 自动注册
4. 单元测试：注册/查询/状态更新逻辑

### Phase 3: TUI 数据模型与翻译（依赖 Phase 1-2）
1. `ChatItem::SubagentCall` + `UiState` 新字段
2. `ChatState::apply_ui_event` 新分支
3. Ingest 路由 + ManagerEvent 处理
4. 单元测试：事件→ChatItem 翻译

### Phase 4: TUI 渲染与交互（依赖 Phase 3）
1. Sidebar 树状渲染 + 扁平化导航
2. SubagentCall 卡片渲染
3. 导航跳转（sidebar + card → AgentDetail）
4. 手动集成验证

### Phase 5: Backend trait + Daemon RPC（依赖 Phase 2）
1. `list_subagents` trait 方法
2. Daemon RPC 方法
3. Embedded/Daemon backend 实现

## 8. 测试计划

### 单元测试（隔离逻辑）

**协议层：**
- Codex 翻译器：`collabAgentToolCall` item/started JSON → `SubagentSpawned`（验证 model/prompt/thread_ids 提取）
- Codex 翻译器：`collabAgentToolCall` item/completed → `SubagentStatusUpdated` + `SubagentClosed`
- 序列化往返：`SubagentSpawned` → JSON → `SubagentSpawned`
- Opencode 翻译器：自动注册 session 的 `session.created` → `SubagentSpawned`

**运行时层：**
- 父子映射：`register_subagent` → maps 正确更新
- Opencode parent 归属：pending Task + 未知 sessionID → 正确 parent

**TUI 翻译：**
- `SubagentSpawned` → `ChatItem::SubagentCall` 在父线程 items 中 + sub ChatState 创建
- `SubagentStatusUpdated` → status 更新
- sub_thread_id 的 ingest 帧 → 路由到 sub ChatState

**TUI 渲染：**
- `SubagentCall` → 正确 label/model/status/prompt 行
- Sidebar 树 → 缩进 + 扁平化索引正确

### 集成验证（手动）
- Codex：触发 subagent → sidebar 实时显示 → 切换查看 transcript → 卡片显示 model/prompt/status
- Opencode：触发 Task tool → subagent 自动注册 → sidebar 显示 → transcript 可查看
- 从卡片 Enter 跳转到 subagent detail → Back 返回父 agent
- Subagent 完成后状态指示器更新

## 9. 设计决策记录

| 决策 | 选择 | 理由 |
|------|------|------|
| 协议建模方式 | 新增 `UiEventMessage` 变体 | 清晰的父子关系语义，符合架构分层 |
| Sidebar 展示 | 嵌套树状 | 直观表达从属关系 |
| Codex 事件获取 | 订阅全局通知流 | codex 已广播所有线程事件，只需停止忽略 |
| Opencode 事件获取 | 自动注册未知 sessionID | SSE 全局流已包含事件，只需停止丢弃 |
| Opencode parent 归属 | 基于最近 pending Task 工具 | opencode 不暴露 parent sessionID，启发式是最佳选择 |
| 竞态处理 | 推测性注册 | 避免缓冲复杂性，orphan 线程在超时后清理 |
| 主 agent 卡片 | 专属 `SubagentCall` 变体 | 区别于普通 tool call，展示 model/prompt/status |
