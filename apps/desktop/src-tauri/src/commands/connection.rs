use crate::app_state::AppState;
use crate::daemon::ConnectionDto;
use tauri::State;

#[tauri::command]
pub async fn daemon_connect(
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<ConnectionDto, String> {
    Ok(state.daemon.connect(url).await)
}

#[tauri::command]
pub async fn daemon_status(state: State<'_, AppState>) -> Result<ConnectionDto, String> {
    Ok(state.daemon.status().await)
}
