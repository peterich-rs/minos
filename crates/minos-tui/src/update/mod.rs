//! Update layer skeleton: consume actions, mutate state, return effects.

mod agent;
mod global;
mod nav;
mod room;

use crate::action::{Action, EffectResult, InputAction, InputTarget};
use crate::agent_route::short_thread_id;
use crate::effect::{Effect, StateChange};
use crate::focus::PaneId;
use crate::state::AppState;
use crate::ui::{AgentPickerState, DeleteConfirmState, UiState};

pub fn update(
    state: &mut AppState,
    ui: &mut UiState,
    action: Action,
) -> (StateChange, Vec<Effect>) {
    match action {
        Action::Global(action) => global::handle(state, ui, action),
        Action::Room(action) => room::handle(state, ui, action),
        Action::Agent(action) => agent::handle(state, ui, action),
        Action::Input(target, action) => handle_input(state, ui, target, action),
        Action::EffectCompleted(result) => handle_effect_result(state, ui, result),
        Action::Nav(nav_action) => nav::handle(state, ui, nav_action),
    }
}

fn handle_input(
    state: &mut AppState,
    ui: &mut UiState,
    target: InputTarget,
    action: InputAction,
) -> (StateChange, Vec<Effect>) {
    if matches!(action, InputAction::Submit) {
        if matches!(target, InputTarget::Room)
            && matches!(ui.nav_level, crate::nav::NavLevel::Sessions { .. })
        {
            return nav::handle(state, ui, crate::nav::NavAction::SubmitSessionInput);
        }
        return match target {
            InputTarget::Room => room::handle_submit(state, ui),
            InputTarget::Agent => agent::handle_submit(state, ui),
        };
    }

    let workspace = state.workspace.as_path();
    let change = match target {
        InputTarget::Room => {
            let candidates = ui.room_agent_mention_candidates();
            crate::input::apply_input_action(
                &mut ui.room_input,
                action,
                Some(workspace),
                candidates.as_slice(),
            )
        }
        InputTarget::Agent => {
            crate::input::apply_input_action(&mut ui.agent_input, action, Some(workspace), &[])
        }
    };
    (change, vec![])
}

fn handle_effect_result(
    _state: &mut AppState,
    ui: &mut UiState,
    result: EffectResult,
) -> (StateChange, Vec<Effect>) {
    match result {
        EffectResult::AgentStarted {
            agent,
            thread_id,
            cwd,
            text,
        } => (
            StateChange::none(),
            vec![Effect::AgentStartedForPrompt {
                agent,
                thread_id,
                cwd,
                text,
            }],
        ),
        EffectResult::SendFailed { thread_id, error } => {
            ui.set_error(format!(
                "Failed to send message to {}: {error}",
                short_thread_id(&thread_id)
            ));
            (StateChange::redraw(), vec![])
        }
        EffectResult::IngestArrived(frame) => {
            (StateChange::none(), vec![Effect::HandleIngest(frame)])
        }
        EffectResult::ManagerEvent(event) => {
            (StateChange::none(), vec![Effect::HandleManagerEvent(event)])
        }
        EffectResult::ProjectCreated(project) => {
            let project_id = project.project_id.clone();
            ui.projects.push(project);
            ui.nav_level = crate::nav::NavLevel::Sessions {
                project_id: project_id.clone(),
            };
            ui.selected_project = Some(ui.projects.len().saturating_sub(1));
            ui.project_list_state.select(ui.selected_project);
            (
                StateChange::redraw(),
                vec![Effect::LoadProjectThreads { project_id }],
            )
        }
        EffectResult::ProjectThreadsLoaded { project_id, threads } => {
            ui.project_sessions = threads;
            ui.selected_thread = if ui.project_sessions.is_empty() {
                None
            } else {
                Some(0)
            };
            ui.room_list_state.select(ui.selected_thread);
            ui.nav_level = crate::nav::NavLevel::Sessions { project_id };
            (StateChange::redraw(), vec![])
        }
        EffectResult::ProjectSessionStarted {
            project_id,
            agent,
            thread_id,
            cwd,
            text,
        } => {
            if !ui.project_sessions.iter().any(|s| s.thread_id == thread_id) {
                let first_line = text.lines().next().unwrap_or("").trim();
                let title = if first_line.is_empty() {
                    None
                } else {
                    Some(first_line.chars().take(80).collect::<String>())
                };
                ui.project_sessions.push(crate::backend::ThreadSummaryEntry {
                    thread_id: thread_id.clone(),
                    agent,
                    title,
                    first_ts_ms: 0,
                    last_ts_ms: 0,
                    message_count: 0,
                    ended_at_ms: None,
                });
            }
            ui.selected_thread = ui
                .project_sessions
                .iter()
                .position(|session| session.thread_id == thread_id);
            ui.room_list_state.select(ui.selected_thread);
            ui.nav_level = crate::nav::NavLevel::Session {
                project_id,
                thread_id: thread_id.clone(),
            };
            (
                StateChange::redraw(),
                vec![Effect::AgentStartedForPrompt {
                    agent,
                    thread_id,
                    cwd,
                    text,
                }],
            )
        }
        EffectResult::ProjectSessionOpened { thread_id } => {
            let project_id = ui
                .nav_level
                .project_id()
                .map(|s| s.to_owned())
                .unwrap_or_default();
            if let Some(idx) = ui.threads.iter().position(|t| t.thread_id == thread_id) {
                ui.selected_thread = Some(idx);
            }
            ui.nav_level = crate::nav::NavLevel::Session {
                project_id,
                thread_id,
            };
            (StateChange::redraw(), vec![])
        }
        EffectResult::ProjectFailed(error) => {
            ui.set_error(format!("Project operation failed: {error}"));
            (StateChange::redraw(), vec![])
        }
    }
}

pub(super) fn select_room(ui: &mut UiState, index: usize) -> StateChange {
    if index >= ui.rooms.len() {
        return StateChange::none();
    }
    ui.selected_room = Some(index);
    ui.room_list_state.select(Some(index));
    StateChange::redraw()
}

pub(super) fn select_thread(ui: &mut UiState, index: usize) -> StateChange {
    if index >= ui.threads.len() {
        return StateChange::none();
    }
    ui.selected_thread = Some(index);
    ui.agent_list_state.select(Some(index));
    StateChange::redraw()
}

pub(super) fn sync_room_agent_picker(ui: &mut UiState) {
    let candidates = ui.room_agent_mention_candidates();
    ui.room_input
        .sync_agent_picker(candidates.as_slice(), ui.focus.is(PaneId::RoomInput));
}

pub(super) fn group_user_text_for_thread(
    ui: &UiState,
    thread_id: &str,
    text: &str,
) -> Option<String> {
    let thread = ui
        .threads
        .iter()
        .find(|thread| thread.thread_id == thread_id)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "@{}#{} {trimmed}",
        thread.agent.bin_name(),
        short_thread_id(&thread.thread_id)
    ))
}

pub(super) fn cycle_focus(ui: &mut UiState) -> StateChange {
    ui.focus.cycle_next();
    sync_room_agent_picker(ui);
    StateChange::redraw()
}

pub(super) fn cycle_focus_prev(ui: &mut UiState) -> StateChange {
    ui.focus.cycle_prev();
    sync_room_agent_picker(ui);
    StateChange::redraw()
}

pub(super) fn handle_escape(ui: &mut UiState) -> StateChange {
    if ui.agent_picker.is_some() {
        ui.agent_picker = None;
        return StateChange::redraw();
    }

    if ui.room_input.has_agent_picker() {
        ui.room_input.clear_agent_picker();
        return StateChange::redraw();
    }

    if ui.agent_detail_visible
        && (ui.focus.is(PaneId::AgentChat) || ui.focus.is(PaneId::AgentInput))
    {
        ui.agent_detail_visible = false;
        ui.focus.switch_layout(false);
        ui.focus.focus(PaneId::AgentList);
        sync_room_agent_picker(ui);
        return StateChange::redraw();
    }

    let fallback_focus = if ui.agent_detail_visible {
        PaneId::GroupChat
    } else {
        PaneId::RoomList
    };

    if !ui.focus.is(fallback_focus) {
        ui.focus.focus(fallback_focus);
        sync_room_agent_picker(ui);
        return StateChange::redraw();
    }

    StateChange::none()
}

pub(super) fn open_agent_picker(ui: &mut UiState) -> StateChange {
    let selected = ui
        .selected_thread
        .and_then(|index| ui.threads.get(index))
        .and_then(|thread| {
            ui.status
                .agents
                .iter()
                .position(|agent| agent.name == thread.agent)
        })
        .or_else(|| {
            ui.status
                .agents
                .iter()
                .position(|agent| matches!(agent.status, minos_domain::AgentStatus::Ok))
        })
        .unwrap_or(0);
    ui.agent_picker = Some(AgentPickerState { selected });
    StateChange::redraw()
}

pub(super) fn focus_from_enter(ui: &mut UiState) -> StateChange {
    match ui.focus.current() {
        PaneId::RoomList => {
            ui.focus.focus(PaneId::GroupChat);
            StateChange::redraw()
        }
        PaneId::GroupChat => {
            ui.focus.focus(PaneId::RoomInput);
            StateChange::redraw()
        }
        PaneId::AgentList => {
            if ui.selected_thread.is_none() {
                return StateChange::none();
            }
            ui.agent_detail_visible = true;
            ui.focus.switch_layout(true);
            ui.focus.focus(PaneId::AgentChat);
            StateChange::redraw()
        }
        PaneId::AgentChat => {
            ui.focus.focus(PaneId::AgentInput);
            StateChange::redraw()
        }
        PaneId::RoomInput | PaneId::AgentInput => StateChange::none(),
    }
}

pub(super) fn request_delete_current_thread(ui: &mut UiState) -> StateChange {
    let Some(selected) = ui.selected_thread else {
        return StateChange::none();
    };
    let Some(thread) = ui.threads.get(selected) else {
        return StateChange::none();
    };

    ui.delete_confirm = Some(DeleteConfirmState {
        thread_id: thread.thread_id.clone(),
        agent: thread.agent,
        workspace: thread.workspace.clone(),
        selected_index: selected,
    });
    StateChange::redraw()
}

pub(super) fn scroll_group_chat(
    ui: &mut UiState,
    direction: crate::action::ScrollDirection,
    lines: u16,
) -> StateChange {
    match direction {
        crate::action::ScrollDirection::Up => ui.group_chat.scroll_up(lines),
        crate::action::ScrollDirection::Down => ui.group_chat.scroll_down(lines),
        crate::action::ScrollDirection::Top => {
            ui.group_chat.auto_scroll = false;
            ui.group_chat.scroll_offset = 0;
        }
        crate::action::ScrollDirection::Bottom => {
            ui.group_chat.auto_scroll = true;
            ui.group_chat.scroll_offset = 0;
        }
    }
    StateChange::redraw()
}

pub(super) fn scroll_current_chat(
    ui: &mut UiState,
    direction: crate::action::ScrollDirection,
    lines: u16,
) -> StateChange {
    let Some(chat) = ui.current_chat_mut() else {
        return StateChange::none();
    };
    match direction {
        crate::action::ScrollDirection::Up => chat.scroll_up(lines),
        crate::action::ScrollDirection::Down => chat.scroll_down(lines),
        crate::action::ScrollDirection::Top => chat.scroll_to_top(),
        crate::action::ScrollDirection::Bottom => chat.scroll_to_bottom(),
    }
    StateChange::redraw()
}
