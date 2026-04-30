//! Auto-reject builder for codex `ServerRequest` approval prompts.
//!
//! codex is started with `approval_policy=never`, so any approval server
//! request that still lands on our pump is unexpected. Rather than crash, we
//! reply with the schema-correct typed reject (`decline` / `denied` /
//! empty-grant per variant), and forward the original payload as a synthetic
//! `server_request/<name>` `RawIngest` (unchanged from before — see
//! `runtime.rs::event_pump_loop`).
//!
//! This module is `pub(crate)` — it is exhaustively dispatched against the
//! typed `ServerRequest` enum from `minos-codex-protocol`. New schema variants
//! become a non-exhaustive-match compile error on regeneration.

use minos_codex_protocol::{
    ApplyPatchApprovalResponse, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalResponse, ExecCommandApprovalResponse,
    FileChangeApprovalDecision, FileChangeRequestApprovalResponse, GrantedPermissionProfile,
    PermissionGrantScope, PermissionsRequestApprovalResponse, ReviewDecision, ServerRequest,
};

/// Build the typed reply payload to auto-reject an approval `ServerRequest`.
///
/// Returns `Some(value)` for the five approval-shaped variants; the caller
/// passes that value to `CodexClient::reply`. Returns `None` for non-approval
/// variants (`item/tool/requestUserInput`, `mcpServer/elicitation/request`,
/// `account/chatgptAuthTokens/refresh`, `item/tool/call` / DynamicToolCall) —
/// the runtime warns and does not reply, since those are not approval prompts.
///
/// Reject choice per variant:
/// - `decline` for `CommandExecution` / `FileChange` (agent continues turn).
/// - `denied` for legacy v1 `ApplyPatchApproval` / `ExecCommandApproval`.
/// - empty `GrantedPermissionProfile` for `PermissionsRequestApproval` (which
///   has no `decision` field at all in its response schema).
pub(crate) fn auto_reject(req: &ServerRequest) -> Option<serde_json::Value> {
    let value = match req {
        ServerRequest::ApplyPatchApproval(_) => serde_json::to_value(ApplyPatchApprovalResponse {
            decision: ReviewDecision::Denied,
        }),
        ServerRequest::ExecCommandApproval(_) => {
            serde_json::to_value(ExecCommandApprovalResponse {
                decision: ReviewDecision::Denied,
            })
        }
        ServerRequest::CommandExecutionRequestApproval(_) => {
            serde_json::to_value(CommandExecutionRequestApprovalResponse {
                decision: CommandExecutionApprovalDecision::Decline,
            })
        }
        ServerRequest::FileChangeRequestApproval(_) => {
            serde_json::to_value(FileChangeRequestApprovalResponse {
                decision: FileChangeApprovalDecision::Decline,
            })
        }
        ServerRequest::PermissionsRequestApproval(_) => {
            serde_json::to_value(PermissionsRequestApprovalResponse {
                permissions: GrantedPermissionProfile::default(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: None,
            })
        }
        ServerRequest::ToolRequestUserInput(_)
        | ServerRequest::McpServerElicitationRequest(_)
        | ServerRequest::ChatgptAuthTokensRefresh(_)
        | ServerRequest::DynamicToolCall(_) => return None,
    };
    Some(value.expect("typed approval response serialisation is infallible"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_codex_protocol::{
        CommandExecutionRequestApprovalParams, FileChangeRequestApprovalParams, ServerRequest,
    };
    use serde_json::json;

    fn dummy_command_exec_params() -> CommandExecutionRequestApprovalParams {
        CommandExecutionRequestApprovalParams {
            approval_id: None,
            command: None,
            command_actions: None,
            cwd: None,
            item_id: "item-1".into(),
            network_approval_context: None,
            proposed_execpolicy_amendment: None,
            proposed_network_policy_amendments: None,
            reason: None,
            thread_id: "thr-1".into(),
            turn_id: "turn-1".into(),
        }
    }

    fn dummy_file_change_params() -> FileChangeRequestApprovalParams {
        FileChangeRequestApprovalParams {
            grant_root: None,
            item_id: "item-1".into(),
            reason: None,
            thread_id: "thr-1".into(),
            turn_id: "turn-1".into(),
        }
    }

    #[test]
    fn auto_reject_command_execution_returns_typed_decline() {
        let req = ServerRequest::CommandExecutionRequestApproval(dummy_command_exec_params());
        let reply = auto_reject(&req).expect("approval should auto-reject");
        assert_eq!(reply["decision"], json!("decline"));
    }

    #[test]
    fn auto_reject_file_change_returns_typed_decline() {
        let req = ServerRequest::FileChangeRequestApproval(dummy_file_change_params());
        let reply = auto_reject(&req).expect("approval should auto-reject");
        assert_eq!(reply["decision"], json!("decline"));
    }

    #[test]
    fn auto_reject_apply_patch_returns_typed_denied() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "applyPatchApproval",
            "params": {
                "callId": "call-1",
                "conversationId": "conv-1",
                "fileChanges": {}
            }
        }))
        .expect("apply-patch params decode");
        let reply = auto_reject(&req).expect("approval should auto-reject");
        assert_eq!(reply["decision"], json!("denied"));
    }

    #[test]
    fn auto_reject_exec_command_returns_typed_denied() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "execCommandApproval",
            "params": {
                "callId": "call-1",
                "command": ["ls"],
                "conversationId": "conv-1",
                "cwd": "/tmp",
                "parsedCmd": []
            }
        }))
        .expect("exec-command params decode");
        let reply = auto_reject(&req).expect("approval should auto-reject");
        assert_eq!(reply["decision"], json!("denied"));
    }

    #[test]
    fn auto_reject_permissions_returns_empty_grant() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "item/permissions/requestApproval",
            "params": {
                "cwd": "/tmp",
                "itemId": "item-1",
                "permissions": {},
                "threadId": "thr-1",
                "turnId": "turn-1"
            }
        }))
        .expect("permissions params decode");
        let reply = auto_reject(&req).expect("permissions should auto-reject");
        assert!(
            reply.get("permissions").is_some(),
            "permissions field required"
        );
    }

    #[test]
    fn auto_reject_tool_request_user_input_returns_none() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "item/tool/requestUserInput",
            "params": {
                "itemId": "item-1",
                "questions": [],
                "threadId": "thr-1",
                "turnId": "turn-1"
            }
        }))
        .expect("tool/requestUserInput params decode");
        assert!(
            auto_reject(&req).is_none(),
            "non-approval requests must not auto-reject",
        );
    }
}
