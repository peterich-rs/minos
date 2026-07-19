# P0-B: AppState 提取 + Action/Effect 层 Introduction Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 `app.rs` 提取纯业务状态为 `AppState`,引入 `Action`/`Effect`/`StateChange` 类型定义,建立 `event_to_actions()` → `update()` → `execute_effects()` 的骨架。此阶段不迁移所有逻辑 — 只建立类型和骨架,现有 `handle_*` 方法暂时保留但委托给新架构。

**Architecture:** 渐进式引入新类型。先定义 `Action`/`Effect`/`StateChange` enums 和 `AppState` struct。然后把 `app.rs` 的业务字段搬入 `AppState`,`App` 持有 `AppState`。最后建立 `update()` 入口和 `event_to_actions()` 骨架(大部分映射为 TODO,逐个迁移在 P0-C)。

**Tech Stack:** Rust, edition 2021。

**Spec:** `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md` §3.1–3.4

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`

**Prerequisite:** P0-A (translation split) 已完成。

**Status 2026-06-18:** P0-B 的过渡 scaffold 已收敛为最终形态。`Action::Passthrough`、未接线的 submit intent、no-op global/agent intent、`Effect::None`、`ApprovalDecision` 和未执行的占位 backend effects 已删除；当前 `Action`/`Effect` 只保留 update/effect executor 实际消费的分支。

---

## File Structure

| File | Responsibility |
|---|---|
| `src/action.rs` | `Action`, `GlobalAction`, `RoomAction`, `AgentAction`, `InputAction`, `InputTarget` + 辅助 enums |
| `src/effect.rs` | `Effect`, `EffectResult`, `StateChange` |
| `src/state/mod.rs` | `AppState` struct — 从 `App` 提取的业务字段 |
| `src/state/thread_hydration.rs` | 水化/水位线/replay 方法 (从 app.rs 搬移) |
| `src/state/ingest_dedup.rs` | ingest 去重 (从 app.rs 搬移) |
| `src/state/workspace_filter.rs` | workspace 过滤 (从 app.rs 搬移) |
| `src/state/selection.rs` | 鼠标选区 (从 app.rs 搬移) |
| `src/app.rs` | 瘦身后的事件循环 |

**关键约束:** `AppState` 不持有 `backend: Arc<dyn AgentBackend>` — backend 留在 `App` 中,通过 Effect 间接调用。`AppState` 只持有纯数据和状态集合。

---

## Task 1: 创建 action.rs — Action enum 层

**Files:**
- Create: `src/action.rs`
- Modify: `src/main.rs` — 添加 `mod action;`

- [ ] **Step 1: 定义所有 Action 类型**

```rust
//! 语义化的用户意图, 由事件映射层生成, 由 update 层消费。

use std::path::PathBuf;

use minos_domain::AgentName;
use minos_protocol::LocalIngestFrame;

use crate::event::McpToolEvent;

/// 顶层 Action, 按作用域分层。
pub enum Action {
    Global(GlobalAction),
    Room(RoomAction),
    Agent(AgentAction),
    Input(InputTarget, InputAction),
    /// Effect 回流: 后台副作用完成后产生。
    EffectCompleted(EffectResult),
    /// 原始事件透传 (迁移过渡期使用, 逐步替换为具体 Action)。
    Passthrough(crate::event::AppEvent),
}

pub enum GlobalAction {
    Quit,
    CycleFocus,
    ToggleAgentDetail,
    OpenAgentPicker,
    Scroll(ScrollTarget, ScrollDirection, u16),
    CopySelection,
    Paste(String),
    MouseClick { target: ClickTarget, x: u16, y: u16 },
    MouseDrag { x: u16, y: u16 },
    MouseScroll { target: ScrollTarget, direction: ScrollDirection },
    McpToolCall(McpToolEvent),
    SyncDaemonThreads,
    RetryPendingAgentGroupResults,
    ExpireErrorFlash,
    RefreshBackendState,
    LoadGroupChatHistory,
    RequestRedraw,
    Escape,
    Enter,
    Delete,
    SelectIndex(usize),
}

pub enum RoomAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    SubmitInput(String),
    CycleFocusFromRoom,
}

pub enum AgentAction {
    Select(usize),
    Scroll(ScrollDirection, u16),
    SubmitInput(String),
    Interrupt,
    Close,
    Delete,
    Resume,
    ToggleToolExpansion,
    AnswerApproval(String),
    StartNew(AgentName),
    InviteToRoom(AgentName),
}

pub enum InputTarget {
    Room,
    Agent,
}

pub enum InputAction {
    InsertChar(char),
    InsertText(String),
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

pub enum CursorDirection { Left, Right, LineStart, LineEnd }
pub enum CursorLineDirection { Up, Down }
pub enum HistoryDirection { Previous, Next }
pub enum ScrollDirection { Up, Down, Top, Bottom }
pub enum ScrollTarget { GroupChat, AgentChat, ActivePane }
pub enum ClickTarget { RoomList, GroupChat, AgentList, AgentChat, RoomInput, AgentInput }

/// Effect 执行完成后的回流结果。
pub enum EffectResult {
    AgentStarted { agent: AgentName, thread_id: String, cwd: PathBuf, text: String },
    SendFailed { thread_id: String, error: String },
    IngestArrived(LocalIngestFrame),
    ManagerEvent(minos_agent_runtime::ManagerEvent),
}
```

- [ ] **Step 2: 添加 `mod action;` 到 main.rs**

在 `src/main.rs` 的 mod 声明区(line ~14–21)添加:
```rust
mod action;
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED (action.rs 目前只有类型定义,无引用)

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/action.rs crates/minos-tui/src/main.rs
git commit -m "refactor(tui): add Action/GlobalAction/RoomAction/AgentAction/InputAction types"
```

---

## Task 2: 创建 effect.rs — Effect + StateChange

**Files:**
- Create: `src/effect.rs`
- Modify: `src/main.rs` — 添加 `mod effect;`

- [ ] **Step 1: 定义 Effect 和 StateChange**

```rust
//! 副作用描述与状态变更标记。

use std::path::PathBuf;

use minos_domain::AgentName;
use minos_protocol::LocalGroupChatMessage;

/// update() 返回的待执行副作用。
/// App 的事件循环负责执行这些 Effect (调用 backend)。
#[derive(Debug)]
pub enum Effect {
    StartAgent { agent: AgentName, workspace: PathBuf },
    SendMessage { thread_id: String, text: String },
    SendApproval { thread_id: String, decision: ApprovalDecision },
    InterruptThread(String),
    CloseThread(String),
    DeleteThread(String),
    ResumeThread(String),
    HydrateThreadHistory(String),
    SyncDaemonThreads,
    WriteGroupChat { message: LocalGroupChatMessage },
    UpsertGroupChat { message: LocalGroupChatMessage },
    RetryPendingAgentGroupResults,
    CopyToClipboard(String),
    /// 无副作用, 仅状态变更。
    None,
}

#[derive(Debug)]
pub enum ApprovalDecision {
    CodexApproval { request_id: String, decision: serde_json::Value },
    OpencodePermission { request_id: String, response: serde_json::Value },
    OpencodeQuestion { request_id: String, answers: Vec<Vec<String>> },
}

/// update() 返回的状态变更标记。
#[derive(Debug, Default)]
pub struct StateChange {
    pub needs_redraw: bool,
}

impl StateChange {
    pub fn redraw() -> Self {
        Self { needs_redraw: true }
    }

    pub fn none() -> Self {
        Self { needs_redraw: false }
    }

    pub fn or_redraw(&mut self, other: Self) {
        self.needs_redraw |= other.needs_redraw;
    }
}
```

- [ ] **Step 2: 添加 `mod effect;` 到 main.rs**

```rust
mod effect;
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/effect.rs crates/minos-tui/src/main.rs
git commit -m "refactor(tui): add Effect, ApprovalDecision, and StateChange types"
```

---

## Task 3: 创建 state/ 目录 + AppState struct

**Files:**
- Create: `src/state/mod.rs`
- Create: `src/state/ingest_dedup.rs` (placeholder)
- Create: `src/state/workspace_filter.rs` (placeholder)
- Modify: `src/main.rs` — 添加 `mod state;`
- Modify: `src/app.rs` — App 持有 AppState

- [ ] **Step 1: 创建 `src/state/mod.rs`**

```rust
//! 纯业务状态, 不持有 backend 引用。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::group_chat::GroupChatStore;

/// 从 App 提取的纯数据状态。
/// backend 留在 App 中, 通过 Effect 间接调用。
pub struct AppState {
    pub workspace: PathBuf,
    pub hydrated_threads: HashSet<String>,
    pub thread_watermarks: HashMap<String, u64>,
    pub applied_ingest_fingerprints: HashSet<String>,
    pub group_chat_store: GroupChatStore,
    pub recorded_agent_results: HashMap<String, String>,
    pub last_daemon_history_sync: Option<std::time::Instant>,
    pub last_group_result_retry: Option<std::time::Instant>,
}

impl AppState {
    pub fn new(workspace: PathBuf, group_chat_store: GroupChatStore) -> Self {
        Self {
            workspace,
            hydrated_threads: HashSet::new(),
            thread_watermarks: HashMap::new(),
            applied_ingest_fingerprints: HashSet::new(),
            group_chat_store,
            recorded_agent_results: HashMap::new(),
            last_daemon_history_sync: None,
            last_group_result_retry: None,
        }
    }
}
```

- [ ] **Step 2: 创建 placeholder 子模块**

`src/state/ingest_dedup.rs`:
```rust
//! Ingest 去重逻辑 (从 app.rs 搬移, P0-C 完成)。
```

`src/state/workspace_filter.rs`:
```rust
//! Workspace 过滤逻辑 (从 app.rs 搬移, P0-C 完成)。
```

在 `state/mod.rs` 添加:
```rust
mod ingest_dedup;
mod workspace_filter;
```

- [ ] **Step 3: 修改 `App` struct 持有 AppState**

在 `src/app.rs`:

把 App struct (lines 31–44) 的业务字段替换为 `state: AppState`:

```rust
pub struct App {
    backend: Arc<dyn AgentBackend>,
    state: AppState,
    ui: UiState,
    should_quit: bool,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<AppEvent>>,
}
```

- [ ] **Step 4: 更新 App::new / with_group_chat_store**

`with_group_chat_store` (lines 52–79) 改为构造 `AppState`:

```rust
fn with_group_chat_store(
    backend: Arc<dyn AgentBackend>,
    readonly: bool,
    workspace: PathBuf,
    group_chat_store: GroupChatStore,
) -> Self {
    let mut ui = UiState::new(readonly);
    ui.rooms.push(RoomEntry {
        room_id: workspace_room_id(workspace.as_path()),
        title: default_room_title(workspace.as_path()),
    });
    ui.selected_room = Some(0);
    ui.room_list_state.select(Some(0));
    Self {
        backend,
        state: AppState::new(workspace, group_chat_store),
        ui,
        should_quit: false,
        event_tx: None,
    }
}
```

- [ ] **Step 5: 全局替换 `self.workspace` → `self.state.workspace`**

所有引用 `self.workspace` 的方法需要改为 `self.state.workspace`。用 `rg` 找出所有出现:

Run: `rg "self\.workspace\b" crates/minos-tui/src/app.rs -n`
Expected: 约 5–10 处。逐一替换为 `self.state.workspace`。

同理替换:
- `self.hydrated_threads` → `self.state.hydrated_threads`
- `self.thread_watermarks` → `self.state.thread_watermarks`
- `self.applied_ingest_fingerprints` → `self.state.applied_ingest_fingerprints`
- `self.group_chat_store` → `self.state.group_chat_store`
- `self.recorded_agent_results` → `self.state.recorded_agent_results`
- `self.last_daemon_history_sync` → `self.state.last_daemon_history_sync`
- `self.last_group_result_retry` → `self.state.last_group_result_retry`

- [ ] **Step 6: 添加 `mod state;` 到 main.rs**

- [ ] **Step 7: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -30`
Expected: BUILD SUCCEEDED

如果失败,按编译器提示修复遗漏的 `self.xxx` → `self.state.xxx` 替换。

- [ ] **Step 8: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 所有 43 个测试通过

- [ ] **Step 9: Commit**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): extract AppState from App, move business fields to state/"
```

---

## Task 4: 建立 update() 骨架 + event_to_actions() 过渡层

**Files:**
- Create: `src/update.rs`
- Modify: `src/main.rs` — 添加 `mod update;`
- Modify: `src/app.rs` — handle_event 委托给 update

- [ ] **Step 1: 创建 `src/update.rs` 骨架**

```rust
//! update 层: 消费 Action, 变更状态, 返回 StateChange + Effect 列表。

use crate::action::Action;
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::ui::UiState;

/// 处理单个 Action, 返回 (状态变更标记, 待执行 Effect 列表)。
///
/// 迁移过渡期: 大部分 Action 走 Passthrough 路径,
/// 逐步把 handle_* 逻辑搬入此函数。
pub fn update(_state: &mut AppState, _ui: &mut UiState, action: Action) -> (StateChange, Vec<Effect>) {
    match action {
        Action::Passthrough(_) => {
            // 过渡: Passthrough 由 App::handle_event 原地处理
            (StateChange::redraw(), vec![])
        }
        _ => {
            // TODO: 逐步迁移每个 Action variant 的处理逻辑
            (StateChange::redraw(), vec![])
        }
    }
}
```

- [ ] **Step 2: 添加 `mod update;` 到 main.rs**

- [ ] **Step 3: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/update.rs crates/minos-tui/src/main.rs
git commit -m "refactor(tui): add update() skeleton entry point"
```

---

## Task 5: 搬迁 app.rs 自由函数到 state/ 子模块

**Files:**
- Fill: `src/state/ingest_dedup.rs`
- Fill: `src/state/workspace_filter.rs`
- Create: `src/state/selection.rs`
- Modify: `src/app.rs` — 删除搬迁的函数,改为调用 `state::*`
- Modify: `src/state/mod.rs` — 添加 `mod selection;`

**目标:** 把 `app.rs` 的 private free functions (lines 3024–3461) 按职责搬到 `state/` 子模块。

- [ ] **Step 1: 搬迁 workspace_filter.rs**

从 app.rs 搬移:
- `workspace_path_belongs_to_current_workspace` (App 方法 408–410 → 提取为 free function)
- `group_message_belongs_to_current_workspace` (412–416)
- `filter_group_messages_for_current_workspace` (418–426)
- `prune_external_threads` (428–463)
- `remove_thread_local_state` (465–493)
- `workspace_paths_match` (3053–3055)
- `normalized_workspace_path` (3057–3059)
- `workspace_room_id` (3061–3063)
- `default_room_title` (3065–3067)

这些函数引用 `self.workspace` — 改为接收 `workspace: &Path` 参数:
```rust
pub(super) fn workspace_path_belongs_to_current_workspace(
    workspace: &Path,
    candidate: &Path,
) -> bool { ... }
```

`prune_external_threads` 和 `remove_thread_local_state` 修改 `AppState` + `UiState`:
```rust
pub(super) fn prune_external_threads(
    state: &mut AppState,
    ui: &mut UiState,
) -> bool { ... }
```

在 `app.rs` 中,把这些方法从 `impl App` 改为调用 free function:
```rust
// Before:
fn workspace_path_belongs_to_current_workspace(&self, workspace: &Path) -> bool {
    workspace_paths_match(&self.state.workspace, workspace)
}
// After:
fn workspace_path_belongs_to_current_workspace(&self, workspace: &Path) -> bool {
    super::state::workspace_filter::workspace_path_belongs_to_current_workspace(&self.state.workspace, workspace)
}
```

或者更简洁: 直接在调用处替换。

- [ ] **Step 2: 搬迁 ingest_dedup.rs**

从 app.rs 搬移:
- `mark_ingest_applied` (App 方法 495–523 → 提取为 free function)
- `ingest_fingerprint` (3152–3158)
- `frame_marks_agent_result_done` (3093–3101)
- `thread_is_done` (3089–3091)

```rust
pub(super) fn mark_ingest_applied(
    state: &mut AppState,
    frame: &minos_protocol::LocalIngestFrame,
) -> bool { ... }
```

- [ ] **Step 3: 搬迁 selection.rs**

从 app.rs 搬移几何函数:
- `rect_contains` (3287–3294)
- `chat_content_area` (3296–3306)
- `chat_selection_point` (3308–3332)
- `clicked_thread_index` (3447–3463)

以及 App 上的选区方法:
- `current_chat_selection_active` (1540–1546)
- `begin_chat_selection` (1548–1563)
- `handle_chat_selection_mouse` (1565–1592)

选区方法操作 `UiState`:
```rust
pub(super) fn begin_selection(
    ui: &mut UiState,
    column: u16,
    row: u16,
) -> bool { ... }
```

- [ ] **Step 4: 更新 state/mod.rs**

```rust
mod ingest_dedup;
mod selection;
mod workspace_filter;

pub(super) use ingest_dedup::*;
pub(super) use selection::*;
pub(super) use workspace_filter::*;
```

- [ ] **Step 5: 更新 app.rs 调用点**

所有原来调用 `self.mark_ingest_applied(frame)` 的地方改为:
```rust
crate::state::ingest_dedup::mark_ingest_applied(&mut self.state, frame)
```

或通过 `use` 引入:
```rust
use crate::state::{ingest_dedup, workspace_filter, selection};
// 然后:
ingest_dedup::mark_ingest_applied(&mut self.state, frame)
```

- [ ] **Step 6: 搬迁其他自由函数**

剩余 app.rs 自由函数(非几何/过滤/去重):
- `is_text_input_key`, `is_input_focus`, `normalize_pasted_text`, `thread_can_receive_message` — 搬到 `src/input.rs` (P0-C 创建)
- `format_error_chain` — 留在 app.rs 或搬到 `src/state/mod.rs`
- `default_group_chat_store` — 留在 app.rs (构造时使用)
- `thread_is_done`, `frame_marks_agent_result_done` — 已搬入 ingest_dedup.rs
- `group_agent_result_message_id`, `short_thread_id` — 搬到 `src/state/mod.rs`
- `AgentRouteTarget` + routing parser — 搬到 `src/state/mod.rs` 或新的 `src/routing.rs`
- `codex_user_input_decision` 等 decision builders — 搬到 `src/effect.rs` 或 `src/state/mod.rs`
- clipboard 函数 — 搬到 `src/clipboard.rs` (新建)

对于本任务,只搬迁 workspace_filter/ingest_dedup/selection 三组。其余在 P0-C 搬迁。

- [ ] **Step 7: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -30`
Expected: BUILD SUCCEEDED

- [ ] **Step 8: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 所有测试通过

- [ ] **Step 9: Commit**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): migrate workspace_filter, ingest_dedup, selection to state/ modules"
```

---

## Task 6: 更新 architecture-tui.md

**Files:**
- Modify: `docs/architecture-tui.md`

- [ ] **Step 1: 更新文档**

在文件清单中添加新文件:
```markdown
| `action.rs` | Action/GlobalAction/RoomAction/AgentAction/InputAction | ~120 |
| `effect.rs` | Effect/ApprovalDecision/StateChange | ~80 |
| `update.rs` | update() 入口骨架 | ~30 |
| `state/mod.rs` | AppState 纯业务状态 | ~60 |
| `state/ingest_dedup.rs` | ingest 去重 | ~80 |
| `state/workspace_filter.rs` | workspace 过滤 | ~120 |
| `state/selection.rs` | 鼠标选区 | ~100 |
```

更新"核心状态"章节,描述 App vs AppState 分离。

- [ ] **Step 2: Commit**

```bash
git add docs/architecture-tui.md
git commit -m "docs: update architecture-tui.md for AppState extraction and Action/Effect layer"
```
