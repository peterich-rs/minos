//! `MinosRpcServer` impl that routes to inner services, plus an
//! envelope-aware [`invoke_forwarded`] entry point.
//!
//! Pre-relay this struct fronted a jsonrpsee WS server; post-Phase-F it is
//! invoked directly from the relay-client dispatch loop when the relay
//! delivers a peer-originated [`Envelope::Forwarded`] frame. Pairing
//! state moved to the relay, so the corresponding fields and the active
//! token / connection-state plumbing are gone.
//!
//! Holds `Arc`s only — cheap to clone once and pass into the dispatcher.

use std::sync::Arc;
use std::time::Instant;

use jsonrpsee::core::async_trait;
use jsonrpsee::types::ErrorObjectOwned;
use minos_cli_detect::{detect_all, CommandRunner};
use minos_domain::MinosError;
use minos_protocol::envelope::Envelope;
use minos_protocol::{
    HealthResponse, ListClisResponse, MinosRpcServer, PairRequest, PairResponse,
    SendUserMessageRequest, StartAgentRequest, StartAgentResponse,
};
use serde_json::{json, Map, Value};

use crate::agent::AgentGlue;

pub struct RpcServerImpl {
    pub started_at: Instant,
    pub runner: Arc<dyn CommandRunner>,
    pub agent: Arc<AgentGlue>,
}

#[async_trait]
impl MinosRpcServer for RpcServerImpl {
    async fn pair(&self, _req: PairRequest) -> jsonrpsee::core::RpcResult<PairResponse> {
        // Pairing is owned end-to-end by the backend broker (plan 05 Phase F.3).
        // The Mac receives a Paired event from the backend's `Pair` LocalRpc
        // handler — it never sees a peer-originated `pair` JSON-RPC. If a
        // forwarded JSON-RPC frame somehow reaches here, the right answer is
        // that the host explicitly does not trust this surface for pairing.
        Err(rpc_err(MinosError::Unauthorized {
            reason: "pair handled by backend, not host".into(),
        }))
    }

    async fn health(&self) -> jsonrpsee::core::RpcResult<HealthResponse> {
        Ok(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_secs: self.started_at.elapsed().as_secs(),
        })
    }

    async fn list_clis(&self) -> jsonrpsee::core::RpcResult<ListClisResponse> {
        Ok(detect_all(self.runner.clone()).await)
    }

    async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> jsonrpsee::core::RpcResult<StartAgentResponse> {
        self.agent.start_agent(req).await.map_err(rpc_err)
    }

    async fn send_user_message(
        &self,
        req: SendUserMessageRequest,
    ) -> jsonrpsee::core::RpcResult<()> {
        self.agent.send_user_message(req).await.map_err(rpc_err)
    }

    async fn stop_agent(&self) -> jsonrpsee::core::RpcResult<()> {
        self.agent.stop_agent().await.map_err(rpc_err)
    }
}

fn rpc_err(e: MinosError) -> ErrorObjectOwned {
    let code = match e {
        MinosError::PairingStateMismatch { .. } => -32001,
        MinosError::PairingTokenInvalid => -32002,
        MinosError::DeviceNotTrusted { .. } => -32003,
        _ => -32000,
    };
    ErrorObjectOwned::owned(code, e.to_string(), None::<()>)
}

/// JSON-RPC method-not-found code (per spec).
const RPC_METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC invalid-params code (per spec).
const RPC_INVALID_PARAMS: i32 = -32602;
/// JSON-RPC parse-error code (per spec).
const RPC_PARSE_ERROR: i32 = -32700;

/// Dispatch an opaque `Envelope::Forwarded { payload }` JSON-RPC 2.0
/// request onto a `RpcServerImpl` and return the matching JSON-RPC 2.0
/// response value (suitable for wrapping into `Envelope::Forward { payload }`).
///
/// Methods are namespaced `minos_*` per the `#[rpc(namespace = "minos")]`
/// derive on [`MinosRpc`]. The dispatcher is intentionally a small match
/// rather than a jsonrpsee `RpcModule` round-trip so we don't pull in
/// the full server runtime; any future method addition needs a new arm
/// here.
///
/// Errors map to JSON-RPC's standard error object (`-32601` for unknown
/// method, `-32602` for invalid params, `-32700` for malformed envelope).
/// Method-level errors flow through [`rpc_err`] and surface with the
/// existing `-3200x` codes.
pub async fn invoke_forwarded(payload: Value, server: &Arc<RpcServerImpl>) -> Value {
    let id = payload.get("id").cloned().unwrap_or(Value::Null);

    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return jsonrpc_error(id, RPC_PARSE_ERROR, "missing 'method'");
    };
    let params = payload.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "minos_pair" => {
            let req: PairRequest = match parse_params(&params) {
                Ok(r) => r,
                Err(msg) => return jsonrpc_error(id, RPC_INVALID_PARAMS, &msg),
            };
            into_jsonrpc(id, server.pair(req).await)
        }
        "minos_health" => into_jsonrpc(id, server.health().await),
        "minos_list_clis" => into_jsonrpc(id, server.list_clis().await),
        "minos_start_agent" => {
            let req: StartAgentRequest = match parse_params(&params) {
                Ok(r) => r,
                Err(msg) => return jsonrpc_error(id, RPC_INVALID_PARAMS, &msg),
            };
            into_jsonrpc(id, server.start_agent(req).await)
        }
        "minos_send_user_message" => {
            let req: SendUserMessageRequest = match parse_params(&params) {
                Ok(r) => r,
                Err(msg) => return jsonrpc_error(id, RPC_INVALID_PARAMS, &msg),
            };
            into_jsonrpc(id, server.send_user_message(req).await)
        }
        "minos_stop_agent" => into_jsonrpc(id, server.stop_agent().await),
        // `subscribe_events` cannot meaningfully cross a forwarded RPC
        // boundary — the peer would need a streaming subscription which
        // the envelope protocol does not model. Reject explicitly.
        other => jsonrpc_error(
            id,
            RPC_METHOD_NOT_FOUND,
            &format!("method '{other}' is not forwarded-callable"),
        ),
    }
}

/// Wrap a successful or failing `RpcResult<T>` into a JSON-RPC 2.0
/// response object (the shape the relay echoes back to the original peer
/// inside an [`Envelope::Forward`] payload).
fn into_jsonrpc<T: serde::Serialize>(id: Value, result: jsonrpsee::core::RpcResult<T>) -> Value {
    match result {
        Ok(v) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": serde_json::to_value(v).unwrap_or(Value::Null),
        }),
        Err(e) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": e.code(),
                "message": e.message(),
            },
        }),
    }
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// JSON-RPC params are conventionally either a positional array or a
/// named object. Our generated client uses the named-object form, so we
/// accept that as the authoritative shape and reject positional.
fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, String> {
    if params.is_null() {
        // Some methods take `()` — try the empty-object decode for them.
        return serde_json::from_value(Value::Object(Map::new()))
            .map_err(|e| format!("missing params; tried empty object: {e}"));
    }
    serde_json::from_value(params.clone()).map_err(|e| format!("invalid params: {e}"))
}

/// Wrap a fully formed JSON-RPC response into an `Envelope::Forward`
/// suitable for pushing back through the outbound queue. Kept as a
/// helper so the dispatch loop and tests share one phrasing.
#[must_use]
pub fn wrap_response_envelope(response: Value) -> Envelope {
    Envelope::Forward {
        version: 1,
        payload: response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_cli_detect::CommandOutcome;
    use std::time::Duration;

    /// In-test runner that satisfies the trait without forking a process.
    /// `list_clis` will receive `None`/empty stdout for every probed binary
    /// — that's fine, the dispatcher tests don't assert on the contents.
    struct NoopRunner;

    #[async_trait]
    impl CommandRunner for NoopRunner {
        async fn which(&self, _bin: &str) -> Option<String> {
            None
        }
        async fn run(
            &self,
            _bin: &str,
            _args: &[&str],
            _timeout: Duration,
        ) -> Result<CommandOutcome, MinosError> {
            Ok(CommandOutcome {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn fake_server() -> Arc<RpcServerImpl> {
        Arc::new(RpcServerImpl {
            started_at: Instant::now(),
            runner: Arc::new(NoopRunner),
            agent: Arc::new(AgentGlue::new(
                std::env::temp_dir().join("minos-rpc-test"),
                Arc::new(std::collections::HashMap::new()),
            )),
        })
    }

    #[tokio::test]
    async fn invoke_forwarded_health_returns_jsonrpc_result() {
        let server = fake_server();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "minos_health",
            "params": {},
        });
        let resp = invoke_forwarded(req, &server).await;
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        let result = &resp["result"];
        assert!(result["version"].is_string());
        assert!(result["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn invoke_forwarded_pair_returns_unauthorized_error() {
        let server = fake_server();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "minos_pair",
            "params": {
                "device_id": "00000000-0000-0000-0000-000000000000",
                "name": "x",
                "token": "tok",
            },
        });
        let resp = invoke_forwarded(req, &server).await;
        assert_eq!(resp["id"], 7);
        let err = &resp["error"];
        assert!(err.is_object(), "expected error object, got {resp}");
        let msg = err["message"].as_str().unwrap_or_default();
        assert!(
            msg.contains("backend"),
            "expected 'backend'-mentioning message, got {msg}"
        );
    }

    #[tokio::test]
    async fn invoke_forwarded_unknown_method_returns_minus_32601() {
        let server = fake_server();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "minos_does_not_exist",
            "params": {},
        });
        let resp = invoke_forwarded(req, &server).await;
        assert_eq!(resp["id"], 99);
        assert_eq!(resp["error"]["code"], RPC_METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn invoke_forwarded_missing_method_returns_parse_error() {
        let server = fake_server();
        let req = json!({ "jsonrpc": "2.0", "id": 5 });
        let resp = invoke_forwarded(req, &server).await;
        assert_eq!(resp["id"], 5);
        assert_eq!(resp["error"]["code"], RPC_PARSE_ERROR);
    }

    #[test]
    fn wrap_response_envelope_uses_forward_variant() {
        let v = json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}});
        let env = wrap_response_envelope(v.clone());
        match env {
            Envelope::Forward { version, payload } => {
                assert_eq!(version, 1);
                assert_eq!(payload, v);
            }
            other => panic!("expected Forward, got {other:?}"),
        }
    }
}
