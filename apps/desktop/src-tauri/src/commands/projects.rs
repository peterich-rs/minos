use crate::app_state::AppState;
use crate::daemon::ProjectDto;
use tauri::State;

#[tauri::command]
pub async fn daemon_list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectDto>, String> {
    state
        .daemon
        .list_projects()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_create_project(
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<ProjectDto, String> {
    state
        .daemon
        .create_project(workspace_path)
        .await
        .map_err(|e| e.to_string())
}
