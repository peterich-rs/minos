use jsonrpsee::proc_macros::rpc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalIngestFrame {
    pub thread_id: String,
    pub agent: minos_domain::AgentName,
    pub payload: serde_json::Value,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocalManagerEvent {
    ThreadAdded {
        thread_id: String,
        workspace: String,
        agent: minos_domain::AgentName,
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
    },
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

    #[method(name = "interrupt_thread")]
    async fn interrupt_thread(
        &self,
        req: crate::InterruptThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[method(name = "close_thread")]
    async fn close_thread(
        &self,
        req: crate::CloseThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    #[subscription(name = "subscribe_ingest", item = LocalIngestFrame)]
    async fn subscribe_ingest(&self) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(name = "subscribe_manager_events", item = LocalManagerEvent)]
    async fn subscribe_manager_events(&self) -> jsonrpsee::core::SubscriptionResult;
}
