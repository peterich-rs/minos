//! The shared service trait. `jsonrpsee` macros generate a server stub
//! (implemented by `minos-daemon`) plus a typed client retained for Rust-side
//! callers and tests; `minos-mobile` now uses the envelope/local-RPC path.

use crate::{
    HealthResponse, ListClisResponse, PairRequest, PairResponse, SendUserMessageRequest,
    StartAgentRequest, StartAgentResponse,
};
use jsonrpsee::proc_macros::rpc;

#[rpc(server, client, namespace = "minos")]
pub trait MinosRpc {
    /// Confirm a fresh pairing handshake. Idempotent only when the same
    /// (token, `device_id`) tuple is supplied.
    #[method(name = "pair")]
    async fn pair(&self, req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse>;

    /// Cheap liveness probe.
    #[method(name = "health")]
    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse>;

    /// Snapshot of locally detected CLI agents.
    #[method(name = "list_clis")]
    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse>;

    /// Launch an agent session. Errors with `AgentAlreadyRunning` if one is
    /// already active. Response carries the `session_id` consumers must pass
    /// to `send_user_message`. See spec §5.2.
    #[method(name = "start_agent")]
    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse>;

    /// Send user text into the active session. Fire-and-observe: streaming
    /// output arrives via the backend's ingest pipeline (plan §B6), not as
    /// this RPC's response. See spec §5.2.
    #[method(name = "send_user_message")]
    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    /// Stop the active session. Idempotent when no session is running.
    /// See spec §5.2.
    #[method(name = "stop_agent")]
    async fn stop_agent(&self) -> jsonrpsee::core::RpcResult<()>;
}
