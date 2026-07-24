# 终端 UI (minos-tui) 架构文档

`minos-tui` 是 Minos 的 Ratatui/Crossterm 终端客户端。它通过 JSON-RPC 连接 `minos-daemon`（仅 daemon 路径，无 embedded agent runtime）。当前 TUI 的主业务层级是 conversation-centric:

```text
Project
  -> Conversations
       -> Conversation timeline + agent sessions
            -> AgentDetail
```

旧的 `Project -> Sessions`、room list 和 `agent_detail_visible` 三栏切换已经不再是 TUI 导航模型。agent run 仍以 thread/session 的形式存在，但它们挂在 conversation 下，作为某个 agent 的具体执行会话。

## CLI

```text
minos-tui [OPTIONS]
```

| 参数 | 用途 |
|------|------|
| `-a, --agent <AGENT>` | 自动启动指定 agent |
| `-w, --workspace <PATH>` | 工作目录，默认 `.` |
| `--readonly` | 禁用消息发送 |
| `--daemon-url <URL>` | 显式 daemon local RPC URL；未提供时读取 discovery，失败则托管本地 daemon |

## 启动序列

1. `main.rs` 解析 CLI、工作目录和日志路径。
2. `logging::init` 打开共享 `SharedLogWriter`（`Arc<Mutex<File>>`）：日志 I/O 失败只吞掉，不会 `try_clone().expect` 拖垮 TUI。
3. 安装 teamwork skills 到支持的 agent 配置目录。
4. 连接或托管 `DaemonBackend`（TUI 仅 daemon 路径）。
5. 创建 `App` 并执行 `app.init()`：检测 CLI、同步 daemon 线程、解析启动 project。
6. 初始化 terminal、鼠标捕获和 bracketed paste。
7. 启动 terminal、tick、ingest、manager、conversation message 事件泵。
8. 进入主循环：`tokio::select! { biased; frame, event }`，**优先处理 frame**，并 drain 合并的 frame token，避免滚轮事件洪峰饿死绘制。
9. 状态变更通过 `FrameRequester` 合并 draw 请求（≈60 FPS 上限；`schedule_frame_in` 支持延迟帧）。
10. 每帧在 crossterm **Synchronized Update** 内 `terminal.draw`，减少中间态闪烁。
11. 退出时恢复 terminal，并停止 TUI 托管的 daemon。

滚动流畅性要点:

- Tick 的 daemon `list_sessions` 在有 event pump 时走后台任务，结果经 `AppEvent::DaemonThreadsListed` 回主循环；不在 tick 上 await 网络。
- **`DaemonThreadsListed` 只做同步 metadata merge**（`apply_daemon_thread_metadata`）：O(1) `session_id` 索引、workspace 路径 canonicalize 结果按次缓存、不为未 hydrate 线程 await history。History 水化留给 init / 无 event pump 的 headless tick（`apply_daemon_thread_snapshots`）以及打开 AgentDetail 时的按需 hydrate。
- 已 hydrate 的 thread 依赖 live ingest 更新；周期 poll **从不**对 thread 做 raw history 回放（避免每 2s 卡滚动帧）。
- Workspace prune/match 用 `WorkspaceMatcher`：每个 known workspace / candidate 在单次 pass 内最多 canonicalize 一次，避免 `sessions × workspaces` 同步 syscall。
- Conversation timeline cache 用 `messages_revision` 做 O(1) 有效性检查，滚动帧不再全量 hash message body。
- **Agent chat 布局（对齐 Grok `LayoutCache`）**：
  - 全量 item 只做 **cheap height estimate** + `virtual_y`。
  - `prepare_layout` → `settle_visible`：只 exact 量视口 + below margin；**无 above-margin**。
  - **纯滚动 fast path**：视口已 measured 时 **不**调用 `find_runs`（此前每帧 O(n) 是「改了没感觉」的主因之一）。
  - **`find_runs` 复用**：`RenderCache` 按 `(session_id, structure_version, items.len, expanded_hash)` 缓存 fold runs；`build_segment_visual_lines` / streaming 路径接收传入的 `runs`，不再每个 exact item 重扫 O(n)。
  - **每帧 exact 预算**：最多 ~6–8 条 markdown，避免 estimate→exact 连锁把整段 history 在一帧量完。
  - follow/底部：从高 index 往低量，保证 live tail 优先 exact。
  - 流式 tail / 近尾 append 立即 exact。
- 侧栏 `recent_files` 按 chat version 缓存，滚动帧不扫全量 tool。
- 主循环 drain + 合并同向滚轮事件。

## 后端抽象

`backend/mod.rs` 的 `AgentBackend` 是 TUI 唯一依赖的后端接口。conversation 相关方法是 project 导航的主路径:

| 方法 | 用途 |
|------|------|
| `list_projects()` / `create_project()` | Project 层数据 |
| `list_conversations(project_id)` | Conversations 层列表 |
| `create_conversation(project_id, title)` | 创建 conversation |
| `list_conversation_messages(conversation_id)` | 主时间线消息 |
| `list_conversation_agent_sessions(conversation_id)` | 右侧 agent session 列表 |
| `start_agent_in_conversation(conversation_id, agent, workspace, profile_id?)` | 在 conversation 内创建 agent run；可选 `profile_id`（daemon `resolve_launch_options` 填 model/effort/instructions） |
| `list_agent_profiles()` | Host agent profiles（@ 补全与 bare `@agent` 最新 profile convenience） |
| `append_conversation_message(...)` | 写 conversation 主时间线 |
| `list_sessions()` / `read_session_raw_history()` | daemon replay 和 AgentDetail 历史水化 |
| `start_agent()` / `send_message()` / `resume_session()` | 直接 agent session 控制 |

`DaemonBackend` 是唯一生产后端，调用 `minos_local_*` 本地 RPC。TUI 本地控制使用 `minos_local_list_conversations`、`minos_local_create_conversation`、`minos_local_start_agent_in_conversation` 和 `minos_local_append_conversation_message`。agent 运行、teamwork MCP、delegation 完成与 source 回推均由 daemon 拥有；TUI 只订阅 conversation / ingest 事件做展示。

## 导航模型

`nav.rs` 定义栈式导航:

```rust
pub enum NavLevel {
    Projects,
    Conversations { project_id: String },
    Conversation { project_id: String, conversation_id: String },
    AgentDetail {
        project_id: String,
        conversation_id: String,
        session_id: String,
        agent: AgentName,
    },
}
```

`UiState.nav.stack: Vec<NavLevel>` 是唯一导航状态。下钻 push，Esc/uplevel pop，Projects 层 Esc 退出。`NavLevel` 只提供 `project_id()`、`conversation_id()` 和 `esc_quits()` 这类查询，不再通过单字段重建父级。

导航流:

1. **Projects**: `Up/Down` 选择 project，`Enter` 加载 conversations，`n` 打开 project 创建对话框，`Esc` 退出。
2. **Conversations**: `Up/Down` 选择 conversation，`Enter` 打开主时间线；输入 prompt 时会创建新 conversation，并在该 project 的 `workspace_path` 下启动指定/default agent。这里没有 active conversation，mention 候选只展示 installed agents，不展示任何已有 session 的 `#short`。
3. **Conversation**: 中间列显示 conversation messages，右侧显示该 conversation 下的 agent sessions。session 列表按 `SessionSummaryEntry.parent_session_id` 扁平化为父子树；subagent 作为父 session 的只读子项展示。输入 `@agent message` 会写主时间线：**优先复用**该 conversation 内同 runtime 最近的顶层未关闭 session（与 desktop bare `@agent` 一致）；没有可复用 session 时才新建，并 convenience 绑定该 runtime **最新** host profile 的 `profile_id`。`@ProfileName` / `@p/<id>` **始终新建**并传显式 `profile_id`（model/effort/instructions 由 daemon 解析，client 不拷贝字段）；`@agent#short message` 只匹配当前 conversation 的顶层、未关闭 agent session，不路由到 subagent，也不回退到全局 thread 列表。@ 补全候选：installed runtimes → host profiles（hint `profile · runtime`；重名或非 clean token 用 `@p/<id>`）→ continue sessions。
4. **AgentDetail**: 显示单个 agent run 的 direct chat；顶层 run 的 Agent Input 以 `@agent#short ...` 形式写入 conversation 主时间线，并把 clean prompt 发给该 run。subagent 的 AgentDetail 只读，只用于观察 transcript、工具状态和终态。

正常导航上下文中，`Ctrl+P` 直接截断到 Projects，`Ctrl+T` 截断到当前 project 的 Conversations；创建 project、删除确认等阻塞弹层保留自己的键盘上下文。旧 `n` new-agent modal picker 已删除，启动 agent 统一走 conversation input 的 `@agent` / `@profile` 路由。

## UI 状态与布局

`ui/mod.rs::UiState` 按职责拆成子结构（字段仍 `pub`，不做 getter 封装）：

```rust
UiState {
    nav: NavPanel,                          // stack: Vec<NavLevel>
    projects: ListPanel<ProjectEntry>,      // items + selected + list_state
    conversations: ListPanel<ConversationEntry>,
    conversation: ConversationPanel {
        messages, scroll_offset, auto_scroll, max_scroll,
        agent_sessions: ListPanel<SessionSummaryEntry>, // selected = flat sidebar index
        subagent_info,
        chat_cache: ConversationChatRenderCache,
    },
    session_panel: SessionPanel {
        list: ListPanel<SessionEntry>,       // legacy / hydrate sessions
        chat_states: HashMap<String, ChatState>,
    },
    inputs: InputsPanel,                    // conversation / agent InputState + metrics
    overlays: OverlaysPanel,                // delete_confirm / project_create / approval_selected
    focus, panel_areas, status, error_flash, flash_copied,
    render_cache: RenderCache,              // agent chat cache — top-level for split borrow
}
```

`ListPanel<T>`（`ui/list_panel.rs`）统一列表三件套，用 `select` / `navigate` / `navigate_with_len` / `replace_items` / `clear` 同步 `selected` 与 ratatui `ListState`，避免只改一端。

Ownership 要点:

- **跨子结构查询仍在 `UiState` 上**：`current_session_id`、`current_chat_mut` / `current_chat_and_cache_mut`、flat session helpers、`conversation_agent_mention_candidates`。这些要同时看 `nav` + conversation sessions + `session_panel`，不能硬塞进某一个 panel。
- **Agent session 选择是 flat 索引**：`conversation.agent_sessions.items` 是源 `SessionSummaryEntry` 列表；`selected` 是 sidebar 扁平树 index（父 + subagent 子行）。键盘、鼠标、Enter 和 `current_session_id()` 都通过 flat helper 映射回源 session。不要把 `selected` 当成 `items[i]`。
- **双轨并存**：`conversation` 是 conversation-centric 路径；`session_panel` 仍服务 direct agent panel、daemon hydration、session lifecycle 和旧 MCP/teamwork 入口。两者各自拥有 `ListState`（不再共享单一 `agent_list_state`）。
- **Cache 归属**：conversation 时间线 cache 在 `conversation.chat_cache`；agent direct chat 的 `render_cache` 留在 `UiState` 顶层，方便与 `session_panel.chat_states` split borrow。
- **Ctrl+P / Ctrl+T** 只截断 `nav.stack`，不清空 conversation/list 数据。

`SessionSummaryEntry.parent_session_id.is_some()` 表示 subagent。用户可见的 project 层列表不再把 thread 当成 conversation。

当前主要组件:

| 文件 | 作用 |
|------|------|
| `ui/list_panel.rs` | 通用 `ListPanel<T>` 三件套 |
| `ui/panels.rs` | `NavPanel` / `ConversationPanel` / `SessionPanel` / `InputsPanel` / `OverlaysPanel` |
| `ui/project_list.rs` | Project 主列表和侧栏 |
| `ui/conversation_list.rs` | Conversations 列表和项目侧栏 |
| `ui/conversation_detail.rs` | Agent session 侧栏（flat 树） |
| `ui/conversation_view.rs` | Conversation 主时间线与 chat cache |
| `ui/chat.rs` | AgentDetail 聊天视图（tool 折叠/diff/json 详情、markdown transcript） |
| `ui/approval_overlay.rs` | AgentDetail pending approval/permission 可选项覆盖层 |
| `ui/input_bar.rs` | 多行输入、agent mention、路径补全 |
| `ui/project_create_dialog.rs` | Project 创建模态 |
| `ui/status_bar.rs` | 后端状态、agent 状态、快捷键提示 |
| `render/markdown.rs` | AgentDetail markdown：heading/list/code/diff/table/tasklist/strikethrough |

Conversation 层布局是状态栏 + 主体 + 输入行。主体左侧为 list/timeline，右侧为 project 或 agent session 侧栏；AgentDetail 在右侧增加 direct agent chat/input。已删除 `ui/room_list.rs`、`ui/project_sessions.rs`、`ui/thread_list.rs` 和旧 overview/detail render 分支。

### AgentDetail transcript 渲染（统一展示，视觉借鉴 Grok）

**消费契约（agent-agnostic）：** 上层只吃 daemon 投影后的 `LocalIngestFrame.ui_events: Vec<UiEventMessage>`，**不**解析 Grok/Codex/Claude 原生 JSON。Translator 的价值就是把各 CLI 磨成同一事件面；TUI 展示逻辑必须对所有 agent 同一套路径：

```text
LocalIngestFrame.ui_events
  → ChatState::apply_ui_events  (translation/chat_state.rs)
  → Vec<ChatItem>
  → ui/chat.rs paint
```

| UiEventMessage | ChatItem / 行为 |
|----------------|-----------------|
| MessageStarted + TextDelta | UserMessage / AssistantText（tool 后可新开气泡） |
| ReasoningDelta | Reasoning（可折叠 Thought） |
| ToolCallPlaced / Completed | ToolCall（动词 + bare target + kind body） |
| SubagentSpawned / StatusUpdated | SubagentCall 卡片 |
| Raw(approval/*) | PendingAgentRequest 审批 overlay |
| Raw(其它) | 默认丢弃（debug log） |
| SessionClosed / Error | SystemMessage / Error |

视觉与文案**借鉴** GrokNight（中性灰底 + TokyoNight 强调色），但**不是** Grok pager 状态机的复刻：不依赖 Grok crates，也不按 agent 分支渲染。tool 分类用 `ToolKind::from_tool_name`，兼容统一名 `"read: path"` 与各 CLI 原生 tool 名。

| 块 | Grok 风格 |
|----|-----------|
| User | `❯ ` 前缀 + body，无 `[You]` |
| Agent | **无 role chrome**，直接 markdown body |
| Thinking | 流式 `Thinking…` / 结束 `Thought`；折叠默认；展开 body 带 `│ ` quote bar |
| Tool header | `ToolKind` 动词 + **裸 path/cmd**（无 `file=` 标签）：`Read src/main.rs`、`Edited x.rs +3/-1`、`Ran cargo test` |
| Tool body | 按 kind：Edit = 无边框 single-gutter diff（**完成且像 patch 时自动展开**）；Execute = `$ cmd` + stdout 截断；Read = 行号 + syntax HL；Other = 缩进 preformatted |
| Subagent | `Running/Ran subagent <agent> #short` |
| Diff（tool） | indent + 单列 new-line gutter + add/del 底色；折叠 header 彩色 `+N/-M`；**无** `┌─ diff ─` 盒 |
| Diff（assistant ````diff`） | 保留 bordered 双 gutter code fence |

**模块**

| 文件 | 职责 |
|------|------|
| `translation/tool_kind.rs` | `ToolKind` + 动词表（Read/Edit/Execute/…） |
| `translation/tool_summary.rs` | bare target 摘要、`+N/-M` diffstat、`parse_diffstat` |
| `ui/chat.rs` | header / kind-aware expanded body |
| `render/markdown.rs` | `render_tool_diff` / `render_tool_preformatted` / `render_tool_read_body` |

**Verb-group 折叠**（Grok `scrollback/state/verb_group`）:

- 连续可折叠 tool（Read/Search/List/WebFetch/WebSearch/Skill）+ Subagent 收成一行：`Read 3 files, Searched 2 patterns`
- Execute / Edit / Other **不**参与 eager fold
- 折叠 idle Thought 可并入 run，但不计入 label
- 单成员也 fold（`Read 1 file`），避免第二成员到来时布局跳动
- 点 header / `e` 在 run 起点时展开组，显示聚合 header + 各 member 行
- 实现：`translation/verb_group.rs` + `ChatState.verb_group_expanded`

**明确不移植**：OSC8 路径链接、progressive full-file edit HL、mermaid/媒体、truncation “N more” dense fold、raw-markdown toggle。

**交互**

- 键盘 `e`：切换最近 tool/thinking 折叠
- 鼠标点 header：`try_toggle_fold_at_click`（render cache hit-test）
- 消息块之间 **空行 gap**（无全宽 `─`）
- 绘制硬裁剪 `truncate_line_to_width(inner.width)`，防止侵入右侧 sidebar

Markdown：tables / strikethrough / tasklists；GrokNight 色：heading teal、code blue1、quote muted。

## 焦点

`focus.rs` 的 pane 枚举已简化为:

```rust
pub enum PaneId {
    MainList,
    MainChat,
    Sidebar,
    Input,
    ApprovalOverlay,
}
```

`FocusManager` 维护线性顺序 `[MainList, MainChat, Sidebar, Input]`。`Input` 在 `Conversation` 层映射到 conversation input（`InputTarget::Conversation` / `conversation_input`），在 `AgentDetail` 层映射到 agent input。`Tab` / `BackTab` 只在这四个 pane 间循环。`ApprovalOverlay` 是 pending request 存在时的模态键盘上下文，不进入 Tab 顺序。旧的 `Room*` 命名（`room_input`、`RoomAction`、`InviteAgentToRoom` 等）已收敛为 conversation 词表。

## Action / Effect 流

原始事件进入 `app/event_loop.rs` 后被映射为语义 `Action`:

```text
AppEvent -> Action -> update() -> Effect -> App effect executor -> AppEvent/EffectResult
```

关键模块:

| 文件 | 职责 |
|------|------|
| `app/event_mapping.rs` | 键盘/焦点到 Action 或 InputTarget |
| `update/nav.rs` | Project/conversation 导航和 conversation input submit |
| `update/agent.rs` | AgentDetail submit、pending approval/question |
| `app/event_loop.rs` | async effect 执行，RPC 调用，事件回流；鼠标命中走表驱动 `MouseHit`（panel area → click/scroll target） |
| `app/conversation_ops.rs` | create/start conversation 共用 RPC 序列（append 失败会报错，不再静默丢弃） |
| `app/submission.rs` | thread resume/send（`resume_session(..., auto_continue=false)`）、直接 agent 消息发送 |
| `app/conversation_ops.rs` | open/hydrate 时对最多一个 top-level `needs_continue` session 调 `resume_session(..., true)` |
| `app/lifecycle.rs` | init、daemon replay、ingest/manager/tick；tick 只做轻量 flash/status，daemon list 后台化（`DaemonThreadsListed` → metadata-only）；ingest/manager 直接处理 |

**Agent session resume after exit：** TUI 退出若 stop managed daemon，daemon 将 sessions **suspend**（非 close）并可选 `needs_continue`。重开 conversation 时 auto-continue 至多一次；发消息路径只 reattach，用户文本抢占 CONTINUE。详见 `architecture-daemon.md` 停机与 resume。

Conversation input submit 统一走 `NavAction::SubmitConversationInput`（`update/nav.rs`），不再有独立的旧 Room submit 双路径。分离两个文本:

- `message_body`: 用户输入原文，去掉尾随空白后写入 `chat_messages`，例如 `@codex#abc fix tests`（effect 字段也统一用 `message_body`，不再用 `group_text`）。
- `prompt`: 去掉 `@agent`/`#short` 路由前缀后真正发送给 agent 的文本。

新建或邀请 agent 时，TUI 先乐观插入一条 pending `ConversationMessageEntry`，后台再调用 `append_conversation_message` 持久化；append/list 失败会 `ProjectFailed` / error flash，不会 `unwrap_or_default` 静默空列表。`@agent` 空 body 允许只把 agent 拉进 conversation，不发送空 prompt。未检测到任何 agent 时拒绝提交，不再静默默认 Codex。

**文本选区自动复制**（对齐 Grok CLI scrollback drag-select）:

- **Conversation 主时间线**与 **AgentDetail agent chat** 均支持鼠标拖选：`Down` 锚定、`Drag` 更新 focus、`Up` 提取纯文本并 `Effect::CopyToClipboard`。
- 实现：`state/selection.rs`（命中 content area、选区状态）、conversation 侧 `ConversationPanel.selection` + `conversation_view` 高亮/抽取、agent 侧 `ChatState.selection` + `ui/chat` 高亮/抽取。
- Conversation / agent 选区互斥；鼠标松开或复制成功后立即清空高亮（`flash_copied`），避免后续拖动继续改选区。`Ctrl+C` 在 `MainChat` 焦点下优先复制当前选区。
- Conversation 的 `MouseHit.allow_drag = true`；列表/input 区域不参与拖选。

路径补全不在输入渲染路径同步读目录。`PathCandidate` 与 `list_path_candidates` 在底层 `path_complete.rs`（event/effect 可直接依赖，不经过 `ui::`）。`InputState::sync_path_picker()` 只记录 token/sequence 并返回 `Effect::ResolvePathCandidates`；`app/event_loop.rs` 用 `tokio::task::spawn_blocking` 调用 `path_complete::list_path_candidates`，完成后发送 `AppEvent::PathCandidatesResolved`。输入框只接受当前 sequence 且 token/range 仍匹配的结果，旧异步结果会被丢弃；`accept_path_completion` 对 selected index 与 replace range 做边界/`char_boundary` 检查。agent mention 和路径候选都使用大小写不敏感 substring 匹配，不引入 fuzzy 依赖。

## Daemon 本地 RPC

TUI daemon 模式只依赖 local RPC surface:

| 方法 | 作用 |
|------|------|
| `minos_local_list_projects` / `minos_local_create_project` | Project 管理 |
| `minos_local_list_conversations` / `minos_local_create_conversation` | Conversation 列表/创建 |
| `minos_local_list_conversation_messages` | 主时间线 |
| `minos_local_list_conversation_agent_sessions` | agent session 列表 |
| `minos_local_start_agent_in_conversation` | conversation 内启动 agent |
| `minos_local_append_conversation_message` | 写主时间线消息 |
| `minos_local_read_session_raw_history` | AgentDetail 历史回放 |
| `minos_local_subscribe_ingest` / `minos_local_subscribe_manager_events` / `minos_local_subscribe_conversation_events` | 实时更新 |

本地 RPC 不再注册 `list_project_sessions` 或 `start_agent_in_project`。

## 持久化模型

daemon 的本地 SQLite 以最新目标结构为准，不维护旧 schema 兼容层。核心关系:

```text
projects 1:N conversations 1:N sessions
sessions 1:N subagent sessions via parent_session_id
conversations 1:N chat_messages
sessions 1:N events
```

性能要点:

- `conversations(project_id, updated_at_ms DESC, conversation_id)` 支撑 project 下 conversation 列表。
- `sessions(conversation_id, last_activity_at DESC, session_id)` 支撑 conversation 右侧 session 列表。
- `sessions(conversation_id, agent, last_activity_at DESC, session_id)` 支撑 `@agent#short` 候选查找。
- `sessions(parent_session_id, last_activity_at DESC, session_id)` 支撑 subagent 子项查询；TUI 当前复用 conversation session 列表并在前端按 parent id 扁平化，不新增 `list_subagents` RPC。
- `chat_messages(conversation_id, message_seq DESC)` 支撑时间线分页。
- conversation 行冗余 `message_count`、`agent_session_count`、`last_message_preview`，避免列表页 N+1 `COUNT(*)`。

## Translation 与 AgentDetail

每个 agent run 的 direct chat 存在 `ChatState` 中。**raw agent 事件 → `UiEventMessage` 的投影在 daemon / `minos-ui-protocol` 完成**；TUI 只消费已投影的 `LocalIngestFrame.ui_events`。history replay 通过 `read_session_raw_history` 拉取帧后，`ChatState::apply_ui_events` 把 `UiEventMessage` 投影到 `ChatItem`:

| 模块 | 职责 |
|------|------|
| `translation/chat_state.rs` | `UiEventMessage` -> `ChatItem`（按到达顺序 append；连续 `ReasoningDelta` 只更新 timeline 末尾同 `message_id` 的 reasoning item，中间插入 tool/text 后再次 thinking 开新 item） |
| `translation/pending_request.rs` | approval/permission/question 请求 |
| `translation/tool_summary.rs` | 工具参数和输出摘要 |
| `ui/chat/cache.rs` | direct chat 可见行缓存；`structure_version` 不变时只重建 fingerprint 变化的 segment（流式 tail） |
| `frame.rs` | `schedule_frame` / `schedule_frame_in` / `schedule_frame_streaming`；帧合并 |
| `path_complete.rs` | path 补全类型 + 目录列举（event / effect / UI 共享；避免 event 层依赖 UI） |
| `agent_route.rs` | `@agent` / `@agent#short` / `@ProfileName` / `@p/<id>` 解析（与 desktop 对齐）；bare `@agent` **先复用**同 conversation 顶层未关闭 session，否则新建时选该 runtime **最新** host profile 并传 `profile_id`；`short_session_id(&str) -> &str` 为全 TUI 唯一 short-id 实现 |

旧的 TUI 内嵌 raw-event `translation/agent.rs`（`AgentTranslationState` / `translate`）已删除：投影只在 daemon / `minos-ui-protocol` 完成。同步删除的死路径包括 `Effect::HandleIngest`、`HandleManagerEvent`、`HandleMcpToolCall`、`InviteAgentToRoom`、`DispatchPromptToExistingAgent`、`AppEvent::McpToolCall` 与 in-process MCP tool 事件。

AgentDetail 文本渲染走 `render/markdown.rs` 的 `pulldown-cmark` 事件流；围栏代码块通过 `render/highlight.rs` 使用 `syntect` + `two-face` 做语法高亮，超过大小/行数护栏或语言未知时降级为普通 code 样式。diff/patch 代码块和裸 diff 行保留 TUI 自己的增删行着色。

AgentDetail scroll 状态在 `ChatState` 中使用 `u32`，渲染层只在把行号传给缓存窗口时转为 `usize`，不再把最大滚动行数截到 `u16::MAX`。

### 渲染性能模型

Minos TUI **不是** Codex 式「定稿进终端 scrollback」；历史仍在应用 viewport 内滚动。性能靠：

1. **按需帧**：`FrameRequester` 合并请求，默认最小间隔 16ms；ingest 走 `schedule_frame_streaming`。
2. **Synchronized Update**：`main::draw_ui` 包一层 `stdout().sync_update`。
3. **可见窗口 paint**：`RenderCache` / `ConversationChatRenderCache` 只把可见行交给 `Paragraph`（非整 transcript 每帧 clone 进 widget 树）。
4. **流式 tail**：
   - `ChatState.version`：任意视觉变更；`structure_version`：item 增删/结构变化。
   - 纯 `TextDelta` 等原地更新只 bump `version`，cache 按 fingerprint 只重建脏 segment。
   - 流式 markdown holdback（Codex 思路）：`render/table_detect.rs` + `ui/stream_holdback.rs`
     - fence tracker（忽略 code fence 内的 `|`）
     - pending header / confirmed table 整表留在 mutable tail，直到 `is_streaming=false`
     - 未完成 partial 行 holdback
5. **流式 commit 快照**（`RenderCache` + `build_streaming_segment_with_commit`）：
   - 流式 item 的 holdback-stable 源文本冻结为 `StreamCommitSnapshot`（`stable_source` + `body_visual_lines`）。
   - **始终对完整 `stable` 做 markdown 全量渲染**（不再对 delta fragment 单独 `render_markdown` 再 append）。delta 路径会把每个 token 当成新 Paragraph，并无法对末行 visual wrap 续接。
   - holdback 只保证 fence/table 不抖动；prose/inline 的 stable 仍可能不完整，全量投影至少语义一致。
   - `is_streaming=false` 时清空 commit，终态同样全量渲染。
6. **Conversation 时间线 cache**：支持前缀不变时的 append-only 与「仅最后一条变更」的局部重建。
7. **Agent chat 懒测量窗口**（Grok 模型）：estimate 全量 + exact 视口；Conversation timeline 仍用 `messages_revision` + 最近 256 条 full layout 护栏。
8. **帧时机**：Resize 走 `schedule_frame_in(50ms)`；copied flash 在 TTL 到期再 schedule 一帧清除。

刻意未做：终端 cell-level diff 自研 Terminal、历史卸载到 OS scrollback、完整 Grok 滚轮加速度状态机 / `AdaptiveChunkingPolicy` 队列 drain、off-screen render-cache eviction（`EVICT_KEEP_MARGIN`）。

pending approval/permission/question 有明确选项时，`ui/approval_overlay.rs` 用 `Clear` 在 AgentChat 底部覆盖固定高度区域。数字键、Up/Down、Enter、`y`/`n`/`a`/`s`/`q` 和 Esc 会被 overlay 模态处理，最终仍复用 `Effect::SubmitPendingAgentRequest`。

| Pending 类型 | 来源 | 回复路径 |
|--------------|------|----------|
| CodexApproval | `approval/request` | `approval_decision` |
| CodexUserInput | `item/tool/requestUserInput` | `approval_decision`（answers map） |
| GrokPlanApproval | `x.ai/exit_plan_mode` | `approval_decision` → outcome |
| GrokUserQuestion | `x.ai/ask_user_question` | `approval_decision` → `{outcome, answers}` |
| OpencodePermission | `opencode/permission.updated` | `respond_opencode_permission` |
| OpencodeQuestion | `opencode/question.asked` | `respond_opencode_question` |

自由文本或多问题 pending request 继续走 Agent Input。Claude 的权限/提问尚未接入。

`ChatItem::ToolCall` 保留自动展开状态 `is_expanded`，并用 `is_user_toggled: Option<bool>` 表示用户覆盖。`e` 键只翻转 transcript 中最后一个 tool call；`None` 时按自动规则渲染，`Some` 时按用户选择渲染。

`ChatState::last_completed_assistant_text()` 只从明确完成的 assistant message 取最终文本；中间 streaming 文本不会作为最终回复记录。daemon 的 `conversation_completion` 用同一语义（工具/思考打断后的 last segment）写 `agent-result`，避免 Grok 式过程叙述污染 conversation。TUI 在打开 conversation 和启动 agent 时维护 `session_id -> conversation_id` 映射，因此 agent 在后台或其他 project 可见时完成，也会写回对应 conversation，而不是只更新当前屏幕上的 timeline。

`UiEventMessage::SubagentSpawned` 在父线程 transcript 中生成 `ChatItem::SubagentCall`，并把 subagent 补进当前 conversation 的右侧 session 列表；`SubagentStatusUpdated` 更新该卡片状态。subagent 自身 transcript 仍是普通 session history，通过相同 `read_session_raw_history` replay。

Opencode 的 subagent 来自父 session 的 `task` 工具：driver 在唯一 active task 时注册子 thread 并发出合成 `minos.subagent.spawned`；`translate_opencode` 在对应 `task` tool 进入 `completed`/`error` 时再发 `SubagentStatusUpdated`（与 Codex collab agent 终态对齐）。`ChatState` 额外在 `ToolCallCompleted` 与 `SubagentCall.tool_call_id` 匹配时关闭卡片，避免旧 projection 回放后仍显示 `running`。

## Teamwork/MCP 现状

Teamwork MCP 绑定当前 conversation，由 **daemon** MCP socket 处理（非 TUI in-process）。工具：`list_conversation_messages`、`delegate_to_agent`、`get_delegation_status`、`wait_delegation`、`cancel_delegation`、`post_conversation_update`。消息写入后发布 `ConversationMessageAppended`；TUI 订阅并刷新当前 conversation。

`delegate_to_agent` 可选 `profile_id`（稳定）或 `target_profile`（按唯一 name 解析）；仅 `target_agent` 时与 desktop bare `@agent` 一样应用该 runtime **最新** host profile。启动走 daemon `start_agent_in_conversation_with_options`（与 RPC `profile_id` 路径同一 launch 合并语义：profile 填 model/effort/instructions）。深度限制：被委托 thread 只能 delegate 回 source agent。可见消息带 `delegation_id` / `mentions` 元数据。目标 agent 终态由 daemon `conversation_completion` 写回 conversation、完成 delegation，并按 busy 策略投递/排队到 source thread。`wait_delegation` 阻塞到 terminal 或 timeout，并可返回 `source_delivery`。conversation timeline 渲染支持 `@agent` / `@agent#short` 高亮；带 `reply_to_message_id` 的消息在当前页能解析到父消息时展示引用预览（作者标签 + 正文摘要，最多 2 行），父消息不在当前加载页则回退为 unavailable 提示。

## 验证命令

常用校验:

```bash
cargo check -p minos-protocol -p minos-daemon -p minos-tui
cargo test -p minos-tui -- --test-sessions=1
cargo test -p minos-daemon -- --test-sessions=1
cargo test -p minos-daemon --features test-support --test local_rpc project_methods_are_registered_on_local_rpc
```
