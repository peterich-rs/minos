//! JSON-RPC 2.0 envelope shapes as codex's app-server emits them.
//!
//! Codex omits the `"jsonrpc": "2.0"` discriminator on its requests/responses
//! (see `schemas/JSONRPCRequest.json`). We accept either shape on inbound and
//! emit without the field on outbound to mirror remote behavior.

use serde::{Deserialize, Serialize};

/// A JSON-RPC request: `{ id, method, params }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest<P> {
    pub id: serde_json::Value,
    pub method: String,
    pub params: P,
}

/// A JSON-RPC response: `{ id, result }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse<R> {
    pub id: serde_json::Value,
    pub result: R,
}

/// A JSON-RPC error response: `{ id, error: { code, message, data? } }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub id: serde_json::Value,
    pub error: JsonRpcErrorPayload,
}

/// The `error` object inside a `JsonRpcError`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorPayload {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn request_round_trips_with_object_params() {
        let r: JsonRpcRequest<serde_json::Value> = JsonRpcRequest {
            id: json!("req-1"),
            method: "thread/start".into(),
            params: json!({ "cwd": "/tmp/x" }),
        };
        let bytes = serde_json::to_vec(&r).unwrap();
        let back: JsonRpcRequest<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.id, json!("req-1"));
        assert_eq!(back.method, "thread/start");
        assert_eq!(back.params, json!({ "cwd": "/tmp/x" }));
    }

    #[test]
    fn response_round_trips_with_typed_result() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Reply {
            ok: bool,
        }
        let r: JsonRpcResponse<Reply> = JsonRpcResponse {
            id: json!(7),
            result: Reply { ok: true },
        };
        let bytes = serde_json::to_vec(&r).unwrap();
        let back: JsonRpcResponse<Reply> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.id, json!(7));
        assert_eq!(back.result, Reply { ok: true });
    }

    #[test]
    fn error_payload_omits_data_field_when_none() {
        let e = JsonRpcError {
            id: json!("req-2"),
            error: JsonRpcErrorPayload {
                code: -32000,
                message: "boom".into(),
                data: None,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(!s.contains("\"data\""), "expected `data` omitted: {s}");
    }
}
