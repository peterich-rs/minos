//! The shared service trait. `jsonrpsee` macros generate a server stub
//! (implemented by `minos-daemon`) plus a typed client retained for Rust-side
//! callers and tests; `minos-mobile` now uses the envelope/local-RPC path.

use crate::{
    CloseThreadRequest, GetThreadParams, GetThreadResponse, HealthResponse,
    InterruptThreadRequest, ListClisResponse, ListThreadsParams, ListThreadsResponse, PairRequest,
    PairResponse, SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
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

    /// Launch (or join) an agent session for the given `workspace`. Multi-
    /// session: subsequent calls with the same `workspace` reuse the existing
    /// codex app-server child while distinct workspaces each spawn their own
    /// instance. Response carries the `session_id` consumers must pass to
    /// `send_user_message` / `interrupt_thread` / `close_thread`. See spec §5.2.
    #[method(name = "start_agent")]
    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse>;

    /// Send user text into the named thread. Fire-and-observe: streaming
    /// output arrives via the backend's ingest pipeline (plan §B6), not as
    /// this RPC's response. See spec §5.2.
    #[method(name = "send_user_message")]
    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    /// Pause an in-flight turn on the named thread. Best-effort: the codex
    /// app-server may have already finished the turn — that is fine, the
    /// thread transitions to `Suspended { UserInterrupt }` either way.
    #[method(name = "interrupt_thread")]
    async fn interrupt_thread(
        &self,
        req: InterruptThreadRequest,
    ) -> jsonrpsee::core::RpcResult<()>;

    /// Permanently close the named thread. Idempotent — re-closing a closed
    /// thread is a no-op.
    #[method(name = "close_thread")]
    async fn close_thread(&self, req: CloseThreadRequest) -> jsonrpsee::core::RpcResult<()>;

    /// Paginated history list. Keyed by `last_activity_at` desc.
    #[method(name = "list_threads")]
    async fn list_threads(
        &self,
        req: ListThreadsParams,
    ) -> jsonrpsee::core::RpcResult<ListThreadsResponse>;

    /// Snapshot one thread's metadata + live state (intended for the chat
    /// detail screen first paint).
    #[method(name = "get_thread")]
    async fn get_thread(
        &self,
        req: GetThreadParams,
    ) -> jsonrpsee::core::RpcResult<GetThreadResponse>;
}
