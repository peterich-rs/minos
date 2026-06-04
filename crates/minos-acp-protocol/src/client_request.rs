use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::types::{
    AgentCapabilities, AuthMethod, ClientCapabilities, ContentBlock, Implementation, McpServer,
    SessionConfigId, SessionConfigOption, SessionConfigValueId, SessionId, SessionInfo,
    SessionModeId, SessionModeState, StopReason,
};

pub trait AcpClientRequest: Serialize {
    const METHOD: &'static str;
    type Response: DeserializeOwned;
}

pub trait AcpClientNotification: Serialize {
    const METHOD: &'static str;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InitializeParams {
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_capabilities: Option<ClientCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_info: Option<Implementation>,
}

impl AcpClientRequest for InitializeParams {
    const METHOD: &'static str = "initialize";
    type Response = InitializeResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InitializeResponse {
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_info: Option<Implementation>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthenticateParams { pub method_id: String }
impl AcpClientRequest for AuthenticateParams {
    const METHOD: &'static str = "authenticate";
    type Response = AuthenticateResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthenticateResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NewSessionParams {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}
impl AcpClientRequest for NewSessionParams {
    const METHOD: &'static str = "session/new";
    type Response = NewSessionResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NewSessionResponse {
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadSessionParams {
    pub session_id: SessionId,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}
impl AcpClientRequest for LoadSessionParams {
    const METHOD: &'static str = "session/load";
    type Response = LoadSessionResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadSessionResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeSessionParams {
    pub session_id: SessionId,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mcp_servers: Option<Vec<McpServer>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}
impl AcpClientRequest for ResumeSessionParams {
    const METHOD: &'static str = "session/resume";
    type Response = ResumeSessionResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeSessionResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptParams {
    pub session_id: SessionId,
    pub prompt: Vec<ContentBlock>,
}
impl AcpClientRequest for PromptParams {
    const METHOD: &'static str = "session/prompt";
    type Response = PromptResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptResponse { pub stop_reason: StopReason }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseSessionParams { pub session_id: SessionId }
impl AcpClientRequest for CloseSessionParams {
    const METHOD: &'static str = "session/close";
    type Response = CloseSessionResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseSessionResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetSessionModeParams { pub session_id: SessionId, pub mode_id: SessionModeId }
impl AcpClientRequest for SetSessionModeParams {
    const METHOD: &'static str = "session/set_mode";
    type Response = SetSessionModeResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetSessionModeResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetConfigOptionParams { pub session_id: SessionId, pub config_id: SessionConfigId, pub value: SessionConfigValueId }
impl AcpClientRequest for SetConfigOptionParams {
    const METHOD: &'static str = "session/set_config_option";
    type Response = SetConfigOptionResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetConfigOptionResponse { pub config_options: Vec<SessionConfigOption> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListSessionsParams {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<String>,
}
impl AcpClientRequest for ListSessionsParams {
    const METHOD: &'static str = "session/list";
    type Response = ListSessionsResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutParams {}
impl AcpClientRequest for LogoutParams {
    const METHOD: &'static str = "logout";
    type Response = LogoutResponse;
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutResponse {}
