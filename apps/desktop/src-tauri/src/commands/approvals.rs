use crate::app_state::AppState;
use tauri::State;

#[tauri::command]
pub async fn daemon_resolve_approval(
    state: State<'_, AppState>,
    request_id: String,
    session_id: String,
    decision: serde_json::Value,
) -> Result<(), String> {
    state
        .daemon
        .resolve_approval(request_id, session_id, decision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_respond_opencode_permission(
    state: State<'_, AppState>,
    session_id: String,
    permission_id: String,
    response: String,
) -> Result<(), String> {
    state
        .daemon
        .respond_opencode_permission(session_id, permission_id, response)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_respond_opencode_question(
    state: State<'_, AppState>,
    session_id: String,
    question_id: String,
    answers: Vec<Vec<String>>,
) -> Result<(), String> {
    state
        .daemon
        .respond_opencode_question(session_id, question_id, answers)
        .await
        .map_err(|e| e.to_string())
}
