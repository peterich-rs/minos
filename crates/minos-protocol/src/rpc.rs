//! The shared service trait. `jsonrpsee` macros generate a server stub
//! (implemented by `minos-daemon`) and a typed client (used by `minos-mobile`).

use crate::{AgentEvent, HealthResponse, ListClisResponse, PairRequest, PairResponse};
use jsonrpsee::core::SubscriptionResult;
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

    /// Streaming agent events. **MVP**: server returns "not implemented";
    /// shape and naming are pinned now so plan P1 only fills in the producer.
    #[subscription(name = "subscribe_events" => "agent_event", item = AgentEvent)]
    async fn subscribe_events(&self) -> SubscriptionResult;
}
