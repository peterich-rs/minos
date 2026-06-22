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
        GlobalAction::OpenAgentPicker => super::open_agent_picker(ui),
        GlobalAction::Scroll(target, direction, lines) => match target {
            ScrollTarget::RoomList | ScrollTarget::AgentList => StateChange::none(),
            ScrollTarget::GroupChat => scroll_conversation_or_group_chat(ui, direction, lines),
            ScrollTarget::AgentChat => super::scroll_current_chat(ui, direction, lines),
            ScrollTarget::ActivePane => match ui.focus.current() {
                PaneId::MainChat
                    if matches!(ui.nav_level(), crate::nav::NavLevel::AgentDetail { .. }) =>
                {
                    super::scroll_current_chat(ui, direction, lines)
                }
                PaneId::MainChat => scroll_conversation_or_group_chat(ui, direction, lines),
                _ => StateChange::none(),
            },
        },
        GlobalAction::Escape => super::handle_escape(ui),
        GlobalAction::Enter if ui.agent_picker.is_some() => {
            let len = ui.status.agents.len();
            if len == 0 {
                ui.agent_picker = None;
                ui.set_error("No agent detection results available for picker".into());
                StateChange::redraw()
            } else {
                let Some(index) = ui.agent_picker.as_ref().map(|picker| picker.selected) else {
                    return (StateChange::none(), vec![]);
                };
                return (StateChange::none(), vec![Effect::StartAgentAt(index)]);
            }
        }
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
        GlobalAction::MouseScroll { target, direction } => {
            handle_mouse_scroll(ui, target, direction)
        }
        GlobalAction::ConfirmDelete => {
            return (StateChange::none(), vec![Effect::ConfirmDeleteThread]);
        }
        GlobalAction::CancelDelete => {
            ui.delete_confirm = None;
            StateChange::redraw()
        }
        GlobalAction::Quit => return (StateChange::none(), vec![Effect::Quit]),
        GlobalAction::InterruptOrQuit => {
            return (StateChange::none(), vec![Effect::InterruptOrQuit]);
        }
        GlobalAction::Tick => return (StateChange::none(), vec![Effect::HandleTick]),
        GlobalAction::McpToolCall(event) => {
            return (StateChange::none(), vec![Effect::HandleMcpToolCall(event)]);
        }
        GlobalAction::SelectPrevious => {
            let len = ui.status.agents.len();
            if len == 0 {
                ui.agent_picker = None;
                ui.set_error("No agent detection results available for picker".into());
                StateChange::redraw()
            } else if let Some(picker) = ui.agent_picker.as_mut() {
                picker.selected = if picker.selected == 0 {
                    len - 1
                } else {
                    picker.selected - 1
                };
                StateChange::redraw()
            } else {
                StateChange::none()
            }
        }
        GlobalAction::SelectNext => {
            let len = ui.status.agents.len();
            if len == 0 {
                ui.agent_picker = None;
                ui.set_error("No agent detection results available for picker".into());
                StateChange::redraw()
            } else if let Some(picker) = ui.agent_picker.as_mut() {
                picker.selected = (picker.selected + 1) % len;
                StateChange::redraw()
            } else {
                StateChange::none()
            }
        }
        GlobalAction::SelectIndex(index) if ui.agent_picker.is_some() => {
            let len = ui.status.agents.len();
            if len == 0 {
                ui.agent_picker = None;
                ui.set_error("No agent detection results available for picker".into());
                StateChange::redraw()
            } else if index < len {
                if let Some(picker) = ui.agent_picker.as_mut() {
                    picker.selected = index;
                }
                return (StateChange::none(), vec![Effect::StartAgentAt(index)]);
            } else {
                StateChange::none()
            }
        }
        GlobalAction::RequestRedraw | GlobalAction::SelectIndex(_) => StateChange::redraw(),
    };
    (change, vec![])
}

fn handle_mouse_click(ui: &mut UiState, target: ClickTarget, x: u16, y: u16) -> StateChange {
    match target {
        ClickTarget::RoomList => {
            ui.focus.focus(PaneId::MainList);
            match ui.nav_level() {
                crate::nav::NavLevel::Projects => {
                    if let Some(index) = state::clicked_thread_index(
                        ui.panel_areas.room_list,
                        &ui.project_list_state,
                        y,
                        ui.projects.len(),
                    ) {
                        ui.selected_project = Some(index);
                        ui.project_list_state.select(Some(index));
                    }
                }
                crate::nav::NavLevel::Conversations { .. } => {
                    if let Some(index) = state::clicked_thread_index(
                        ui.panel_areas.room_list,
                        &ui.conversation_list_state,
                        y,
                        ui.conversations.len(),
                    ) {
                        ui.selected_conversation = Some(index);
                        ui.conversation_list_state.select(Some(index));
                    }
                }
                _ => {}
            }
            StateChange::redraw()
        }
        ClickTarget::GroupChat => {
            ui.focus.focus(PaneId::MainChat);
            StateChange::redraw()
        }
        ClickTarget::AgentList => {
            ui.focus.focus(PaneId::Sidebar);
            let len = if ui.conversation_agent_sessions.is_empty() {
                ui.threads.len()
            } else {
                ui.flat_agent_session_count()
            };
            if let Some(index) =
                state::clicked_thread_index(ui.panel_areas.agent_list, &ui.agent_list_state, y, len)
            {
                if ui.conversation_agent_sessions.is_empty() {
                    super::select_thread(ui, index);
                } else {
                    ui.selected_agent_session = Some(index);
                    ui.agent_list_state.select(Some(index));
                }
            }
            StateChange::redraw()
        }
        ClickTarget::AgentChat => {
            ui.focus.focus(PaneId::MainChat);
            super::sync_room_agent_picker(ui);
            state::begin_chat_selection(ui, x, y);
            StateChange::redraw()
        }
        ClickTarget::RoomInput => {
            ui.focus.focus(PaneId::Input);
            handle_input_click(ui, 0, x, y);
            super::sync_room_agent_picker(ui);
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
) -> StateChange {
    match target {
        ScrollTarget::RoomList => {
            ui.focus.focus(PaneId::MainList);
            match ui.nav_level() {
                crate::nav::NavLevel::Projects => {
                    scroll_selection(
                        &mut ui.selected_project,
                        &mut ui.project_list_state,
                        ui.projects.len(),
                        direction,
                    );
                }
                crate::nav::NavLevel::Conversations { .. } => {
                    scroll_selection(
                        &mut ui.selected_conversation,
                        &mut ui.conversation_list_state,
                        ui.conversations.len(),
                        direction,
                    );
                }
                _ => {}
            }
            StateChange::redraw()
        }
        ScrollTarget::GroupChat => {
            ui.focus.focus(PaneId::MainChat);
            scroll_conversation_or_group_chat(ui, direction, 3)
        }
        ScrollTarget::AgentList => {
            ui.focus.focus(PaneId::Sidebar);
            if ui.conversation_agent_sessions.is_empty() {
                match direction {
                    ScrollDirection::Up => {
                        if let Some(selected) = ui.selected_thread {
                            super::select_thread(ui, selected.saturating_sub(1));
                        }
                    }
                    ScrollDirection::Down => {
                        if let Some(selected) = ui.selected_thread {
                            let last = ui.threads.len().saturating_sub(1);
                            super::select_thread(ui, (selected + 1).min(last));
                        }
                    }
                    ScrollDirection::Top | ScrollDirection::Bottom => {}
                }
            } else {
                match direction {
                    ScrollDirection::Up => {
                        if let Some(selected) = ui.selected_agent_session {
                            ui.selected_agent_session = Some(selected.saturating_sub(1));
                            ui.agent_list_state.select(ui.selected_agent_session);
                        }
                    }
                    ScrollDirection::Down => {
                        if let Some(selected) = ui.selected_agent_session {
                            let last = ui.flat_agent_session_count().saturating_sub(1);
                            ui.selected_agent_session = Some((selected + 1).min(last));
                            ui.agent_list_state.select(ui.selected_agent_session);
                        }
                    }
                    ScrollDirection::Top | ScrollDirection::Bottom => {}
                }
            }
            StateChange::redraw()
        }
        ScrollTarget::AgentChat => {
            ui.focus.focus(PaneId::MainChat);
            super::sync_room_agent_picker(ui);
            super::scroll_current_chat(ui, direction, 3)
        }
        ScrollTarget::ActivePane => StateChange::none(),
    }
}

fn scroll_conversation_or_group_chat(
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
        super::scroll_group_chat(ui, direction, lines)
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
    let metrics = ui.input_metrics[input_index];
    if !state::rect_contains(metrics.editor_area, column, row) {
        return;
    }
    let visual_row = usize::from(row.saturating_sub(metrics.editor_area.y)) + metrics.start_row;
    let visual_col = usize::from(column.saturating_sub(metrics.editor_area.x));
    let input = match input_index {
        0 => &mut ui.room_input,
        _ => &mut ui.agent_input,
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
