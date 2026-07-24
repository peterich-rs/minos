# Conversation-Centric TUI 层级设计

> 日期: 2026-06-18
> 状态: 待审核
> 类型: TUI 导航 + 本地数据模型重构
> 关联: 取代 `2026-06-17-tui-nav-ux-redesign.md` 的 Sessions 层语义

## 1. 背景与动机

### 1.1 问题:层级错乱

当前 TUI 的 `Project → Sessions` 导航存在系统性层级错乱。`list_project_sessions` 直接返回**单个 agent 的 session**(每次 agent run),跳过了"对话容器"这一层。用户在 Sessions 列表看到的是零散的 agent run,而非一个可以容纳多 agent 的对话。

根因在 daemon 存储层:`sessions` 表一行 = 一次 agent run,`project_id` 只是平铺外键,没有任何"对话容器"把多个 agent session 归并。协议层、TUI 层都继承了这个错误前提。

### 1.2 两个割裂的系统

当前 daemon 有两套并行的聊天存储:

- **sessions/events 系统**:per-agent-session 的完整 streaming 日志(tool call、diff、reasoning)。seq 是 per-session 的,无法跨 thread 合并成统一时间线。
- **chat_rooms/chat_messages 系统**:workspace-derived 的群聊,只有粗粒度(user 消息 + agent 最终回复),已经是跨 agent 全局时间线。但 `room_id` 锁死成一个 workspace 一个 room,和 project 体系完全脱节。

两者没有 FK 关联,只有 `chat_messages.session_id` 软链接(非 FK)。

### 1.3 范围

本设计只关注 TUI 所需的本地 Project → Conversation → Agent session 层级，其它端的 conversation 模型不作为本次约束。

### 1.4 命名统一

全文统一术语:

| 概念 | 术语 | 面向 |
|------|------|------|
| 对话容器(project 下的聊天室) | **conversation** | 用户可见、RPC 命名、NavLevel、UI |
| 单个 agent 的一次 run | **thread** (= agent session) | 内部存储、events 键 |
| agent session 的唯一标识 | **agent_session_id** (= session_id) | NavLevel 检索、ChatItem 归属 |

`session_id` 只出现在 daemon store 和 protocol 的 `SessionSummary` 内部类型中。TUI 用户可见层一律用 `agent_session_id`。

## 2. 关键设计决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 层级模型 | Project → Conversation → (chat_messages 主时间线 + agent sessions 细节) | 支撑 TUI 四层导航,修正层级错乱 |
| 对话消息流渲染源 | 两者结合(C) | 主时间线用 chat_messages(轻量统一视图);AgentDetail 子视图用 events(完整 streaming 细节) |
| Conversation 创建时机 | 手动建空频道(B) | 用户在 Conversations 列表按 `n` 建对话(填 title),再进对话 @mention 拉 agent |
| 旧 workspace-room 数据 | 干净启动,不迁(B) | latest-only 开发态策略,无历史包袱 |
| 拉进 agent 的方式 | @mention 指定(A) | `@codex 帮我重构 foo` → 为该 agent 创建新 session |
| chat_messages 改造 | 复用 + room_id→conversation_id(A) | chat_messages 已是跨 agent 全局时间线,只差键的语义 |
| AgentDetail 唯一标识 | agent_session_id(非 AgentName) | 同一 conversation 可有多个同名 agent session |
| 布局比例 | 80% 主内容 / 20% 侧栏 | 对标 opencode sidebar,信息层次分明 |

## 3. 数据模型(终态 schema)

latest-only:直接维护终态 schema,不保留历史数据、不写兼容迁移、不做旧表迁移。实现时直接改 daemon canonical migrations/初始化 SQL;已有开发库按需 reset。

性能约束:
- 列表页不做 N+1 `COUNT(*)`;`conversations` 冗余保存 `message_count`、`agent_session_count`、`last_message_preview`。
- 分页全部走 keyset:conversation 列表按 `(project_id, updated_at_ms DESC, conversation_id)`,消息按 `(conversation_id, message_seq DESC)`。
- SQLite `INTEGER PRIMARY KEY` 已是 rowid 自增;不用 `AUTOINCREMENT`,避免额外开销。
- FK 明确 `ON DELETE` 行为,避免孤儿数据。

### 3.1 conversations 表

```sql
CREATE TABLE conversations (
    conversation_id      TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    title                TEXT NOT NULL,
    last_message_preview TEXT,
    message_count        INTEGER NOT NULL DEFAULT 0,
    agent_session_count  INTEGER NOT NULL DEFAULT 0,
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL,
    CHECK(length(title) > 0)
);
CREATE INDEX conversations_by_project_updated
    ON conversations(project_id, updated_at_ms DESC, conversation_id);
```

- `conversation_id`:daemon 生成的 UUID(不再用 `room_id_for_workspace` 的 workspace-derived 值)。
- `message_count` / `agent_session_count` / `last_message_preview`:列表展示用,由写入消息/创建 agent session 的事务同步维护。
- `updated_at_ms`:收到新消息时 `touch_conversation` 刷新,列表按此倒序。

### 3.2 chat_messages 表(全新,全局时间线)

```sql
CREATE TABLE chat_messages (
    message_seq     INTEGER PRIMARY KEY,
    message_id      TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    session_id       TEXT REFERENCES sessions(session_id) ON DELETE SET NULL,
    created_at_ms   INTEGER NOT NULL,
    sender_role     TEXT NOT NULL CHECK(sender_role IN ('user', 'agent')),
    agent           TEXT,
    body            TEXT NOT NULL,
    CHECK(
        (sender_role = 'user' AND session_id IS NULL AND agent IS NULL)
        OR
        (sender_role = 'agent' AND session_id IS NOT NULL AND agent IS NOT NULL)
    )
);
CREATE INDEX chat_messages_by_conversation_seq
    ON chat_messages(conversation_id, message_seq DESC);
CREATE INDEX chat_messages_by_session_seq
    ON chat_messages(session_id, message_seq DESC)
    WHERE session_id IS NOT NULL;
```

- `message_seq`:全局递增 rowid,跨 conversation。这是统一时间线的排序键。
- `session_id`:真 FK。user 消息为 NULL;agent_result 指向产出该消息的 agent session。
- `agent`:agent_result 时填 agent 标签(如 "codex");user 时 NULL。

### 3.3 sessions 表(加 conversation_id,去 project_id)

```sql
CREATE TABLE sessions (
    session_id           TEXT PRIMARY KEY,
    conversation_id     TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    workspace_root      TEXT NOT NULL REFERENCES workspaces(root),
    agent               TEXT NOT NULL,
    provider_session_id TEXT,
    status              TEXT NOT NULL CHECK(status IN ('active', 'running', 'idle', 'closed')),
    last_pause_reason   TEXT,
    last_close_reason   TEXT,
    last_seq            INTEGER NOT NULL DEFAULT 0,
    started_at          INTEGER NOT NULL,
    last_activity_at    INTEGER NOT NULL,
    ended_at            INTEGER
);
CREATE INDEX sessions_by_conversation_last
    ON sessions(conversation_id, last_activity_at DESC, session_id);
CREATE INDEX sessions_by_conversation_agent_last
    ON sessions(conversation_id, agent, last_activity_at DESC, session_id);
CREATE INDEX sessions_by_workspace ON sessions(workspace_root, last_activity_at DESC);
CREATE INDEX sessions_by_status ON sessions(status, last_activity_at DESC);
```

改动点(相对旧 schema):
- 新增 `conversation_id`(NOT NULL FK),替代 `project_id`。project 归属通过 `conversation → project` 间接获取。
- `codex_session_id` 改名 `provider_session_id`(原名误导,实际所有 agent 都用)。
- 删除 `project_id` 列。

### 3.4 events 表 — 不变

```sql
events (session_id, seq, body_kind, body_inline, artifact_id, ..., projection_json, ts_ms)
PRIMARY KEY (session_id, seq)
```

per-session event log。AgentDetail 子视图按 `session_id` 读,逻辑完全复用。

### 3.5 删除的表

- `chat_rooms` → 被 `conversations` 取代
- `chat_agent_sessions` → 信息从 `sessions.conversation_id + sessions.agent` 派生
- 旧 `chat_messages`(workspace-derived room_id 版)→ 被新 `chat_messages` 取代

### 3.6 ER 关系图

```
projects (project_id, name, workspace_path)
    │ 1:N
    ▼
conversations (conversation_id, project_id, title, updated_at_ms)
    │ 1:N                          │ 1:N
    ▼                              ▼
chat_messages                  sessions (agent sessions)
  (conversation_id,              (session_id, conversation_id,
   message_seq [全局],            agent, status, ...)
   sender_role,                     │ 1:N
   agent, body)                    ▼
                                events
                                  (session_id, seq, ...)
```

### 3.7 数据流总结(呼应决策 C)

- **Conversation 列表**:查 `conversations WHERE project_id = ? ORDER BY updated_at_ms DESC, conversation_id` → 直接拿计数和摘要,不额外 count。
- **Conversation 主时间线**(群聊视图):查 `chat_messages WHERE conversation_id = ? AND message_seq < ? ORDER BY message_seq DESC LIMIT ?` → keyset 分页后 UI 反转展示。
- **Agent 细节**(AgentDetail 子视图):查 `sessions WHERE conversation_id = ?` 得到 agent 列表;选中某 agent 后查 `events WHERE session_id = ?` 渲染完整 streaming 细节。
- **新 agent session 创建**:`@mention` → 创建 `sessions` 行(填 conversation_id,同事务更新 `agent_session_count`)→ 启动 agent → streaming 写 events → 完成后 upsert 一条 `chat_messages`(sender_role='agent')并更新 conversation 摘要/计数。

## 4. 协议层 RPC

### 4.1 新增类型

```rust
// minos-protocol/src/messages.rs

pub struct LocalConversationSummary {
    pub conversation_id: String,
    pub project_id: String,
    pub title: String,
    pub last_message_preview: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u32,
    pub agent_session_count: u32,
    pub participating_agents: Vec<AgentName>,
}

pub struct LocalConversationMessage {
    pub message_seq: i64,
    pub message_id: String,
    pub conversation_id: String,
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,        // "user" | "agent"
    pub agent: Option<AgentName>,
    pub body: String,
}
```

### 4.2 RPC 总览

| RPC | 参数 | 返回 | 数据源 |
|-----|------|------|--------|
| `list_projects` | — | `Vec<ProjectSummary>` | projects 表 |
| `create_project` | name, workspace_path | `ProjectSummary` | projects 表 |
| **`list_conversations`** | project_id, limit | `Vec<LocalConversationSummary>` | conversations 表 |
| **`create_conversation`** | project_id, title | `LocalConversationSummary` | conversations 表 |
| **`list_conversation_messages`** | conversation_id, before_seq, limit | `Vec<LocalConversationMessage>` | chat_messages 表 |
| **`list_conversation_agent_sessions`** | conversation_id | `Vec<SessionSummary>` | sessions 表 |
| **`start_agent_in_conversation`** | conversation_id, agent, workspace | `StartAgentOutcome` | sessions/events 表 |
| **`append_conversation_message`** | conversation_id, message_id, session_id, role, agent, body | message_seq | chat_messages 表 |
| `read_session_raw_history` | session_id, from_seq, limit | `Vec<LocalIngestFrame>` | events 表(不变) |

### 4.3 删除的 RPC

- `list_project_sessions`(被 `list_conversations` 取代)
- `start_agent_in_project`(被 `start_agent_in_conversation` 取代)

### 4.4 新 RPC 定义

```rust
pub struct ListConversationsParams {
    pub project_id: String,
    pub limit: Option<u32>,
}
pub struct ListConversationsResponse {
    pub conversations: Vec<LocalConversationSummary>,
}

pub struct CreateConversationParams {
    pub project_id: String,
    pub title: String,
}
pub struct CreateConversationResponse {
    pub conversation: LocalConversationSummary,
}

pub struct ListConversationMessagesParams {
    pub conversation_id: String,
    pub before_seq: Option<i64>,
    pub limit: Option<u32>,
}
pub struct ListConversationMessagesResponse {
    pub messages: Vec<LocalConversationMessage>,
    pub has_more: bool,
}

pub struct ListConversationAgentSessionsParams {
    pub conversation_id: String,
}
pub struct ListConversationAgentSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

pub struct StartAgentInConversationRequest {
    pub conversation_id: String,
    pub agent: AgentName,
    pub workspace: PathBuf,
    pub workspace_slug: String,
}

pub struct AppendConversationMessageParams {
    pub conversation_id: String,
    pub message_id: String,
    pub session_id: Option<String>,
    pub sender_role: String,        // "user" | "agent"
    pub agent: Option<AgentName>,
    pub body: String,
}
pub struct AppendConversationMessageResponse {
    pub message_seq: i64,
}
```

## 5. daemon 实现层

### 5.1 store 层新增方法

```rust
// conversation CRUD
pub async fn create_conversation(&self, project_id, title) -> Result<ConversationRow>;
pub async fn list_conversations_by_project(&self, project_id, limit: Option<u32>) -> Result<Vec<ConversationRow>>;
pub async fn get_conversation(&self, conversation_id) -> Result<ConversationRow>;
pub async fn touch_conversation(&self, conversation_id, preview: Option<String>);
pub async fn list_agents_for_conversations(&self, conversation_ids) -> Result<HashMap<String, Vec<AgentName>>>;

// chat_messages
pub async fn append_message(&self, conversation_id, message_id, session_id, role, agent, body) -> Result<i64>;
pub async fn list_conversation_messages(&self, conversation_id, before_seq, limit) -> Result<Vec<ChatMessageRow>>;

// sessions(改用 conversation_id)
pub async fn list_sessions_by_conversation(&self, conversation_id) -> Result<Vec<SessionRow>>;
pub async fn assign_thread_to_conversation(&self, session_id, conversation_id);
```

Row 结构:

```rust
pub struct ConversationRow {
    pub conversation_id: String,
    pub project_id: String,
    pub title: String,
    pub last_message_preview: Option<String>,
    pub message_count: i64,
    pub agent_session_count: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub struct ChatMessageRow {
    pub message_seq: i64,
    pub message_id: String,
    pub conversation_id: String,
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    pub sender_role: String,
    pub agent: Option<String>,
    pub body: String,
}
```

`SessionRow` 改动:去掉 `project_id`,加 `conversation_id`;`codex_session_id` 改名 `provider_session_id`。

### 5.2 LocalConversationSummary 组装

```rust
fn conversation_summary_from_row(
    row: ConversationRow,
    participating_agents: Vec<AgentName>,
) -> LocalConversationSummary { ... }
```

`list_conversations` 实现:先查一页 `list_conversations_by_project`,再用这一页的 conversation_id 批量查参与 agent。计数和摘要来自 `conversations` 行,不做逐 conversation 的 count 查询。

### 5.3 start_agent_in_conversation 流程

```
start_agent_in_conversation(conversation_id, agent, workspace):
  1. get_conversation(conversation_id) → 拿 project_id
  2. get_project(project_id) → 拿 workspace_path / workspace_slug
  3. AgentManager::start_agent_in_conversation(agent, workspace, conversation_id) → 生成 session_id，并注入 conversation-bound teamwork MCP
  4. persist_thread_parent_rows(session_id, workspace, agent, ...)
     - insert_session 填 conversation_id(不再填 project_id)
     - 同事务 conversations.agent_session_count += 1
  5. 返回 { session_id, conversation_id }
```

### 5.4 删除的 daemon 代码

- `list_project_sessions` 相关 store/agent/local_rpc 代码全删
- `assign_thread_to_project`(被 `assign_thread_to_conversation` 取代)
- 旧 group_chat store 方法和 workspace-derived room API

### 5.5 minos-chat-store crate 处理

`minos-chat-store` 只保留 teamwork MCP/socket/tool catalog 以及 conversation-scoped delegation 存储；TUI 的 conversation 消息持久化全部走 daemon RPC。

## 6. TUI 层重构

### 6.1 NavLevel(栈式)

```rust
pub enum NavLevel {
    Projects,
    Conversations { project_id: String },
    Conversation { conversation_id: String },
    AgentDetail { conversation_id: String, agent_session_id: String, agent: AgentName },
}
```

```rust
pub struct UiState {
    pub nav_stack: Vec<NavLevel>,  // 取代单字段 nav_level
    // ...
}
```

- `nav_stack.last()` 决定当前渲染层级
- Enter → `push(下一级)`;Esc → `pop()`;栈空退出程序

### 6.2 AgentRef(ChatItem 归属)

```rust
pub struct AgentRef {
    pub session_id: String,   // 唯一标识(= session_id)
    pub agent: AgentName,     // 显示用
}
```

### 6.3 ChatItem 加 agent 归属

```rust
pub enum ChatItem {
    UserMessage { message_id, text_parts, is_streaming },
    AssistantText { message_id, text_parts, is_streaming, source: AgentRef },
    Reasoning    { message_id, text, is_streaming, source: AgentRef },
    ToolCall     { message_id, tool_call_id, name, args_summary, args_detail, output_summary, output_detail, is_error, is_expanded, is_streaming, source: AgentRef },
    SystemMessage { text },
    Error { message_id, text, source: Option<AgentRef> },
}
```

Conversation 群聊视图按 `source` 显示每条消息来自哪个 agent。AgentDetail 层过滤 `source.session_id == selected_agent_session_id`。

### 6.4 Focus 枚举简化

```rust
pub enum Focus {
    MainList,   // Projects / Conversations 列表
    MainChat,   // Conversation / AgentDetail 对话区
    Sidebar,    // agent cards
    Input,      // 底部输入栏
}
```

Tab 在 `MainChat ↔ Sidebar ↔ Input` 间循环(列表层是 `MainList ↔ Sidebar ↔ Input`)。删除现有 6 变体 `PaneId` + `FocusManager` 树。

### 6.5 UiState 数据字段

```rust
pub struct UiState {
    pub nav_stack: Vec<NavLevel>,

    // Projects 层
    pub projects: Vec<ProjectEntry>,
    pub selected_project: Option<usize>,

    // Conversations 层
    pub conversations: Vec<LocalConversationSummary>,
    pub selected_conversation: Option<usize>,

    // Conversation 层(当前打开的对话)
    pub conversation_messages: Vec<LocalConversationMessage>,
    pub conversation_agents: Vec<AgentSessionCard>,
    pub selected_agent_card: Option<usize>,
    pub conversation_input: String,

    // AgentDetail 层(复用现有 chat_states: HashMap<session_id, ChatState>)
    // 通用
    pub focus: Focus,
    // ...
}
```

删除:`rooms`、`selected_room`、`sessions`(旧 per-agent list)、`agent_detail_visible`、`project_sessions`。

### 6.6 新建/修改/删除的 UI 文件

| 文件 | 职责 | 状态 |
|------|------|------|
| `src/nav.rs` | NavLevel 栈 push/pop | 改(单字段→栈) |
| `src/ui/project_list.rs` | Projects 列表 + 侧栏 | 已有,适配新布局 |
| `src/ui/conversation_list.rs` | Conversations 列表 + 侧栏 + 创建对话框 + `n` 新建 | 新建(取代 project_sessions.rs) |
| `src/ui/conversation_view.rs` | Conversation 80/20 群聊 + agent card sidebar | 新建 |
| `src/ui/agent_card.rs` | Agent 卡片 widget(状态灯+当前文件+token) | 新建 |
| `src/ui/agent_detail.rs` | AgentDetail 80/20 子视图 + 侧栏 | 新建 |
| `src/ui/project_create_dialog.rs` | Project 创建对话框 | 已有,保留 |

删除:`ui/room_list.rs`、`ui/project_sessions.rs`、`render_overview_tree`、`render_detail_tree`。

### 6.7 统一布局函数

```rust
fn split_main_sidebar(area: Rect) -> (Rect, Rect, Rect) {
    let middle = area;  // 去掉顶部状态栏 1 行
    let rows = Layout::default()
        .direction(Vertical)
        .constraints([Min(0), Length(3)])
        .split(middle);
    let cols = Layout::default()
        .direction(Horizontal)
        .constraints([Percentage(78), Percentage(22)])
        .split(rows[0]);
    (cols[0], cols[1], rows[1])  // main, sidebar, bottom
}
```

### 6.8 四层视图

**Projects 层**:`MainList`(project 列表)+ `Sidebar`(project 信息)+ bottom(快捷键提示)

**Conversations 层**:`MainList`(conversation 列表,按 title 分行,updated_at 倒序)+ `Sidebar`(选中 conversation 统计)+ bottom(快捷键:`n` 新建对话)

**Conversation 层**:`MainChat`(群聊主时间线,chat_messages 渲染)+ `Sidebar`(agent cards)+ bottom(输入栏)

**AgentDetail 层**:`MainChat`(该 agent 完整对话,ChatState + RenderCache)+ `Sidebar`(context 用量+文件列表)+ bottom(输入栏,消息仍发到同一 conversation)

### 6.9 交互流程

**新建 conversation**(手动建空频道):

1. Conversations 列表按 `n` → 弹创建对话框(输入 title)
2. Enter → `create_conversation` RPC → 新 conversation 加入列表 → 自动 `push(Conversation { conversation_id })`
3. 进入空对话,sidebar 无 agent,底部输入栏可输入

**拉 agent 进 conversation**(@mention):

1. 输入栏打 `@codex 帮我重构 foo` + Enter
2. 先 `append_conversation_message(role="user", body="@codex 帮我重构 foo")` 写入 chat_messages
3. 解析 `@codex` → `start_agent_in_conversation(conversation_id, codex, workspace)`
4. 新 thread 行创建(填 conversation_id)→ agent 开始 streaming → events 写入
5. sidebar agent list 刷新(新 agent card 出现,状态 ●running)
6. agent 完成后 → `append_conversation_message(role="agent", agent_session_id, body=final_text)` → 主时间线刷新

**查看 agent 细节**:

1. sidebar 聚焦 → 选中 agent card → Enter
2. `push(AgentDetail { conversation_id, agent_session_id, agent })`
3. `hydrate_thread_if_needed(agent_session_id)` 读 events → ChatState
4. 渲染该 agent 完整对话
5. Esc → `pop()` 回 Conversation 群聊

### 6.10 删除的旧 TUI 代码

| 代码 | 原因 |
|------|------|
| `UiState.rooms` / `selected_room` | Room 概念被 conversation 取代 |
| `ui/room_list.rs` | 不再需要 |
| `GroupChatState`(workspace-derived 版) | 删除；Conversation 层使用 `conversation_messages` + conversation scroll state |
| `render_overview_tree` / `render_detail_tree` | 三栏布局废弃 |
| `agent_detail_visible: bool` | 被 nav_stack 取代 |
| `PaneId` 6 变体 + `FocusManager` | 被简化 Focus 取代 |
| `project_sessions` / `SessionListRenderable` | 改名为 conversation |
| `room_id_for_workspace` 调用 | conversation_id 由 daemon 生成 |

## 7. 分阶段实施计划

7 个阶段,每阶段可独立编译验证,自底向上推进。

### Phase 1: daemon 存储层(schema + store)

| 任务 | 文件 |
|------|------|
| 改 daemon canonical schema 到终态;开发库 reset,不写兼容迁移 | `minos-daemon/migrations/*.sql` |
| SessionRow 改字段(conversation_id 替代 project_id, codex_session_id→provider_session_id) | `store/mod.rs` |
| ConversationRow / ChatMessageRow 结构 + CRUD 方法 | `store/mod.rs` |
| list_conversations_by_project / list_agents_for_conversations / list_sessions_by_conversation / append_message / touch_conversation | `store/mod.rs` |
| 单元测试:conversation CRUD、message 全局 seq 递增、跨 conversation 查询 | `store/mod.rs` tests |

**验证**:`cargo test -p minos-daemon -- store::`
**无外部依赖**,纯存储层。

### Phase 2: 协议层(types + RPC trait)

| 任务 | 文件 |
|------|------|
| LocalConversationSummary / LocalConversationMessage 类型 | `minos-protocol/src/messages.rs` |
| 6 个新 RPC 的 params/response | `minos-protocol/src/messages.rs` |
| LocalDaemonRpc trait 加新方法 | `minos-protocol/src/local_rpc.rs` |
| 删除 list_project_sessions / start_agent_in_project 的 protocol 定义 | `local_rpc.rs` |

**验证**:`cargo check -p minos-protocol`
**依赖**:Phase 1(类型引用的 Row 转换在 daemon 侧,protocol 层可先编译)。

### Phase 3: daemon RPC 实现(glue + local_rpc)

| 任务 | 文件 |
|------|------|
| LocalRpcImpl 实现 6 个新 RPC | `minos-daemon/src/local_rpc.rs` |
| AgentGlue: conversation_summary_from_row / list_conversations / create_conversation / list_conversation_agent_sessions | `minos-daemon/src/agent.rs` |
| start_agent_in_conversation(替代 start_agent_in_project,填 conversation_id) | `agent.rs` |
| append_conversation_message RPC | `agent.rs` + `local_rpc.rs` |
| 删除 list_project_sessions / assign_thread_to_project 相关代码 | `agent.rs` + `store/mod.rs` + `local_rpc.rs` |

**验证**:`cargo test -p minos-daemon` + 手动用 wscall 测 RPC
**依赖**:Phase 1 + 2。

### Phase 4: TUI backend trait + ChatItem 改造

| 任务 | 文件 |
|------|------|
| AgentBackend trait:6 个新方法 | `backend/mod.rs` |
| 删除 list_project_sessions / start_agent_in_project(trait 签名) | `backend/mod.rs` |
| DaemonBackend 实现新 trait 方法(转发 RPC) | `backend/daemon.rs` |
| EmbeddedBackend 实现(内存 Vec<LocalConversationSummary>) | `backend/embedded.rs` |
| ConversationEntry / ConversationMessageEntry / AgentRef TUI 侧类型 | `backend/mod.rs` |
| ChatItem 加 source: AgentRef | `translation/chat_item.rs` |
| ChatState.agent 改为 Option<AgentRef> | `translation/chat_state.rs` |

**验证**:`cargo check -p minos-tui`
**依赖**:Phase 2 + 3。

### Phase 5: TUI NavLevel 栈 + 启动流程

| 任务 | 文件 |
|------|------|
| NavLevel 改为 4 变体 + agent_session_id | `nav.rs` |
| UiState: nav_stack: Vec<NavLevel> | `ui/mod.rs` |
| 删除 rooms/selected_room/agent_detail_visible/project_sessions | `ui/mod.rs` |
| Focus 简化为 4 变体 + Tab 循环 | `focus.rs` |
| 启动 cwd→project 匹配改为进 Conversations 层 | `app/lifecycle.rs` |
| update/nav.rs 按 NavLevel 分发(push/pop 栈) | `update/nav.rs` |
| event_mapping 按 NavLevel 映射键位 | `event_mapping.rs` |

**验证**:`cargo check -p minos-tui`
**依赖**:Phase 4。

### Phase 6: 四层视图 80/20 渲染

| 任务 | 文件 |
|------|------|
| split_main_sidebar 通用布局函数 | `ui/mod.rs` |
| Projects 层渲染(适配新布局) | `ui/mod.rs` + `project_list.rs` |
| 新建 conversation_list.rs:列表 + 侧栏 + 创建对话框 | `ui/conversation_list.rs` |
| 新建 conversation_view.rs:80/20 群聊 + agent card sidebar | `ui/conversation_view.rs` |
| 新建 agent_card.rs:Agent 卡片 widget | `ui/agent_card.rs` |
| 新建 agent_detail.rs:AgentDetail 80/20 子视图 | `ui/agent_detail.rs` |
| 删除 render_overview_tree / render_detail_tree / project_sessions.rs / room_list.rs | `ui/mod.rs` |
| render_ui 改为按 nav_stack.last() 分发 | `ui/mod.rs` |

**验证**:`cargo run -p minos-tui` 手动走完四层导航;`cargo test -p minos-tui -- ui::`
**依赖**:Phase 5。

### Phase 7: 交互衔接 + @mention + agent 完成回写

| 任务 | 文件 |
|------|------|
| Conversation 输入栏:@mention 解析 → AgentName + start_agent_in_conversation | `update/nav.rs` + `app/submission.rs` |
| user 消息发送:append_conversation_message(role="user") | `app/submission.rs` |
| agent 完成回写:ChatState.last_completed_assistant_text → append_conversation_message(role="agent") | `app/conversation_result.rs` |
| conversation_agents 填充:list_conversation_agent_sessions → sidebar | `app/lifecycle.rs` |
| conversation_messages 初始加载 + 增量刷新 | `app/lifecycle.rs` |
| touch_conversation 联动 | `app/lifecycle.rs` |
| 删除旧 group chat / workspace-derived room 调用 | `app.rs` + `app/conversation_result.rs` + `teamwork.rs` |

**验证**:端到端——建 project → 建 conversation → @mention 拉 agent → 看群聊 → 下钻 AgentDetail → Esc 回群聊。`cargo test` 全量。
**依赖**:Phase 6。

### 阶段依赖图

```
Phase 1 (store)
    ↓
Phase 2 (protocol)
    ↓
Phase 3 (daemon RPC)
    ↓
Phase 4 (TUI backend)
    ↓
Phase 5 (TUI nav)
    ↓
Phase 6 (TUI 渲染)
    ↓
Phase 7 (交互衔接)
```

每阶段结束都能 `cargo check` 通过。Phase 6 结束可手动跑 TUI,Phase 7 结束端到端可用。

## 8. 风险与缓解

| 风险 | 缓解 |
|------|------|
| conversations 冗余计数与消息表不一致 | 写消息/创建 session 必须和更新 conversation 摘要、计数放在同一事务;测试覆盖计数更新 |
| participating_agents 批量聚合实现复杂 | 先对当前页 conversation_ids 做一次 `WHERE conversation_id IN (...)` 查询,不做逐行查询 |
| AgentRef.session_id 与 session_id 概念混淆 | 文档明确:用户可见层用 agent_session_id,内部存储用 session_id,二者值相同 |
| minos-chat-store 边界变更过大 | 已收窄为 teamwork MCP/socket/tool catalog + conversation-scoped delegation 存储 |
| @mention 解析鲁棒性(部分匹配、多 agent、无空格) | Phase 7 实现时参考现有 agent_picker 的补全逻辑;先支持单 agent `@name 消息`,多 agent 后续迭代 |

## 9. 与 `2026-06-17-tui-nav-ux-redesign.md` 的关系

本 spec 取代旧 spec 的 Sessions 层语义:

- 旧 spec 的 `Thread = Room 合并` → 本 spec 明确为 `conversation` 统一术语
- 旧 spec 的 `list_project_sessions` → 本 spec 删除,改为 `list_conversations`
- 旧 spec 的 `start_agent_in_project` → 本 spec 改为 `start_agent_in_conversation`
- 旧 spec 的 Thread 视图(群聊 + Agent 卡片)→ 本 spec 保留 80/20 布局设计,数据源明确为 chat_messages + events 两层
- 旧 spec 的 AgentDetail 用 session_id → 本 spec 改为 agent_session_id(值相同,命名统一)

Projects 层、cwd→project 匹配、Project 创建对话框、响应式 overlay 等设计沿用旧 spec,不重复。
