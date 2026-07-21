//! Minos desktop shell — Tauri host + local daemon JSON-RPC bridge.

mod daemon;

use daemon::{
    CliDto, ConnectionDto, ConversationDto, DaemonBridge, MessagePageDto, ProjectDto, SessionDto,
    StartAgentResultDto, TranscriptPageDto,
};
use std::sync::Arc;
use tauri::State;

struct AppState {
    daemon: Arc<DaemonBridge>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: String,
    version: String,
    shell: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Minos".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        shell: "tauri".into(),
    }
}

#[tauri::command]
async fn daemon_connect(
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<ConnectionDto, String> {
    Ok(state.daemon.connect(url).await)
}

#[tauri::command]
async fn daemon_status(state: State<'_, AppState>) -> Result<ConnectionDto, String> {
    Ok(state.daemon.status().await)
}

#[tauri::command]
async fn daemon_list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectDto>, String> {
    state
        .daemon
        .list_projects()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_create_project(
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<ProjectDto, String> {
    state
        .daemon
        .create_project(workspace_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_list_conversations(
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
async fn daemon_list_messages(
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
async fn daemon_list_sessions(
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
async fn daemon_create_conversation(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
) -> Result<ConversationDto, String> {
    state
        .daemon
        .create_conversation(project_id, title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_update_conversation(
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
async fn daemon_append_user_message(
    state: State<'_, AppState>,
    conversation_id: String,
    body: String,
) -> Result<(), String> {
    state
        .daemon
        .append_user_message(conversation_id, body)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_list_clis(state: State<'_, AppState>) -> Result<Vec<CliDto>, String> {
    state.daemon.list_clis().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_start_agent_in_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    agent: String,
    workspace: String,
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
            model,
            reasoning_effort,
            instructions,
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_list_models(
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
async fn daemon_list_agent_profiles(
    state: State<'_, AppState>,
) -> Result<minos_protocol::ListAgentProfilesResponse, String> {
    state
        .daemon
        .list_agent_profiles()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_create_agent_profile(
    state: State<'_, AppState>,
    name: String,
    description: String,
    runtime_agent: String,
    model: String,
    reasoning_effort: String,
    instructions: Option<String>,
) -> Result<minos_protocol::AgentProfileSummary, String> {
    use minos_domain::AgentName;
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
async fn daemon_delete_agent_profile(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .daemon
        .delete_agent_profile(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_send_user_message(
    state: State<'_, AppState>,
    thread_id: String,
    text: String,
) -> Result<(), String> {
    state
        .daemon
        .send_user_message(thread_id, text)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_resume_thread(
    state: State<'_, AppState>,
    thread_id: String,
    auto_continue: Option<bool>,
) -> Result<(), String> {
    state
        .daemon
        .resume_thread(thread_id, auto_continue.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_resolve_approval(
    state: State<'_, AppState>,
    request_id: String,
    thread_id: String,
    decision: serde_json::Value,
) -> Result<(), String> {
    state
        .daemon
        .resolve_approval(request_id, thread_id, decision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_respond_opencode_permission(
    state: State<'_, AppState>,
    thread_id: String,
    permission_id: String,
    response: String,
) -> Result<(), String> {
    state
        .daemon
        .respond_opencode_permission(thread_id, permission_id, response)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_respond_opencode_question(
    state: State<'_, AppState>,
    thread_id: String,
    question_id: String,
    answers: Vec<Vec<String>>,
) -> Result<(), String> {
    state
        .daemon
        .respond_opencode_question(thread_id, question_id, answers)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_list_project_sessions(
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
async fn daemon_read_transcript(
    state: State<'_, AppState>,
    thread_id: String,
    from_seq: Option<u64>,
    limit: Option<u32>,
    full: Option<bool>,
) -> Result<TranscriptPageDto, String> {
    state
        .daemon
        .read_transcript(thread_id, from_seq, limit, full.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let daemon = Arc::new(DaemonBridge::new());
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            daemon: Arc::clone(&daemon),
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let daemon = Arc::clone(&daemon);
            // Attach AppHandle so connect() can start JSON-RPC subscription pumps.
            tauri::async_runtime::spawn(async move {
                daemon.attach_app(handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            daemon_connect,
            daemon_status,
            daemon_list_projects,
            daemon_create_project,
            daemon_list_conversations,
            daemon_list_messages,
            daemon_list_sessions,
            daemon_list_project_sessions,
            daemon_read_transcript,
            daemon_create_conversation,
            daemon_update_conversation,
            daemon_append_user_message,
            daemon_list_clis,
            daemon_start_agent_in_conversation,
            daemon_list_models,
            daemon_list_agent_profiles,
            daemon_create_agent_profile,
            daemon_delete_agent_profile,
            daemon_send_user_message,
            daemon_resume_thread,
            daemon_resolve_approval,
            daemon_respond_opencode_permission,
            daemon_respond_opencode_question,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Minos desktop");
}
