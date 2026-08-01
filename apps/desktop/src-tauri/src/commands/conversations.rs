use crate::app_state::AppState;
use crate::daemon::{
    ConversationDto, GitStatusDto, MessagePageDto, RemoveConversationAgentDto,
    ToggleReactionResultDto,
};
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConversationAgentArg {
    pub agent: String,
    #[serde(default)]
    pub brief: Option<String>,
}

#[tauri::command]
pub async fn daemon_create_conversation(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
    priority: Option<String>,
    agents: Option<Vec<CreateConversationAgentArg>>,
    git_mode: Option<String>,
) -> Result<ConversationDto, String> {
    let agents = agents
        .unwrap_or_default()
        .into_iter()
        .map(|a| (a.agent, a.brief))
        .collect();
    state
        .daemon
        .create_conversation(project_id, title, priority, agents, git_mode)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_git_get_status(
    state: State<'_, AppState>,
    conversation_id: String,
    refresh_conversation: Option<bool>,
) -> Result<GitStatusDto, String> {
    state
        .daemon
        .git_get_status(conversation_id, refresh_conversation.unwrap_or(true))
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
pub async fn daemon_remove_conversation_agent(
    state: State<'_, AppState>,
    conversation_id: String,
    agent: String,
) -> Result<RemoveConversationAgentDto, String> {
    state
        .daemon
        .remove_conversation_agent(conversation_id, agent)
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendUserMessageDto {
    pub message_seq: i64,
}

#[tauri::command]
pub async fn daemon_append_user_message(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
    body: String,
) -> Result<AppendUserMessageDto, String> {
    let message_seq = state
        .daemon
        .append_user_message(conversation_id, message_id, body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(AppendUserMessageDto { message_seq })
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
