# 终端 UI (minos-tui) 架构文档

> 本文档详细描述 `minos-tui` crate 的架构、UI 组件和交互逻辑。

## 概述

`minos-tui` 是一个全功能终端 UI 客户端，使用 Ratatui + Crossterm 构建。支持嵌入式模式（直接管理 agent 子进程）和守护进程模式（通过 JSON-RPC 连接 `minos-daemon`）。核心特性包括多 agent 管理、群聊房间、Agent 间 MCP 协调。

**源码路径**: `crates/minos-tui/`

## CLI 参数

```
minos-tui [OPTIONS]
minos-tui chat-mcp [OPTIONS]   (隐藏子命令，MCP 服务器)
```

| 参数 | 用途 |
|------|------|
| `-a, --agent <AGENT>` | 自动启动指定 agent（codex/claude/gemini/opencode） |
| `-w, --workspace <PATH>` | 工作目录（默认 `.`） |
| `--readonly` | 禁用消息发送 |
| `--backend <KIND>` | `embedded`（默认）或 `daemon` |
| `--daemon-url <URL>` | Daemon WS URL（或从 `~/.minos/run/tui-daemon-rpc.json` 自动发现） |

## 启动序列 (`src/main.rs`)

1. 解析 CLI 参数
2. 解析工作空间路径，初始化文件日志
3. 安装 teamwork skills 到所有 agent 配置目录
4. 构建后端（`EmbeddedBackend` 或 `DaemonBackend`）
5. 创建 `App`，调用 `app.init()`（检测 CLI、加载群聊历史、同步 daemon 线程）
6. 设置终端（ratatui + crossterm 鼠标捕获）
7. 启动 4 个事件泵（terminal, tick, ingest, manager）
8. 可选自动启动 agent（`--agent` 参数）
9. 进入主渲染循环
10. 退出时 `app.shutdown()`，恢复终端

## 后端抽象 (`src/backend/`)

### `AgentBackend` trait

| 方法 | 用途 |
|------|------|
| `detect_clis()` | 检测已安装 CLI agent |
| `start_agent(agent, workspace)` | 启动 agent 线程 |
| `send_message(thread_id, text)` | 发送用户消息 |
| `send_approval_decision(...)` | 审批 codex 请求 |
| `respond_opencode_permission(...)` | 审批 opencode 权限 |
| `interrupt_thread(thread_id)` | 中断线程 |
| `close_thread(thread_id)` | 关闭线程 |
| `delete_thread(thread_id)` | 删除线程 |
| `list_threads()` | 列出所有线程 |
| `resume_thread(thread_id)` | 恢复挂起线程 |
| `read_thread_raw_history(...)` | 读取原始事件历史 |
| `read_group_chat(...)` | 读取群聊消息 |
| `subscribe_ingest()` | 订阅原始事件流 |
| `subscribe_manager_events()` | 订阅生命周期事件 |
| `connection_state()` | 当前连接状态 |

### `EmbeddedBackend`

- 直接使用 `minos_agent_runtime::AgentManager`（进程内）
- 配置 chat MCP 集成（启动 `minos-tui chat-mcp` 子进程）
- 不支持线程恢复和原始历史读取

### `DaemonBackend`

- 通过 WebSocket 连接 `minos-daemon`（jsonrpsee WS client）
- 所有操作转为 JSON-RPC 调用（`minos_local_*` 前缀）
- 两个订阅泵转发 daemon 事件到本地 broadcast channel

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

叠加层: Agent Picker（选择 agent 的模态框）、Delete Confirm（删除确认模态框）。

### UI 组件

| 文件 | 组件 | 描述 |
|------|------|------|
| `ui/mod.rs` | `UiState`, `Focus`, `PanelAreas` | 布局编排、状态管理 |
| `ui/chat.rs` | Agent 聊天视图 | Markdown 渲染、diff 高亮、工具调用、代码块 |
| `ui/group_chat.rs` | 群聊视图 | 房间消息渲染 |
| `ui/input_bar.rs` | `InputState` | 多行输入编辑器 + `@` agent mention 自动补全 |
| `ui/thread_list.rs` | 线程列表 | agent 线程列表（状态颜色编码） |
| `ui/room_list.rs` | 房间列表 | 群聊房间列表 |
| `ui/status_bar.rs` | 状态栏 | 后端状态 + agent + 快捷键 |
| `ui/agent_picker.rs` | Agent 选择器 | 编号快速选择（1-9） |
| `ui/theme.rs` | 主题 | 颜色/样式常量 |

### 聊天渲染特性

- Markdown: 标题、列表、内联代码、围栏代码块
- Diff 语法高亮（绿色添加、红色删除、青色块头）
- 工具调用块（可展开 args/output）
- 推理/思考块（深灰色）
- 流式光标（闪烁块字符）
- Unicode 感知的自动换行

## 事件系统 (`src/event.rs`)

### `AppEvent` 枚举

```
AppEvent::Ingest(RawIngest)               // Agent 原始事件
AppEvent::ManagerEvent(ManagerEvent)       // 线程生命周期事件
AppEvent::AgentStartedForPrompt { ... }    // 后台 agent 启动完成
AppEvent::SendMessageFailed { ... }        // 异步发送错误
AppEvent::Key(KeyEvent)                    // 键盘事件
AppEvent::Mouse(MouseEvent)               // 鼠标事件
AppEvent::Resize(u16, u16)                // 终端大小变化
AppEvent::Tick                            // 200ms 定时器
```

### 4 个事件泵

1. **终端事件泵**（std 线程，250ms 轮询 crossterm）
2. **Tick 泵**（tokio interval，200ms）
3. **Ingest 泵**（转发 backend ingest broadcast → MPSC）
4. **Manager Event 泵**（转发 backend manager events → MPSC）

## 核心状态 (`src/app.rs`)

`App` 是中央状态机（~2950 行），协调所有 UI 和业务逻辑。

### 关键字段

```rust
App {
    backend: Arc<dyn AgentBackend>,
    ui: UiState,
    hydrated_threads: HashSet<String>,           // 已回放历史的线程
    thread_watermarks: HashMap<String, u64>,     // 每线程最高 seq
    applied_ingest_fingerprints: HashSet<String>, // 去重 ingest
    group_chat_store: GroupChatStore,             // SQLite 群聊持久化
    recorded_agent_results: HashMap<String, String>, // 去重群聊 agent 结果
}
```

### 线程水化

首次查看线程时，从 `read_thread_raw_history()` 加载历史，通过翻译管线重建 `ChatState.messages`。

### Ingest 去重

使用 `{thread_id}:{payload_json}` 指纹 + seq 水位线防止重复处理。

## 翻译管线 (`src/translation.rs`)

### `ChatState`（每线程状态）

```rust
ChatState {
    thread_id, agent,
    translation_state: AgentTranslationState,  // Agent 特定协议解析器
    messages: Vec<RenderedMessage>,
    pending_requests: Vec<PendingAgentRequest>,
    scroll_offset, auto_scroll, max_scroll,
    selection: Option<ChatSelection>,
}
```

### `RenderedMessage`

```rust
RenderedMessage {
    message_id, role (User/Assistant/System),
    text_parts: Vec<TextPart>,      // 文本或代码块
    tool_calls: Vec<ToolCallBlock>,  // 可展开工具调用
    reasoning: Option<String>,       // "Thinking" 文本
    is_streaming: bool,
    error: Option<String>,
}
```

## 群聊系统 (`src/group_chat.rs`)

### `GroupChatStore`

SQLite 持久化群聊房间和消息。支持:
- 多房间管理
- 分页加载
- Agent 结果自动发布（线程 Idle/Closed 时）
- MCP 命令队列处理

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

1. 按 `n` 或聚焦 agent 列表 → 打开 Agent Picker
2. 选择 agent → `backend.start_agent()` → `ManagerEvent::ThreadAdded`
3. Agent 输出通过 Ingest 事件流式传输 → 翻译 → 渲染

### 发送消息

1. 在 Room Input 输入文本
2. `@agent` 语法路由到特定 agent（`@codex`, `@claude#shortid` 等）
3. 消息同时记录到群聊
4. Agent 响应通过 ingest 流式返回

### 审批流程

1. Agent 遇到审批请求 → `Raw` 事件
2. 创建 `PendingAgentRequest` + 系统消息
3. 输入栏标签变为 "Agent Input: Reply Required"
4. 用户输入 "yes"/"approve" = 批准，其他 = 拒绝

### 线程管理

- **中断**: Ctrl+C（运行中则中断，否则退出）
- **关闭**: Ctrl+D（优雅关闭）
- **删除**: Delete 键 → 确认模态 → Enter/Y 或 Esc/N
- **恢复**: 向挂起线程发送消息时自动尝试

## 文件清单

| 文件 | 行数 | 职责 |
|------|------|------|
| `main.rs` | 363 | 入口、CLI、启动编排 |
| `app.rs` | ~2950 | 中央状态机、事件分发、业务逻辑 |
| `event.rs` | 109 | AppEvent 枚举、事件泵 |
| `backend/mod.rs` | 95 | AgentBackend trait |
| `backend/embedded.rs` | 181 | 进程内后端 |
| `backend/daemon.rs` | 462 | WS RPC 后端 |
| `translation.rs` | 1517 | 协议翻译、ChatState、RenderedMessage |
| `ui/mod.rs` | 547 | 布局、UiState |
| `ui/chat.rs` | 631 | Agent 聊天渲染 |
| `ui/input_bar.rs` | 813 | 多行输入 + mention 自动补全 |
| `ui/group_chat.rs` | 153 | 群聊渲染 |
| `group_chat.rs` | 221 | SQLite 群聊持久化 |
| `skills.rs` | 104 | Teamwork skill 安装 |
