use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest<P> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub method: String,
    pub params: P,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse<R> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub result: R,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub error: JsonRpcErrorPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorPayload {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification<P> {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: P,
}

const JSONRPC_VERSION: &str = "2.0";

pub fn make_request(
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "method": method,
        "params": params,
    })
}

pub fn make_notification(method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params,
    })
}

pub fn make_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    })
}

pub fn make_error(
    id: serde_json::Value,
    code: i64,
    message: impl Into<String>,
    data: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut error = serde_json::json!({
        "code": code,
        "message": message.into(),
    });
    if let Some(data) = data {
        error["data"] = data;
    }
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn make_request_includes_jsonrpc_field() {
        let frame = make_request(serde_json::json!(1), "initialize", serde_json::json!({}));
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], 1);
        assert_eq!(frame["method"], "initialize");
    }

    #[test]
    fn make_notification_omits_id_field() {
        let frame = make_notification("session/cancel", serde_json::json!({}));
        assert_eq!(frame["jsonrpc"], "2.0");
        assert!(frame.get("id").is_none(), "notifications must not carry id");
        assert_eq!(frame["method"], "session/cancel");
    }

    #[test]
    fn make_response_includes_jsonrpc_field() {
        let frame = make_response(
            serde_json::json!("req-1"),
            serde_json::json!({"outcome": "allow"}),
        );
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], "req-1");
        assert_eq!(frame["result"]["outcome"], "allow");
    }

    #[test]
    fn make_error_includes_jsonrpc_error_payload() {
        let frame = make_error(serde_json::json!("req-2"), -32601, "not supported", None);
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], "req-2");
        assert_eq!(frame["error"]["code"], -32601);
        assert_eq!(frame["error"]["message"], "not supported");
    }
}
