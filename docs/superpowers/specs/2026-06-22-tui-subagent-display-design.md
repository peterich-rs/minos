# TUI Subagent 全量展示设计

> 日期: 2026-06-22
> 状态: 待审核
> 类型: TUI + 协议层 + 运行时层 + 本地持久化功能增强
> 关联: `2026-06-18-conversation-centric-hierarchy-design.md`（Agent Sessions 列表）

## 1. 背景与动机

### 1.1 问题：Subagent 不可见

当前 TUI 中，当主 agent（codex/opencode）调用 subagent 时，用户只能看到一个普通的 tool call 行（`Tool Task · running task=xxx`），长时间看不到 subagent 的执行情况。Subagent 的完整会话内容无处查看。

### 1.2 根因

1. **协议层**：`UiEventMessage` 没有 subagent 生命周期事件。Codex 的 `CollabAgentToolCall` item 在 `item/started` 时落入 `Raw` 逃逸（`codex.rs:217`），在 `item/completed` 时被**静默丢弃**（`codex.rs:524` 的 `_ => Vec::new()`）。
2. **运行时 / 持久化层**：`AgentManager` 只显式追踪用户启动的顶层线程。Codex pump 对未知 provider thread id 会回退用原 id 转发，但没有 `ThreadHandle` / SQLite `threads` 父行时，daemon `events.thread_id` 外键会拒绝持久化；TUI 也会因为没有 `ChatState` 丢弃该 thread 的 ingest。Opencode SSE pump 的 `resolve_thread_id` 会主动丢弃所有未注册 sessionID 的事件（`opencode_driver.rs:341`）。
3. **TUI 层**：`ChatItem` 没有 subagent 概念——一切是扁平的 `ToolCall`。Sidebar 没有父子线程层级，也没有只读 subagent transcript 入口。

### 1.3 目标

- Subagent 在 sidebar（Agent Sessions 列表）中作为嵌套子条目展示，标记为 subagent，用户可切换查看其完整实时 transcript
- 主 agent 会话记录中，subagent 调用展示为专属卡片（包含 subagent 标记、模型、prompt、状态），取代当前的 `Tool Task` 行
- Codex subagent 可靠支持；Opencode subagent 在可从 Task 工具调用归属到未知 sessionID 时 best-effort 展示
- 本期只解决“可读、可观测”。不支持给 subagent 发送输入、`@agent#short` 路由到 subagent、关闭/删除 subagent、或把 subagent 当作可主动驱动的 agent run

### 1.4 范围

本设计覆盖：
- `minos-ui-protocol`：新增 subagent 生命周期事件 + codex/opencode 翻译器扩展
- `minos-agent-runtime`：线程父子关系注册 + codex subagent 事件缓冲/路由 + opencode subagent session best-effort 自动注册
- `minos-daemon` / `minos-protocol`：本地 SQLite `threads` 父子关系持久化 + thread summary 数据形状扩展
- `minos-tui`：数据模型、翻译、sidebar 树状渲染、卡片渲染、只读导航

不覆盖：claude/gemini 的 subagent。原因：Claude CLI 的 `--output-format stream-json` 和 Gemini 的 ACP v1 协议均**不暴露 subagent 的实时事件流**。Claude 的 Task tool 执行是 CLI 内部黑盒（`tool_use: Task` → 静默 → `tool_result`）；Gemini 的 ACP 是单 session 协议，无 spawn 子 agent 原语。两者均无可订阅的子 transcript 事件流。Claude Code 自己的 TUI 能看到 subagent 实时数据是因为它内嵌在进程内，不经过 NDJSON stdout——这是 CLI 外部接入的根本限制，不是 Minos 的解析问题。Claude/Gemini 的 Task/subagent tool call 保持现有 `ToolCall` 展示不变。

### 1.5 设计原则

- Runtime 仍只广播 `RawIngest`，不能依赖 `minos-ui-protocol` translator 输出做线程注册。需要注册 subagent 时，runtime 直接轻量解析 provider raw payload。
- Subagent transcript 不能先广播、后补线程父行。必须先注册 `ThreadHandle` 并持久化 `threads` 行，再释放该 sub_thread_id 的 `RawIngest`，否则 daemon event writer 会因为外键失败丢事件。
- TUI 的 `ChatState` 只负责当前 thread transcript 投影，不跨线程创建或维护其他 `ChatState`。跨线程父子关系由 `App` / `UiState` 层维护。
- Subagent 是只读观察对象。Agent input、`@agent#short` 查找、关闭、删除等主动控制保持只支持顶层 agent session。

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
        title: Option<String>,      // optional display label; usually None until ThreadOpened/title update
    },
    SubagentStatusUpdated {
        sub_thread_id: String,
        status: SubagentStatus,
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

Subagent 自己的会话内容使用**现有** `UiEventMessage` 变体，以 `LocalIngestFrame.thread_id = sub_thread_id` / `RawIngest.thread_id = sub_thread_id` 为路由 key 流动：
- `ThreadOpened { thread_id: sub_thread_id, agent, ... }`
- `MessageStarted { ... }`, `TextDelta { ... }`, `ToolCallPlaced { ... }` 等
- `ThreadClosed { thread_id: sub_thread_id, ... }`

单个 `UiEventMessage` 的 message/tool 事件本身不携带 thread id，不能靠 `ChatState` 内部跨线程路由。TUI 必须在 `App::handle_ingest` 层按 frame thread_id 找到或懒创建对应 `ChatState`。

### 2.3 Codex 翻译器扩展 (`codex.rs`)

**`CodexTranslatorState` 新增字段：**
```rust
pub struct CodexTranslatorState {
    // ... existing ...
    emitted_subagent_ids: HashSet<(String, String)>,  // (collab item id, receiver thread id)
}
```

**`translate()` 的 `item/started` 分支：**
- 新增 `"collabAgentToolCall"` arm：解析 `senderThreadId`、`receiverThreadIds`、`model`、`prompt`、`tool`、`status`。若 status 为 `inProgress` 且未发射过，对每个 `receiverThreadId` 发射 `SubagentSpawned`。记录 `(item_id, receiver_thread_id)` 到 `emitted_subagent_ids`。
- `title` 不从 collab item 读取。Codex generated schema 中 `agentNickname` / `agentRole` 属于 `Thread`，不是 `CollabAgentToolCallThreadItem`；如果后续 subagent `thread/started` payload 暴露这些字段，则由 `ThreadOpened.title` 或 `ThreadTitleUpdated` 展示。

**`translate_item_completed()` 的新增 arm：**
- `"collabAgentToolCall"`：解析最终 `status`。发射 `SubagentStatusUpdated`。若 item/started 未处理到，则先补发 `SubagentSpawned`，再发终态更新。
- 不新增 `SubagentClosed`。终态由 `SubagentStatusUpdated` 表示，transcript 生命周期继续使用现有 `ThreadClosed`。

### 2.4 Opencode 翻译器扩展 (`opencode.rs`)

`OpencodeTranslatorState` 不负责 pending Task → unknown session 的归属。原因：translator 只看到已经由 SSE pump 路由到某个 Minos thread 的事件；未知 sessionID 在 driver 层解析前就可能被丢弃。

Opencode subagent 展示使用 runtime/driver 合成的 Minos raw event：

```json
{
  "type": "minos.subagent.spawned",
  "properties": {
    "parent_thread_id": "parent-thread",
    "sub_thread_id": "opencode-sub-...",
    "tool_call_id": "task-call-id",
    "model": "optional-model",
    "prompt": "optional-prompt"
  }
}
```

`opencode.rs` 新增对 `type == "minos.subagent.spawned"` 的 arm，投影为 `UiEventMessage::SubagentSpawned`。普通 opencode provider 事件仍按现有逻辑翻译。

## 3. 运行时层设计 (`minos-agent-runtime`)

### 3.1 线程父子关系跟踪与持久化

Subagent 必须作为普通 thread 被注册，只是带 `parent_thread_id`。这样：
- runtime 能为 subagent 建立 `ThreadHandle`
- daemon 能先插入 SQLite `threads` 父行，再写 `events`
- TUI / local RPC 能通过现有 thread/session 列表拿到完整层级

**`ThreadHandle` 新增字段：**
```rust
pub struct ThreadHandle {
    // ... existing ...
    pub parent_thread_id: Option<String>,
}
```

**SQLite `threads` 表新增字段：**

```sql
parent_thread_id TEXT REFERENCES threads(thread_id) ON DELETE CASCADE
```

Minos 当前处于主动开发阶段，不做旧 schema 兼容层；直接更新 canonical migration / schema / fixtures 到新形状。

**`ThreadSummary` / `BackendThreadSnapshot` 新增字段：**

```rust
pub struct ThreadSummary {
    // ... existing ...
    pub parent_thread_id: Option<String>,
}
```

`parent_thread_id.is_some()` 即表示 subagent，不单独增加 `is_subagent`。

### 3.2 `ManagerEvent` 扩展

不新增单独的 `SubagentThreadAdded`。Subagent 本质上也是 thread，复用 `ThreadAdded`，加可选父 id：

```rust
pub enum ManagerEvent {
    ThreadAdded {
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        parent_thread_id: Option<String>,
    },
    // ... existing variants unchanged ...
}
```

`LocalManagerEvent::ThreadAdded` 同步增加 `parent_thread_id`，TUI daemon mode 和 embedded mode 使用同一事件形状。

### 3.3 Codex event pump 路由 (`event_pump_loop`)

**当前问题：** pump 会把未知 provider thread id 回退为 logical thread id 并广播，但此时没有 `ThreadHandle` / SQLite `threads` 行 / TUI `ChatState`。结果不是 runtime 立即丢事件，而是后续持久化或 TUI 投影丢事件。

**注册原则：**
- parent collab item 事件可以立即广播，因为 parent thread 已存在
- sub_thread_id 的事件必须在 subagent 注册并持久化 parent row 后才能广播
- runtime 不能等待 translator 输出；它直接轻量解析 raw `item/started` / `item/completed` 中的 `collabAgentToolCall`

**Phase 1 — 父线程收到 CollabAgentToolCall：**
```
pump 收到: item/started, params.threadId = parent_thread_id, item.type = collabAgentToolCall
  → pump 直接解析 receiverThreadIds
  → 对每个 receiverThreadId 调用 register_subagent_thread(parent, sub)
  → 在 self.threads 注册 ThreadHandle { parent_thread_id: Some(parent) }
  → 发射 ManagerEvent::ThreadAdded { parent_thread_id: Some(parent), ... }
  → daemon manager-event bridge 用 parent 的 conversation_id/workspace 插入 subagent threads 行
  → pump 为 parent_thread_id 创建 RawIngest（含 SubagentSpawned）
```

`SubagentSpawned` 本身仍由 codex translator 从 parent collab item 投影出来，用于主 transcript 卡片和 TUI 元数据。

**Phase 2 — Subagent 自己的事件到达：**
```
pump 收到: thread/started, params.threadId = sub_thread_id
  → pump 在 self.threads 找到 sub_thread_id（Phase 1 已注册）
  → translator 正常处理 → ThreadOpened { thread_id: sub_thread_id }
  → pump 为 sub_thread_id 创建 RawIngest
```

**竞态处理（推测性注册）：**

Subagent 的 `thread/started` 可能在父线程的 `CollabAgentToolCall` item/started **之前**到达。处理策略：
- pump 遇到未知 `threadId` 时，不立即广播该 frame，也不写 SQLite
- 将未知 thread 的原始 notification 暂存在 per-instance orphan buffer（key = provider thread id）
- 当后续 `CollabAgentToolCall` 到达时，通过 `receiverThreadIds` 匹配 orphan，此时才：
  1. 注册 `ThreadHandle { parent_thread_id: Some(parent) }`
  2. 发射 `ManagerEvent::ThreadAdded`
  3. 等 daemon thread row 可用后，按原顺序释放 orphan buffer 中的 sub_thread_id RawIngest
- 若 orphan 线程在 30s 内未被关联到任何 parent，清理并丢弃已缓冲的事件，同时打结构化日志（thread_id、workspace、buffered_count）

### 3.4 Opencode SSE pump 自动注册 (`spawn_sse_pump`)

**当前：** `resolve_thread_id` 对未知 sessionID 返回 `None` → 事件被丢弃。

**修改后：**
```rust
async fn resolve_thread_id_or_register(
    payload: &Value,
    session_map: Arc<Mutex<HashMap<String, String>>>,
    pending_tasks: Arc<Mutex<HashMap<String, PendingTaskTool>>>,
    threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    manager_tx: &broadcast::Sender<ManagerEvent>,
    events_tx: &IngestSink,
    workspace: PathBuf,
) -> Option<String> {
    let session_id = extract_session_id(payload)?;
    let map = session_map.lock().await;

    // 已知顶层 session → 返回其 thread_id
    if let Some(tid) = map.iter().find_map(|(t, s)| (s == &session_id).then(|| t.clone())) {
        return Some(tid);
    }
    drop(map);

    // 未知 session → 尝试归属到 pending Task 工具调用的 session
    let pending_task = pending_tasks.lock().await.values()
        .find(|pt| pt.is_active())
        .cloned()?;
    let parent = pending_task.parent_thread_id.clone();

    let sub_thread_id = format!("opencode-sub-{}", &session_id[..8.min(session_id.len())]);
    session_map.lock().await.insert(sub_thread_id.clone(), session_id.clone());

    threads.lock().await.insert(
        sub_thread_id.clone(),
        ThreadHandle::new_subagent(
            sub_thread_id.clone(),
            workspace.clone(),
            AgentName::Opencode,
            Some(parent.clone()),
            Some(session_id.clone()),
        ),
    );
    manager_tx.send(ManagerEvent::ThreadAdded {
        thread_id: sub_thread_id.clone(),
        workspace: workspace.clone(),
        agent: AgentName::Opencode,
        parent_thread_id: Some(parent.clone()),
    }).ok()?;

    // Also emit a synthetic parent-thread event so TUI renders a SubagentCall card.
    emit_opencode_subagent_spawned(events_tx, &parent, &sub_thread_id, &pending_task).await.ok()?;

    Some(sub_thread_id)
}
```

**Parent 归属启发式：** 追踪每个 workspace 中处于 `pending`/`running` 状态的 Task 工具调用。当未知 sessionID 出现且只有一个 active Task 时，归属到它；存在多个候选时不注册，避免误挂 parent。

**Pending Task 跟踪：** opencode driver 在 SSE pump 中解析已经归属到 parent session 的 `message.part.updated` 事件：`part.type == "tool"` 且 `part.tool == "task"`。记录 parent opencode session id、parent Minos thread id、tool_call_id、prompt、状态。当状态变为 `completed`/`error` 或超过短 TTL 时清除。

**Best-effort 限制：**
- Opencode 当前未暴露明确的 parent session id → child session id 关系
- 多个 Task 并发时，最近 pending Task 可能误配；此时宁可不注册，也不要把 child transcript 挂错 parent
- 需要结构化日志记录归属决策：parent_thread_id、provider_session_id、tool_call_id、workspace、reason

### 3.5 Daemon 持久化

daemon 的 manager-event bridge 处理 `ThreadAdded { parent_thread_id: Some(parent), ... }` 时：
1. 查询 parent thread 的 `conversation_id`、`workspace_root`
2. 插入 subagent `threads` 行，`parent_thread_id = parent`
3. 不增加 conversation 的 `agent_session_count`（subagent 是顶层 agent run 的子项，不参与 conversation 主计数）
4. 后续该 sub_thread_id 的 ingest 才允许写入 `events`

`list_conversation_agent_sessions(conversation_id)` 返回该 conversation 下所有 thread，包括 subagent；TUI 按 `parent_thread_id` 渲染树。无需新增 `list_subagents` RPC。

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
    /// sub_thread_id → SubagentInfo（模型、prompt、状态）
    pub subagent_info: HashMap<String, SubagentInfo>,
}

pub struct SubagentInfo {
    pub agent: AgentName,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub status: SubagentStatus,
    pub tool_call_id: String,
}
```

`ThreadEntry` / `ThreadSummaryEntry` 增加：

```rust
pub parent_thread_id: Option<String>,
```

Sidebar 父子关系以 `ThreadSummaryEntry.parent_thread_id` 为准，不在 `UiState` 里再维护一份 `subagent_children` / `subagent_parents`，避免三份状态互相漂移。

### 4.3 `ChatState` 翻译扩展 (`translation/chat_state.rs`)

`apply_ui_event` 新增分支：

- **`SubagentSpawned`：**
  1. 在**当前父线程**的 ChatState 中创建或更新 `ChatItem::SubagentCall`
  2. 不创建其他 thread 的 `ChatState`
  3. 不更新 `UiState` 父子 map（这些由 `App` / `UiState` 层处理）

- **`SubagentStatusUpdated`：**
  1. 更新父线程 ChatState 中对应 `SubagentCall` 的 `status`
  2. 找不到对应卡片时静默忽略，等待历史重放或后续 `SubagentSpawned`

- **正常事件 keyed by `sub_thread_id`：** 不在 `ChatState` 内处理。`App::handle_ingest` 按 `LocalIngestFrame.thread_id` 找到 sub 的 `ChatState` 后再调用 `apply_ui_events`。

### 4.4 Ingest 路由调整 (`lifecycle.rs`)

`handle_ingest` 当前逻辑：如果 `chat_states` 中没有该 thread_id 的条目，则丢弃。

**修改：** 不再直接丢弃未知 thread ingest。对 `LocalIngestFrame`：
- 若 `chat_states` 没有该 `thread_id`，用 `frame.agent` 懒创建 `ChatState`
- 若 frame 中包含 `SubagentSpawned`，在 `UiState.subagent_info` 记录模型、prompt、状态、tool_call_id
- `SubagentStatusUpdated` 同步更新 `UiState.subagent_info`
- 再把 `frame.ui_events` 投影到当前 frame 的 `ChatState`

ManagerEvent 处理（`handle_manager_event`）新增：
- `ThreadAdded { parent_thread_id: Some(parent), ... }`：在 `ui.threads` 中添加 `ThreadEntry { parent_thread_id: Some(parent), ... }`，并懒创建 `ChatState`
- `ThreadClosed` / `ThreadStateChanged`：对 subagent 和顶层 thread 使用相同状态更新逻辑

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
- `conversation_agent_sessions` 包含顶层 session 和 subagent session
- 先取 `parent_thread_id.is_none()` 的顶层 session，保持现有排序
- 对每个顶层 session，查 `parent_thread_id == top.thread_id` 的子条目，按 `last_ts_ms` 或插入顺序展示
- 子条目缩进前缀 `  └ `，显示 agent bin name、短 thread id、模型（dimmed）、状态指示器
- 状态指示器：`⠋`（running，可旋转动画）、`✓`（done，绿色）、`✗`（failed，红色）

**导航：** 将树扁平化为一个 `Vec<FlatEntry { thread_id, depth, is_subagent }>`。`selected_agent_session` 在 Conversation / AgentDetail 层表示这个扁平列表索引，不再直接等于 `conversation_agent_sessions` 原数组索引。

需要提供统一 helper：
```rust
fn flat_conversation_agent_sessions(ui: &UiState) -> Vec<FlatAgentSession>;
fn selected_flat_agent_session(ui: &UiState) -> Option<FlatAgentSession>;
```

覆盖范围：
- 键盘 Up/Down
- 鼠标点击 sidebar
- Enter 从 sidebar 打开 selected flat entry
- `UiState::current_thread_id()` 在 Conversation / AgentDetail 层读取 selected flat entry

不覆盖：
- `@agent#short` 查找 subagent
- Agent input submit 给 subagent
- 删除/关闭 subagent

### 4.6 Subagent 卡片渲染 (`ui/chat.rs`)

`build_item_lines` 新增 `ChatItem::SubagentCall` 分支：

```
 Subagent · codex · gpt-4.1 · running
 ┐ Prompt: Refactor the parser module to use...
 └ Open from Agent Sessions: #e5f6g7h8
```

**视觉设计：**
- `Subagent` 标签用紫色/品红色（区别于 `Tool` 的默认色）
- Model 名 dimmed
- 状态：`running`（带旋转动画字符）、`done`（绿色 ✓）、`failed`（红色 ✗）
- Prompt 摘要：首行截断到终端宽度
- 展开时：显示完整 prompt
- 不在卡片上实现 Enter 跳转。当前 chat 视图没有 item selection，强行加会扩大交互面。用户从 sidebar 子条目进入 subagent transcript。

### 4.7 导航

现有 `NavLevel::AgentDetail { thread_id, ... }` 无需修改。新增：
- 从 sidebar 选中 subagent 条目按 Enter → push `AgentDetail { sub_thread_id }`
- Back → pop 回父 agent 的 `AgentDetail`
- 进入 subagent `AgentDetail` 后，Agent input 显示只读状态，不发送消息；`handle_submit` 对 subagent 直接 no-op 并提示 “Subagent transcript is read-only”

**ChatState 水合：** 导航进入 subagent 时，调用 `read_thread_raw_history(sub_thread_id, ...)` 拉取历史。

- **Daemon 模式：** 所有 subagent 事件（含 `SubagentSpawned`）作为 `RawIngest` 行持久化到 SQLite，重放时通过翻译器重建 `ChatState`。无需特殊处理。
- **Embedded 模式：** `read_thread_raw_history` 返回空（现有 stub 行为，对顶层线程也一样）。subagent transcript 仅通过实时事件积累，不支持重连恢复。这与顶层线程行为一致。

## 5. Backend 数据形状扩展

### 5.1 不新增 `list_subagents`

不新增 `AgentBackend::list_subagents()`，也不新增 `minos_local_list_subagents` RPC。原因：
- sidebar 已经通过 `list_conversation_agent_sessions(conversation_id)` 获取 agent session 列表
- subagent 是 conversation 下某个顶层 session 的子 thread，只需要在线程摘要里带 `parent_thread_id`
- 单独 RPC 会制造第二个数据源，TUI 还要同步 `conversation_agent_sessions` 和 `subagent_children`

### 5.2 `ThreadSummary` 扩展

```rust
pub struct ThreadSummary {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
    pub parent_thread_id: Option<String>,
}
```

`ThreadSummaryEntry::from_summary` 同步复制该字段。

### 5.3 `BackendThreadSnapshot` / manager event 扩展

`list_threads()` / manager live events 同步携带 `parent_thread_id`，用于 embedded mode 和 direct agent panel 的只读标识。

Subagent 不进入 `room_agent_mention_candidates()` 的 existing thread 候选，不参与 `@agent#short` 路由。

## 6. 文件变更清单

### `minos-ui-protocol`
| 文件 | 变更 |
|------|------|
| `src/message.rs` | 新增 `SubagentSpawned`、`SubagentStatusUpdated` 变体 + `SubagentStatus` enum |
| `src/codex.rs` | `CodexTranslatorState` 加 `emitted_subagent_ids`；`translate()` 加 `collabAgentToolCall` arm；`translate_item_completed()` 加 `collabAgentToolCall` arm |
| `src/opencode.rs` | 识别 runtime 合成的 `minos.subagent.spawned` raw event 并投影为 `SubagentSpawned` |

### `minos-agent-runtime`
| 文件 | 变更 |
|------|------|
| `src/thread_handle.rs` | `ThreadHandle` 加 `parent_thread_id: Option<String>` |
| `src/manager.rs` | `ThreadAdded` 携带 `parent_thread_id`；`event_pump_loop` 轻量解析 Codex collab item、注册 subagent、缓冲/释放 orphan events |
| `src/manager_event.rs` | `ThreadAdded` 加 `parent_thread_id: Option<String>` |
| `src/opencode_driver.rs` | `resolve_thread_id` → `resolve_thread_id_or_register`；driver 层 pending Task 跟踪 + synthetic parent RawIngest |
| `src/store_facing.rs` | `ThreadSnapshot` 加 `parent_thread_id` 字段 |

### `minos-daemon`
| 文件 | 变更 |
|------|------|
| `migrations/0001_initial.sql` | `threads` 表新增 `parent_thread_id`，更新索引/fixtures |
| `src/store/mod.rs` | thread row / list query / insert API 读写 `parent_thread_id`；subagent 不增加 conversation agent_session_count |
| `src/store/event_writer.rs` | 保持 events FK 等待逻辑，新增测试覆盖 subagent thread row 先写入 |
| `src/agent.rs` | manager event bridge 按 parent thread 的 conversation/workspace 插入 subagent thread row |
| `src/local_rpc.rs` | `LocalManagerEvent::ThreadAdded` 转发 `parent_thread_id` |

### `minos-tui`
| 文件 | 变更 |
|------|------|
| `src/translation/chat_item.rs` | 新增 `SubagentCall` 变体；`message_id()`/`set_streaming()` 覆盖新变体 |
| `src/translation/chat_state.rs` | `apply_ui_event` 加 `SubagentSpawned`/`StatusUpdated` 分支，只更新当前 thread transcript |
| `src/translation/tool_summary.rs` | 可选：为 SubagentCall 提供 prompt 摘要辅助函数 |
| `src/ui/mod.rs` | `UiState` 加 `subagent_info`；`ThreadEntry` / `ThreadSummaryEntry` 加 `parent_thread_id`；current thread selection 走 flat helper |
| `src/ui/conversation_detail.rs` | `AgentSessionListRenderable` 支持树状渲染 + 扁平化导航 |
| `src/ui/chat.rs` | `build_item_lines` 加 `SubagentCall` 渲染分支 |
| `src/app/lifecycle.rs` | `handle_ingest` 对未知 thread 懒创建 ChatState；处理 `parent_thread_id` manager event；subagent input 只读 |
| `src/app/event_mapping.rs` | Conversation sidebar Up/Down 使用扁平化索引 |
| `src/app/event_loop.rs` | `OpenAgentSession` 支持 subagent thread_id 历史水合 |
| `src/backend/mod.rs` | `BackendThreadSnapshot` / `ThreadSummaryEntry` 加 `parent_thread_id` |
| `src/backend/embedded.rs` | 复制 manager/list_threads 的 `parent_thread_id` |
| `src/backend/daemon.rs` | 从 RPC `ThreadSummary` 复制 `parent_thread_id` |

### `minos-protocol`
| 文件 | 变更 |
|------|------|
| `src/messages.rs` | `ThreadSummary` 加 `parent_thread_id` |
| `src/local_rpc.rs` | `LocalManagerEvent::ThreadAdded` 加 `parent_thread_id` |

## 7. 实施阶段

### Phase 1: 协议层（无运行时依赖）
1. `UiEventMessage` 新增 `SubagentSpawned` / `SubagentStatusUpdated` + `SubagentStatus` enum
2. Codex 翻译器：`collabAgentToolCall` arm
3. Opencode 翻译器：`minos.subagent.spawned` synthetic raw event arm
4. 单元测试：翻译器输入/输出验证

### Phase 2: 持久化与数据形状（依赖 Phase 1）
1. `threads.parent_thread_id` schema 更新（直接更新 canonical migration）
2. `ThreadSummary` / `LocalManagerEvent::ThreadAdded` / `BackendThreadSnapshot` 加 `parent_thread_id`
3. daemon store / local RPC / backend 转换同步字段
4. 单元测试：conversation session 列表包含 subagent，且 subagent 不增加顶层 agent_session_count

### Phase 3: 运行时注册与路由（依赖 Phase 2）
1. `ThreadHandle` / `ManagerEvent::ThreadAdded` 携带 `parent_thread_id`
2. Codex event pump 直接轻量解析 collab item，注册 subagent thread，缓冲/释放 orphan sub events
3. Opencode SSE pump 在 driver 层 best-effort 归属未知 sessionID，合成 parent-thread `minos.subagent.spawned` RawIngest
4. 单元测试：Codex parent-first / child-first 事件顺序；Opencode pending Task + 未知 sessionID 归属

### Phase 4: TUI 数据模型与翻译（依赖 Phase 1-3）
1. `ChatItem::SubagentCall` + `UiState.subagent_info`
2. `ChatState::apply_ui_event` 新分支，只更新当前 thread
3. `handle_ingest` 对未知 thread 懒创建 ChatState，并同步 subagent info
4. 单元测试：事件→ChatItem 翻译；sub_thread_id ingest 路由到 sub ChatState

### Phase 5: TUI 渲染与只读交互（依赖 Phase 4）
1. Sidebar 树状渲染 + 扁平化选择 helper
2. SubagentCall 卡片渲染（不实现卡片 Enter 跳转）
3. Sidebar subagent 条目 → `AgentDetail` 导航
4. Subagent detail 禁用 Agent input submit / close / delete
5. 手动集成验证

## 8. 测试计划

### 单元测试（隔离逻辑）

**协议层：**
- Codex 翻译器：`collabAgentToolCall` item/started JSON → `SubagentSpawned`（验证 model/prompt/thread_ids 提取）
- Codex 翻译器：`collabAgentToolCall` item/completed → `SubagentStatusUpdated`
- 序列化往返：`SubagentSpawned` → JSON → `SubagentSpawned`
- Opencode 翻译器：synthetic `minos.subagent.spawned` → `SubagentSpawned`

**持久化层：**
- `threads.parent_thread_id` 插入/查询正确
- `list_conversation_agent_sessions` 返回顶层 session + subagent session
- subagent thread insert 不增加 conversation `agent_session_count`
- subagent event 写入前已有 thread parent row，避免 `events.thread_id` 外键失败

**运行时层：**
- Codex parent-first：collab item 到达后注册 subagent，再广播 sub thread event
- Codex child-first：未知 sub thread event 先缓冲；collab item 到达后注册并按顺序释放
- Codex orphan timeout：未匹配 parent 的 buffered events 被清理并记录日志
- Opencode parent 归属：pending Task + 未知 sessionID → best-effort 注册 subagent
- Opencode 并发/歧义：无法唯一归属时不注册，避免挂错 parent

**TUI 翻译：**
- `SubagentSpawned` → `ChatItem::SubagentCall` 在父线程 items 中
- `SubagentStatusUpdated` → status 更新
- sub_thread_id 的 ingest 帧 → `handle_ingest` 懒创建并路由到 sub ChatState
- subagent AgentDetail input submit → no-op + 只读提示

**TUI 渲染：**
- `SubagentCall` → 正确 label/model/status/prompt 行
- Sidebar 树 → 缩进 + 扁平化索引正确
- Sidebar mouse / keyboard selection 读取同一 flat helper

### 集成验证（手动）
- Codex：触发 subagent → sidebar 实时显示 → 切换查看 transcript → 卡片显示 model/prompt/status
- Codex child-first fixture：sub thread event 早于 collab item 时不丢 transcript
- Opencode：触发 Task tool → best-effort subagent 自动注册 → sidebar 显示 → transcript 可查看
- 从 sidebar subagent 条目跳转到 subagent detail → Back 返回父 agent
- Subagent detail 输入被禁用，不会发送消息
- Subagent 完成后状态指示器更新

## 9. 设计决策记录

| 决策 | 选择 | 理由 |
|------|------|------|
| 协议建模方式 | 新增 `UiEventMessage` 变体 | 清晰的父子关系语义，符合架构分层 |
| Sidebar 展示 | 嵌套树状 | 直观表达从属关系 |
| Runtime 注册方式 | 直接轻量解析 raw payload | runtime 不能依赖 UI translator 输出；translator 只负责 projection |
| Codex 事件获取 | 注册 sub thread 后再释放其 RawIngest | 避免 SQLite `events.thread_id` FK 失败和 TUI 丢 ChatState |
| Opencode 事件获取 | driver 层 best-effort 自动注册未知 sessionID | SSE 全局流可能包含 child session，但缺少可靠 parent link |
| Opencode parent 归属 | 基于唯一 pending Task 工具 | opencode 不暴露 parent sessionID；存在歧义时不注册，避免误挂 |
| 竞态处理 | unknown sub thread events 先缓冲 | 子事件早于 parent collab item 时不丢 transcript，也不提前写无父 row |
| Backend 数据源 | 扩展 `ThreadSummary.parent_thread_id` | 复用现有 `list_conversation_agent_sessions`，不新增 `list_subagents` 第二数据源 |
| 主 agent 卡片 | 专属 `SubagentCall` 变体 | 区别于普通 tool call，展示 model/prompt/status |
| 卡片跳转 | 不做 Enter 跳转 | 当前 chat 没有 item selection；sidebar 已满足只读观察入口 |
| Subagent 控制 | 只读，不支持输入/关闭/删除/`@agent#short` | 本期目标是可观测，避免把 subagent 扩成可主动驱动 session |
| Claude/Gemini subagent | 不做特殊处理 | CLI 协议不暴露 subagent 实时事件流（Claude stream-json 是黑盒；ACP v1 单 session）；外部接入无法获取子 transcript |
