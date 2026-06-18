# 终端 UI (minos-tui) 架构文档

> 本文档详细描述 `minos-tui` crate 的架构、UI 组件和交互逻辑。

## 概述

`minos-tui` 是一个全功能终端 UI 客户端，使用 Ratatui + Crossterm 构建。支持嵌入式模式（直接管理 agent 子进程）和守护进程模式（通过 JSON-RPC 连接 `minos-daemon`）。核心特性包括多 agent 管理、群聊房间、Agent 间 MCP 协调。

**源码路径**: `crates/minos-tui/`

## CLI 参数

```
minos-tui [OPTIONS]
```

| 参数 | 用途 |
|------|------|
| `-a, --agent <AGENT>` | 自动启动指定 agent（codex/claude/gemini/opencode） |
| `-w, --workspace <PATH>` | 工作目录（默认 `.`） |
| `--readonly` | 禁用消息发送 |
| `--backend <KIND>` | `embedded`（默认）或 `daemon` |
| `--daemon-url <URL>` | Daemon WS URL（显式外部 daemon；未提供时先从 `~/.minos/run/tui-daemon-rpc.json` 自动发现，发现失败则由 TUI 托管本地 daemon） |

## 启动序列 (`src/main.rs`)

1. 解析 CLI 参数
2. 解析工作空间路径，初始化文件日志
3. 安装 teamwork skills 到所有 agent 配置目录
4. 构建后端（`EmbeddedBackend` 或 `DaemonBackend`；daemon 模式未发现外部 daemon 时，TUI 在当前进程内启动 `DaemonHandle::start_with_local_rpc` 并连接本地 RPC）
5. 创建 `App`，调用 `app.init()`（检测 CLI、加载群聊历史、同步 daemon 线程）
6. 设置终端（ratatui + crossterm 鼠标捕获）
7. 启动 4 个事件泵（terminal, tick, ingest, manager）
8. 可选自动启动 agent（`--agent` 参数）
9. 进入主渲染循环
10. 退出时 `app.shutdown()`，恢复终端，并停止 TUI 托管的 daemon（如果有）

## 后端抽象 (`src/backend/`)

### `AgentBackend` trait

| 方法 | 用途 |
|------|------|
| `detect_clis()` | 检测已安装 CLI agent |
| `start_agent(agent, workspace)` | 启动 agent 线程 |
| `send_message(thread_id, text)` | 发送用户消息 |
| `send_approval_decision(...)` | 审批 codex 请求 |
| `respond_opencode_permission(...)` | 审批 opencode 权限 |
| `respond_opencode_question(...)` | 回答 opencode question 请求 |
| `interrupt_thread(thread_id)` | 中断线程 |
| `close_thread(thread_id)` | 关闭线程 |
| `delete_thread(thread_id)` | 删除线程 |
| `list_threads()` | 列出所有线程 |
| `list_projects()` | 列出所有 project（项目导航层入口） |
| `create_project(name, workspace_path)` | 创建 project，返回新 `ProjectEntry` |
| `list_project_threads(project_id)` | 列出绑定到某 project 的 agent session |
| `start_agent_in_project(project_id, agent, workspace)` | 在 project 内启动 agent（不传 prompt；首条消息由 `AgentStartedForPrompt` effect 携带） |
| `resume_thread(thread_id)` | 恢复挂起线程 |
| `read_thread_raw_history(...)` | 读取原始事件历史 |
| `read_group_chat(...)` | 读取群聊消息 |
| `subscribe_ingest()` | 订阅原始事件流 |
| `subscribe_manager_events()` | 订阅生命周期事件 |
| `connection_state()` | 当前连接状态 |

### `EmbeddedBackend`

- 直接使用 `minos_agent_runtime::AgentManager`（进程内）
- 配置 teamwork MCP 集成。开发态不要求额外 `minos-teamwork-mcp` 可执行文件；`AgentRuntimeConfig` 会优先使用同目录独立 sidecar，找不到时使用当前 `minos-tui __minos-teamwork-mcp` hidden 子命令作为 stdio MCP server。
- 不支持线程恢复和原始历史读取

### `DaemonBackend`

- 通过 WebSocket 连接 `minos-daemon`（jsonrpsee WS client）
- 未传 `--daemon-url` 且 discovery 缺失/连接失败时，TUI 自启动托管 daemon handle，暴露本地 RPC 后再连接；这让 `minos-tui --backend daemon` 在开发态也是完整体验，不依赖用户单独启动 `minos-daemon`。
- 所有操作转为 JSON-RPC 调用（`minos_local_*` 前缀）
- project 导航使用 `minos_local_list_projects`、`minos_local_create_project`、`minos_local_list_project_threads`、`minos_local_start_agent_in_project`；relay-forwarded `minos_*` host command surface 不用于 TUI 本地控制。
- 两个订阅泵转发 daemon 事件到本地 broadcast channel
- `minos_local_list_local_threads` 返回 daemon 本机最近线程集合，不按 TUI 当前 workspace 预过滤；`App` 在接收列表、manager event、群聊历史/session 恢复时只保留 `--workspace` 对应的线程和消息，避免一个 TUI 房间混入其它 workspace 的 agent 会话。

## UI 布局 (`src/ui/`)

### 概览模式（agent_detail_visible = false）

```
+------------------+------------------------+----------+
| Room List (20%)  | Group Chat (55%)       | Agents   |
|                  |                        | (25%)    |
+------------------+------------------------+----------+
|               Chat Room Input Bar                    |
+------------------------------------------------------+
```

### 详情模式（agent_detail_visible = true）

```
+------------------------+----------+------------------+
| Group Chat (45%)       | Agents   | Agent Chat (35%) |
|                        | (20%)    |                  |
+------------------------+----------+------------------+
| Room Input (65%)       | Agent Input (35%)           |
+------------------------+-----------------------------+
```

顶部 **状态栏**（1 行）：后端状态、检测到的 agent、快捷键提示。

叠加层: Project Create Dialog（新建 project 模态，含启动时 workspace 未匹配 project 的自动创建入口）、Agent Picker（选择 agent 的模态框）、Delete Confirm（删除确认模态框）。

### UI 组件

| 文件 | 组件 | 描述 |
|------|------|------|
| `ui/mod.rs` | `UiState`, `PanelAreas`, render tree assembly | 布局编排、状态管理、鼠标命中区域同步 |
| `focus.rs` | `FocusManager`, `FocusNode`, `PaneId` | overview/detail 两套焦点树、深度优先 Tab 循环、布局切换时焦点保留/回退 |
| `render/mod.rs` | `Renderable`, `Column`, `Row` | Frame-backed render trait、纵向 fill 布局、横向比例布局、cursor 传播和 area 计算 |
| `ui/chat.rs`, `ui/chat/cache.rs` | Agent 聊天视图、`AgentChatRenderable`, `RenderCache` | Markdown 渲染、diff 高亮、工具调用、代码块、按 item 缓存的可见行切片 |
| `ui/group_chat.rs` | 群聊视图、`GroupChatRenderable` | 房间消息渲染和按消息 block 缓存的可见行切片 |
| `ui/input_bar.rs`, `ui/input_bar/render.rs` | `InputState`, `InputBarRenderable` | 多行输入编辑器、`@` agent mention/路径补全、输入栏渲染与鼠标坐标映射 |
| `ui/thread_list.rs` | `ThreadListRenderable` | agent 线程列表（状态颜色编码） |
| `ui/project_list.rs` | `ProjectListRenderable`, `ProjectSidebarRenderable` | 项目导航层 Projects 主列表 + 侧栏（project 名称、workspace、session 计数） |
| `ui/project_sessions.rs` | `ProjectSessionsRenderable` | 项目导航层 Sessions 列表（project 绑定的 agent session 摘要） |
| `ui/project_create_dialog.rs` | create dialog render | 新建 project 模态（名称/路径字段、Tab 切换） |
| `ui/room_list.rs` | `RoomListRenderable` | 群聊房间列表 |
| `ui/status_bar.rs` | `StatusBarRenderable` | 后端状态 + agent + 快捷键 |
| `ui/agent_picker.rs` | `AgentPickerRenderable` | 编号快速选择（1-9） |
| `ui/delete_confirm.rs` | `DeleteConfirmRenderable` | 删除确认 overlay |
| `ui/theme.rs` | 主题 | 颜色/样式常量 |

### Renderable 树与焦点树

P2 后渲染入口仍是 `ui::render_ui(f, &mut UiState)`，但 `render_ui` 的职责收敛为：

1. 根据 `FocusManager` 更新 `room_input.focused` / `agent_input.focused`
2. 按 `nav_level` 分派渲染：`Projects` → `render_projects_level`（project 列表 + 侧栏 + 底部 hint 行），`Sessions` → `render_sessions_level`（session 列表 + 侧栏 + 输入栏），`Session`/`AgentDetail` → `render_legacy`（overview/detail 树，见下）
3. 渲染叠加层：`project_create_dialog`、`agent_picker`、`delete_confirm`
4. legacy 树内部根据 `agent_detail_visible` 再分 overview/detail 两套比例布局，使用 `Column::with_fill` 组装状态栏、主体 row、输入 row
5. overview 模式用 `Row(20/55/25)` 渲染 Room List / Group Chat / Agents；detail 模式用 `Row(45/20/35)` 渲染 Group Chat / Agents / Agent Chat，输入区用 `Row(65/35)`
6. 用同一组 `Row::areas_for` 比例写回 `PanelAreas` 和 `InputLayoutMetrics`，保证鼠标命中区域与实际渲染区域一致
7. render tree 完成后通过 `Renderable::cursor_pos()` 向上传播输入栏 cursor，并调用 `Frame::set_cursor_position`

`Renderable` 采用 `fn render(&mut self, frame: &mut Frame, area: Rect)`，而不是 Buffer-only 接口，因为当前 Ratatui 面板需要在 render 阶段更新 list state、scroll max、render cache 和 input hit-test metrics。`desired_height(width)` 用于 input row 高度协商；`cursor_pos(area)` 仅由输入栏返回，`Row`/`Column` 递归向上传播；主体 row 作为 `Column::with_fill(..., fill_index = 1)` 的 fill child 占满剩余高度。

`FocusManager` 替代旧扁平 `Focus` enum。overview 焦点树包含 `RoomList -> GroupChat -> AgentList -> RoomInput`；detail 焦点树包含 `GroupChat -> AgentList -> AgentChat -> RoomInput -> AgentInput`。`Tab` 深度优先前进，`BackTab` 反向循环。打开/关闭 agent detail 时调用 `switch_layout(detail)`，若当前 pane 不存在于新树则回退到新布局首个 pane。

### 聊天渲染特性

- Markdown: 标题、列表、内联代码、围栏代码块
- Diff 语法高亮（绿色添加、红色删除、青色块头）
- 工具调用块（可展开 args/output）
- 推理/思考块（深灰色）
- 流式光标（闪烁块字符）
- Unicode 感知的自动换行

## 项目导航层 (`src/nav.rs`)

TUI 顶层是一个三级项目导航 shell，把早期的 "room/thread 扁平列表" 收敛为按 project 组织的 agent session 视图。`NavLevel` 描述当前所在层级：

```rust
enum NavLevel {
    Projects,                                  // project 列表
    Sessions { project_id: String },           // 某 project 绑定的 agent session 列表
    Session { project_id, thread_id },          // 单个 session（复用 Thread/ChatState 模型）
    AgentDetail { project_id, thread_id, agent }, // legacy detail 视图
}
```

`NavLevel` 提供 `go_up()`（返回上一层，Projects 停在 Projects）、`project_id()`、`thread_id()` 和 `esc_quits()`（仅 Projects 层 Esc 触发退出）。`NavAction` 枚举（`Downlevel`/`Uplevel`/`SelectNext`/`SelectPrev`/`OpenCreateProject`/`ConfirmCreateProject`/`CancelDialog`/`SwitchField`/`TypeChar`/`Backspace`/`SubmitSessionInput`）是导航层唯一的输入语义集合，由 `app/event_mapping.rs` 的 `projects_level_mapping` / `sessions_level_mapping` / `create_dialog_mapping` 从原始按键翻译而来。

### 导航流

1. **Projects 层**：列出所有 project，`Up`/`Down` 循环选择，`Enter` 下钻到 Sessions，`n` 打开创建对话框，`Esc` 退出。
2. **Sessions 层**：列出某 project 绑定的 agent session，`Up`/`Down` 选择，`Enter` 打开单个 session，`Esc` 返回 Projects；输入栏提交时调用 `start_agent_in_project` 新建 session。
3. **Session 层**：单个 agent session，复用 legacy chat 渲染；`Esc` 返回 Sessions。
4. **AgentDetail 层**：legacy agent detail，`Esc` 返回 Session。

### 启动 cwd 匹配 (`resolve_startup_project`)

`App::init()` 在检测 CLI、加载群聊后调用 `resolve_startup_project`：拉取 project 列表，把 `state.workspace` 与每个 project 的 `workspace_path` 比对（`workspace_path_belongs_to_current_workspace`）。匹配成功则直接进入该 project 的 Sessions 层；未匹配则自动打开 `ProjectCreateDialogState`，默认 name/path 来自传入的 `--workspace`。

### update 层

`update/nav.rs` 按 `nav_level` 分发 `NavAction`：Projects 层调用 `navigate()` 循环移动 `selected_project`，下钻返回 `Effect::LoadProjectThreads`；Sessions 层返回 `Effect::OpenProjectSession` 或 `Effect::StartAgentInProject`；创建对话框直接编辑 `ProjectCreateDialogState`，确认时返回 `Effect::CreateProject`。`navigate()` 是带循环边界的纯函数（空列表返回 `None`，单元素自循环，越界 wrap）。

## 事件系统 (`src/event.rs`)

### `AppEvent` 枚举

```
AppEvent::Ingest(LocalIngestFrame)         // seq + UiEventMessage projection
AppEvent::ManagerEvent(ManagerEvent)       // 线程生命周期事件
AppEvent::AgentStartedForPrompt { ... }    // 后台 agent 启动完成
AppEvent::SendMessageFailed { ... }        // 异步发送错误
AppEvent::ProjectsLoaded(Vec<ProjectEntry>)         // project 列表加载完成
AppEvent::ProjectCreated(ProjectEntry)              // 新 project 创建完成
AppEvent::ProjectThreadsLoaded { project_id, threads } // project session 列表加载完成
AppEvent::ProjectSessionStarted { project_id, agent, thread_id, cwd, text } // project 内新 session 启动完成
AppEvent::ProjectFailed(String)                     // project 操作错误（create/list/start）
AppEvent::Key(KeyEvent)                    // 键盘事件
AppEvent::Paste(String)                    // bracketed paste 文本，按整体插入输入栏
AppEvent::Mouse(MouseEvent)                // 鼠标事件
AppEvent::Resize(u16, u16)                 // 终端大小变化
AppEvent::Tick                             // 200ms 定时器
```

project 相关事件由 `execute_effect` 内 `tokio::spawn` 的异步任务回传（`LoadProjectThreads`、`CreateProject`、`StartAgentInProject` effect），`App::handle_event` 把它们统一映射为 `Action::EffectCompleted(EffectResult::*)`，由 update 层同步更新 `UiState.projects` / `project_sessions` / `nav_level`。

### 4 个事件泵

1. **终端事件泵**（std 线程，250ms 轮询 crossterm；启用 bracketed paste 后把多行粘贴转为单个 `Paste` 事件，避免换行被当作 Enter 提交）
2. **Tick 泵**（tokio interval，200ms）
3. **Ingest 泵**（转发 backend `LocalIngestFrame` broadcast → MPSC）
4. **Manager Event 泵**（转发 backend manager events → MPSC）

## 核心状态与事件流 (`src/app.rs`, `src/app/`, `src/state/`, `src/update/`)

`App` 现在是轻量 shell，持有 backend、`AppState`、`UiState`、退出标志和事件回流 channel。运行时逻辑拆到 `src/app/` 子模块：事件分发与 effect 执行在 `app/event_loop.rs`，生命周期/daemon replay 在 `app/lifecycle.rs`，提交/发送在 `app/submission.rs`，群聊协调在 `app/group_chat.rs`，MCP 处理在 `app/mcp.rs`。

### 关键字段

```rust
App {
    backend: Arc<dyn AgentBackend>,
    state: AppState,
    ui: UiState,
}

AppState {
    workspace: PathBuf,
    hydrated_threads: HashSet<String>,           // 已回放历史的线程
    thread_watermarks: HashMap<String, u64>,     // 每线程最高 seq
    applied_ingest_fingerprints: HashSet<String>, // 去重 ingest
    group_chat_store: GroupChatStore,             // SQLite 群聊持久化
    recorded_agent_results: HashMap<String, String>, // 去重群聊 agent 结果
}
```

P0-C 后事件路径为 `AppEvent`/按键语义 → `Action` → `update()` → `Effect` → App effect executor。`app/event_mapping.rs` 负责把键盘事件映射为语义 Action 或输入目标；`input.rs` 负责输入栏按键到 `InputAction` 的映射和参数化 `InputState` 编辑；`update/mod.rs` 分发 `GlobalAction`、`RoomAction`、`AgentAction`、`InputAction` 和 effect 回流结果；`update/global.rs`、`update/room.rs`、`update/agent.rs` 执行同步状态变更并返回 effect。

`Ingest`、`ManagerEvent`、`Tick` 和 `McpToolCall` 不再直接在 event match 中执行业务逻辑，而是映射为 `Action::EffectCompleted` 或 `GlobalAction` 后返回 `Effect::Handle*`。输入 submit 也由 update 层清空/记录输入、解析 `@agent` 路由、判断 pending approval/question，再返回 `DispatchPromptToAgent`、`SendTextToThread`、`SubmitPendingAgentRequest` 等显式 effect。

Action/Effect 类型只保留当前执行器真正消费的语义分支。P0 迁移阶段的 passthrough、未接线 submit intent、no-op global intent、`Effect::None` 和未执行的占位 backend effect 已删除，避免事件边界继续承载旧路径。

`agent_route.rs` 提供共享 agent 路由解析、thread short id 和 closed-thread 判定，避免 App 与 update 层重复实现同一套 `@agent[#thread]` 规则。

### 线程水化

首次查看线程时，从 `read_thread_raw_history()` 加载历史 projection frame，按 `UiEventMessage` 重建 `ChatState.items`。daemon backend 的 replay 不需要 TUI 重新解析 raw JSON；embedded backend 没有 daemon/EventWriter 时，会在进程内用同一套 translator 临时生成 projection。
daemon 模式下，TUI 只会水化当前 workspace 的线程。来自其它 workspace 的 daemon 线程、`ThreadAdded` lifecycle event、群聊 `chat_agent_sessions` 或历史消息会在进入 `UiState.threads` / `GroupChatState.messages` 前被丢弃。

### Ingest 去重

优先使用 `LocalIngestFrame.seq` + 每线程水位线去重。没有 seq 的 embedded/test frame 才使用 `ui_events` 序列化指纹作为兜底。

## 渲染性能

`frame.rs` 提供 `FrameRequester` 和内部 `FrameScheduler`。`App` 在 `Action` / `Effect` 路径返回 redraw 时调用 `request_frame()`；scheduler 接收高频请求后合并为最早可绘制 deadline，并按 `MIN_FRAME_INTERVAL = 33ms` 输出 frame token。`main.rs` 的主循环同时监听 app event channel 和 frame channel，收到 frame token 后立即 draw；节流不再阻塞主事件循环，因此连续 ingest 或滚轮事件不会被 terminal draw sleep 拖慢。

Agent chat 使用 `ui/chat/cache.rs::RenderCache`。cache 按 `(thread_id, version, width)` 建索引，并为每个 `ChatItem` 计算 fingerprint；同线程同宽度下，仅变更 item 会重新生成 visual segment，滚动时直接从缓存的 visual lines 切出 viewport，不再每帧重跑 Markdown/diff/tool wrapping。

Group chat 使用 `GroupChatRenderCache`。`GroupChatState` 对消息变更加 `version`，cache 按 `(version, width)` 缓存每条消息 block 的完整 wrapped lines、起始视觉行和总行数；`render_group_chat()` 只克隆当前 viewport 的 cached lines。重复 daemon 历史 refresh 或 duplicate upsert 不会 bump version，因此不会让 render cache 失效。

P2 后大型渲染/测试文件也被拆分以保持局部性：`ui/chat/cache.rs` 持有 agent chat 可见窗口索引；`ui/input_bar/render.rs` 持有输入栏 render/editor-coordinate 逻辑；`app_tests.rs` 仅保留共享测试 harness，具体 app 行为测试分布在 `app_tests/` 子模块中。

## 翻译管线 (`src/translation/`)

`translation/mod.rs` 是门面模块，保持 `crate::translation::{ChatState, ChatItem, ...}` 的外部导入路径稳定；具体职责拆到聚焦子模块：

| 模块 | 职责 |
|------|------|
| `agent.rs` | `AgentTranslationState`，封装 codex/claude/gemini/opencode translator |
| `chat_state.rs` | `ChatState` 与 `UiEventMessage` → `ChatItem` 投影 |
| `chat_item.rs` | `ChatItem`、`TextPart` 与 item 内部辅助方法 |
| `tool_summary.rs` | 工具参数/输出摘要、diff 识别 |
| `pending_request.rs` | Codex approval、opencode permission/question 的待处理请求模型 |
| `json_helpers.rs` | pending request 使用的 JSON 递归查找辅助 |
| `selection.rs` | 聊天文本选区模型 |
| `translation_tests.rs` | 翻译投影与摘要单元测试 |

### `ChatState`（每线程状态）

```rust
ChatState {
    thread_id, agent,
    translation_state: AgentTranslationState,  // 仅 embedded/test raw path 使用
    items: Vec<ChatItem>,
    pending_requests: Vec<PendingAgentRequest>,
    completed_assistant_message_ids: HashSet<String>,
    scroll_offset, auto_scroll, max_scroll,
    selection: Option<ChatSelection>,
}
```

`ChatState::apply_ui_event()` 只消费 `UiEventMessage`。`DisplayPayload` 在 TUI 层渲染为 preview 文本；raw body/artifact 全文不进入 `ChatItem`。
`last_completed_assistant_text()` 只从显式收到 terminal `MessageCompleted` 的 assistant message 取群聊结果；仅因下一条消息开始而停止 streaming 的中间文本不会被当成最终回复。
聊天室 agent result 仍以 terminal `MessageCompleted` 为提交条件。`TextDelta` / `TextReplace` 只更新 direct agent panel；`AppEvent::Ingest` 只有在当前 frame 标记完成时才调用完成态记录路径，并通过稳定 `agent-result:{room_id}:{thread_id}:{source_message_id}` upsert 群聊消息，避免 live ingest、manager idle 和历史 replay 重复写入。opencode `finish:"tool-calls"` 这类工具调用前中间完成不会被 translator 映射为 terminal `MessageCompleted`，因此不会进入聊天室。

### `ChatItem`

```rust
enum ChatItem {
    UserMessage { message_id, text_parts, is_streaming },
    AssistantText { message_id, text_parts, is_streaming },
    Reasoning { message_id, text, is_streaming },
    ToolCall { message_id, tool_call_id, args/output, is_streaming },
    SystemMessage { text },
    Error { message_id, text },
}
```

## 群聊系统 (`src/group_chat.rs`)

### `GroupChatStore`

SQLite 持久化群聊房间和消息。支持:
- 多房间管理
- 分页加载
- Agent 结果流式 upsert（ingest 增量到达时更新当前 thread turn 的同一条群聊结果消息；完成/关闭路径保留为缺失 live ingest 的补偿）
- MCP 命令队列处理

群聊消息会记录 `agent`、`thread_id`、`thread_short_id` 和 `workspace`。发送到已有 agent 线程的用户消息在可见文本中使用 `@agent#short_id ...`，即使输入来自 Agent Input 面板也保持同一会话标识；新建 agent 的首条房间消息在线程创建前可以只有 `@agent ...`。

## Agent 间协调（MCP）

### 流程

1. Agent 使用 `minos_chat` MCP 工具调用 `request_agent_help`
2. TUI tick 泵调用 `claim_pending_mcp_commands()`
3. 处理 `MentionAgent` 命令：启动目标 agent，转发 prompt
4. Agent 结果回传到群聊
5. `MentionUser` 命令显示为 agent 消息

### Teamwork Skill 安装

启动时将 `skills/minos-teamwork/SKILL.md` 复制到:
- `~/.agents/skills/`
- `~/.claude/skills/`
- `~/.gemini/skills/`
- `~/.config/opencode/skills/`

## 关键用户流程

### 启动 Agent

项目导航 shell 下的启动路径：

1. 启动时 `resolve_startup_project` 匹配 cwd：命中已有 project 直接进入其 Sessions 层；未命中自动打开 Project Create Dialog，确认后创建 project 并进入 Sessions 层
2. 在 Sessions 层输入栏提交首条 prompt → `start_agent_in_project(project_id, agent, workspace)` → `ProjectSessionStarted` 事件 → 进入 Session 层并触发 `AgentStartedForPrompt` 把 prompt 发给新线程
3. `n` 在 Projects 层打开 Project Create Dialog；在 Session 层（legacy 视图）打开 Agent Picker 切换当前线程的 agent

### 发送消息

1. 在 Room Input 输入文本
2. `@agent` 语法路由到特定 agent（`@codex`, `@claude#shortid` 等）
3. 消息同时记录到群聊
4. Agent Input 面板发送到已选线程时，也会以 `@agent#shortid` 形式写入群聊，避免看起来像重新 @ 了一个新 agent
5. Agent 响应通过 ingest 流式返回

### 审批流程

1. Agent 遇到审批请求 → `Raw` 事件
2. 创建 `PendingAgentRequest` + 系统消息
3. 输入栏标签变为 "Agent Input: Reply Required"
4. 用户输入 "yes"/"approve" = 批准，其他 = 拒绝

### opencode question 流程

1. opencode 发出 `question.asked` → projection 为 `Raw(kind="opencode/question.asked")`
2. `ChatState` 创建 `PendingAgentRequestKind::OpencodeQuestion`，系统消息展示问题、选项和输入格式
3. 用户在 Agent Input 输入选项编号、选项文本或自定义答案；多选用逗号/分号分隔
4. TUI 调用 `respond_opencode_question()`，最终由 opencode driver POST `/question/{requestID}/reply`，body 为 `{ "answers": [[...]] }`

### opencode 消息完成规则

opencode 的 `message.updated` 可能带 `time.completed` 且 `finish:"tool-calls"`，表示当前 assistant message 为工具调用前的中间步骤，不是最终回复。translator 只在非 `tool-calls` 的 terminal completion 上发出 `MessageCompleted`，避免把中间消息写回群聊。

### 线程管理

- **中断**: Ctrl+C（运行中则中断，否则退出）
- **关闭**: Ctrl+D（优雅关闭）
- **删除**: Delete 键 → 确认模态 → Enter/Y 或 Esc/N
- **恢复**: 向挂起线程发送消息时自动尝试

## 文件清单

| 文件 | 行数 | 职责 |
|------|------|------|
| `main.rs` | 522 | 入口、CLI、启动编排、frame request select loop |
| `action.rs` | 171 | `Action`/`GlobalAction`/`RoomAction`/`AgentAction`/`InputAction`/`EffectResult` 类型 |
| `agent_route.rs` | 51 | `@agent[#thread]` 路由解析、thread short id、closed-thread 判定 |
| `effect.rs` | 87 | `Effect` 与 `StateChange` |
| `nav.rs` | 108 | `NavLevel`、`NavAction` 项目导航层类型 |
| `frame.rs` | 119 | `FrameRequester`、`FrameScheduler`、33ms draw interval |
| `input.rs` | 297 | 输入栏按键映射与参数化 `InputState` action 应用 |
| `app.rs` | 85 | `App` shell、构造器、测试模块挂载 |
| `app/event_mapping.rs` | 303 | key event 到 semantic Action/input target 的纯映射（含 projects/sessions/dialog/startup 分支） |
| `app/event_loop.rs` | 486 | `AppEvent`/key/mouse/paste 分发、Action 应用、Effect 执行器（含 project effect spawn） |
| `app/lifecycle.rs` | 642 | init/shutdown、`resolve_startup_project`、daemon thread sync、ingest/manager/tick 处理 |
| `app/submission.rs` | 387 | async submit effect 执行：启动 agent、发送消息、approval/question 回复 |
| `app/group_chat.rs` | 398 | 群聊历史加载、agent result 记录、group chat store 写入 |
| `app/mcp.rs` | 224 | teamwork MCP socket request 处理 |
| `app/thread_ops.rs` | 99 | thread 关闭/删除/选择/start picker 辅助 |
| `app/clipboard.rs` | 114 | 剪贴板读写与测试剪贴板 |
| `app/helpers.rs` | 196 | approval/question 解析、错误格式化和 App 局部 helper |
| `app_tests.rs` | 386 | App 行为测试共享 harness（TestBackend） |
| `app_tests/nav_integration.rs` | 125 | 项目导航层集成测试（projects 导航、对话框、Esc 行为、project-bound session 创建） |
| `app_tests/input_and_routing.rs` | 412 | 输入、agent picker、prompt routing 行为测试 |
| `app_tests/group_and_agent.rs` | 530 | 群聊、daemon history、agent pending request 行为测试 |
| `app_tests/ingest.rs` | 548 | ingest/group result/opencode idle 行为测试 |
| `app_tests/navigation_and_lifecycle.rs` | 479 | 导航、鼠标、删除、daemon lifecycle 行为测试 |
| `update/mod.rs` | 385 | update 层入口、input submit、effect result 回流、共享 UI helper |
| `update/global.rs` | 260 | `GlobalAction` 处理、鼠标和全局键状态变更 |
| `update/room.rs` | 110 | `RoomAction`、room submit 路由决策 |
| `update/agent.rs` | 83 | `AgentAction`、agent submit/pending request 决策 |
| `update/nav.rs` | 296 | `NavAction` 处理、projects/sessions/dialog/startup prompt 状态变更与 effect 触发 |
| `state/mod.rs` | 41 | `AppState` 业务状态聚合 |
| `state/ingest_dedup.rs` | 58 | ingest 去重和完成帧判断 |
| `state/workspace_filter.rs` | 118 | workspace 过滤、线程裁剪、房间标题/ID |
| `state/selection.rs` | 128 | 鼠标选区和列表点击几何辅助 |
| `event.rs` | 123 | AppEvent 枚举、事件泵 |
| `backend/mod.rs` | 177 | AgentBackend trait、`ProjectEntry`、`ThreadSummaryEntry` |
| `backend/embedded.rs` | 262 | 进程内后端 |
| `backend/daemon.rs` | 464 | WS RPC 后端 |
| `translation/mod.rs` | 20 | 翻译模块门面、公开类型重导出 |
| `translation/agent.rs` | 60 | `AgentTranslationState` 与 translator 错误日志 |
| `translation/chat_state.rs` | 574 | `ChatState` 与 UI event 投影 |
| `translation/chat_item.rs` | 69 | `ChatItem`、`TextPart` |
| `translation/tool_summary.rs` | 264 | 工具参数/输出格式化 |
| `translation/pending_request.rs` | 385 | 待处理审批/权限/问题请求 |
| `translation/json_helpers.rs` | 80 | JSON 递归查找辅助 |
| `translation/selection.rs` | 25 | 聊天选区状态 |
| `translation/translation_tests.rs` | 740 | 翻译投影单元测试 |
| `focus.rs` | 171 | `FocusManager`、focus tree、`PaneId` |
| `focus_tests.rs` | 56 | focus tree 单元测试 |
| `render/mod.rs` | 230 | `Renderable`、`Column`、`Row`、cursor 传播 |
| `ui/mod.rs` | 831 | 布局、UiState（含 nav_level/projects/project_sessions/project_create_dialog）、nav-level render dispatch、GroupChatState version/cache 状态 |
| `ui/project_list.rs` | 113 | `ProjectListRenderable`、`ProjectSidebarRenderable`（项目导航 Projects 层） |
| `ui/project_sessions.rs` | 121 | `ProjectSessionsRenderable`（项目导航 Sessions 层） |
| `ui/project_create_dialog.rs` | 59 | 新建 project 模态渲染 |
| `ui/chat.rs` | 717 | Agent 聊天渲染与 selection |
| `ui/chat/cache.rs` | 174 | Agent chat 按 item segment 缓存和可见行切片 |
| `ui/chat_tests.rs` | 404 | Agent chat render/cache/selection 单元测试 |
| `ui/input_bar.rs` | 730 | `InputState`、history、mention/path completion 状态 |
| `ui/input_bar/render.rs` | 692 | 输入栏 render/editor-coordinate 逻辑 |
| `ui/input_bar_tests.rs` | 339 | input bar 编辑、completion、坐标映射测试 |
| `ui/delete_confirm.rs` | 90 | 删除确认 overlay renderable |
| `ui/group_chat.rs` | 324 | 群聊渲染、GroupChatRenderCache、cached line viewport 切片 |
| `group_chat.rs` | 346 | SQLite 群聊持久化 |
| `skills.rs` | 119 | Teamwork skill 安装 |
