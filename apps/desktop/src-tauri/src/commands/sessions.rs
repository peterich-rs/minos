use crate::app_state::AppState;
use crate::daemon::{SessionDto, TranscriptPageDto};
use tauri::State;

#[tauri::command]
pub async fn daemon_list_sessions(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<SessionDto>, String> {
    state
        .daemon
        .list_sessions(conversation_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_list_project_sessions(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<SessionDto>, String> {
    state
        .daemon
        .list_project_sessions(project_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_read_transcript(
    state: State<'_, AppState>,
    session_id: String,
    from_seq: Option<u64>,
    limit: Option<u32>,
    full: Option<bool>,
) -> Result<TranscriptPageDto, String> {
    state
        .daemon
        .read_transcript(session_id, from_seq, limit, full.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_send_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    origin_message_id: Option<String>,
) -> Result<(), String> {
    state
        .daemon
        .send_user_message(session_id, text, origin_message_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_resume_session(
    state: State<'_, AppState>,
    session_id: String,
    auto_continue: Option<bool>,
) -> Result<(), String> {
    state
        .daemon
        .resume_session(session_id, auto_continue.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}
