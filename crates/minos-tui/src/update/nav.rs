use crate::effect::{Effect, StateChange};
use crate::nav::{NavAction, NavLevel};
use crate::state::AppState;
use crate::ui::UiState;

pub fn handle(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    if ui.startup_create_prompt.is_some() {
        return handle_startup_prompt(ui, action);
    }
    if ui.project_create_dialog.is_some() {
        return handle_create_dialog(ui, action);
    }
    match &ui.nav_level {
        NavLevel::Projects => handle_projects_level(ui, action),
        NavLevel::Sessions { .. } => handle_sessions_level(state, ui, action),
        NavLevel::Session { .. } => handle_session_level(ui, action),
        NavLevel::AgentDetail { .. } => handle_agent_detail_level(ui, action),
    }
}

fn handle_startup_prompt(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::AcceptStartupPrompt => {
            let prompt = ui.startup_create_prompt.take().unwrap();
            (
                StateChange::redraw(),
                vec![Effect::CreateProject {
                    name: prompt.dir_name,
                    workspace_path: prompt.path.into(),
                }],
            )
        }
        NavAction::DismissStartupPrompt => {
            ui.startup_create_prompt = None;
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_create_dialog(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    let dialog = match ui.project_create_dialog.as_mut() {
        Some(d) => d,
        None => return (StateChange::none(), vec![]),
    };
    match action {
        NavAction::CancelDialog => {
            ui.project_create_dialog = None;
            (StateChange::redraw(), vec![])
        }
        NavAction::SwitchField => {
            dialog.editing_name = !dialog.editing_name;
            (StateChange::redraw(), vec![])
        }
        NavAction::ConfirmCreateProject => {
            let name = dialog.name.clone();
            let path = dialog.path.clone().into();
            ui.project_create_dialog = None;
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

fn handle_projects_level(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::SelectNext => {
            navigate(&mut ui.selected_project, ui.projects.len(), 1);
            ui.project_list_state.select(ui.selected_project);
            (StateChange::redraw(), vec![])
        }
        NavAction::SelectPrev => {
            navigate(&mut ui.selected_project, ui.projects.len(), -1);
            ui.project_list_state.select(ui.selected_project);
            (StateChange::redraw(), vec![])
        }
        NavAction::Downlevel => {
            if let Some(idx) = ui.selected_project {
                if let Some(project) = ui.projects.get(idx) {
                    let project_id = project.project_id.clone();
                    return (
                        StateChange::redraw(),
                        vec![Effect::LoadProjectThreads { project_id }],
                    );
                }
            }
            (StateChange::none(), vec![])
        }
        NavAction::OpenCreateProject => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let dir_name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned());
            ui.project_create_dialog = Some(crate::ui::ProjectCreateDialogState {
                name: dir_name,
                path: cwd.to_string_lossy().into_owned(),
                editing_name: true,
            });
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_sessions_level(
    state: &mut AppState,
    ui: &mut UiState,
    action: NavAction,
) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::SelectNext => {
            navigate(&mut ui.selected_thread, ui.project_sessions.len(), 1);
            ui.room_list_state.select(ui.selected_thread);
            (StateChange::redraw(), vec![])
        }
        NavAction::SelectPrev => {
            navigate(&mut ui.selected_thread, ui.project_sessions.len(), -1);
            ui.room_list_state.select(ui.selected_thread);
            (StateChange::redraw(), vec![])
        }
        NavAction::Uplevel => {
            ui.nav_level = ui.nav_level.go_up();
            (StateChange::redraw(), vec![])
        }
        NavAction::Downlevel => {
            if let Some(idx) = ui.selected_thread {
                if let Some(thread) = ui.project_sessions.get(idx) {
                    let thread_id = thread.thread_id.clone();
                    return (
                        StateChange::none(),
                        vec![Effect::OpenProjectSession { thread_id }],
                    );
                }
            }
            (StateChange::none(), vec![])
        }
        NavAction::SubmitSessionInput => {
            let text = ui.room_input.content.clone();
            if text.trim().is_empty() {
                return (StateChange::redraw(), vec![]);
            }
            let (agent, prompt_text) = match crate::agent_route::parse_agent_routing(text.as_str())
            {
                Some((target, body)) => (target.agent, body),
                None => {
                    let agent = ui
                        .status
                        .agents
                        .first()
                        .map(|a| a.name)
                        .unwrap_or(minos_domain::AgentName::Codex);
                    (agent, text.clone())
                }
            };
            if prompt_text.trim().is_empty() {
                ui.set_error("Cannot start a session with an empty prompt.".into());
                return (StateChange::redraw(), vec![]);
            }
            ui.room_input.take_input();
            ui.room_input.history.record(text.as_str());
            let project_id = ui
                .nav_level
                .project_id()
                .map(|s| s.to_owned())
                .unwrap_or_default();
            let workspace = state.workspace.clone();
            (
                StateChange::redraw(),
                vec![Effect::StartAgentInProject {
                    project_id,
                    agent,
                    workspace,
                    prompt: prompt_text,
                }],
            )
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_session_level(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::Uplevel => {
            if let Some(thread_id) = ui.nav_level.thread_id().map(str::to_owned) {
                if let Some(idx) = ui
                    .project_sessions
                    .iter()
                    .position(|session| session.thread_id == thread_id)
                {
                    ui.selected_thread = Some(idx);
                    ui.room_list_state.select(Some(idx));
                }
            }
            ui.agent_detail_visible = false;
            ui.nav_level = ui.nav_level.go_up();
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

fn handle_agent_detail_level(ui: &mut UiState, action: NavAction) -> (StateChange, Vec<Effect>) {
    match action {
        NavAction::Uplevel => {
            ui.agent_detail_visible = false;
            ui.nav_level = ui.nav_level.go_up();
            (StateChange::redraw(), vec![])
        }
        _ => (StateChange::none(), vec![]),
    }
}

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
