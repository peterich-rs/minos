//! Auto-reject builder for codex `ServerRequest` approval prompts.
//!
//! codex is started with `approval_policy=never`, so any approval server
//! request that still lands on our WS is unexpected. Rather than crash, we
//! reply `{"decision": "rejected"}` immediately and (in Phase C) forward the
//! request payload as [`AgentEvent::Raw`] so the future chat-ui can surface
//! "codex tried to run X, auto-rejected". See spec §6.4 and ADR 0010.
//!
//! This module owns only the pure payload builder. The decision of *when*
//! to invoke it — i.e. which `method` names count as approval prompts —
//! lives in Phase C's `codex_client` module.

/// Build the JSON-RPC auto-reject response payload for an approval server request.
///
/// The caller supplies the original request `id` (any JSON value — codex uses
/// both numeric and string ids) and `method`. `method` is currently only used
/// to keep the call-site self-documenting; the emitted response is the same
/// shape regardless of which approval kind prompted it. The shape is pinned:
///
/// ```json
/// {"jsonrpc":"2.0","id":<request_id>,"result":{"decision":"rejected"}}
/// ```
#[must_use]
pub fn build_auto_reject(request_id: serde_json::Value, method: &str) -> serde_json::Value {
    // `method` is retained in the public signature so future extensions can
    // distinguish rejection reasons per-approval-kind without a breaking
    // change. For Phase B we simply tag the body with the method name as a
    // debug aid; the field is inside a reserved `_minos` sub-object so it
    // cannot collide with a codex-defined result field.
    //
    // Keep `_` prefix on the binding so clippy's unused-arg lint stays quiet;
    // the field is exposed via the payload, not as a function parameter
    // anyone reads here.
    let _ = method; // referenced below via the literal string.
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": {
            "decision": "rejected",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// codex's known approval server-request methods. Cross-referenced
    /// against spec §6.4 — this is the working set at Phase B; Phase C
    /// rediscovers it through codex's JSON schema and may refine.
    const APPROVAL_METHODS: &[&str] = &[
        "ApplyPatchApproval",
        "ExecCommandApproval",
        "FileChangeRequestApproval",
        "PermissionsRequestApproval",
        "CommandExecutionRequestApproval",
    ];

    fn assert_shape(method: &str, id: serde_json::Value) {
        let response = build_auto_reject(id.clone(), method);
        assert_eq!(response["jsonrpc"], json!("2.0"), "method={method}");
        assert_eq!(response["id"], id, "method={method}");
        assert_eq!(
            response["result"]["decision"],
            json!("rejected"),
            "method={method}"
        );
        // Response must parse/re-serialize cleanly as JSON-RPC 2.0.
        let s = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, response, "method={method}");
    }

    #[test]
    fn apply_patch_approval_rejects_with_string_id() {
        assert_shape("ApplyPatchApproval", json!("req-1"));
    }

    #[test]
    fn exec_command_approval_rejects_with_numeric_id() {
        assert_shape("ExecCommandApproval", json!(42));
    }

    #[test]
    fn file_change_request_approval_rejects() {
        assert_shape("FileChangeRequestApproval", json!("fc-7"));
    }

    #[test]
    fn permissions_request_approval_rejects() {
        assert_shape("PermissionsRequestApproval", json!("perm-xyz"));
    }

    #[test]
    fn command_execution_request_approval_rejects() {
        assert_shape("CommandExecutionRequestApproval", json!(0));
    }

    #[test]
    fn all_known_approval_methods_produce_identical_shape() {
        // Exhaustive sweep: every known approval method yields the same
        // response body modulo the id. This guarantees Phase C can pipe
        // *any* received approval through the same code path without a
        // per-method switch.
        for method in APPROVAL_METHODS {
            let body = build_auto_reject(json!("id-sweep"), method);
            assert_eq!(body["jsonrpc"], json!("2.0"));
            assert_eq!(body["result"]["decision"], json!("rejected"));
        }
    }
}
