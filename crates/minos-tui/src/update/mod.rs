//! Update layer skeleton: consume actions, mutate state, return effects.

mod agent;
mod conversation;
mod global;
mod nav;

use crate::action::{Action, EffectResult, InputAction, InputTarget};
use crate::agent_route::short_session_id;
use crate::effect::{Effect, StateChange};
use crate::focus::PaneId;
use crate::state::AppState;
use crate::ui::{DeleteConfirmState, UiState};

pub fn update(
    state: &mut AppState,
    ui: &mut UiState,
    action: Action,
) -> (StateChange, Vec<Effect>) {
    match action {
        Action::Global(action) => global::handle(state, ui, action),
        Action::Conversation(action) => conversation::handle(state, ui, action),
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
        return match target {
            // Conversation input always goes through the conversation-centric submit path.
            InputTarget::Conversation => {
                nav::handle(state, ui, crate::nav::NavAction::SubmitConversationInput)
            }
            InputTarget::Agent => agent::handle_submit(state, ui),
        };
    }

    let workspace = state.workspace.as_path();
    let (change, effects) = match target {
        InputTarget::Conversation => {
            let candidates = ui.conversation_agent_mention_candidates();
            crate::input::apply_input_action(
                &mut ui.inputs.conversation,
                action,
                target,
                Some(workspace),
                candidates.as_slice(),
            )
        }
        InputTarget::Agent => crate::input::apply_input_action(
            &mut ui.inputs.agent,
            action,
            target,
            Some(workspace),
            &[],
        ),
    };
    (change, effects)
}

fn handle_effect_result(
    state: &mut AppState,
    ui: &mut UiState,
    result: EffectResult,
) -> (StateChange, Vec<Effect>) {
    match result {
        EffectResult::AgentStarted {
            agent,
            session_id,
            cwd,
            text,
        } => (
            StateChange::none(),
            vec![Effect::AgentStartedForPrompt {
                agent,
                session_id,
                cwd,
                text,
            }],
        ),
        EffectResult::SendFailed { session_id, error } => {
            ui.set_error(format!(
                "Failed to send message to {}: {error}",
                short_session_id(&session_id)
            ));
            (StateChange::redraw(), vec![])
        }
        EffectResult::ProjectCreated(project) => {
            let project_id = project.project_id.clone();
            ui.projects.items.push(project);
            ui.nav.stack = vec![
                crate::nav::NavLevel::Projects,
                crate::nav::NavLevel::Conversations {
                    project_id: project_id.clone(),
                },
            ];
            ui.projects
                .select(Some(ui.projects.items.len().saturating_sub(1)));
            (
                StateChange::redraw(),
                vec![Effect::LoadConversations { project_id }],
            )
        }
        EffectResult::ConversationsLoaded {
            project_id,
            conversations,
        } => {
            ui.conversations.replace_items(conversations);
            ui.conversation.clear_messages();
            ui.conversation.reset_scroll();
            ui.conversation.agent_sessions.clear();
            ui.nav.stack = vec![
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
            for session in &sessions {
                state
                    .session_conversations
                    .insert(session.session_id.clone(), conversation_id.clone());
            }
            ui.conversations.select(
                ui.conversations
                    .items
                    .iter()
                    .position(|conversation| conversation.conversation_id == conversation_id),
            );
            ui.conversation.set_messages(messages);
            ui.conversation.reset_scroll();
            ui.conversation.agent_sessions.items = sessions;
            ui.conversation.agent_sessions.select(
                if ui.conversation.agent_sessions.items.is_empty() {
                    None
                } else {
                    Some(0)
                },
            );
            ui.nav.stack = vec![
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
            session_id,
            cwd,
            text,
        } => {
            state
                .session_conversations
                .insert(session_id.clone(), conversation_id.clone());
            let inserted_session = if ui
                .conversation
                .agent_sessions
                .items
                .iter()
                .any(|s| s.session_id == session_id)
            {
                false
            } else {
                let first_line = text.lines().next().unwrap_or("").trim();
                let title = if first_line.is_empty() {
                    None
                } else {
                    Some(first_line.chars().take(80).collect::<String>())
                };
                ui.conversation
                    .agent_sessions
                    .items
                    .push(crate::backend::SessionSummaryEntry {
                        session_id: session_id.clone(),
                        agent,
                        title,
                        first_ts_ms: 0,
                        last_ts_ms: 0,
                        message_count: 0,
                        ended_at_ms: None,
                        parent_session_id: None,
                        state: minos_agent_runtime::SessionState::Starting,
                        needs_continue: false,
                    });
                true
            };
            ui.conversation
                .agent_sessions
                .select(ui.flat_session_index_for_thread(&session_id));
            if inserted_session {
                if let Some(conversation) = ui
                    .conversations
                    .items
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
                    session_id,
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
    if index >= ui.session_panel.list.items.len() {
        return StateChange::none();
    }
    ui.session_panel.list.select(Some(index));
    StateChange::redraw()
}

pub(super) fn sync_conversation_agent_picker(ui: &mut UiState) {
    let candidates = ui.conversation_agent_mention_candidates();
    ui.inputs
        .conversation
        .sync_agent_picker(candidates.as_slice(), ui.focus.is(PaneId::Input));
}

pub(super) fn group_user_text_for_thread(
    ui: &UiState,
    session_id: &str,
    text: &str,
) -> Option<String> {
    let agent = ui
        .conversation
        .agent_sessions
        .items
        .iter()
        .find(|session| session.session_id == session_id)
        .map(|session| session.agent)
        .or_else(|| {
            ui.session_panel
                .list
                .items
                .iter()
                .find(|thread| thread.session_id == session_id)
                .map(|thread| thread.agent)
        })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "@{}#{} {trimmed}",
        agent.bin_name(),
        short_session_id(session_id)
    ))
}

pub(super) fn session_runtime_state<'a>(
    ui: &'a UiState,
    session_id: &str,
) -> Option<&'a minos_agent_runtime::SessionState> {
    ui.conversation
        .agent_sessions
        .items
        .iter()
        .find(|session| session.session_id == session_id)
        .map(|session| &session.state)
        .or_else(|| {
            ui.session_panel
                .list
                .items
                .iter()
                .find(|thread| thread.session_id == session_id)
                .map(|thread| &thread.state)
        })
}

pub(super) fn push_pending_conversation_user_message(
    ui: &mut UiState,
    conversation_id: &str,
    body: &str,
) {
    let message_seq = ui
        .conversation
        .messages
        .last()
        .map(|message| message.message_seq.saturating_add(1))
        .unwrap_or(1);
    ui.conversation
        .messages
        .push(crate::backend::ConversationMessageEntry {
            message_seq,
            message_id: format!("pending-{conversation_id}-{message_seq}"),
            conversation_id: conversation_id.to_owned(),
            session_id: None,
            created_at_ms: 0,
            sender_role: "user".to_owned(),
            agent: None,
            body: body.to_owned(),
            reply_to_message_id: None,
            delegation_id: None,
            mentions: Vec::new(),
        });
    ui.conversation.auto_scroll = true;
    if let Some(conversation) = ui
        .conversations
        .items
        .iter_mut()
        .find(|conversation| conversation.conversation_id == conversation_id)
    {
        conversation.message_count = conversation.message_count.saturating_add(1);
        conversation.last_message_preview = Some(body.chars().take(120).collect());
    }
}

pub(super) fn cycle_focus(ui: &mut UiState) -> StateChange {
    ui.focus.cycle_next();
    sync_conversation_agent_picker(ui);
    StateChange::redraw()
}

pub(super) fn cycle_focus_prev(ui: &mut UiState) -> StateChange {
    ui.focus.cycle_prev();
    sync_conversation_agent_picker(ui);
    StateChange::redraw()
}

pub(super) fn handle_escape(ui: &mut UiState) -> StateChange {
    if ui.inputs.conversation.has_agent_picker() {
        ui.inputs.conversation.clear_agent_picker();
        return StateChange::redraw();
    }

    let fallback_focus = PaneId::MainList;

    if !ui.focus.is(fallback_focus) {
        ui.focus.focus(fallback_focus);
        sync_conversation_agent_picker(ui);
        return StateChange::redraw();
    }

    StateChange::none()
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
            if ui.conversation.agent_sessions.selected.is_none() {
                return StateChange::none();
            }
            ui.focus.focus(PaneId::MainChat);
            StateChange::redraw()
        }
        PaneId::ApprovalOverlay => StateChange::none(),
        PaneId::Input => StateChange::none(),
    }
}

pub(super) fn request_delete_current_thread(ui: &mut UiState) -> StateChange {
    if ui.current_thread_is_subagent() {
        ui.set_error("Subagent transcripts are read-only.".into());
        return StateChange::redraw();
    }
    let Some(session_id) = ui.current_session_id().map(str::to_owned) else {
        return StateChange::none();
    };
    let Some(selected) = ui
        .session_panel
        .list
        .items
        .iter()
        .position(|thread| thread.session_id == session_id)
    else {
        return StateChange::none();
    };
    let Some(thread) = ui.session_panel.list.items.get(selected) else {
        return StateChange::none();
    };

    ui.overlays.delete_confirm = Some(DeleteConfirmState {
        session_id: thread.session_id.clone(),
        agent: thread.agent,
        workspace: thread.workspace.clone(),
        selected_index: selected,
    });
    StateChange::redraw()
}

pub(super) fn scroll_conversation(
    ui: &mut UiState,
    direction: crate::action::ScrollDirection,
    lines: u16,
) -> StateChange {
    match direction {
        crate::action::ScrollDirection::Up => {
            ui.conversation.auto_scroll = false;
            ui.conversation.scroll_offset = ui
                .conversation
                .scroll_offset
                .saturating_sub(u32::from(lines));
        }
        crate::action::ScrollDirection::Down => {
            ui.conversation.auto_scroll = false;
            ui.conversation.scroll_offset = ui
                .conversation
                .scroll_offset
                .saturating_add(u32::from(lines))
                .min(ui.conversation.max_scroll);
        }
        crate::action::ScrollDirection::Top => {
            ui.conversation.auto_scroll = false;
            ui.conversation.scroll_offset = 0;
        }
        crate::action::ScrollDirection::Bottom => {
            ui.conversation.auto_scroll = true;
            ui.conversation.scroll_offset = ui.conversation.max_scroll;
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
