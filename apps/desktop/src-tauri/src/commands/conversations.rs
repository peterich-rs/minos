use crate::app_state::AppState;
use crate::daemon::{ConversationDto, MessagePageDto, ToggleReactionResultDto};
use tauri::State;

#[tauri::command]
pub async fn daemon_list_conversations(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ConversationDto>, String> {
    state
        .daemon
        .list_conversations(project_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_create_conversation(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
    priority: Option<String>,
    agents: Option<Vec<String>>,
) -> Result<ConversationDto, String> {
    state
        .daemon
        .create_conversation(project_id, title, priority, agents.unwrap_or_default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_update_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    title: Option<String>,
    priority: Option<String>,
    progress: Option<String>,
) -> Result<ConversationDto, String> {
    state
        .daemon
        .update_conversation(conversation_id, title, priority, progress)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_list_messages(
    state: State<'_, AppState>,
    conversation_id: String,
    before_seq: Option<i64>,
    limit: Option<u32>,
) -> Result<MessagePageDto, String> {
    state
        .daemon
        .list_messages(conversation_id, before_seq, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_append_user_message(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
    body: String,
) -> Result<i64, String> {
    state
        .daemon
        .append_user_message(conversation_id, message_id, body)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_toggle_message_reaction(
    state: State<'_, AppState>,
    message_id: String,
    emoji: String,
) -> Result<ToggleReactionResultDto, String> {
    state
        .daemon
        .toggle_message_reaction(message_id, emoji)
        .await
        .map_err(|e| e.to_string())
}
