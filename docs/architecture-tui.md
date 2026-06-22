# 终端 UI (minos-tui) 架构文档

`minos-tui` 是 Minos 的 Ratatui/Crossterm 终端客户端。它支持嵌入式模式直接管理 agent 子进程，也支持通过 JSON-RPC 连接 `minos-daemon`。当前 TUI 的主业务层级是 conversation-centric:

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
| `--backend <KIND>` | `embedded` 或 `daemon` |
| `--daemon-url <URL>` | 显式 daemon WS URL；未提供时读取 discovery，失败则托管本地 daemon |

## 启动序列

1. `main.rs` 解析 CLI、工作目录和日志路径。
2. 安装 teamwork skills 到支持的 agent 配置目录。
3. 构建 `EmbeddedBackend` 或 `DaemonBackend`。
4. 创建 `App` 并执行 `app.init()`：检测 CLI、同步 daemon 线程、解析启动 project。
5. 初始化 terminal、鼠标捕获和 bracketed paste。
6. 启动 terminal、tick、ingest、manager event 四类事件泵。
7. 进入主循环；状态变更通过 `FrameRequester` 合并 draw 请求。
8. 退出时恢复 terminal，并停止 TUI 托管的 daemon。

## 后端抽象

`backend/mod.rs` 的 `AgentBackend` 是 TUI 唯一依赖的后端接口。conversation 相关方法是 project 导航的主路径:

| 方法 | 用途 |
|------|------|
| `list_projects()` / `create_project()` | Project 层数据 |
| `list_conversations(project_id)` | Conversations 层列表 |
| `create_conversation(project_id, title)` | 创建 conversation |
| `list_conversation_messages(conversation_id)` | 主时间线消息 |
| `list_conversation_agent_sessions(conversation_id)` | 右侧 agent session 列表 |
| `start_agent_in_conversation(conversation_id, agent, workspace)` | 在 conversation 内创建 agent run |
| `append_conversation_message(...)` | 写 conversation 主时间线 |
| `list_threads()` / `read_thread_raw_history()` | daemon replay 和 AgentDetail 历史水化 |
| `start_agent()` / `send_message()` / `resume_thread()` | 直接 agent thread 控制 |

`DaemonBackend` 调用 `minos_local_*` 本地 RPC。TUI 本地控制使用 `minos_local_list_conversations`、`minos_local_create_conversation`、`minos_local_start_agent_in_conversation` 和 `minos_local_append_conversation_message`，不再使用 `list_project_threads` / `start_agent_in_project`。

`EmbeddedBackend` 维护内存 project/conversation/message/session 集合，方便开发和测试；agent 运行仍由进程内 `AgentManager` 驱动。

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
        thread_id: String,
        agent: AgentName,
    },
}
```

`UiState.nav_stack: Vec<NavLevel>` 是唯一导航状态。下钻 push，Esc/uplevel pop，Projects 层 Esc 退出。`NavLevel` 只提供 `project_id()`、`conversation_id()` 和 `esc_quits()` 这类查询，不再通过单字段重建父级。

导航流:

1. **Projects**: `Up/Down` 选择 project，`Enter` 加载 conversations，`n` 打开 project 创建对话框，`Esc` 退出。
2. **Conversations**: `Up/Down` 选择 conversation，`Enter` 打开主时间线；输入 prompt 时会创建新 conversation 并启动指定/default agent。
3. **Conversation**: 中间列显示 conversation messages，右侧显示该 conversation 下的 agent sessions。session 列表按 `ThreadSummaryEntry.parent_thread_id` 扁平化为父子树；subagent 作为父 session 的只读子项展示。输入 `@agent message` 会写主时间线并启动 agent run，`@agent#short message` 只匹配顶层 agent run，不路由到 subagent。
4. **AgentDetail**: 显示单个 agent run 的 direct chat；顶层 run 的 Agent Input 以 `@agent#short ...` 形式写入 conversation 主时间线，并把 clean prompt 发给该 run。subagent 的 AgentDetail 只读，只用于观察 transcript、工具状态和终态。

## UI 状态与布局

`ui/mod.rs::UiState` 的 conversation 关键字段:

```rust
nav_stack: Vec<NavLevel>,
projects: Vec<ProjectEntry>,
conversations: Vec<ConversationEntry>,
conversation_messages: Vec<ConversationMessageEntry>,
conversation_agent_sessions: Vec<ThreadSummaryEntry>,
selected_project: Option<usize>,
selected_conversation: Option<usize>,
selected_agent_session: Option<usize>,
subagent_info: HashMap<String, SubagentInfo>,
chat_states: HashMap<String, ChatState>,
```

`ThreadSummaryEntry.parent_thread_id.is_some()` 表示 subagent。`selected_agent_session` 存储的是 sidebar 扁平树 index；键盘、鼠标、Enter 和 `current_thread_id()` 都通过同一个 flat helper 映射回原始 session。`threads` / `selected_thread` 仍存在，用于 direct agent panel、daemon history hydration、thread lifecycle 和旧 MCP/teamwork 入口。用户可见的 project 层列表不再把 thread 当成 conversation。

当前主要组件:

| 文件 | 作用 |
|------|------|
| `ui/project_list.rs` | Project 主列表和侧栏 |
| `ui/conversation_list.rs` | Conversations 列表和项目侧栏 |
| `ui/conversation_detail.rs` | Conversation 主时间线和 agent session 列表 |
| `ui/chat.rs` | AgentDetail 聊天视图 |
| `ui/input_bar.rs` | 多行输入、agent mention、路径补全 |
| `ui/project_create_dialog.rs` | Project 创建模态 |
| `ui/agent_picker.rs` | Agent 选择/mention 补全 |
| `ui/status_bar.rs` | 后端状态、agent 状态、快捷键提示 |

Conversation 层布局是状态栏 + 主体 + 输入行。主体左侧为 list/timeline，右侧为 project 或 agent session 侧栏；AgentDetail 在右侧增加 direct agent chat/input。已删除 `ui/room_list.rs`、`ui/project_sessions.rs`、`ui/thread_list.rs` 和旧 overview/detail render 分支。

## 焦点

`focus.rs` 的 pane 枚举已简化为:

```rust
pub enum PaneId {
    MainList,
    MainChat,
    Sidebar,
    Input,
}
```

`FocusManager` 维护线性顺序 `[MainList, MainChat, Sidebar, Input]`。`Input` 在 `Conversation` 层映射到 room/conversation input，在 `AgentDetail` 层映射到 agent input。`Tab` / `BackTab` 只在这四个 pane 间循环。

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
| `app/event_loop.rs` | async effect 执行，RPC 调用，事件回流 |
| `app/submission.rs` | thread resume/send、直接 agent 消息发送 |
| `app/lifecycle.rs` | init、daemon replay、ingest/manager/tick |

Conversation input submit 分离两个文本:

- `message_body`: 用户输入原文，去掉尾随空白后写入 `chat_messages`，例如 `@codex#abc fix tests`。
- `prompt`: 去掉 `@agent`/`#short` 路由前缀后真正发送给 agent 的文本。

新建或邀请 agent 时，TUI 先乐观插入一条 pending `ConversationMessageEntry`，后台再调用 `append_conversation_message` 持久化。`@agent` 空 body 允许只把 agent 拉进 conversation，不发送空 prompt。

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
| `minos_local_read_thread_raw_history` | AgentDetail 历史回放 |
| `minos_local_subscribe_ingest` / `minos_local_subscribe_manager_events` | 实时更新 |

本地 RPC 不再注册 `list_project_threads` 或 `start_agent_in_project`。

## 持久化模型

daemon 的本地 SQLite 以最新目标结构为准，不维护旧 schema 兼容层。核心关系:

```text
projects 1:N conversations 1:N threads
threads 1:N subagent threads via parent_thread_id
conversations 1:N chat_messages
threads 1:N events
```

性能要点:

- `conversations(project_id, updated_at_ms DESC, conversation_id)` 支撑 project 下 conversation 列表。
- `threads(conversation_id, last_activity_at DESC, thread_id)` 支撑 conversation 右侧 session 列表。
- `threads(conversation_id, agent, last_activity_at DESC, thread_id)` 支撑 `@agent#short` 候选查找。
- `threads(parent_thread_id, last_activity_at DESC, thread_id)` 支撑 subagent 子项查询；TUI 当前复用 conversation session 列表并在前端按 parent id 扁平化，不新增 `list_subagents` RPC。
- `chat_messages(conversation_id, message_seq DESC)` 支撑时间线分页。
- conversation 行冗余 `message_count`、`agent_session_count`、`last_message_preview`，避免列表页 N+1 `COUNT(*)`。

## Translation 与 AgentDetail

每个 agent run 的 direct chat 存在 `ChatState` 中。daemon history replay 通过 `read_thread_raw_history` 拉取 `LocalIngestFrame`，再把 `UiEventMessage` 投影到 `ChatItem`:

| 模块 | 职责 |
|------|------|
| `translation/chat_state.rs` | `UiEventMessage` -> `ChatItem` |
| `translation/agent.rs` | provider translator 状态 |
| `translation/pending_request.rs` | approval/permission/question 请求 |
| `translation/tool_summary.rs` | 工具参数和输出摘要 |
| `ui/chat/cache.rs` | direct chat 可见行缓存 |

`ChatState::last_completed_assistant_text()` 只从明确完成的 assistant message 取最终文本；中间 streaming 文本不会作为最终回复记录。

`UiEventMessage::SubagentSpawned` 在父线程 transcript 中生成 `ChatItem::SubagentCall`；`SubagentStatusUpdated` 更新该卡片状态。subagent 自身 transcript 仍是普通 thread history，通过相同 `read_thread_raw_history` replay。

## Teamwork/MCP 现状

`group_chat.rs` 和 `app/group_chat.rs` 仍承担 teamwork MCP 命令队列、历史补偿和旧 agent-result 记录路径。它不再驱动主导航层，也没有 room list UI。后续若把 MCP 协作完全迁到 `chat_messages`，应同时删除 `GroupChatStore`、`minos_local_read_group_chat` 和对应测试。

## 验证命令

常用校验:

```bash
cargo check -p minos-protocol -p minos-daemon -p minos-tui
cargo test -p minos-tui -- --test-threads=1
cargo test -p minos-daemon -- --test-threads=1
cargo test -p minos-daemon --features test-support --test local_rpc project_methods_are_registered_on_local_rpc
```
