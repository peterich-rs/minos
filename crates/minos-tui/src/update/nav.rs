use crate::effect::{Effect, StateChange};
use crate::nav::{NavAction, NavLevel};
use crate::state::AppState;
use crate::ui::UiState;

pub fn handle(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    if ui.overlays.project_create.is_some() {
        return handle_create_dialog(ui, action);
    }
    match action {
        NavAction::JumpToProjects => {
            ui.nav.stack = vec![NavLevel::Projects];
            return (StateChange::redraw(), vec![]);
        }
        NavAction::JumpToConversations => {
            if let Some(project_id) = ui.nav_level().project_id().map(str::to_owned) {
                ui.nav.stack = vec![NavLevel::Projects, NavLevel::Conversations { project_id }];
                return (StateChange::redraw(), vec![]);
            }
            return (StateChange::none(), vec![]);
        }
        _ => {}
    }
    match ui.nav_level() {
        NavLevel::Projects => handle_projects_level(state, ui, action),
        NavLevel::Conversations { .. } => handle_conversations_level(state, ui, action),
        NavLevel::Conversation { .. } => handle_conversation_level(state, ui, action),
        NavLevel::AgentDetail { .. } => handle_agent_detail_level(state, ui, action),
    }
}

fn handle_create_dialog(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    let dialog = match ui.overlays.project_create.as_mut() {
        Some(d) => d,
        None => return (StateChange::none(), vec![]),
    };
    match action {
        NavAction::CancelDialog => {
            ui.overlays.project_create = None;
            (StateChange::redraw(), vec![])
        }
        NavAction::SwitchField => {
            dialog.editing_name = !dialog.editing_name;
            (StateChange::redraw(), vec![])
        }
        NavAction::ConfirmCreateProject => {
            let name = dialog.name.clone();
            let path = dialog.path.clone().into();
            ui.overlays.project_create = None;
            (
                StateChange::redraw(),
                vec![Effect::CreateProject {
                    name,
                    workspace_path: path,
                }],
            )
        }
        NavAction::TypeChar(c) => {
            if dialog.editing_name {
                dialog.name.push(c);
            } else {
                dialog.path.push(c);
            }
            (StateChange::redraw(), vec![])
        }
        NavAction::Backspace => {
            if dialog.editing_name {
                dialog.name.pop();
            } else {
                dialog.path.pop();
            }
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_projects_level(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::SelectNext => {
            ui.projects.navigate(1);
            (StateChange::redraw(), vec![])
        }
        NavAction::SelectPrev => {
            ui.projects.navigate(-1);
            (StateChange::redraw(), vec![])
        }
        NavAction::Downlevel => {
            if let Some(idx) = ui.projects.selected {
                if let Some(project) = ui.projects.items.get(idx) {
                    let project_id = project.project_id.clone();
                    return (
                        StateChange::redraw(),
                        vec![Effect::LoadConversations { project_id }],
                    );
                }
            }
            (StateChange::none(), vec![])
        }
        NavAction::OpenCreateProject => {
            let workspace = &state.workspace;
            let dir_name = workspace
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned());
            ui.overlays.project_create = Some(crate::ui::ProjectCreateDialogState {
                name: dir_name,
                path: workspace.to_string_lossy().into_owned(),
                editing_name: true,
            });
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_conversations_level(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::SelectNext => {
            ui.conversations.navigate(1);
            (StateChange::redraw(), vec![])
        }
        NavAction::SelectPrev => {
            ui.conversations.navigate(-1);
            (StateChange::redraw(), vec![])
        }
        NavAction::Uplevel => {
            ui.pop_nav();
            (StateChange::redraw(), vec![])
        }
        NavAction::Downlevel => {
            if let Some(idx) = ui.conversations.selected {
                if let Some(conversation) = ui.conversations.items.get(idx) {
                    let conversation_id = conversation.conversation_id.clone();
                    return (
                        StateChange::none(),
                        vec![Effect::OpenConversation { conversation_id }],
                    );
                }
            }
            (StateChange::none(), vec![])
        }
        NavAction::SubmitConversationInput => submit_conversation_input(state, ui),
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_conversation_level(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::SelectNext => {
            let len = ui.flat_agent_session_count();
            ui.conversation.agent_sessions.navigate_with_len(len, 1);
            (StateChange::redraw(), vec![])
        }
        NavAction::SelectPrev => {
            let len = ui.flat_agent_session_count();
            ui.conversation.agent_sessions.navigate_with_len(len, -1);
            (StateChange::redraw(), vec![])
        }
        NavAction::Uplevel => {
            ui.pop_nav();
            (StateChange::redraw(), vec![])
        }
        NavAction::Downlevel => {
            if let Some(idx) = ui.conversation.agent_sessions.selected {
                if let Some(session) = ui.flat_session_entry(idx) {
                    let thread_id = session.thread_id.clone();
                    let agent = session.agent;
                    let project_id = ui
                        .nav_level()
                        .project_id()
                        .map(str::to_owned)
                        .unwrap_or_default();
                    let conversation_id = ui
                        .nav_level()
                        .conversation_id()
                        .map(str::to_owned)
                        .unwrap_or_default();
                    ui.push_nav(NavLevel::AgentDetail {
                        project_id,
                        conversation_id,
                        thread_id: thread_id.clone(),
                        agent,
                    });
                    return (
                        StateChange::redraw(),
                        vec![Effect::OpenAgentSession { thread_id }],
                    );
                }
            }
            (StateChange::none(), vec![])
        }
        NavAction::SubmitConversationInput => submit_conversation_input(state, ui),
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_agent_detail_level(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::Uplevel => {
            ui.pop_nav();
            (StateChange::redraw(), vec![])
        }
        NavAction::SubmitConversationInput => submit_conversation_input(state, ui),
        _ => (StateChange::none(), vec![]),
    }
}

fn submit_conversation_input(state: &mut AppState, ui: &mut UiState) -> (StateChange, Vec<Effect>) {
    let text = ui.inputs.conversation.content.clone();
    if text.trim().is_empty() {
        return (StateChange::redraw(), vec![]);
    }
    let message_body = text.trim_end().to_owned();
    let parsed = crate::agent_route::parse_agent_routing(text.as_str());
    let has_explicit_target = parsed.is_some();
    let (agent, prompt_text) = match parsed.as_ref() {
        Some((target, body)) => (target.agent, body.clone()),
        None => {
            let Some(agent) = ui.status.agents.first().map(|a| a.name) else {
                ui.set_error(
                    "No agents detected. Install codex/claude/gemini/opencode.".into(),
                );
                return (StateChange::redraw(), vec![]);
            };
            (agent, text.clone())
        }
    };
    if prompt_text.trim().is_empty() && !has_explicit_target {
        ui.set_error("Cannot start an agent session with an empty prompt.".into());
        return (StateChange::redraw(), vec![]);
    }

    ui.inputs.conversation.take_input();
    ui.inputs.conversation.history.record(text.as_str());

    if let Some((target, _)) = parsed.as_ref() {
        if let Some(thread_short_id) = target.thread_short_id.as_deref() {
            let thread_id = find_conversation_thread(ui, target.agent, thread_short_id);
            let Some(thread_id) = thread_id else {
                ui.set_error(format!(
                    "No existing {} session matches #{}",
                    target.agent.bin_name(),
                    thread_short_id
                ));
                return (StateChange::redraw(), vec![]);
            };
            if let Some(state) = super::thread_runtime_state(ui, &thread_id) {
                if !crate::agent_route::thread_can_receive_message(state) {
                    ui.set_error(format!(
                        "{} session #{} is closed. Use @{} to start a new session.",
                        target.agent.bin_name(),
                        crate::agent_route::short_thread_id(&thread_id),
                        target.agent.bin_name()
                    ));
                    return (StateChange::redraw(), vec![]);
                }
            }
            if let Some(conversation_id) = ui.nav_level().conversation_id().map(str::to_owned) {
                super::push_pending_conversation_user_message(ui, &conversation_id, &message_body);
            }
            if prompt_text.trim().is_empty() {
                return (StateChange::redraw(), vec![]);
            }
            return (
                StateChange::redraw(),
                vec![Effect::SendTextToThread {
                    thread_id,
                    text: prompt_text,
                    message_body: Some(message_body),
                }],
            );
        }
    }

    match ui.nav_level() {
        NavLevel::Conversations { project_id } => {
            let project_id = project_id.clone();
            (
                StateChange::redraw(),
                vec![Effect::CreateConversationAndStartAgent {
                    workspace: project_workspace(state, ui, &project_id),
                    project_id,
                    agent,
                    message_body,
                    prompt: prompt_text,
                }],
            )
        }
        NavLevel::Conversation {
            project_id,
            conversation_id,
        }
        | NavLevel::AgentDetail {
            project_id,
            conversation_id,
            ..
        } => {
            let project_id = project_id.clone();
            let conversation_id = conversation_id.clone();
            let workspace = project_workspace(state, ui, &project_id);
            super::push_pending_conversation_user_message(ui, &conversation_id, &message_body);
            (
                StateChange::redraw(),
                vec![Effect::StartAgentInConversation {
                    project_id,
                    conversation_id,
                    agent,
                    workspace,
                    message_body,
                    prompt: prompt_text,
                }],
            )
        }
        NavLevel::Projects => (StateChange::none(), vec![]),
    }
}

fn project_workspace(state: &AppState, ui: &UiState, project_id: &str) -> std::path::PathBuf {
    ui.projects.items
        .iter()
        .find(|project| project.project_id == project_id)
        .map(|project| project.workspace_path.clone())
        .unwrap_or_else(|| state.workspace.clone())
}

fn find_conversation_thread(
    ui: &UiState,
    agent: minos_domain::AgentName,
    short_id: &str,
) -> Option<String> {
    ui.nav_level().conversation_id()?;
    let short_id = short_id.to_ascii_lowercase();
    ui.conversation.agent_sessions.items
        .iter()
        .filter(|session| session.parent_thread_id.is_none())
        .filter(|session| crate::agent_route::thread_can_receive_message(&session.state))
        .find(|session| {
            session.agent == agent
                && (crate::agent_route::short_thread_id(&session.thread_id).to_ascii_lowercase()
                    == short_id
                    || session
                        .thread_id
                        .to_ascii_lowercase()
                        .starts_with(&short_id))
        })
        .map(|session| session.thread_id.clone())
}

#[cfg(test)]
fn navigate(selected: &mut Option<usize>, len: usize, delta: i32) {
    if len == 0 {
        *selected = None;
        return;
    }
    let current = selected.unwrap_or(0) as i32;
    let mut next = current + delta;
    if next < 0 {
        next = len as i32 - 1;
    }
    if next >= len as i32 {
        next = 0;
    }
    *selected = Some(next as usize);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_empty_list_deselects() {
        let mut selected = Some(3);
        navigate(&mut selected, 0, 1);
        assert_eq!(selected, None);
    }

    #[test]
    fn navigate_single_item_wraps() {
        let mut selected = Some(0);
        navigate(&mut selected, 1, 1);
        assert_eq!(selected, Some(0));
        navigate(&mut selected, 1, -1);
        assert_eq!(selected, Some(0));
    }

    #[test]
    fn navigate_wraps_forward() {
        let mut selected = Some(2);
        navigate(&mut selected, 3, 1);
        assert_eq!(selected, Some(0));
    }

    #[test]
    fn navigate_wraps_backward() {
        let mut selected = Some(0);
        navigate(&mut selected, 3, -1);
        assert_eq!(selected, Some(2));
    }

    #[test]
    fn navigate_none_selected_starts_at_zero() {
        let mut selected = None;
        navigate(&mut selected, 3, 1);
        assert_eq!(selected, Some(1));
    }
}
