use crate::app_state::AppState;
use crate::daemon::{CliDto, StartAgentResultDto};
use minos_domain::AgentName;
use tauri::State;

#[tauri::command]
pub async fn daemon_list_clis(state: State<'_, AppState>) -> Result<Vec<CliDto>, String> {
    state.daemon.list_clis().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_list_models(
    state: State<'_, AppState>,
    runtime: String,
) -> Result<minos_protocol::ListModelsResponse, String> {
    state
        .daemon
        .list_models(runtime)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_list_agent_profiles(
    state: State<'_, AppState>,
) -> Result<minos_protocol::ListAgentProfilesResponse, String> {
    state
        .daemon
        .list_agent_profiles()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_create_agent_profile(
    state: State<'_, AppState>,
    name: String,
    description: String,
    runtime_agent: String,
    model: String,
    reasoning_effort: String,
    instructions: Option<String>,
) -> Result<minos_protocol::AgentProfileSummary, String> {
    let runtime = match runtime_agent.as_str() {
        "codex" => AgentName::Codex,
        "claude" => AgentName::Claude,
        "gemini" => AgentName::Gemini,
        "opencode" => AgentName::Opencode,
        "grok" => AgentName::Grok,
        other => return Err(format!("unknown runtime: {other}")),
    };
    let req = minos_protocol::CreateAgentProfileRequest {
        name,
        description,
        runtime_agent: runtime,
        model,
        reasoning_effort,
        instructions: instructions.unwrap_or_default(),
    };
    state
        .daemon
        .create_agent_profile(req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_delete_agent_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .daemon
        .delete_agent_profile(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn daemon_start_agent_in_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    agent: String,
    workspace: String,
    profile_id: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    instructions: Option<String>,
) -> Result<StartAgentResultDto, String> {
    state
        .daemon
        .start_agent_in_conversation(
            conversation_id,
            agent,
            workspace,
            profile_id,
            model,
            reasoning_effort,
            instructions,
        )
        .await
        .map_err(|e| e.to_string())
}
