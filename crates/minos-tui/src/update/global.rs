use crate::action::{ClickTarget, GlobalAction, ScrollDirection, ScrollTarget};
use crate::effect::{Effect, StateChange};
use crate::focus::PaneId;
use crate::state;
use crate::state::AppState;
use crate::ui::UiState;

pub fn handle(
    _state: &mut AppState,
    ui: &mut UiState,
    action: GlobalAction,
) -> (StateChange, Vec<Effect>) {
    let change = match action {
        GlobalAction::CycleFocus => super::cycle_focus(ui),
        GlobalAction::CycleFocusPrev => super::cycle_focus_prev(ui),
        GlobalAction::Scroll(target, direction, lines) => match target {
            ScrollTarget::MainList | ScrollTarget::AgentList => StateChange::none(),
            ScrollTarget::ConversationChat => scroll_conversation_if_visible(ui, direction, lines),
            ScrollTarget::AgentChat => super::scroll_current_chat(ui, direction, lines),
            ScrollTarget::ActivePane => match ui.focus.current() {
                PaneId::MainChat
                    if matches!(ui.nav_level(), crate::nav::NavLevel::AgentDetail { .. }) =>
                {
                    super::scroll_current_chat(ui, direction, lines)
                }
                PaneId::MainChat => scroll_conversation_if_visible(ui, direction, lines),
                _ => StateChange::none(),
            },
        },
        GlobalAction::Escape => super::handle_escape(ui),
        GlobalAction::Enter => super::focus_from_enter(ui),
        GlobalAction::MouseClick { target, x, y } => handle_mouse_click(ui, target, x, y),
        GlobalAction::MouseDrag { x, y, release } => {
            let result = state::handle_chat_selection_mouse(
                ui,
                crossterm::event::MouseEvent {
                    kind: if release {
                        crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left)
                    } else {
                        crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left)
                    },
                    column: x,
                    row: y,
                    modifiers: crossterm::event::KeyModifiers::empty(),
                },
            );
            if let Some(text) = result.selected_text {
                return (StateChange::redraw(), vec![Effect::CopyToClipboard(text)]);
            }
            if result.changed {
                StateChange::redraw()
            } else {
                StateChange::none()
            }
        }
        GlobalAction::MouseScroll {
            target,
            direction,
            lines,
        } => handle_mouse_scroll(ui, target, direction, lines.max(1)),
        GlobalAction::ConfirmDelete => {
            return (StateChange::none(), vec![Effect::ConfirmDeleteThread]);
        }
        GlobalAction::CancelDelete => {
            ui.overlays.delete_confirm = None;
            StateChange::redraw()
        }
        GlobalAction::Quit => return (StateChange::none(), vec![Effect::Quit]),
        GlobalAction::InterruptOrQuit => {
            return (StateChange::none(), vec![Effect::InterruptOrQuit]);
        }
        GlobalAction::Tick => return (StateChange::none(), vec![Effect::HandleTick]),
        GlobalAction::RequestRedraw => StateChange::redraw(),
    };
    (change, vec![])
}

fn handle_mouse_click(ui: &mut UiState, target: ClickTarget, x: u16, y: u16) -> StateChange {
    match target {
        ClickTarget::MainList => {
            ui.focus.focus(PaneId::MainList);
            match ui.nav_level() {
                crate::nav::NavLevel::Projects => {
                    if let Some(index) = state::clicked_thread_index(
                        ui.panel_areas.main_list,
                        &ui.projects.list_state,
                        y,
                        ui.projects.items.len(),
                    ) {
                        ui.projects.select(Some(index));
                    }
                }
                crate::nav::NavLevel::Conversations { .. } => {
                    if let Some(index) = state::clicked_thread_index(
                        ui.panel_areas.main_list,
                        &ui.conversations.list_state,
                        y,
                        ui.conversations.items.len(),
                    ) {
                        ui.conversations.select(Some(index));
                    }
                }
                _ => {}
            }
            StateChange::redraw()
        }
        ClickTarget::ConversationChat => {
            ui.focus.focus(PaneId::MainChat);
            state::begin_conversation_selection(ui, x, y);
            StateChange::redraw()
        }
        ClickTarget::AgentList => {
            ui.focus.focus(PaneId::Sidebar);
            let len = if ui.conversation.agent_sessions.items.is_empty() {
                ui.thread_panel.list.items.len()
            } else {
                ui.flat_agent_session_count()
            };
            let list_state = if ui.conversation.agent_sessions.items.is_empty() {
                &ui.thread_panel.list.list_state
            } else {
                &ui.conversation.agent_sessions.list_state
            };
            if let Some(index) =
                state::clicked_thread_index(ui.panel_areas.agent_list, list_state, y, len)
            {
                if ui.conversation.agent_sessions.items.is_empty() {
                    super::select_thread(ui, index);
                } else {
                    ui.conversation.agent_sessions.select(Some(index));
                }
            }
            StateChange::redraw()
        }
        ClickTarget::AgentChat => {
            ui.focus.focus(PaneId::MainChat);
            super::sync_conversation_agent_picker(ui);
            // Header click on tool/thinking folds; otherwise start text selection.
            if !state::try_toggle_fold_at_click(ui, x, y) {
                state::begin_chat_selection(ui, x, y);
            }
            StateChange::redraw()
        }
        ClickTarget::ConversationInput => {
            ui.focus.focus(PaneId::Input);
            handle_input_click(ui, 0, x, y);
            super::sync_conversation_agent_picker(ui);
            StateChange::redraw()
        }
        ClickTarget::AgentInput => {
            ui.focus.focus(PaneId::Input);
            handle_input_click(ui, 1, x, y);
            StateChange::redraw()
        }
    }
}

fn handle_mouse_scroll(
    ui: &mut UiState,
    target: ScrollTarget,
    direction: ScrollDirection,
    lines: u16,
) -> StateChange {
    match target {
        ScrollTarget::MainList => {
            ui.focus.focus(PaneId::MainList);
            // List selection still steps one row per wheel event; bursts step once
            // per coalesced event (already collapsed in main).
            match ui.nav_level() {
                crate::nav::NavLevel::Projects => {
                    scroll_selection(
                        &mut ui.projects.selected,
                        &mut ui.projects.list_state,
                        ui.projects.items.len(),
                        direction,
                    );
                }
                crate::nav::NavLevel::Conversations { .. } => {
                    scroll_selection(
                        &mut ui.conversations.selected,
                        &mut ui.conversations.list_state,
                        ui.conversations.items.len(),
                        direction,
                    );
                }
                _ => {}
            }
            StateChange::redraw()
        }
        ScrollTarget::ConversationChat => {
            ui.focus.focus(PaneId::MainChat);
            scroll_conversation_if_visible(ui, direction, lines)
        }
        ScrollTarget::AgentList => {
            ui.focus.focus(PaneId::Sidebar);
            if ui.conversation.agent_sessions.items.is_empty() {
                match direction {
                    ScrollDirection::Up => {
                        if let Some(selected) = ui.thread_panel.list.selected {
                            super::select_thread(ui, selected.saturating_sub(1));
                        }
                    }
                    ScrollDirection::Down => {
                        if let Some(selected) = ui.thread_panel.list.selected {
                            let last = ui.thread_panel.list.items.len().saturating_sub(1);
                            super::select_thread(ui, (selected + 1).min(last));
                        }
                    }
                    ScrollDirection::Top | ScrollDirection::Bottom => {}
                }
            } else {
                match direction {
                    ScrollDirection::Up => {
                        if let Some(selected) = ui.conversation.agent_sessions.selected {
                            ui.conversation
                                .agent_sessions
                                .select(Some(selected.saturating_sub(1)));
                        }
                    }
                    ScrollDirection::Down => {
                        if let Some(selected) = ui.conversation.agent_sessions.selected {
                            let last = ui.flat_agent_session_count().saturating_sub(1);
                            ui.conversation
                                .agent_sessions
                                .select(Some((selected + 1).min(last)));
                        }
                    }
                    ScrollDirection::Top | ScrollDirection::Bottom => {}
                }
            }
            StateChange::redraw()
        }
        ScrollTarget::AgentChat => {
            ui.focus.focus(PaneId::MainChat);
            super::sync_conversation_agent_picker(ui);
            super::scroll_current_chat(ui, direction, lines)
        }
        ScrollTarget::ActivePane => StateChange::none(),
    }
}

fn scroll_conversation_if_visible(
    ui: &mut UiState,
    direction: ScrollDirection,
    lines: u16,
) -> StateChange {
    if matches!(
        ui.nav_level(),
        crate::nav::NavLevel::Conversation { .. } | crate::nav::NavLevel::AgentDetail { .. }
    ) {
        super::scroll_conversation(ui, direction, lines)
    } else {
        StateChange::none()
    }
}

fn scroll_selection(
    selected: &mut Option<usize>,
    list_state: &mut ratatui::widgets::ListState,
    len: usize,
    direction: ScrollDirection,
) {
    if len == 0 {
        *selected = None;
        list_state.select(None);
        return;
    }
    let current = selected.unwrap_or(0);
    let next = match direction {
        ScrollDirection::Up => current.saturating_sub(1),
        ScrollDirection::Down => (current + 1).min(len.saturating_sub(1)),
        ScrollDirection::Top => 0,
        ScrollDirection::Bottom => len.saturating_sub(1),
    };
    *selected = Some(next);
    list_state.select(Some(next));
}

fn handle_input_click(ui: &mut UiState, input_index: usize, column: u16, row: u16) {
    let metrics = ui.inputs.metrics[input_index];
    if !state::rect_contains(metrics.editor_area, column, row) {
        return;
    }
    let visual_row = usize::from(row.saturating_sub(metrics.editor_area.y)) + metrics.start_row;
    let visual_col = usize::from(column.saturating_sub(metrics.editor_area.x));
    let input = match input_index {
        0 => &mut ui.inputs.conversation,
        _ => &mut ui.inputs.agent,
    };
    let offset = crate::ui::input_bar::byte_offset_for_visual_position(
        &input.content,
        visual_row,
        visual_col,
        metrics.width,
    );
    input.cursor_pos = offset;
    input.preferred_column = None;
}
