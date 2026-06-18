# P0-C: app.rs 事件处理迁移 + InputState 统一 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 把 `app.rs` 的事件处理从直接 `handle_*_key` 原地修改,迁移为 semantic `Action` → `update()` → `execute_effects()` 路径。消灭 `room_input`/`agent_input` 重复 handler。最终 `app.rs` 只保留 `App` shell，事件循环和 effect 执行器拆到 `src/app/event_loop.rs`。

**Architecture:** 逐个事件类型迁移。`Key`/`Mouse`/`Paste` 等事件映射为 `InputAction`/`GlobalAction`/`RoomAction`/`AgentAction`,`update()` 消费这些 Action 修改 `AppState`+`UiState` 并返回 `Effect` 列表,`App` 执行 Effect(调 backend)。`Ingest`/`ManagerEvent` 走 `EffectCompleted` 回流路径，`McpToolCall` 走 `GlobalAction::McpToolCall` → `Effect::HandleMcpToolCall`。

**Tech Stack:** Rust, edition 2021。

**Spec:** `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md` §3.2–3.4, §3.6

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`

**Prerequisite:** P0-A (translation split), P0-B (AppState + Action/Effect) 已完成。

**Status 2026-06-17:** P0-C implementation, documentation sync, and verification are complete in the working tree. Git commit steps are intentionally not performed by Codex unless explicitly requested.

**Completed shape:**
- `app.rs` is 84 lines and only contains the `App` shell, constructors, module declarations, and test-module hook.
- Runtime methods moved into `src/app/` (`event_mapping`, `event_loop`, `lifecycle`, `submission`, `group_chat`, `mcp`, `thread_ops`, `clipboard`, `helpers`).
- Key, paste, mouse, tick, ingest, manager, MCP, delete-confirm, picker, room, agent, and submit paths now dispatch semantic `Action`s into `update()` and execute returned `Effect`s.
- Key mapping is isolated in `src/app/event_mapping.rs`; `app/event_loop.rs` no longer owns pane-specific key mapping.
- Room/agent submit decisions clear and record input in update, resolve routing or pending requests, and return explicit effects such as `DispatchPromptToAgent`, `SendTextToThread`, and `SubmitPendingAgentRequest`.
- Migration placeholder intents/effects have been removed from `Action` and `Effect`.
- Tests were extracted to `src/app_tests.rs`.
- Verification command used: `cargo fmt -p minos-tui && CARGO_BUILD_JOBS=1 cargo test -p minos-tui && CARGO_BUILD_JOBS=1 cargo clippy -p minos-tui --all-targets -- -D warnings`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/input.rs` | 参数化 `InputAction` 处理 + InputState 辅助函数(消灭重复) |
| `src/update/mod.rs` | update 层入口; 扩充: 逐个 Action variant 的处理逻辑 |
| `src/update/global.rs` | GlobalAction 处理 |
| `src/update/room.rs` | RoomAction 处理 |
| `src/update/agent.rs` | AgentAction 处理 |
| `src/app.rs` | `App` shell + 构造器 + 测试模块 hook |
| `src/app/*.rs` | 事件循环、effect 执行、生命周期、提交、群聊、MCP、剪贴板和 thread 操作 |
| `src/app/event_mapping.rs` | key event 到 semantic Action/input target 的纯映射 |
| `src/effect.rs` | Effect 描述与 StateChange |
| `src/agent_route.rs` | 共享 `@agent[#thread]` 路由解析和 short thread id 辅助 |

---

## Task 1: 创建 input.rs — 统一 InputState 处理

**Files:**
- Create: `src/input.rs`
- Modify: `src/main.rs` — 添加 `mod input;`

**目标:** 把 `app.rs` 的 `handle_room_input_key` (721–988) 和 `handle_agent_input_key` (990–1195) 合并为一个参数化 handler。

- [x] **Step 1: 创建 input.rs 骨架**

```rust
//! 参数化的输入栏 Action 处理, 消灭 room_input / agent_input 重复。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{InputAction, InputTarget};
use crate::effect::{Effect, StateChange};
use crate::ui::input_bar::InputState;
use crate::ui::Focus;
use crate::state::AppState;

/// 把按键事件映射为 InputAction (或 None 如果不是输入键)。
/// 只读 state, 不修改。
pub fn key_to_input_action(
    key: KeyEvent,
    input: &InputState,
) -> Option<InputAction> {
    // 搬移 app.rs 的 is_text_input_key 逻辑 (lines 3024–3028)
    // + handle_room_input_key / handle_agent_input_key 中的键映射

    // Ctrl+A/E/B/F/K/U/W, Alt+B/D/F 等 emacs 键
    // Char, Backspace, Delete, Enter, Tab
    // Up/Down 历史
    // @ 触发 mention picker
    // 具体映射从 handle_room_input_key 的 match 分支提取

    // 注: room 和 agent 的键映射几乎完全一致, 差异仅在 Enter 的行为
    // (room Enter=submit, agent Enter=submit_or_answer_pending)
    // submit 统一为 InputAction::Submit, 由 update 层根据 target 区分

    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a' => Some(InputAction::MoveCursor(CursorDirection::LineStart)),
                    'e' => Some(InputAction::MoveCursor(CursorDirection::LineEnd)),
                    'b' => Some(InputAction::MoveCursor(CursorDirection::Left)),
                    'f' => Some(InputAction::MoveCursor(CursorDirection::Right)),
                    'k' => Some(InputAction::DeleteToEndOfLine),
                    'u' => Some(InputAction::DeleteWord), // 或 DeleteToLineStart
                    'w' => Some(InputAction::DeleteWord),
                    _ => None,
                }
            } else if key.modifiers.contains(KeyModifiers::ALT) {
                match c {
                    'b' => Some(InputAction::MoveCursorWord(CursorDirection::Left)),
                    'f' => Some(InputAction::MoveCursorWord(CursorDirection::Right)),
                    'd' => Some(InputAction::DeleteWord),
                    _ => None,
                }
            } else {
                Some(InputAction::InsertChar(c))
            }
        }
        KeyCode::Backspace => Some(InputAction::DeleteBackward),
        KeyCode::Delete => Some(InputAction::DeleteBackward), // 或 ForwardDelete
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::ALT) {
                Some(InputAction::NewLine)
            } else if input.multiline && key.modifiers.contains(KeyModifiers::SHIFT) {
                Some(InputAction::NewLine)
            } else {
                Some(InputAction::Submit)
            }
        }
        KeyCode::Tab => Some(InputAction::TogglePathPicker), // 或 SelectPickerItem
        KeyCode::Up => Some(InputAction::HistoryNavigate(HistoryDirection::Previous)),
        KeyCode::Down => Some(InputAction::HistoryNavigate(HistoryDirection::Next)),
        _ => None,
    }
}

use crate::action::{CursorDirection, HistoryDirection};
```

**注意:** 上面的代码是示意。实际实现需要仔细比对 `handle_room_input_key` 和 `handle_agent_input_key` 的每个 match 分支,确保不遗漏。两个 handler 的差异点:
1. `@` mention picker 的候选列表不同 (room 有所有 agent, agent 只有非当前 agent)
2. Tab/Enter 在 picker 打开时的行为
3. agent input 在 pending request 时的特殊 Enter 行为

这些差异通过 `InputTarget` 参数在 `update()` 层处理,不在 `key_to_input_action` 中。

- [x] **Step 2: 实现 apply_input_action**

```rust
/// 把 InputAction 应用到 InputState, 返回 (StateChange, Option<InputAction::Submit 的信号>)。
/// 不执行 submit — submit 由上层根据 target 决定 Effect。
pub fn apply_input_action(
    input: &mut InputState,
    action: InputAction,
) -> StateChange {
    match action {
        InputAction::InsertChar(c) => { input.insert_char(c); StateChange::redraw() }
        InputAction::InsertText(text) => { input.insert_text(&text); StateChange::redraw() }
        InputAction::DeleteBackward => { input.delete_backward(); StateChange::redraw() }
        InputAction::DeleteWord => { input.delete_word(); StateChange::redraw() }
        InputAction::DeleteToEndOfLine => { input.delete_to_end_of_line(); StateChange::redraw() }
        InputAction::MoveCursor(dir) => { input.move_cursor(dir); StateChange::redraw() }
        InputAction::MoveCursorWord(dir) => { input.move_cursor_word(dir); StateChange::redraw() }
        InputAction::MoveCursorLine(dir) => { input.move_cursor_line(dir); StateChange::redraw() }
        InputAction::NewLine => { input.insert_newline(); StateChange::redraw() }
        InputAction::ToggleMultilineMode => { input.toggle_multiline(); StateChange::redraw() }
        InputAction::HistoryNavigate(dir) => { input.navigate_history(dir); StateChange::redraw() }
        InputAction::ToggleMentionPicker => { input.toggle_mention_picker(); StateChange::redraw() }
        InputAction::TogglePathPicker => { input.toggle_path_picker(); StateChange::redraw() }
        InputAction::SelectPickerItem(idx) => { input.select_picker_item(idx); StateChange::redraw() }
        InputAction::DismissPicker => { input.dismiss_picker(); StateChange::redraw() }
        InputAction::Submit => StateChange::none(), // submit 由上层处理
        InputAction::HistorySearch(_) => StateChange::none(),
    }
}
```

**注意:** `InputState` 当前的编辑方法(insert_char 等)在 `ui/input_bar.rs` 中。如果这些方法不存在(当前编辑逻辑内联在 app.rs 的 handle_*_key 中),需要先从 handle_*_key 提取出 InputState 的编辑方法。查看 `ui/input_bar.rs` 的 `impl InputState`:

Run: `rg "fn " crates/minos-tui/src/ui/input_bar.rs | rg "impl InputState" -A 100`
检查现有方法。如果 `insert_char`, `delete_backward` 等已存在,直接调用。如果不存在,需要从 app.rs 提取。

- [x] **Step 3: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED (input.rs 目前未被引用)

- [x] **Step 4: Commit (skipped by Codex unless explicitly requested)**

```bash
git add crates/minos-tui/src/input.rs crates/minos-tui/src/main.rs
git commit -m "refactor(tui): add parameterized input action handler in input.rs"
```

---

## Task 2: 拆分 update.rs 为子模块

**Files:**
- Create: `src/update/global.rs`
- Create: `src/update/room.rs`
- Create: `src/update/agent.rs`
- Modify: `src/update.rs` → 变为 `src/update/mod.rs`

- [x] **Step 1: 把 update.rs 改为 update/mod.rs**

```bash
mv crates/minos-tui/src/update.rs crates/minos-tui/src/update/mod.rs
```

- [x] **Step 2: 更新 update/mod.rs**

```rust
//! update 层入口。

mod agent;
mod global;
mod room;

use crate::action::Action;
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::ui::UiState;

/// 处理单个 Action。
pub fn update(state: &mut AppState, ui: &mut UiState, action: Action) -> (StateChange, Vec<Effect>) {
    match action {
        Action::Global(a) => global::handle(state, ui, a),
        Action::Room(a) => room::handle(state, ui, a),
        Action::Agent(a) => agent::handle(state, ui, a),
        Action::Input(target, a) => handle_input(state, ui, target, a),
        Action::EffectCompleted(result) => handle_effect_result(state, ui, result),
        Action::Passthrough(evt) => {
            // 过渡: 由 App::handle_event 原地处理
            (StateChange::redraw(), vec![])
        }
    }
}

fn handle_input(
    _state: &mut AppState,
    ui: &mut UiState,
    target: InputTarget,
    action: InputAction,
) -> (StateChange, Vec<Effect>) {
    let input = match target {
        InputTarget::Room => &mut ui.room_input,
        InputTarget::Agent => &mut ui.agent_input,
    };
    let change = crate::input::apply_input_action(input, action);
    (change, vec![])
}

fn handle_effect_result(
    _state: &mut AppState,
    _ui: &mut UiState,
    _result: crate::action::EffectResult,
) -> (StateChange, Vec<Effect>) {
    // 当前实现见 src/update/mod.rs:
    // AgentStarted -> Effect::AgentStartedForPrompt
    // SendFailed -> ui.set_error(...)
    // IngestArrived -> Effect::HandleIngest
    // ManagerEvent -> Effect::HandleManagerEvent
    (StateChange::none(), vec![])
}
```

- [x] **Step 3: 创建 update/global.rs**

```rust
use crate::action::GlobalAction;
use crate::effect::{Effect, StateChange};
use crate::state::AppState;
use crate::ui::UiState;

pub fn handle(
    state: &mut AppState,
    ui: &mut UiState,
    action: GlobalAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        GlobalAction::Quit => {
            return (StateChange::none(), vec![Effect::Quit]);
        }
        GlobalAction::CycleFocus => {
            cycle_focus(ui)
        }
        // 当前实现还覆盖全局滚动、Escape、Delete confirm、picker、鼠标、Tick、MCP 等。
        _ => (StateChange::redraw(), vec![]),
    }
}
```

- [x] **Step 4: 创建 update/room.rs 和 update/agent.rs**

同样的模块结构,`match action { ... }`,并在后续任务中补齐各 action arm。当前 P0-C 实现已覆盖输入、全局键、面板键、鼠标、effect 回流、MCP 和 submit 路径。

- [x] **Step 5: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -10`
Expected: BUILD SUCCEEDED

- [x] **Step 6: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): split update.rs into update/{global,room,agent}.rs modules"
```

---

## Task 3: 迁移 InputAction 到 update 路径 + 实现 Submit

**Files:**
- Modify: `src/update/mod.rs` — `handle_input` 处理 Submit
- Modify: `src/update/room.rs` — RoomAction::SubmitInput
- Modify: `src/update/agent.rs` — AgentAction::SubmitInput
- Modify: `src/app.rs` — `handle_key` 对输入面板委托给 `key_to_input_action`

**目标:** 让 Enter 键走 Action 路径完成消息提交。这是最关键的迁移 — 验证 event→action→update→effect 全链路。

- [x] **Step 1: 在 handle_input 中处理 Submit**

```rust
fn handle_input(
    state: &mut AppState,
    ui: &mut UiState,
    target: InputTarget,
    action: InputAction,
) -> (StateChange, Vec<Effect>) {
    if matches!(action, InputAction::Submit) {
        return match target {
            InputTarget::Room => room::handle_submit(state, ui),
            InputTarget::Agent => agent::handle_submit(state, ui),
        };
    }
    let input = match target {
        InputTarget::Room => &mut ui.room_input,
        InputTarget::Agent => &mut ui.agent_input,
    };
    let change = crate::input::apply_input_action(input, action);
    (change, vec![])
}
```

- [x] **Step 2: 实现 room::handle_submit**

从 `app.rs` 的 `submit_room_input` (1963–2009) 迁移逻辑:

```rust
pub fn handle_submit(
    _state: &mut AppState,
    ui: &mut UiState,
) -> (StateChange, Vec<Effect>) {
    let text = ui.room_input.content.clone();
    if text.trim().is_empty() {
        return (StateChange::none(), vec![]);
    }

    // 解析 @agent 路由
    // 返回 Effect::SendMessage 或 Effect::StartAgent
    // 清空输入栏

    // ... 迁移 submit_room_input 的核心逻辑
    // 注意: 原方法调用 self.backend.* 和 self.record_user_group_message
    // 改为返回 Effect, 由 App 执行

    (StateChange::redraw(), vec![/* effects */])
}
```

**注意:** `submit_room_input` 当前做了很多事(解析路由、echo群聊消息、启动agent、发送消息)。这些需要拆分为:
1. 纯状态变更(echo消息到UI、清空输入) → 在 update 中完成
2. 副作用(启动agent、发送消息、写群聊) → 返回为 Effect

`record_user_group_message` 变为 `Effect::WriteGroupChat`。

- [x] **Step 3: 实现 agent::handle_submit**

从 `submit_agent_input` (2011–2050) 迁移。类似处理。

- [x] **Step 4: 修改 app.rs handle_key 委托输入键**

在 `handle_key` (645–699) 中,对 `Focus::RoomInput` 和 `Focus::AgentInput` 的分支:

```rust
Focus::RoomInput => {
    if let Some(action) = crate::input::key_to_input_action(key, &self.ui.room_input) {
        let (change, effects) = crate::update::update(
            &mut self.state,
            &mut self.ui,
            Action::Input(InputTarget::Room, action),
        );
        self.execute_effects(effects).await;
        return change.needs_redraw;
    }
    // 未识别的键走旧路径
    self.handle_room_input_key(key).await
}
```

**迁移策略:** 先让 `key_to_input_action` 处理它能识别的键,其余继续走旧的 `handle_room_input_key`。逐步扩大 `key_to_input_action` 的覆盖范围,最终删除 `handle_room_input_key`。

- [x] **Step 5: 实现 execute_effects**

在 `app.rs` 添加:

```rust
async fn execute_effects(&mut self, effects: Vec<Effect>) {
    for effect in effects {
        self.execute_effect(effect).await;
    }
}

async fn execute_effect(&mut self, effect: Effect) {
    match effect {
        Effect::None => {}
        Effect::StartAgent { agent, workspace } => {
            // 搬移 dispatch_prompt_to_agent 的 backend 调用部分
            let backend = Arc::clone(&self.backend);
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match backend.start_agent(agent, workspace).await {
                    Ok(thread_id) => { /* 发送 EffectResult::AgentStarted */ }
                    Err(e) => { /* 发送 EffectResult::SendFailed */ }
                }
            });
        }
        Effect::SendMessage { thread_id, text } => {
            let backend = Arc::clone(&self.backend);
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = backend.send_message(&thread_id, &text).await {
                    let _ = tx.send(AppEvent::SendMessageFailed {
                        thread_id,
                        error: e.to_string(),
                    });
                }
            });
        }
        Effect::WriteGroupChat { message } => {
            self.append_group_chat_message(message).await;
        }
        Effect::InterruptThread(thread_id) => {
            let _ = self.backend.interrupt_thread(&thread_id).await;
        }
        Effect::CloseThread(thread_id) => {
            let _ = self.backend.close_thread(&thread_id).await;
        }
        Effect::DeleteThread(thread_id) => {
            let _ = self.backend.delete_thread(&thread_id).await;
        }
        Effect::CopyToClipboard(text) => {
            let _ = crate::clipboard::copy(&text);
        }
        // ... 其余 Effect variants
    }
}
```

- [x] **Step 6: 编译验证**

Run: `cargo build -p minos-tui 2>&1 | tail -20`
Expected: BUILD SUCCEEDED

- [x] **Step 7: 运行测试**

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: 所有测试通过。如果有失败,通常是 submit 路径的行为差异,需要调整 update 逻辑。

- [x] **Step 8: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): wire input submit through Action→update→Effect path"
```

---

## Task 4–8: 逐步迁移其余事件类型

以下任务重复同样的模式: 把 `handle_*_key` 的逻辑搬入 update 子模块,让 `handle_key` 委托给 Action 路径。每个 task 处理一个面板。

**每个 task 的通用步骤:**

1. 在 `key_to_input_action` 或新的 `key_to_*_action` 中添加该面板的键映射
2. 在 `update/{global,room,agent}.rs` 中实现对应的 Action variant
3. 在 `app.rs handle_key` 中让该面板的 Focus 分支委托给 Action 路径
4. 删除旧的 `handle_*_key` 方法
5. 运行测试验证行为不变
6. Commit skipped by Codex unless explicitly requested

### Task 4: 迁移 room_list_key + room_chat_key

- [x] **Step 1: 实现 RoomAction variants**
- [x] **Step 2: 委托 handle_room_list_key / handle_room_chat_key 到 Action 路径**
- [x] **Step 3: 删除旧 handler**
- [x] **Step 4: 测试完成; Commit skipped by Codex unless explicitly requested**

### Task 5: 迁移 agent_list_key + agent_chat_key

- [x] **Step 1: 实现 AgentAction variants (Scroll, ToggleToolExpansion, etc.)**
- [x] **Step 2: 委托到 Action 路径**
- [x] **Step 3: 删除旧 handler**
- [x] **Step 4: 测试完成; Commit skipped by Codex unless explicitly requested**

### Task 6: 迁移全局键 (Ctrl+C, Ctrl+D, Ctrl+V, Esc, Tab, n, Delete, PageUp/Down)

- [x] **Step 1: 实现 GlobalAction variants (Quit, Interrupt, Paste, Escape, etc.)**
- [x] **Step 2: 委托 handle_key 的全局分支到 Action 路径**
- [x] **Step 3: 删除 handle_ctrl_c 等旧方法**
- [x] **Step 4: 测试完成; Commit skipped by Codex unless explicitly requested**

### Task 7: 迁移鼠标事件

- [x] **Step 1: 实现 GlobalAction::MouseClick/MouseDrag/MouseScroll**
- [x] **Step 2: 委托 handle_mouse 到 Action 路径**
- [x] **Step 3: 删除旧 handle_mouse**
- [x] **Step 4: 测试完成; Commit skipped by Codex unless explicitly requested**

### Task 8: 迁移 Ingest + ManagerEvent + McpToolCall

- [x] **Step 1: 实现 EffectResult 回流到 update**
- [x] **Step 2: 委托 handle_manager_event / ingest 处理到 update 路径**
- [x] **Step 3: 删除旧 handler**
- [x] **Step 4: 测试完成; Commit skipped by Codex unless explicitly requested**

---

## Task 9: 迁移测试到新结构

**Files:**
- Modify: `src/app.rs` — 测试模块迁移到 `src/app_tests.rs`
- Create: `src/app_tests.rs`

**目标:** 现有 43 个测试直接调 `app.handle_room_input_key(press(KeyCode::Enter)).await`。迁移后改为测试 Action 路径。

- [x] **Step 1: 把测试提取到 app_tests.rs**

```rust
// src/app.rs:
#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
```

- [x] **Step 2: 逐步改写测试**

测试已提取到 `src/app_tests.rs`。现有测试继续通过 `App::handle_event` / `App::handle_key` 驱动行为；这些入口的 key、paste、mouse、tick、ingest、manager、MCP 和 submit 路径已路由到 `Action` → `update()` → `Effect`。

- [x] **Step 3: 运行全部测试**

Run: `cargo test -p minos-tui 2>&1 | tail -30`
Expected: 所有 43 个测试通过

- [x] **Step 4: clippy**

Run: `cargo clippy -p minos-tui --all-targets -- -D warnings 2>&1 | tail -20`
Expected: 无 warnings

- [x] **Step 5: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): migrate tests to Action→update→Effect path"
```

---

## Task 10: 最终清理 + app.rs 瘦身验证

**Files:**
- Modify: `src/app.rs` — 删除所有已迁移的方法

- [x] **Step 1: 删除已迁移的方法**

删除 app.rs 中所有已迁移到 update/state/input 的方法。最终 app.rs 只保留:
- `App` struct (持有 backend, state, ui, event_tx, should_quit)
- `new` / `with_group_chat_store`
- module declarations and `app_tests.rs` hook

`init` / `shutdown` / `handle_event` / `execute_effects` / `execute_effect` 已拆到 `src/app/lifecycle.rs` 与 `src/app/event_loop.rs`。

- [x] **Step 2: 验证行数**

Run: `wc -l crates/minos-tui/src/app.rs`
Expected: < 400 行 (不含测试)

- [x] **Step 3: 最终测试**

Run: `cargo test -p minos-tui`
Expected: 全部通过

- [x] **Step 4: Commit (skipped by Codex unless explicitly requested)**

```bash
git add -A crates/minos-tui/src/
git commit -m "refactor(tui): complete P0 — app.rs skeleton only, all logic in update/state/input"
```

---

## Task 11: 更新 architecture-tui.md

- [x] **Step 1: 重写文档的"核心状态"和"事件系统"章节**

描述新的四层架构: Input → Action → Update → Effect → Render。

- [x] **Step 2: 更新文件清单**

反映最终的文件结构和行数。

- [x] **Step 3: Commit (skipped by Codex unless explicitly requested)**

```bash
git add docs/architecture-tui.md
git commit -m "docs: update architecture-tui.md for P0 completion (Elm architecture)"
```
