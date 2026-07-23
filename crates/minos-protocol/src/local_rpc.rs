use jsonrpsee::proc_macros::rpc;
use minos_ui_protocol::UiEventMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSessionSnapshot {
    pub session_id: String,
    pub agent: minos_domain::AgentName,
    pub workspace: String,
    pub state: crate::SessionState,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadSessionRawHistoryResponse {
    pub events: Vec<LocalIngestFrame>,
    pub next_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadArtifactRangeRequest {
    pub session_id: String,
    pub artifact_id: String,
    pub offset: u64,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadArtifactRangeResponse {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub total_size: u64,
    pub eof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RespondOpencodePermissionRequest {
    pub session_id: String,
    pub permission_id: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RespondOpencodeQuestionRequest {
    pub session_id: String,
    pub question_id: String,
    pub answers: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalIngestFrame {
    pub session_id: String,
    #[serde(default)]
    pub seq: u64,
    pub agent: minos_domain::AgentName,
    #[serde(default)]
    pub ui_events: Vec<UiEventMessage>,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocalManagerEvent {
    SessionAdded {
        session_id: String,
        workspace: String,
        agent: minos_domain::AgentName,
        parent_session_id: Option<String>,
    },
    SessionStateChanged {
        session_id: String,
        old: crate::SessionState,
        new: crate::SessionState,
        at_ms: i64,
    },
    SessionClosed {
        session_id: String,
        reason: crate::CloseReason,
    },
    InstanceCrashed {
        workspace: String,
        affected_threads: Vec<String>,
        #[serde(default = "default_instance_crashed_reason")]
        reason: crate::PauseReason,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocalConversationEvent {
    ConversationMessageAppended {
        conversation_id: String,
        message_seq: i64,
    },
    /// Local user toggled a reaction; `reactions` is the full aggregate for the message.
    ConversationReactionToggled {
        conversation_id: String,
        message_id: String,
        reactions: Vec<crate::LocalReactionGroup>,
    },
}

fn default_instance_crashed_reason() -> crate::PauseReason {
    crate::PauseReason::InstanceReaped
}

#[rpc(server, client, namespace = "minos_local")]
pub trait LocalDaemonRpc {
    #[method(name = "health")]
    async fn health(&self) -> jsonrpsee::core::RpcResult<crate::HealthResponse>;

    #[method(name = "list_clis")]
    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<crate::ListClisResponse>;

    #[method(name = "start_agent")]
    async fn start_agent(
        &self,
        req: crate::StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

    #[method(name = "send_user_message")]
    async fn send_user_message(
        &self,
        req: crate::SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "approval_decision")]
    async fn approval_decision(
        &self,
        req: crate::ApprovalDecisionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "respond_opencode_permission")]
    async fn respond_opencode_permission(
        &self,
        req: RespondOpencodePermissionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "respond_opencode_question")]
    async fn respond_opencode_question(
        &self,
        req: RespondOpencodeQuestionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "interrupt_session")]
    async fn interrupt_session(
        &self,
        req: crate::InterruptSessionRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "close_session")]
    async fn close_session(&self, req: crate::CloseSessionRequest) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "delete_session")]
    async fn delete_session(&self, req: crate::CloseSessionRequest)
        -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "resume_session")]
    async fn resume_session(
        &self,
        req: crate::ResumeSessionRequest,
    ) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

    #[method(name = "list_local_sessions")]
    async fn list_local_sessions(&self) -> jsonrpsee::core::RpcResult<Vec<LocalSessionSnapshot>>;

    #[method(name = "list_projects")]
    async fn list_projects(&self) -> jsonrpsee::core::RpcResult<crate::ListProjectsResponse>;

    #[method(name = "create_project")]
    async fn create_project(
        &self,
        req: crate::CreateProjectRequest,
    ) -> jsonrpsee::core::RpcResult<crate::CreateProjectResponse>;

    #[method(name = "list_conversations")]
    async fn list_conversations(
        &self,
        req: crate::ListConversationsParams,
    ) -> jsonrpsee::core::RpcResult<crate::ListConversationsResponse>;

    #[method(name = "create_conversation")]
    async fn create_conversation(
        &self,
        req: crate::CreateConversationParams,
    ) -> jsonrpsee::core::RpcResult<crate::CreateConversationResponse>;

    #[method(name = "update_conversation")]
    async fn update_conversation(
        &self,
        req: crate::UpdateConversationParams,
    ) -> jsonrpsee::core::RpcResult<crate::UpdateConversationResponse>;

    #[method(name = "list_conversation_messages")]
    async fn list_conversation_messages(
        &self,
        req: crate::ListConversationMessagesParams,
    ) -> jsonrpsee::core::RpcResult<crate::ListConversationMessagesResponse>;

    #[method(name = "toggle_conversation_message_reaction")]
    async fn toggle_conversation_message_reaction(
        &self,
        req: crate::ToggleConversationMessageReactionParams,
    ) -> jsonrpsee::core::RpcResult<crate::ToggleConversationMessageReactionResponse>;

    #[method(name = "list_conversation_agent_sessions")]
    async fn list_conversation_agent_sessions(
        &self,
        req: crate::ListConversationAgentSessionsParams,
    ) -> jsonrpsee::core::RpcResult<crate::ListConversationAgentSessionsResponse>;

    #[method(name = "start_agent_in_conversation")]
    async fn start_agent_in_conversation(
        &self,
        req: crate::StartAgentInConversationRequest,
    ) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

    #[method(name = "list_models")]
    async fn list_models(
        &self,
        req: crate::ListModelsRequest,
    ) -> jsonrpsee::core::RpcResult<crate::ListModelsResponse>;

    #[method(name = "list_agent_profiles")]
    async fn list_agent_profiles(
        &self,
    ) -> jsonrpsee::core::RpcResult<crate::ListAgentProfilesResponse>;

    #[method(name = "create_agent_profile")]
    async fn create_agent_profile(
        &self,
        req: crate::CreateAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<crate::AgentProfileSummary>;

    #[method(name = "update_agent_profile")]
    async fn update_agent_profile(
        &self,
        req: crate::UpdateAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<crate::AgentProfileSummary>;

    #[method(name = "delete_agent_profile")]
    async fn delete_agent_profile(
        &self,
        req: crate::DeleteAgentProfileRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "append_conversation_message")]
    async fn append_conversation_message(
        &self,
        req: crate::AppendConversationMessageParams,
    ) -> jsonrpsee::core::RpcResult<crate::AppendConversationMessageResponse>;

    #[method(name = "read_session_raw_history")]
    async fn read_session_raw_history(
        &self,
        req: crate::ReadSessionParams,
    ) -> jsonrpsee::core::RpcResult<ReadSessionRawHistoryResponse>;

    #[method(name = "read_artifact_range")]
    async fn read_artifact_range(
        &self,
        req: ReadArtifactRangeRequest,
    ) -> jsonrpsee::core::RpcResult<ReadArtifactRangeResponse>;

    #[subscription(name = "subscribe_ingest", item = LocalIngestFrame)]
    async fn subscribe_ingest(&self) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(name = "subscribe_manager_events", item = LocalManagerEvent)]
    async fn subscribe_manager_events(&self) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(name = "subscribe_conversation_events", item = LocalConversationEvent)]
    async fn subscribe_conversation_events(&self) -> jsonrpsee::core::SubscriptionResult;
}
