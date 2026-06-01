use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ListAgentSessionsInput {
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub before_started_at_ms: Option<i64>,
    pub limit: u32,
    pub caller_account_id: String,
}

#[derive(Debug, Clone)]
pub struct StartAgentSessionInput {
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub agent_id: String,
    pub host_installation_id: Option<String>,
    pub initial_user_message: Option<String>,
    pub client_request_id: String,
    pub caller_account_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartAgentSessionOutput {
    pub session_id: String,
    pub conversation_id: String,
    pub host_installation_id: String,
    pub started_at_ms: i64,
    pub initial_turn_id: Option<String>,
    pub host_command_id: String,
}

#[derive(Debug, Clone)]
pub struct SendInputInput {
    pub session_id: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub client_request_id: String,
    pub caller_account_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendInputOutput {
    pub session_id: String,
    pub turn_id: String,
    pub turn_seq: i64,
}

#[derive(Debug, Clone)]
pub struct StopAgentSessionInput {
    pub session_id: String,
    pub caller_account_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListAgentSessionsOutput {
    pub sessions: Vec<AgentSessionSummary>,
    pub next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSessionSummary {
    pub session_id: String,
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ReadTurnsInput {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub after_turn_seq: Option<i64>,
    pub after_event_seq: Option<i64>,
    pub limit: u32,
    pub caller_account_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadTurnsOutput {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub turns: Vec<ReadTurnMetadata>,
    pub events: Vec<ReadTurnEvent>,
    pub next_turn_seq: Option<i64>,
    pub next_event_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadTurnMetadata {
    pub turn_id: String,
    pub turn_seq: i64,
    pub role: String,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub summary_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadTurnEvent {
    pub turn_id: String,
    pub event_seq: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at_ms: i64,
}