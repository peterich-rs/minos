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
            && matches!(
                ui.nav_level(),
                crate::nav::NavLevel::Conversations { .. }
                    | crate::nav::NavLevel::Conversation { .. }
                    | crate::nav::NavLevel::AgentDetail { .. }
            )
        {
            return nav::handle(state, ui, crate::nav::NavAction::SubmitConversationInput);
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
            ui.nav_stack = vec![
                crate::nav::NavLevel::Projects,
                crate::nav::NavLevel::Conversations {
                    project_id: project_id.clone(),
                },
            ];
            ui.selected_project = Some(ui.projects.len().saturating_sub(1));
            ui.project_list_state.select(ui.selected_project);
            (
                StateChange::redraw(),
                vec![Effect::LoadConversations { project_id }],
            )
        }
        EffectResult::ConversationsLoaded {
            project_id,
            conversations,
        } => {
            ui.conversations = conversations;
            ui.selected_conversation = if ui.conversations.is_empty() {
                None
            } else {
                Some(0)
            };
            ui.conversation_list_state.select(ui.selected_conversation);
            ui.conversation_messages.clear();
            ui.conversation_scroll_offset = 0;
            ui.conversation_auto_scroll = true;
            ui.conversation_max_scroll = 0;
            ui.conversation_agent_sessions.clear();
            ui.selected_agent_session = None;
            ui.agent_list_state.select(None);
            ui.nav_stack = vec![
                crate::nav::NavLevel::Projects,
                crate::nav::NavLevel::Conversations { project_id },
            ];
            (StateChange::redraw(), vec![])
        }
        EffectResult::ConversationOpened {
            project_id,
            conversation_id,
            messages,
            sessions,
        } => {
            ui.selected_conversation = ui
                .conversations
                .iter()
                .position(|conversation| conversation.conversation_id == conversation_id);
            ui.conversation_list_state.select(ui.selected_conversation);
            ui.conversation_messages = messages;
            ui.conversation_auto_scroll = true;
            ui.conversation_scroll_offset = 0;
            ui.conversation_agent_sessions = sessions;
            ui.selected_agent_session = if ui.conversation_agent_sessions.is_empty() {
                None
            } else {
                Some(0)
            };
            ui.agent_list_state.select(ui.selected_agent_session);
            ui.nav_stack = vec![
                crate::nav::NavLevel::Projects,
                crate::nav::NavLevel::Conversations {
                    project_id: project_id.clone(),
                },
                crate::nav::NavLevel::Conversation {
                    project_id,
                    conversation_id,
                },
            ];
            (StateChange::redraw(), vec![])
        }
        EffectResult::ConversationAgentStarted {
            conversation_id,
            agent,
            thread_id,
            cwd,
            text,
        } => {
            let inserted_session = if !ui
                .conversation_agent_sessions
                .iter()
                .any(|s| s.thread_id == thread_id)
            {
                let first_line = text.lines().next().unwrap_or("").trim();
                let title = if first_line.is_empty() {
                    None
                } else {
                    Some(first_line.chars().take(80).collect::<String>())
                };
                ui.conversation_agent_sessions
                    .push(crate::backend::ThreadSummaryEntry {
                        thread_id: thread_id.clone(),
                        agent,
                        title,
                        first_ts_ms: 0,
                        last_ts_ms: 0,
                        message_count: 0,
                        ended_at_ms: None,
                    });
                true
            } else {
                false
            };
            ui.selected_agent_session = ui
                .conversation_agent_sessions
                .iter()
                .position(|session| session.thread_id == thread_id);
            ui.agent_list_state.select(ui.selected_agent_session);
            if inserted_session {
                if let Some(conversation) = ui
                    .conversations
                    .iter_mut()
                    .find(|conversation| conversation.conversation_id == conversation_id)
                {
                    conversation.agent_session_count =
                        conversation.agent_session_count.saturating_add(1);
                    if !conversation.participating_agents.contains(&agent) {
                        conversation.participating_agents.push(agent);
                    }
                }
            }
            let effects = if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![Effect::AgentStartedForPrompt {
                    agent,
                    thread_id,
                    cwd,
                    text,
                }]
            };
            (StateChange::redraw(), effects)
        }
        EffectResult::ProjectFailed(error) => {
            ui.set_error(format!("Project operation failed: {error}"));
            (StateChange::redraw(), vec![])
        }
    }
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
        .sync_agent_picker(candidates.as_slice(), ui.focus.is(PaneId::Input));
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

pub(super) fn push_pending_conversation_user_message(
    ui: &mut UiState,
    conversation_id: &str,
    body: &str,
) {
    let message_seq = ui
        .conversation_messages
        .last()
        .map(|message| message.message_seq.saturating_add(1))
        .unwrap_or(1);
    ui.conversation_messages
        .push(crate::backend::ConversationMessageEntry {
            message_seq,
            message_id: format!("pending-{conversation_id}-{message_seq}"),
            conversation_id: conversation_id.to_owned(),
            thread_id: None,
            created_at_ms: 0,
            sender_role: "user".to_owned(),
            agent: None,
            body: body.to_owned(),
        });
    ui.conversation_auto_scroll = true;
    if let Some(conversation) = ui
        .conversations
        .iter_mut()
        .find(|conversation| conversation.conversation_id == conversation_id)
    {
        conversation.message_count = conversation.message_count.saturating_add(1);
        conversation.last_message_preview = Some(body.chars().take(120).collect());
    }
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

    let fallback_focus = PaneId::MainList;

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
        PaneId::MainList => {
            ui.focus.focus(PaneId::MainChat);
            StateChange::redraw()
        }
        PaneId::MainChat => {
            ui.focus.focus(PaneId::Input);
            StateChange::redraw()
        }
        PaneId::Sidebar => {
            if ui.selected_agent_session.is_none() {
                return StateChange::none();
            }
            ui.focus.focus(PaneId::MainChat);
            StateChange::redraw()
        }
        PaneId::Input => StateChange::none(),
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

pub(super) fn scroll_conversation(
    ui: &mut UiState,
    direction: crate::action::ScrollDirection,
    lines: u16,
) -> StateChange {
    match direction {
        crate::action::ScrollDirection::Up => {
            ui.conversation_auto_scroll = false;
            ui.conversation_scroll_offset = ui.conversation_scroll_offset.saturating_sub(lines);
        }
        crate::action::ScrollDirection::Down => {
            ui.conversation_auto_scroll = false;
            ui.conversation_scroll_offset = ui
                .conversation_scroll_offset
                .saturating_add(lines)
                .min(ui.conversation_max_scroll);
        }
        crate::action::ScrollDirection::Top => {
            ui.conversation_auto_scroll = false;
            ui.conversation_scroll_offset = 0;
        }
        crate::action::ScrollDirection::Bottom => {
            ui.conversation_auto_scroll = true;
            ui.conversation_scroll_offset = ui.conversation_max_scroll;
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
