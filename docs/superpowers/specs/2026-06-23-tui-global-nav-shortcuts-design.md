# TUI Global Navigation Shortcuts + Agent Picker Removal

**Date:** 2026-06-23
**Status:** Design — awaiting review
**Scope:** `crates/minos-tui`

## Summary

Add two global navigation shortcuts to the Minos TUI and remove the obsolete
modal agent picker system:

| Shortcut | Behavior |
|----------|----------|
| **Ctrl+P** | Jump directly to the Projects panel. Truncates the nav stack to `[Projects]`. Works from any normal nav level and any focus state. Blocking overlays keep their own key context. |
| **Ctrl+T** | Jump to the current project's Conversations panel. Truncates the nav stack to `[Projects, Conversations { project_id }]`. Works whenever the current nav level carries a `project_id`; on the Projects panel it is a no-op. |

The existing modal agent picker (opened via the `n` key, rendered as a "New
Thread" overlay) is removed entirely. Starting agents is now exclusively done
through the input bar's `@agent` routing or the conversation input flow.

## Motivation

The current TUI navigation is purely hierarchical and stack-based: `Esc` pops
one level at a time. There is no way to jump to a specific level without
repeatedly pressing `Esc`. For a deep nav stack like `Projects →
Conversations → Conversation → AgentDetail`, returning to the project list
requires up to three `Esc` presses.

Meanwhile, the modal agent picker (`n` key) is redundant with the input bar's
`@agent` routing and adds complexity: a separate overlay widget, a dedicated
key path, three `GlobalAction` variants used only by the picker, a dedicated
`Effect` variant, and a dedicated `start_agent_at` method. Removing it
simplifies the action/effect model and the key dispatch layer.

## Architecture Foundation

### Nav stack shape

The nav stack (`UiState.nav_stack: Vec<NavLevel>`, `ui/mod.rs:84`) is always a
complete path from the `Projects` root to the current level. Every deep level
redundantly carries the full parent context:

```rust
// nav.rs:5-20
pub enum NavLevel {
    Projects,
    Conversations { project_id: String },
    Conversation { project_id: String, conversation_id: String },
    AgentDetail { project_id: String, conversation_id: String, thread_id: String, agent: AgentName },
}
```

When entering a deeper level, the entire stack is reassigned to the complete
path, not incrementally pushed (see `update/mod.rs:107-178` for
`ConversationsLoaded` and `ConversationOpened` effects). The only incremental
push is `AgentDetail` (`update/nav.rs:187`), but its variant carries all parent
IDs.

`NavLevel::project_id()` (`nav.rs:22-30`) returns `Some` for every non-root
level. This means **Ctrl+T can resolve the target `project_id` from the stack
top in one call — no traversal needed.**

### Key dispatch layering

`key_to_mapping` (`event_mapping.rs:23`) dispatches in this order:

1. Delete-confirm modal (`:24-26`)
2. **Navigation Ctrl block** (`:28-42`) — `Ctrl+Q/C/V/D/P/T`. Runs before per-level dispatch, after blocking overlays.
3. Create-project dialog (`:40-42`)
4. Per-NavLevel dispatch (`:43-73`) — each level claims certain keys via early `return`
5. Agent picker modal — removed by this design
6. Global unmodified keys (`:79-115`) — `PageUp/Down`, `Home/End`, `BackTab`
7. Per-PaneId focus dispatch (`:117-125`)

The Ctrl block (step 2) is the correct insertion point for Ctrl+P/T because it
executes before any per-level or per-pane dispatch while still letting blocking
overlays own their key context.

## Design Decisions

### DD-1: Ctrl+P/T are navigation-global, not overlay-global

Ctrl+P and Ctrl+T trigger from any normal navigation level, including when the
input bar is focused. They do not interrupt blocking overlays such as the
project-create dialog or delete confirmation. Rationale:

- The TUI's custom `InputState` does not bind Ctrl+P or Ctrl+T to any editing
  operation (no history navigation, no transpose). The only input-related
  Ctrl bindings that exist are `Ctrl+J` (newline), `Ctrl+Alt+B` (cursor
  toggle), `Ctrl+C` (interrupt).
- Terminal-level readline semantics for Ctrl+P/Ctrl+T (previous-history,
  transpose-chars) belong to the shell, not to a TUI app's self-managed input
  widget. They do not apply here.
- Keeping the shortcuts input-guard-less makes them predictable within normal
  navigation: the user can jump without first defocusing the input bar.
- Blocking overlays are separate user tasks and should not be dismissed or
  bypassed by navigation shortcuts.

**Trade-off:** A user who habitually presses Ctrl+P/Ctrl+T while typing in the
input bar will be navigated away. This is accepted as simpler and more
consistent than adding an input-focus guard. The input buffer content is
preserved on nav stack truncation (UiState fields are independent of nav
stack).

### DD-2: Nav stack truncation via reassignment, not repeated pop

Both shortcuts reassign `ui.nav_stack` to a fresh `Vec` rather than calling
`pop_nav()` in a loop. This matches the existing pattern in
`update/mod.rs:107-178` where `ConversationsLoaded` and `ConversationOpened`
reassign the stack directly.

This is safe because:

- `selected_project`, `selected_conversation`, `selected_agent_session`,
  `conversation_messages`, `conversation_agent_sessions` are independent
  `UiState` fields, not derived from the nav stack. Truncating the stack does
  not clear list selections or loaded data.
- The user can press `Enter` to drill back down into the previously-selected
  conversation/agent session; the selection indices persist.

### DD-3: Ctrl+T at Projects level is a silent no-op

On the Projects panel, `nav_level()` is `NavLevel::Projects`, so
`project_id()` returns `None`. Ctrl+T maps to a handler that checks for
`project_id`, finds none, and returns `StateChange::none()` with no effects.
No error flash, no visual change. This matches the user's stated expectation:
"在 project 面板时 Ctrl+T 根本没法知道是哪一个 project 的 conversation".

### DD-4: Agent picker removal scope

The removal targets the **modal agent picker** only (`AgentPickerState`,
opened by `n` key, rendered as a centered "New Thread" overlay). The
**inline `@mention` agent picker** in the input bar
(`InputAgentPickerState`, `InputPicker::Agent`, `sync_room_agent_picker`,
`agent_picker_status_label`) is a completely separate feature driven by
`InputAction` variants and must not be touched.

Three `GlobalAction` variants — `SelectPrevious`, `SelectNext`,
`SelectIndex(usize)` — are confirmed picker-exclusive (grep proves the only
producer is `agent_picker_key_to_mapping`, the only consumers are picker-gated
arms in `update/global.rs`). They are removed entirely. The unrelated
`NavAction::SelectNext/SelectPrev` and
`InputAction::SelectPreviousPickerItem/SelectNextPickerItem` remain.

`Effect::StartAgentAt(usize)` is also picker-exclusive (only produced by
picker `Enter` and `SelectIndex` arms). It and its `App::start_agent_at`
method are removed. Starting agents continues to work via the
`CreateConversationAndStartAgent` and `StartAgentInConversation` effects,
which are triggered by the input bar submit flow (`update/nav.rs:294-325`).

## Implementation

### Part 1: Add Ctrl+P / Ctrl+T navigation shortcuts

#### 1.1 `crates/minos-tui/src/nav.rs`

Add two variants to `NavAction` (after `SubmitConversationInput`):

```rust
JumpToProjects,
JumpToConversations,
```

#### 1.2 `crates/minos-tui/src/app/event_mapping.rs`

In the global Ctrl block (`:28-38`), after the `Ctrl+D` arm, add:

```rust
KeyCode::Char('p') => {
    return KeyMapping::action(Action::Nav(NavAction::JumpToProjects));
}
KeyCode::Char('t') => {
    return KeyMapping::action(Action::Nav(NavAction::JumpToConversations));
}
```

No input-focus guard (per DD-1). Delete-confirm remains earlier in dispatch,
and project-create dialog still handles the resulting `NavAction` as a no-op,
so blocking overlays keep their own key context.

#### 1.3 `crates/minos-tui/src/update/nav.rs`

In `handle()`, add an early intercept **before** the per-level `match
ui.nav_level()` dispatch (`:14`), after the create-dialog guard (`:11-13`):

```rust
match action {
    NavAction::JumpToProjects => {
        ui.nav_stack = vec![NavLevel::Projects];
        return (StateChange::redraw(), vec![]);
    }
    NavAction::JumpToConversations => {
        if let Some(project_id) = ui.nav_level().project_id().map(str::to_owned) {
            ui.nav_stack = vec![
                NavLevel::Projects,
                NavLevel::Conversations { project_id },
            ];
            return (StateChange::redraw(), vec![]);
        }
        return (StateChange::none(), vec![]);
    }
    _ => {}
}
```

#### 1.4 `crates/minos-tui/src/ui/status_bar.rs`

Update the hint string at `:93-95`. Replace `n new-agent  ` prefix with
`^P projects  ^T conversations  `:

```text
^P projects  ^T conversations  @agent route  Tab focus  Enter inspect/send  Esc back/close-detail  wheel/PgUp/PgDn scroll  Ctrl+J newline  Alt+Enter multi  Ctrl+Alt+B cursor  Ctrl+C interrupt  Ctrl+Q quit
```

### Part 2: Remove the modal agent picker

#### 2.1 State and types

| Location | Change |
|---|---|
| `ui/agent_picker.rs` (entire file) | Delete file |
| `ui/mod.rs:1` | Remove `pub mod agent_picker;` |
| `ui/mod.rs:73` | Remove `pub agent_picker: Option<AgentPickerState>` field |
| `ui/mod.rs:102-104` | Remove `AgentPickerState` struct |
| `ui/mod.rs:130` | Remove `agent_picker: None,` from `UiState::new` |
| `ui/mod.rs:502-506` | Remove picker overlay render block |

#### 2.2 Actions

| Location | Change |
|---|---|
| `action.rs:24` | Remove `GlobalAction::OpenAgentPicker` |
| `action.rs:46` | Remove `GlobalAction::SelectPrevious` |
| `action.rs:47` | Remove `GlobalAction::SelectNext` |
| `action.rs:50` | Remove `GlobalAction::SelectIndex(usize)` |

#### 2.3 Effects

| Location | Change |
|---|---|
| `effect.rs:16` | Remove `Effect::StartAgentAt(usize)` |
| `app/event_loop.rs:122` | Remove `Effect::StartAgentAt(index) => self.start_agent_at(index).await,` arm |
| `app/thread_ops.rs:70-85` | Remove `start_agent_at` method (and the `self.ui.agent_picker = None;` line at `:77`) |

#### 2.4 Event mapping

| Location | Change |
|---|---|
| `event_mapping.rs:75-77` | Remove picker guard `if ui.agent_picker.is_some() { return agent_picker_key_to_mapping(ui, key); }` |
| `event_mapping.rs:108-110` | Remove `n` → `OpenAgentPicker` binding |
| `event_mapping.rs:148-164` | Remove `agent_picker_key_to_mapping` function |

#### 2.5 Update handlers

| Location | Change |
|---|---|
| `update/mod.rs:13` | Remove `AgentPickerState` from `use crate::ui::{...}` import |
| `update/mod.rs:335-339` | In `handle_escape`, remove the `if ui.agent_picker.is_some() { ui.agent_picker = None; ... }` block |
| `update/mod.rs:357-376` | Remove `open_agent_picker` function |
| `update/global.rs:16` | Remove `GlobalAction::OpenAgentPicker => super::open_agent_picker(ui),` arm |
| `update/global.rs:32-44` | Remove `GlobalAction::Enter if ui.agent_picker.is_some() => { ... }` arm (keep `:45` `GlobalAction::Enter => super::focus_from_enter(ui)`) |
| `update/global.rs:88-104` | Remove `GlobalAction::SelectPrevious => { ... }` arm |
| `update/global.rs:105-117` | Remove `GlobalAction::SelectNext => { ... }` arm |
| `update/global.rs:118-132` | Remove `GlobalAction::SelectIndex(index) if ui.agent_picker.is_some() => { ... }` arm |
| `update/global.rs:133` | Change `GlobalAction::RequestRedraw \| GlobalAction::SelectIndex(_) => StateChange::redraw()` to `GlobalAction::RequestRedraw => StateChange::redraw()`. The `SelectIndex(_)` catch-all is no longer reachable since the variant is removed from the enum in §2.2; it must be dropped from this arm. |

#### 2.6 Tests

| Location | Change |
|---|---|
| `app_tests/input_and_routing.rs:94-124` | Remove `open_agent_picker_defaults_to_current_thread_agent` test |

### Part 3: Documentation sync

Per AGENTS.md latest-only policy, update docs to reflect the new architecture:

| File | Change |
|---|---|
| `docs/architecture-tui.md` | Add Ctrl+P/T to the keybinding section; remove agent picker / `n`-key description; note the `SelectPrevious/Next/Index` and `StartAgentAt` removals |
| `docs/superpowers/specs/2026-06-17-tui-nav-ux-redesign.md:439` | Update or mark superseded — references the picker |
| `docs/superpowers/plans/2026-06-17-tui-p2-renderable-focus-tree.md` | Lines 26, 627, 689-691 reference `AgentPickerRenderable`; mark superseded by this design |
| `docs/superpowers/specs/2026-06-17-tui-three-phase-refactor-design.md:106` | Lists `OpenAgentPicker`; update |
| `docs/superpowers/plans/2026-06-16-tui-input-bar-overhaul.md:2043` | Status-bar hint string containing `n new-agent`; update |

## New Tests

Add to `app_tests/` (likely a new `nav_shortcuts.rs` or extend
`input_and_routing.rs`), using the existing `set_test_agent_detail_nav` and
`press_with_modifiers` helpers:

```rust
#[tokio::test]
async fn ctrl_p_from_agent_detail_jumps_to_projects() {
    // Set nav to AgentDetail{pid, cid, tid, agent}
    // Press Ctrl+P
    // Assert nav_stack == vec![NavLevel::Projects]
}

#[tokio::test]
async fn ctrl_p_from_conversations_jumps_to_projects() {
    // Set nav to Conversations{pid}
    // Press Ctrl+P
    // Assert nav_stack == vec![NavLevel::Projects]
}

#[tokio::test]
async fn ctrl_t_from_agent_detail_jumps_to_conversations() {
    // Set nav to AgentDetail{pid, cid, tid, agent}
    // Press Ctrl+T
    // Assert nav_stack == vec![Projects, Conversations{pid}]
}

#[tokio::test]
async fn ctrl_t_from_conversation_jumps_to_conversations() {
    // Set nav to Conversation{pid, cid}
    // Press Ctrl+T
    // Assert nav_stack == vec![Projects, Conversations{pid}]
}

#[tokio::test]
async fn ctrl_t_at_projects_is_noop() {
    // Set nav to Projects
    // Press Ctrl+T
    // Assert nav_stack unchanged == vec![Projects]
}

#[tokio::test]
async fn ctrl_t_preserves_conversation_selection() {
    // Set nav to Conversation, with selected_conversation = Some(2)
    // Press Ctrl+T to jump to Conversations
    // Assert selected_conversation still == Some(2)
}
```

## Verification Plan

1. `cargo build -p minos-tui` — confirms no compile errors after removal
2. `cargo test -p minos-tui` — run all existing + new unit tests
3. `cargo clippy -p minos-tui -- -D warnings` — no new warnings
4. Manual smoke test in the running TUI:
   - Navigate to AgentDetail, press Ctrl+P → lands on Projects
   - Navigate to AgentDetail, press Ctrl+T → lands on Conversations for same project
   - On Projects panel, press Ctrl+T → nothing happens
   - Press `n` on Projects panel → still opens create-project dialog (unchanged)
   - Press `n` on Conversations panel → no longer opens picker (dead key)
   - Verify `@agent` routing in input bar still starts agents

## Out of Scope

- Rebinding the `n` key on the Projects panel (it stays as `OpenCreateProject`)
- Adding Ctrl+P/T support to the macOS app or mobile clients (TUI-only)
- Adding a "jump to conversation" shortcut (would require conversation_id context, which is only available at Conversation/AgentDetail levels)
