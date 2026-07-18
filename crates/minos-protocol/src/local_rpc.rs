use jsonrpsee::proc_macros::rpc;
use minos_ui_protocol::UiEventMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalThreadSnapshot {
    pub thread_id: String,
    pub agent: minos_domain::AgentName,
    pub workspace: String,
    pub state: crate::ThreadState,
    pub parent_thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadThreadRawHistoryResponse {
    pub events: Vec<LocalIngestFrame>,
    pub next_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadArtifactRangeRequest {
    pub thread_id: String,
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
    pub thread_id: String,
    pub permission_id: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RespondOpencodeQuestionRequest {
    pub thread_id: String,
    pub question_id: String,
    pub answers: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalIngestFrame {
    pub thread_id: String,
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
    ThreadAdded {
        thread_id: String,
        workspace: String,
        agent: minos_domain::AgentName,
        parent_thread_id: Option<String>,
    },
    ThreadStateChanged {
        thread_id: String,
        old: crate::ThreadState,
        new: crate::ThreadState,
        at_ms: i64,
    },
    ThreadClosed {
        thread_id: String,
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

    #[method(name = "interrupt_thread")]
    async fn interrupt_thread(
        &self,
        req: crate::InterruptThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "close_thread")]
    async fn close_thread(&self, req: crate::CloseThreadRequest) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "delete_thread")]
    async fn delete_thread(&self, req: crate::CloseThreadRequest)
        -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "resume_thread")]
    async fn resume_thread(
        &self,
        req: crate::ResumeThreadRequest,
    ) -> jsonrpsee::core::RpcResult<crate::StartAgentResponse>;

    #[method(name = "list_local_threads")]
    async fn list_local_threads(&self) -> jsonrpsee::core::RpcResult<Vec<LocalThreadSnapshot>>;

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

    #[method(name = "append_conversation_message")]
    async fn append_conversation_message(
        &self,
        req: crate::AppendConversationMessageParams,
    ) -> jsonrpsee::core::RpcResult<crate::AppendConversationMessageResponse>;

    #[method(name = "read_thread_raw_history")]
    async fn read_thread_raw_history(
        &self,
        req: crate::ReadThreadParams,
    ) -> jsonrpsee::core::RpcResult<ReadThreadRawHistoryResponse>;

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
