# minos-tui 三级导航与交互体验重设计

> 注：标题中的"三级"指 Project → Thread → Agent 三层导航主轴。其中 Thread 层（群聊）下可下钻到 Agent 子视图（该 agent 在此 thread 内的细节），属于 Thread 层的 detail view，不构成独立第四级导航——Esc 从 Agent 子视图回到 Thread 群聊，而非回到 Thread 列表。

> 日期: 2026-06-17
> 状态: 待审核
> 类型: 产品交互 + 架构重构

## 1. 背景与动机

### 1.1 当前问题

当前 minos-tui 的 UI 布局停留在 demo 阶段：

- **三栏平铺**：Overview 模式 20/55/25，Detail 模式 45/20/35。多个面板同时显示，每个都被挤压，信息密度低，焦点分散。
- **Room 列表形同虚设**：启动时只塞入一个 workspace-derived room 条目（`app.rs:59-62`），面板名为 "Threads" 却只列一个 workspace 条目。
- **Agent 详情是窄侧栏**：Detail 模式下 agent 对话只有 35% 宽度，代码/diff 被严重截断。
- **Project 概念完全缺席**：后端、daemon、协议层、mobile、web 全部已实现 project CRUD（`minos-protocol/messages.rs:806-882`，`minos-daemon/rpc_server.rs:284-346`），唯独 TUI 的 `AgentBackend` trait 没有 project 方法（0 引用）。TUI 是唯一按裸 workspace 路径过滤的客户端。
- **无统一导航**：用户无法在"管理多项目 → 选项目 → 选对话 → 看 agent 细节"之间自然流转，因为没有项目层和对话层。

### 1.2 目标

引入 **Project → Thread → Agent** 三级导航，每级采用统一的 `80% 主内容 + 20% 侧栏 + 底部操作区` 布局，让多项目多 agent 管理体验达到 mobile/web 同等水准，并取代当前的三栏平铺。

### 1.3 参考对标

- **opencode TUI** (`packages/tui/src/routes/session/`)：sidebar 固定 `width=42`，对话区 `flexGrow` 吃满，终端宽 >120 时 sidebar inline、≤120 时 overlay。Session sidebar 展示标题、workspace、版本、plugin slots。
- **Minos mobile** (`architecture-mobile.md:36,85`)：authenticated root = `projectList`，路由 `/project/:projectId` → `ProjectDetailPage`。

## 2. 关键设计决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 层级模型 | Project → Thread(=Room) → Agent | Thread 和 Room 合并，消除冗余概念；Project 补齐后端已有能力 |
| 启动层级 | cwd 匹配 project → 直接进 Threads；否则进 Projects | 日常高频场景（在项目目录启动）一步到位 |
| 启动时 cwd 未命中 project | 弹 Y/n 确认创建（方案 B）| 最少操作完成项目初始化 |
| Projects 列表内手动创建 | `n` 快捷键弹输入框（方案 A）| 补充非 cwd 场景 |
| Thread 创建方式 | Threads 列表底部输入栏直接输入消息 + Enter | 与三级导航的"底部操作区"统一模式 |
| Thread 标题 | 首条消息前 N 字自动生成，可后续修改 | 无标题摩擦，参考 chat app |
| 布局比例 | 80% 主内容 / 20% 侧栏 | 对标 opencode sidebar，信息层次分明 |
| 导航方式 | Enter 下钻 / Esc 返回 / 栈式 | 每层一致，心智模型统一 |
| 响应式 | 终端宽 ≤120 时侧栏改 overlay | 对标 opencode，窄屏可用 |

## 3. 数据模型变更

### 3.1 Thread = Room 合并

当前有两个概念：

- **Room**（`UiState.rooms: Vec<RoomEntry>`）：workspace 派生的群聊通道。
- **Thread**（`UiState.threads: Vec<ThreadEntry>`）：1:1 agent 会话。

**合并后**：Thread 就是对话。一个 thread 内可以有多个 agent 参与（通过 `@mention` 邀请）。群聊消息流和 agent 消息流在同一个 thread 内统一呈现。

```rust
// 新 ThreadEntry（取代旧 RoomEntry + ThreadEntry）
pub struct ThreadEntry {
    pub thread_id: String,
    pub project_id: String,
    pub title: String,              // 自动生成或用户设置
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub message_count: usize,
    pub participating_agents: Vec<AgentName>,
    pub files_touched: Vec<String>, // 涉及文件列表
    pub last_activity_summary: String,
}
```

### 3.2 引入 Project 到 TUI 层

```rust
pub struct ProjectEntry {
    pub project_id: String,
    pub name: String,
    pub workspace_path: PathBuf,   // 核心字段：绑定文件系统路径
    pub thread_count: usize,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub common_agents: Vec<AgentName>, // 常用 agent
}
```

### 3.3 AgentBackend trait 扩展

`AgentBackend` trait（`backend/mod.rs:40-106`）需要新增 project 方法：

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync {
    // ... 现有方法 ...

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>>;
    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry>;
    async fn list_project_threads(&self, project_id: &str) -> Result<Vec<ThreadEntry>>;
    async fn start_agent_in_project(
        &self,
        project_id: &str,
        agent: AgentName,
        prompt: &str,
    ) -> Result<StartAgentOutcome>;
}
```

- **`DaemonBackend`**：直接转发到已存在的 `minos_list_projects` / `minos_create_project` / `minos_list_project_threads` / `minos_start_agent_in_project` RPC（`rpc_server.rs:284-346`）。
- **`EmbeddedBackend`**：`AgentManager` 无 project 存储。方案：embedded 模式下维护一个内存 `Vec<ProjectEntry>`，`list_projects` 返回 cwd 自动生成的单个 project（兼容旧行为），`list_project_threads` 等同当前 `list_threads`。不实现 project 持久化——embedded 模式定位是开发/测试，project 管理不是核心需求。

### 3.4 启动路径匹配逻辑

```
启动 → canonicalize(cwd)
     → backend.list_projects()
     → 遍历 projects，找 workspace_path 匹配的
     → 命中: nav_stack.push(NavLevel::Threads { project_id })
     → 未命中:
         → Projects 列表 + 弹确认: "创建 project '<dir_name>' (<cwd>)? [Y/n]"
         → Y: create_project → push Threads
         → n: 留在 Projects 列表
```

路径匹配复用现有 `workspace_paths_match()`（`app.rs:3053-3060`），已处理路径规范化。

## 4. 导航栈

### 4.1 NavLevel 枚举

```rust
pub enum NavLevel {
    /// 顶层：项目列表
    Projects,
    /// 二层：某项目下的对话列表
    Threads { project_id: String },
    /// 三层：某对话（多 agent 群聊）
    Thread { project_id: String, thread_id: String },
    /// 四层：某 agent 在此对话中的细节视图
    Agent { project_id: String, thread_id: String, agent: AgentName },
}
```

### 4.2 UiState 导航字段

```rust
pub struct UiState {
    pub nav_stack: Vec<NavLevel>,      // 导航栈
    pub projects: Vec<ProjectEntry>,   // 所有项目
    pub selected_project: Option<usize>,
    pub threads: Vec<ThreadEntry>,     // 当前项目的对话
    pub selected_thread: Option<usize>,
    pub chat_states: HashMap<String, ChatState>,  // thread_id → 对话状态
    // ... 现有字段保留 ...
}
```

- `nav_stack.last()` 决定当前渲染哪一级。
- Enter → `push(下一级)`
- Esc → `pop()`；栈空时退出程序。

**多 agent ChatState 说明**：当前 `ChatState` 是 per-thread 的，内部 `items: Vec<ChatItem>` 已混合所有参与 agent 的消息（`translation.rs:63-77`）。合并后不变——一个 thread 一个 `ChatState`，包含所有 agent 的消息。Agent 子视图复用同一个 `ChatState`，只过滤 `ChatItem.agent == selected_agent` 的条目渲染。

### 4.3 Focus 枚举重构

当前 `Focus` 枚举（`ui/mod.rs:36-44`）有 6 个变体。新设计简化为：

```rust
pub enum Focus {
    MainList,   // 主内容区的列表（Projects / Threads）
    MainChat,   // 主内容区的对话（Thread / Agent）
    Sidebar,    // 侧栏
    Input,      // 底部输入栏
}
```

Tab 在 Main ↔ Sidebar ↔ Input 之间循环。

## 5. 四级视图详细设计

### 5.1 统一布局骨架

每一级共享相同的布局函数：

```rust
fn split_main_sidebar(area: Rect, sidebar_overlay: bool) -> (Rect, Rect, Rect) {
    // 返回 (main_area, sidebar_area, bottom_area)
    // sidebar_overlay=true 时 sidebar 为 absolute 浮层，main 占 100%
    // 否则 main 占 ~78%，sidebar 占 ~22%
}
```

顶部保留 1 行状态栏（后端状态、agent 检测、快捷键提示）。

### 5.2 第一级：Projects 列表

```
┌──────────────────────────────────┬───────────────────┐
│ Projects                         │ Project 信息      │
│                                  │                   │
│ > Minos    ~/code/.../Minos      │ Name: Minos       │
│   opencode ~/code/.../opencode   │ Path: ~/code/...  │
│   fire     ~/code/.../fire       │ Threads: 12       │
│                                  │ Agents: codex,    │
│                                  │   claude, gemini  │
│                                  │ Last: 2h ago      │
│                                  │                   │
├──────────────────────────────────┤                   │
│ [n]新建  [Enter]打开  [d]删除     │                   │
│              [Esc]退出            │                   │
└──────────────────────────────────┴───────────────────┘
```

- **主内容**：project 列表，每项显示 name + workspace_path（缩短显示）。
- **侧栏**：选中 project 的元信息（thread 数、常用 agent、最近活跃时间）。
- **底部**：快捷键提示（非输入栏，因为 project 创建走确认对话框）。
- **输入**：`n` 弹出 prompt 输入 name，workspace_path 默认填 cwd（可编辑）。
- **Esc**：退出程序。

### 5.3 第二级：Threads 列表

```
┌──────────────────────────────────┬───────────────────┐
│ Minos — 对话                     │ Thread 信息       │
│                                  │                   │
│ > #abcd1234 重构 foo 模块         │ Title: 重构 foo   │
│   #efgh5678 修复登录 bug          │ Created: 昨天     │
│   #ijkl9012 新增 CI 流程          │ Messages: 47      │
│                                  │ Agents: codex(3), │
│                                  │   claude(1)       │
│                                  │ Files: 12 changed │
│                                  │ Last: 10m ago     │
│                                  │                   │
│   ┌──────────────────────────┐   │                   │
│   │ 输入消息开始新对话...     │   │                   │
│   └──────────────────────────┘   │                   │
├──────────────────────────────────┤                   │
│ [@]选agent  [Enter]发送新对话     │ [Esc] 返回 Projects│
└──────────────────────────────────┴───────────────────┘
```

- **主内容**：当前 project 的 thread 列表，每项显示 thread_id 前缀 + 标题。
- **侧栏**：选中 thread 的统计信息。
- **底部输入栏**：直接输入消息 + Enter → 创建新 thread 并进入 Thread 视图。
  - `@codex` 指定初始 agent（可多个）。
  - 无 `@` 时使用 project 的常用 agent 或默认 agent。
- **自动标题**：首条消息前 ~30 字符作为标题，`updated_at` 更新时刷新。用户可在 Thread 视图内 `t` 修改标题。
- **Esc**：返回 Projects。

### 5.4 第三级：Thread 对话（群聊 + Agent 概览）

```
┌──────────────────────────────────┬───────────────────┐
│ 重构 foo 模块 — #abcd1234        │ Agent 概览        │
│                                  │                   │
│ me: 帮我重构 src/foo.rs          │ ┌───────────────┐ │
│ codex: 好的，我来看一下...       │ │codex    ●运行 │ │
│   [tool] Read src/foo.rs         │ │ foo.rs, bar.rs│ │
│ claude: 我生成了 diff...         │ │ 12.3k tokens  │ │
│   diff: ...                      │ └───────────────┘ │
│ me: @gemini 你觉得呢?            │ ┌───────────────┐ │
│ gemini: 我觉得方案B更好...       │ │claude   ○空闲 │ │
│                                  │ │ 3.1k tokens   │ │
│                                  │ └───────────────┘ │
│   ┌──────────────────────────┐   │ ┌───────────────┐ │
│   │ 输入消息...               │   │ │gemini   ○空闲 │ │
│   └──────────────────────────┘   │ └───────────────┘ │
├──────────────────────────────────┤                   │
│ [Tab]切焦点 [Enter]发送          │ [Enter]查看Agent  │
│                                  │ [Esc] 返回 Threads│
└──────────────────────────────────┴───────────────────┘
```

- **左 ~78%**：群聊对话（复用现有 `group_chat.rs` 渲染逻辑）+ 底部输入栏。
- **右 ~22%**：每个参与 agent 一个卡片，显示：
  - Agent 名称 + 状态灯（●running / ○idle / ✕closed）
  - 当前正在操作的文件（从最新 tool call 提取）
  - Context token 用量（从 ingest 事件提取）
- **Enter on agent card**（侧栏聚焦时）→ 进入 Agent 子视图。
- **Esc**：返回 Threads。

### 5.5 第四级：Agent 子视图

```
┌──────────────────────────────────┬───────────────────┐
│ codex — 重构 foo 模块            │ 对话信息          │
│                                  │                   │
│ user: 帮我重构这个函数           │ Context:          │
│ codex: 好的...                   │  12.3k / 200k     │
│   [tool] Read src/foo.rs         │                   │
│   [tool] Edit src/foo.rs         │ Files (2):        │
│   diff: ...                      │  M src/foo.rs     │
│ codex: 完成了                    │  M src/bar.rs     │
│                                  │                   │
│ me: 再加个测试                   │ Duration: 3m 24s  │
│ codex: 好的...                   │ Cost: $0.12       │
│                                  │                   │
│   ┌──────────────────────────┐   │                   │
│   │ 对 codex 说...           │   │                   │
│   └──────────────────────────┘   │ [Esc] 返回 Thread │
├──────────────────────────────────┤                   │
│ [Enter]发送                      │                   │
└──────────────────────────────────┴───────────────────┘
```

- **左 ~78%**：该 agent 在此 thread 的完整对话（含 tool calls、diff、reasoning）。复用现有 `chat.rs` + `RenderCache`。
- **右 ~22%**：对标 opencode sidebar：
  - Context 用量（当前/上限）
  - 涉及文件列表（带修改标记 M/A/D）
  - 持续时间
  - 成本（如后端提供）
- **底部输入栏**：直接对该 agent 输入。消息仍发送到同一 thread（复用 thread 的消息通道），但以 `@<agent>` 前缀路由到指定 agent，不广播到群聊。
- **Esc**：返回 Thread 群聊视图。

### 5.6 响应式：窄屏 overlay

终端宽 ≤120 时，侧栏从 inline 改为 overlay 浮层：

```
┌──────────────────────────────────────────────────────┐
│                                                      │
│  主内容 100%                          ┌────────────┐ │
│                                       │ 侧栏 overlay│ │
│                                       │ (Tab 切出) │ │
│                                       └────────────┘ │
│                                                      │
├──────────────────────────────────────────────────────┤
│  底部输入栏 / Esc 提示                                │
└──────────────────────────────────────────────────────┘
```

- 默认隐藏侧栏，主内容占满。
- Tab 切到 Sidebar focus 时，侧栏浮层出现（半透明背景或边框区分）。
- 再次 Tab 回到 Main，侧栏消失。

## 6. 统一交互键位

### 6.1 全局键位

| 键位 | 适用层级 | 行为 |
|------|---------|------|
| `Esc` | Threads / Thread / Agent | 返回上一级（pop nav_stack） |
| `Esc` | Projects | 退出程序 |
| `Tab` | 所有层 | 在 Main ↔ Sidebar ↔ Input 间循环焦点 |
| `Ctrl+C` | 所有层 | 中断当前 agent（如运行中）/ 复制选中文本 |
| `Ctrl+Q` | 所有层 | 强制退出 |

### 6.2 列表层（Projects / Threads）键位

| 键位 | 行为 |
|------|------|
| `↑` / `↓` | 上下移动选中项 |
| `Enter` | 进入选中项下一级 |
| `n` | 新建（Projects: 弹 name prompt；Threads: 聚焦输入栏） |
| `d` | 删除选中项（弹 Y/n 确认） |
| `r` | 刷新列表（重新拉取） |

### 6.3 对话层（Thread / Agent）键位

| 键位 | 行为 |
|------|------|
| `Enter` | 发送消息 |
| `Alt+Enter` / `Ctrl+J` | 输入栏换行（multiline） |
| `↑` / `↓`（输入栏边界） | 浏览 prompt 历史 |
| `↑` / `↓`（对话聚焦） | 滚动对话内容 |
| `@` | 输入栏触发 agent mention 自动补全 |
| `Enter`（Thread 侧栏 agent card） | 进入 Agent 子视图 |

### 6.4 键位与重构计划的关系

当前有三阶段架构重构 spec（`2026-06-17-tui-three-phase-refactor-design.md`），计划引入 Action 层和 `FrameRequester`。本设计应在三阶段重构完成后实施，届时键位处理通过 `event_to_actions() → update()` 流转，NavLevel push/pop 作为 Action 处理。

若三阶段重构尚未完成，本设计可先在现有 `handle_*_key` 函数中实现，后续迁移到 Action 层。

## 7. Project 创建流程

### 7.1 启动时自动提示（方案 B）

cwd 未命中任何 project 时：

```
┌──────────────────────────────────────┐
│                                      │
│  当前目录未绑定任何 project。         │
│                                      │
│  创建 project "Minos"                │
│  (~/code/github.com/Minos)?          │
│                                      │
│  [Y] 创建并进入  [n] 手动选择        │
│                                      │
└──────────────────────────────────────┘
```

- **Y**：`create_project(dir_name, cwd)` → 进入该 project 的 Threads 列表。
- **n**：关闭对话框，留在 Projects 列表。
- Project name 默认取 `cwd.file_name()`，可编辑。

### 7.2 列表内手动创建（方案 A）

在 Projects 列表按 `n`：

```
┌──────────────────────────────────────┐
│  新建 Project                         │
│                                      │
│  Name: [Minos                      ] │
│  Path: [~/code/github.com/Minos    ] │
│                                      │
│  [Enter] 创建  [Esc] 取消            │
│                                      │
└──────────────────────────────────────┘
```

- Name 默认填 cwd 的目录名。
- Path 默认填 cwd，可修改。
- `Enter` 创建并进入 Threads；`Esc` 取消。

## 8. Thread 创建流程

在 Threads 列表页，底部输入栏直接输入消息 + Enter：

```
┌──────────────────────────────────┐
│ Threads — Minos                  │
│                                  │
│ > #abcd1234 重构 foo 模块         │
│   #efgh5678 修复登录 bug          │
│                                  │
│   ┌──────────────────────────┐   │
│   │ @codex 帮我重构 foo模块   │   │
│   └──────────────────────────┘   │
│                                  │
└──────────────────────────────────┘
```

- 输入 `@codex 帮我重构 foo模块` + Enter：
  1. `start_agent_in_project(project_id, codex, "帮我重构 foo模块")`
  2. 创建新 thread，绑定到此 project
  3. 自动标题 = "帮我重构 foo模块"（截断至 ~30 字符）
  4. `nav_stack.push(Thread { project_id, thread_id })`
  5. 进入 Thread 群聊视图

- 无 `@` 时使用 project 的常用 agent（`ProjectEntry.common_agents[0]`）；若 `common_agents` 为空则弹 agent picker 让用户选择。

## 9. Agent 卡片设计

Thread 视图右侧 20% 区域，每个参与 agent 一个卡片：

```
┌───────────────────┐
│ codex        ●运行 │  ← 名称 + 状态灯
│                   │
│ 正在操作:          │  ← 从最新 tool call 提取
│  src/foo.rs       │
│  src/bar.rs       │
│                   │
│ Context: 12.3k    │  ← token 用量
│ Duration: 3m      │
└───────────────────┘
```

数据来源：
- **状态灯**：现有 `ThreadEntry.state`（Active/Running/Idle/Closed）。
- **正在操作文件**：从该 agent 最新 tool call（Read/Write/Edit/Glob）的参数提取，已在 `ChatState.items` 中。
- **Context token**：从 ingest 事件的 usage 字段提取（需确认 protocol 是否已暴露；若未暴露则先显示 "—"）。
- **Duration**：thread 创建时间到现在。

卡片为自定义 Ratatui widget，可点击或键盘选中后 Enter 进入 Agent 子视图。

## 10. 实现影响分析

### 10.1 需要新建的文件

| 文件 | 职责 |
|------|------|
| `src/nav.rs` | `NavLevel` 枚举、导航栈逻辑、push/pop |
| `src/ui/project_list.rs` | Projects 列表渲染 + 侧栏 |
| `src/ui/thread_list_v2.rs` | Threads 列表渲染 + 侧栏（取代旧 `thread_list.rs`） |
| `src/ui/agent_card.rs` | Agent 卡片 widget |
| `src/ui/agent_detail.rs` | Agent 子视图渲染 + 侧栏 |
| `src/ui/project_create_dialog.rs` | Project 创建对话框 |
| `src/ui/thread_title_editor.rs` | Thread 标题编辑（可选，后续） |

### 10.2 需要修改的文件

| 文件 | 变更 |
|------|------|
| `src/app.rs` | `UiState` 加 `nav_stack`；启动逻辑改为 cwd 匹配；`handle_event` 按 NavLevel 分发 |
| `src/ui/mod.rs` | `render_ui` 改为按 `nav_stack.last()` 渲染对应层级；删除 `overview_mode` / `detail_mode` 二分 |
| `src/ui/theme.rs` | 扩展语义色（project/thread/card 相关） |
| `src/backend/mod.rs` | `AgentBackend` trait 加 project 方法 |
| `src/backend/daemon.rs` | 实现新 trait 方法，转发 RPC |
| `src/backend/embedded.rs` | 实现新 trait 方法，内存 project |
| `src/event.rs` | 无变化（事件泵不变） |
| `src/main.rs` | 启动序列加 cwd → project 匹配 |

### 10.3 可复用的现有代码

| 现有代码 | 复用于 |
|---------|--------|
| `ui/chat.rs` + `RenderCache` | Agent 子视图的对话渲染 |
| `ui/group_chat.rs` | Thread 视图的群聊渲染 |
| `ui/input_bar.rs` | 所有层的底部输入栏 |
| `ui/agent_picker.rs` | Thread 创建时的 agent 选择 |
| `translation.rs` `ChatState` | Thread/Agent 对话状态 |
| `workspace_paths_match()` | 启动 cwd 匹配 |

### 10.4 需要删除/废弃的代码

| 代码 | 原因 |
|------|------|
| `UiState.rooms: Vec<RoomEntry>` | Thread = Room 合并，不再需要独立 room 列表 |
| `ui/room_list.rs` | Room 列表面板删除 |
| `agent_detail_visible: bool` | 被 `nav_stack` 取代 |
| `render_overview_mode` / `render_detail_mode` | 被 `render_level` 取代 |

## 11. 分阶段实施建议

本设计体量较大，建议分阶段实施：

| 阶段 | 内容 | 依赖 |
|------|------|------|
| Phase 1 | `AgentBackend` trait 扩展 + daemon/embedded 实现 | 无 |
| Phase 2 | `NavLevel` + 导航栈 + 启动 cwd 匹配 | Phase 1 |
| Phase 3 | Projects 列表视图 + 创建对话框 | Phase 2 |
| Phase 4 | Threads 列表视图 + 底部输入创建 | Phase 3 |
| Phase 5 | Thread 群聊视图（80/20）+ Agent 卡片 | Phase 4 |
| Phase 6 | Agent 子视图 + 侧栏信息 | Phase 5 |
| Phase 7 | 响应式 overlay + 主题扩展 + 打磨 | Phase 6 |

建议在三阶段架构重构（`2026-06-17-tui-three-phase-refactor-design.md`）完成后开始，以利用 Action 层和 `FrameRequester` 基础设施。

## 12. 风险与缓解

| 风险 | 缓解 |
|------|------|
| Embedded 模式无 project 持久化 | 内存维护单 project，行为兼容旧版；embedded 定位为开发模式 |
| Thread = Room 合并是破坏性变更 | Minos 开发态策略为 latest-only，无兼容包袱（`AGENTS.md` 明确） |
| 后端 protocol 的 context token 字段可能未暴露 | Agent 卡片 token 用量先显示 "—"，后续根据 protocol 补充 |
| 与三阶段重构并行可能冲突 | 本设计应在重构后实施；若并行，Phase 1-2（trait + nav）可与重构 P0 并行 |
