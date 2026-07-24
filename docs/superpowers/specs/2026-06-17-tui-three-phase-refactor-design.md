# minos-tui 三阶段架构重构设计

> 日期: 2026-06-17
> 状态: 已批准
> 方案: A — 严格 Elm 克隆

## 1. 背景与动机

`minos-tui` (~13.7k 行, ~20 个源文件) 功能已相对完整——多 agent 群聊、`@agent` 路由、流式 markdown/diff 渲染、四种 approval 协议、agent 间 MCP 协调——但架构欠成熟:

- `app.rs` 是 5727 行的上帝对象: 事件分发、6 个面板按键处理、鼠标、提交、群聊调和、MCP 路由、线程水化全部集中
- 无命令/动作层: `handle_*_key` 直接原地修改状态并调后端,意图与状态变更没有分离
- 粗糙重绘模型: 返回单个 bool 触发全屏重绘,高频流式时性能差
- `InputState` 重复: `room_input` 和 `agent_input` 两份几乎逐行相同的代码
- 扁平 Focus 枚举: 加面板要手改两处 match 列表
- 过程式渲染: 无法支持动态高度面板

参照 `codex-rs/tui` (~209k 行, 347 文件) 的 Elm 架构,分三阶段彻底重构。

## 2. 约束与决策

| 约束 | 决策 |
|------|------|
| 重构策略 | 大爆炸: 单 feature branch 完成全部三阶段 |
| 测试策略 | 重构优先,测试后补 |
| P3 布局抽象 | 完全替换为 Renderable trait,不保留旧路径 |
| 架构文档 | 每阶段完成后更新 `docs/architecture-tui.md` |
| 兼容性 | 内部 crate,自由重构所有内部类型 |
| 功能范围 | 纯结构重构,不加新功能 |
| Snapshot 测试 | 不引入(视觉后续会改造,当前锁定反而碍事) |

## 3. P0 — 架构骨架: Action 层 + app.rs 拆分

### 3.1 四层分离模型

```
Input Layer        crossterm → AppEvent → handle_* 生成 Action
    │ Action
    ▼
Update Layer       fn update(state, action) → (StateChange, Vec<Effect>)
                   纯逻辑,不执行 IO
    │ Effect
    ▼
Effect Layer       fn execute_effect(effect) → Future
                   后端调用、群聊写入、MCP、剪贴板
                   结果通过新 Action 回流
    │
    ▼
Render Layer       fn render(state) → (P2: Renderable tree)
                   FrameRequester 合并重绘请求 (P1 引入)
```

核心数据流:
```
1. crossterm event → AppEvent
2. event_to_actions(state, event) 生成 Vec<Action>   // 纯函数, 只读 state
3. for action in actions {
       let (changes, effects) = update(&mut app_state, &mut ui_state, action);
       for effect in effects { spawn(execute_effect(effect)) }
       if changes.needs_redraw { frame_requester.schedule_frame() }
    }
4. select! 收到 frame 请求 → render(state)
5. select! 收到 effect 完成的回流 Action → 回到步骤 3
```

Action 生成与状态更新的职责分离:
- `event_to_actions()`: 位于 update.rs 或 app.rs, 把原始 AppEvent (Key/Mouse/Paste/Ingest/ManagerEvent) 映射为语义化的 Action。只读 state, 不修改。
- `update()`: 接收单个 Action, 执行状态变更, 返回 `StateChange` + `Vec<Effect>`。

```rust
pub struct StateChange {
    pub needs_redraw: bool,
}
impl StateChange {
    fn redraw() -> Self { Self { needs_redraw: true } }
    fn none() -> Self { Self { needs_redraw: false } }
}
```

### 3.2 Action 分层设计

适配 minos 多房间多 agent 场景,Action 按作用域分层:

```rust
enum Action {
    Global(GlobalAction),
    Room(RoomId, RoomAction),
    Agent(ThreadId, AgentAction),
    Input(InputTarget, InputAction),
    // Effect 回流: 后台副作用完成后产生, 驱动状态更新
    EffectCompleted(EffectResult),
}

enum EffectResult {
    AgentStarted { agent: AgentName, session_id: String, cwd: PathBuf, text: String },
    SendFailed { session_id: String, error: String },
    IngestArrived(LocalIngestFrame),
    ManagerEvent(ManagerEvent),
    // 剪贴板读取结果等
}

enum GlobalAction {
    Quit,
    CycleFocus,
    ToggleAgentDetail,
    // Superseded on 2026-06-23: modal agent picker removed; use input @agent routing.
    Scroll(ScrollTarget, ScrollDirection, u16),
    CopySelection,
    Paste(String),
    MouseClick { target: ClickTarget, x: u16, y: u16 },
    MouseDrag { x: u16, y: u16 },
    MouseScroll { target: ScrollTarget, direction: ScrollDirection },
    RequestRedraw,
}

enum RoomAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    SubmitInput(String),
}

enum AgentAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    SubmitInput(String),
    Interrupt,
    Close,
    Delete,
    Resume,
    ToggleToolExpansion(usize),
    AnswerApproval(String),
    StartNew(AgentName),
}

enum InputTarget { Room, Agent(ThreadId) }

enum InputAction {
    InsertChar(char),
    InsertText(String),          // paste
    DeleteBackward,
    DeleteWord,
    DeleteToEndOfLine,
    MoveCursor(CursorDirection),
    MoveCursorWord(CursorDirection),
    MoveCursorLine(CursorLineDirection),
    Submit,
    NewLine,
    ToggleMultilineMode,
    HistoryNavigate(HistoryDirection),
    HistorySearch(String),
    ToggleMentionPicker,
    TogglePathPicker,
    SelectPickerItem(usize),
    DismissPicker,
}

enum CursorDirection { Left, Right, LineStart, LineEnd }
enum CursorLineDirection { Up, Down }
enum HistoryDirection { Previous, Next }
enum ScrollDirection { Up, Down, Top, Bottom }
enum ScrollTarget { GroupChat, AgentChat(ThreadId), ActivePane }
enum ClickTarget { RoomList, GroupChat, AgentList, AgentChat, RoomInput, AgentInput }
```

### 3.3 Effect 定义

```rust
enum Effect {
    StartAgent { agent: AgentName, workspace: PathBuf },
    SendMessage { session_id: String, text: String },
    SendApproval { session_id: String, decision: ApprovalDecision },
    InterruptThread(String),
    CloseThread(String),
    DeleteThread(String),
    ResumeSession(String),
    HydrateThreadHistory(String),
    SyncDaemonThreads,
    WriteGroupChat { room: RoomId, message: GroupChatMessage },
    RetryPendingAgentGroupResults,
    CopyToClipboard(String),
    None,
}
```

Effect 执行是 async 的,在事件循环中 spawn 为独立 task,完成后发送回流 Action (如 `AgentStartedForPrompt`、`SendMessageFailed`)。

### 3.4 核心数据流

见 3.1 末尾的"核心数据流"。

### 3.5 app.rs 拆分: 目标目录结构

```
src/
├── main.rs                     # 入口, CLI, 终端 setup (~300行)
├── event.rs                    # AppEvent enum + 4个事件泵 (不变)
├── action.rs                   # Action/GlobalAction/RoomAction/AgentAction/InputAction (~300行)
├── effect.rs                   # Effect enum + Effect 执行器 (~250行)
├── update.rs                   # update() 入口 (~100行)
│   ├── global.rs               # GlobalAction 处理 (~150行)
│   ├── room.rs                 # RoomAction 处理 (~200行)
│   └── agent.rs                # AgentAction 处理 (~250行)
├── input.rs                    # InputAction 处理 + 参数化 InputState (~500行)
│                               # 消灭 room_input/agent_input 重复
├── app.rs                      # App struct + 事件循环骨架 (~300行)
│   # 只做: recv → generate actions → update → execute effects → schedule frame
├── backend/                    # 保持现有结构
│   ├── mod.rs
│   ├── embedded.rs
│   └── daemon.rs
├── state/                      # 从 app.rs + translation.rs 抽出的纯状态
│   ├── mod.rs                  # AppState (替代 App 的业务字段) (~200行)
│   ├── thread_hydration.rs     # 水化/水位线/replay (~300行)
│   ├── ingest_dedup.rs         # ingest 去重 (~100行)
│   ├── workspace_filter.rs     # workspace 过滤 (~100行)
│   └── selection.rs            # 鼠标选区状态 (~150行)
├── translation/                # 拆分当前 2184 行的 translation.rs
│   ├── mod.rs                  # ChatState + apply_ui_event (~500行)
│   ├── chat_state.rs           # ChatState struct 定义 (~300行)
│   ├── chat_item.rs            # ChatItem enum + 渲染辅助 (~200行)
│   ├── agent_translator.rs     # AgentTranslationState (~300行)
│   └── format_helpers.rs       # summarize_tool_args 等 30+ 辅助函数 (~400行)
├── group_chat.rs               # 保持现有 SQLite 持久化
├── ui/                         # P1/P3 阶段重构, P0 保持现有渲染函数
│   ├── mod.rs
│   ├── chat.rs
│   ├── input_bar.rs            # P0: 提取参数化 InputState 到 input.rs
│   ├── group_chat.rs
│   ├── room_list.rs
│   ├── thread_list.rs
│   ├── status_bar.rs
│   └── theme.rs
├── skills.rs                   # 不变
└── logging.rs                  # 不变
```

### 3.6 关键拆分决策

**App vs AppState 分离**:
- `App` (app.rs): 事件循环编排者, 持有 event sender、effect executor、UiState
- `AppState` (state/mod.rs): 纯业务状态 — backend 引用、hydrated_threads、watermarks、group_chat_store、recorded_agent_results
- `update()` 接收 `&mut AppState + &mut UiState`, 不接触 IO

**消灭 InputState 重复**:
当前 `room_input` 和 `agent_input` 是两个独立字段, 配两份几乎逐行相同的 handler。

改为:
```rust
struct InputState { ... }  // 保持现有结构
enum InputTarget { Room, Agent(ThreadId) }

fn handle_input_action(state: &mut InputState, target: InputTarget, action: InputAction)
    -> (StateChange, Vec<Effect>)
```
UiState 持有两个 InputState 字段, 但共用同一个 handler 函数, 通过 target 参数区分提交目标。

**测试迁移策略**:
当前测试直接调 `app.handle_room_input_key(press(KeyCode::Enter)).await`。

重构后改为:
```rust
let action = key_to_action(&state, key);                          // 纯函数
let (changes, effects) = update(&mut state, action);              // 纯函数
assert_eq!(effects, vec![Effect::SendMessage { ... }]);
```
测试从 async 变为同步, 更快更简单。约 120 个现有测试在拆分过程中逐步迁移到新结构。

**鼠标和选区**:
当前 ~15 个鼠标/选区方法散落在 app.rs。拆到 `state/selection.rs`, 鼠标事件走 Action 路径 (`GlobalAction::MouseClick/MouseDrag/MouseScroll`)。

## 4. P1 — 性能与稳定性

### 4.1 帧合并与帧率限制

**当前问题**: `handle_event` 返回 bool, 任何 true 触发立即 `terminal.draw` 全屏重绘。高频流式 ingest 时 (200ms tick + 流式 delta) 终端压力大。

**设计**:
```rust
// 新增 src/frame.rs
pub struct FrameRequester {
    tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl FrameRequester {
    pub fn schedule_frame(&self) {
        // 合并: 如果已有 pending frame 请求, 幂等丢弃
        let _ = self.tx.send(());
    }
}
```

main.rs 事件循环改为 `select!` 5 个源:
```rust
loop {
    tokio::select! {
        Some(event) = event_rx.recv() => {
            let actions = handle_event(&state, event);
            for action in actions {
                let (changes, effects) = update(&mut state, action);
                execute_effects(effects);
                if changes.needs_redraw { frame_requester.schedule_frame(); }
            }
        }
        Some(()) = frame_rx.recv() => {
            let now = Instant::now();
            if now.duration_since(last_draw) >= MIN_FRAME_INTERVAL {
                terminal.draw(|f| render_ui(f, &state));
                last_draw = now;
            }
        }
    }
}
```

- `MIN_FRAME_INTERVAL` = ~33ms (30fps 上限), 实际操作远低于此频率
- 流式 ingest 高峰时多个 delta 合并为一帧
- update 不再返回 bool, 改为按需 `schedule_frame()`

### 4.2 Group Chat RenderCache

**当前问题**: `ui/group_chat.rs` 每帧遍历全部消息 (最多 500 条) 重新 wrap 渲染, 无缓存。

**目标**: 复用 agent chat 已有的 `RenderCache` 模式。

**设计**:
```rust
// ui/group_chat.rs
pub struct GroupChatRenderCache {
    cache: RenderCache,  // 复用 ui/chat.rs 的 RenderCache struct
}

impl GroupChatState {
    fn version(&self) -> u64 { self.version }
    fn render_cache(&mut self, width: u16) -> &mut RenderCache { ... }
}

fn render_group_chat(f: &mut Frame, area: Rect, state: &mut GroupChatState) {
    let cache = state.render_cache.get_or_rebuild(
        &state.messages,
        area.width,
        state.version,
    );
    let visible = cache.visible_window(area.height, state.scroll_offset);
    // 只渲染可见窗口的消息
}
```

GroupChatState 需新增 `version: u64` (任何消息增删改时递增) 和 `render_cache: GroupChatRenderCache`。

## 5. P2 — Renderable trait + 焦点树

### 5.1 Renderable trait

**当前**: `ui/mod.rs` 的 `render_ui` 是过程式函数, 硬编码两种布局, 每个面板是独立渲染函数。无法支持动态高度面板。

**设计**:
```rust
// 新增 src/render/mod.rs
pub trait Renderable {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn desired_height(&self, width: u16) -> u16;
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> { None }
}

// flex 容器: 子节点声明 desired_height, 容器分配空间
pub struct Column {
    children: Vec<Box<dyn Renderable>>,
}

impl Renderable for Column {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let heights = self.layout(area);
        for (child, child_area) in self.children.iter().zip(heights) {
            child.render(child_area, buf);
        }
    }
    fn desired_height(&self, width: u16) -> u16 {
        self.children.iter().map(|c| c.desired_height(width)).sum()
    }
}

// 固定比例行容器
pub struct Row {
    children: Vec<Box<dyn Renderable>>,
    ratios: Vec<u16>,
}
```

### 5.2 面板迁移为 Renderable

每个面板成为 struct 实现 Renderable:
```rust
pub struct RoomListRenderable<'a> { state: &'a RoomListState, focused: bool }
pub struct GroupChatRenderable<'a> { state: &'a GroupChatState, focused: bool }
pub struct AgentChatRenderable<'a> { state: &'a ChatState, focused: bool }
pub struct InputBarRenderable<'a> { state: &'a InputState, target: InputTarget, focused: bool }
pub struct StatusBarRenderable<'a> { state: &'a AppState }
```

`render_ui` 变为组装 renderable 树:
```rust
fn build_render_tree(state: &AppState, focus: &FocusManager) -> Box<dyn Renderable> {
    if state.ui.agent_detail_visible {
        Column::new(vec![
            Row::new(vec![
                GroupChatRenderable::new(&state.group_chat, focus.is(PaneId::GroupChat)),
                AgentListRenderable::new(&state.sessions, focus.is(PaneId::AgentList)),
                AgentChatRenderable::new(&state.active_chat, focus.is(PaneId::AgentChat)),
            ], vec![45, 20, 35]),
            Row::new(vec![
                InputBarRenderable::new(&state.room_input, InputTarget::Room, focus.is(PaneId::RoomInput)),
                InputBarRenderable::new(&state.agent_input, InputTarget::Agent, focus.is(PaneId::AgentInput)),
            ], vec![65, 35]),
        ])
    } else {
        // overview layout...
    }
}
```

`desired_height` 让 InputBar 按内容行数动态伸缩 (当前已通过 `required_height` 计算, 正好迁移)。

### 5.3 焦点树

**当前**: 扁平 `Focus` enum 有 6 个变体, `cycle_focus` 手工硬编码顺序。

**设计**:
```rust
// 新增 src/focus.rs
#[derive(Clone, Debug)]
pub enum FocusNode {
    Pane(PaneId),
    Group(Vec<FocusNode>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneId {
    RoomList, GroupChat, AgentList, AgentChat, RoomInput, AgentInput,
}

fn default_focus_tree(detail: bool) -> FocusNode {
    if detail {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::AgentList),
                FocusNode::Pane(PaneId::AgentChat),
            ]),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::RoomInput),
                FocusNode::Pane(PaneId::AgentInput),
            ]),
        ])
    } else {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::RoomList),
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Pane(PaneId::AgentList),
            FocusNode::Pane(PaneId::RoomInput),
        ])
    }
}

pub struct FocusManager {
    tree: FocusNode,
    path: Vec<usize>,
}

impl FocusManager {
    pub fn current(&self) -> PaneId { ... }
    pub fn cycle_next(&mut self) -> PaneId { ... }  // 深度优先遍历
    pub fn cycle_prev(&mut self) -> PaneId { ... }
    pub fn focus(&mut self, pane: PaneId) { ... }
    pub fn is(&self, pane: PaneId) -> bool { self.current() == pane }
    pub fn switch_layout(&mut self, detail: bool) { ... }  // 布局切换时重建树
}
```

加面板只需修改树定义, cycle/focus 逻辑通用。

### 5.4 P2 迁移路径

P0/P1 阶段渲染仍用现有函数式。P2 开始:
1. 引入 Renderable trait + Column/Row 容器
2. 逐个面板迁移为 Renderable struct
3. 用焦点树替换扁平 Focus enum
4. 删除旧的 `render_ui` 过程式函数和旧 `Focus` enum

## 6. 文件变更总览

### P0 新增文件
| 文件 | 职责 |
|------|------|
| `src/action.rs` | Action enum 层 |
| `src/effect.rs` | Effect enum + 执行器 |
| `src/update.rs` | update() 入口 (~100行) |
| `src/update/global.rs` | GlobalAction 处理 |
| `src/update/room.rs` | RoomAction 处理 |
| `src/update/agent.rs` | AgentAction 处理 |
| `src/input.rs` | 参数化 InputAction 处理 |
| `src/state/mod.rs` | AppState |
| `src/state/thread_hydration.rs` | 线程水化 |
| `src/state/ingest_dedup.rs` | ingest 去重 |
| `src/state/workspace_filter.rs` | workspace 过滤 |
| `src/state/selection.rs` | 鼠标选区 |
| `src/translation/mod.rs` | ChatState + apply_ui_event |
| `src/translation/chat_state.rs` | ChatState 定义 |
| `src/translation/chat_item.rs` | ChatItem + 渲染辅助 |
| `src/translation/agent_translator.rs` | AgentTranslationState |
| `src/translation/format_helpers.rs` | 格式化辅助函数 |

### P0 重构文件
| 文件 | 变更 |
|------|------|
| `src/app.rs` | 5727→~300行, 只保留事件循环骨架 |
| `src/main.rs` | 适配新 update/effect 模型 |
| `src/event.rs` | 不变 |
| `src/translation.rs` | 删除, 拆分为 translation/ 目录 |
| `src/ui/input_bar.rs` | 提取 InputState 到 input.rs |

### P1 新增文件
| 文件 | 职责 |
|------|------|
| `src/frame.rs` | FrameRequester |

### P1 重构文件
| 文件 | 变更 |
|------|------|
| `src/main.rs` | 事件循环 select! 加 frame 源 |
| `src/ui/group_chat.rs` | 引入 RenderCache |
| `src/ui/mod.rs` | GroupChatState 加 version + render_cache |

### P2 新增文件
| 文件 | 职责 |
|------|------|
| `src/render/mod.rs` | Renderable trait + Column/Row |
| `src/render/primitives.rs` | Insets 等布局原语 |
| `src/focus.rs` | FocusManager + FocusNode |

### P2 重构文件
| 文件 | 变更 |
|------|------|
| `src/ui/mod.rs` | render_ui 改为组装 renderable 树 |
| `src/ui/*.rs` | 每个面板迁移为 Renderable struct |
| `src/ui/input_bar.rs` | 实现 Renderable |
| 删除旧 Focus enum | 替换为 FocusManager |

## 7. 成功标准

- [ ] `cargo build` 通过
- [ ] `cargo test` 通过 (含迁移后的 ~120 个测试)
- [ ] `app.rs` < 400 行
- [ ] 所有源文件 < 800 行 (理想 < 500)
- [ ] update() 是纯函数, 无 IO 调用
- [ ] InputState 不再有重复 handler
- [ ] 帧率限制生效: 流式高峰时终端不卡
- [ ] Group chat 有 RenderCache
- [ ] 所有面板实现 Renderable trait
- [ ] 焦点树替换扁平 Focus enum
- [ ] `docs/architecture-tui.md` 更新

## 8. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 大爆炸重构期间无法编译 | 在 feature branch 上工作, 定期 `cargo check` |
| ~120 个测试迁移工作量 | 测试从 async 变同步后更简单, 逐步迁移 |
| Action/Effect 拆分粒度不当 | 先从现有 handle_* 方法签名提取, 保持行为一致 |
| Renderable trait 设计不适合 minos 布局 | P2 才引入, 有 P0/P1 的经验基础 |
| 群聊调和逻辑复杂, 拆分易出 bug | 保持 group_chat.rs 核心逻辑不变, 只移动调用入口 |
