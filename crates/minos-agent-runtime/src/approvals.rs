//! Approval helpers for codex `ServerRequest` prompts.
//!
//! The runtime forwards approval-shaped server requests to the backend/mobile
//! path and keeps the original typed request around so it can either validate
//! an explicit user decision or synthesize the schema-correct timeout reject
//! (`decline` / `denied` / empty-grant per variant).
//!
//! This module is `pub(crate)` — it is exhaustively dispatched against the
//! typed `ServerRequest` enum from `minos-codex-protocol`. New schema variants
//! become a non-exhaustive-match compile error on regeneration.

use minos_codex_protocol::{
    ApplyPatchApprovalResponse, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalResponse, ExecCommandApprovalResponse,
    FileChangeApprovalDecision, FileChangeRequestApprovalResponse, GrantedPermissionProfile,
    McpElicitationPrimitiveSchema, McpServerElicitationAction, McpServerElicitationRequestParams,
    McpServerElicitationRequestResponse, PermissionGrantScope, PermissionsRequestApprovalResponse,
    ReviewDecision, ServerRequest, ToolRequestUserInputResponse,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

pub(crate) fn is_approval_request(req: &ServerRequest) -> bool {
    matches!(
        req,
        ServerRequest::ApplyPatchApproval(_)
            | ServerRequest::ExecCommandApproval(_)
            | ServerRequest::CommandExecutionRequestApproval(_)
            | ServerRequest::FileChangeRequestApproval(_)
            | ServerRequest::PermissionsRequestApproval(_)
            | ServerRequest::ToolRequestUserInput(_)
    )
}

/// Build the typed reply payload to auto-reject an approval `ServerRequest`.
///
/// Returns `Some(value)` for the five approval-shaped variants; the caller
/// passes that value to `CodexClient::reply`. Returns `None` for non-approval
/// variants, since those are not approval prompts.
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NonApprovalContext {
    pub conversation_id: Option<String>,
}

/// Build a conservative fallback reply for non-approval `ServerRequest`s that
/// still require a client response to unblock the Codex turn.
///
/// Most replies are deliberately negative/no-input. The one positive case is
/// the built-in Minos teamwork MCP server: Codex may ask the client to satisfy
/// a form elicitation before reading conversation history, and cancelling that
/// form makes the model believe the conversation read was aborted.
pub(crate) fn auto_resolve_non_approval(
    req: &ServerRequest,
    context: NonApprovalContext,
) -> Option<serde_json::Value> {
    let value = match req {
        ServerRequest::McpServerElicitationRequest(params) => {
            serde_json::to_value(resolve_mcp_server_elicitation(params, &context))
        }
        ServerRequest::ToolRequestUserInput(_) => return None,
        ServerRequest::ApplyPatchApproval(_)
        | ServerRequest::ExecCommandApproval(_)
        | ServerRequest::CommandExecutionRequestApproval(_)
        | ServerRequest::FileChangeRequestApproval(_)
        | ServerRequest::PermissionsRequestApproval(_)
        | ServerRequest::ChatgptAuthTokensRefresh(_)
        | ServerRequest::DynamicToolCall(_) => return None,
    };
    Some(value.expect("typed non-approval fallback response serialisation is infallible"))
}

fn resolve_mcp_server_elicitation(
    params: &McpServerElicitationRequestParams,
    context: &NonApprovalContext,
) -> McpServerElicitationRequestResponse {
    if let Some(content) = minos_teamwork_form_elicitation_content(params, context) {
        return McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Accept,
            content: Some(content),
            meta: None,
        };
    }

    McpServerElicitationRequestResponse {
        action: McpServerElicitationAction::Cancel,
        content: None,
        meta: None,
    }
}

fn minos_teamwork_form_elicitation_content(
    params: &McpServerElicitationRequestParams,
    context: &NonApprovalContext,
) -> Option<Value> {
    let (server_name, requested_schema) = match params {
        McpServerElicitationRequestParams::Variant0 {
            server_name,
            requested_schema,
            ..
        } => (server_name, requested_schema),
        McpServerElicitationRequestParams::Variant1 { .. } => return None,
    };
    if server_name != "minos_teamwork" {
        return None;
    }

    let required = requested_schema.required.as_deref().unwrap_or_default();
    let mut content = Map::new();
    for (name, schema) in &requested_schema.properties {
        if let Some(value) = default_value_for_minos_teamwork_field(name, schema, context) {
            content.insert(name.clone(), value);
        }
    }

    if required.iter().any(|name| !content.contains_key(name)) {
        return None;
    }
    Some(Value::Object(content))
}

fn default_value_for_minos_teamwork_field(
    name: &str,
    schema: &McpElicitationPrimitiveSchema,
    context: &NonApprovalContext,
) -> Option<Value> {
    let normalized = normalized_field_name(name);
    if matches!(
        normalized.as_str(),
        "conversationid" | "defaultconversationid"
    ) {
        return context.conversation_id.as_ref().cloned().map(Value::String);
    }

    let schema_value = serde_json::to_value(schema).ok()?;
    if let Some(default) = schema_value
        .get("default")
        .filter(|value| !value.is_null())
        .cloned()
    {
        return Some(default);
    }

    match schema_value.get("type").and_then(Value::as_str) {
        Some("number") if normalized == "limit" => Some(Value::from(100)),
        Some("boolean")
            if matches!(
                normalized.as_str(),
                "allow" | "approve" | "approved" | "confirm" | "continue" | "read"
            ) =>
        {
            Some(Value::Bool(true))
        }
        _ => None,
    }
}

fn normalized_field_name(name: &str) -> String {
    name.chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn validate_decision(
    req: &ServerRequest,
    decision: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    match req {
        ServerRequest::ApplyPatchApproval(_) => {
            validate_typed::<ApplyPatchApprovalResponse>(decision, "applyPatchApproval")
        }
        ServerRequest::ExecCommandApproval(_) => {
            validate_typed::<ExecCommandApprovalResponse>(decision, "execCommandApproval")
        }
        ServerRequest::CommandExecutionRequestApproval(_) => {
            validate_typed::<CommandExecutionRequestApprovalResponse>(
                decision,
                "item/commandExecution/requestApproval",
            )
        }
        ServerRequest::FileChangeRequestApproval(_) => {
            validate_typed::<FileChangeRequestApprovalResponse>(
                decision,
                "item/fileChange/requestApproval",
            )
        }
        ServerRequest::PermissionsRequestApproval(_) => {
            validate_typed::<PermissionsRequestApprovalResponse>(
                decision,
                "item/permissions/requestApproval",
            )
        }
        ServerRequest::ToolRequestUserInput(_) => {
            validate_typed::<ToolRequestUserInputResponse>(decision, "item/tool/requestUserInput")
        }
        ServerRequest::McpServerElicitationRequest(_)
        | ServerRequest::ChatgptAuthTokensRefresh(_)
        | ServerRequest::DynamicToolCall(_) => {
            anyhow::bail!("server request does not accept an approval decision")
        }
    }
}

fn validate_typed<T>(
    decision: &serde_json::Value,
    method: &str,
) -> anyhow::Result<serde_json::Value>
where
    T: DeserializeOwned + Serialize,
{
    let typed: T = serde_json::from_value(decision.clone())
        .map_err(|error| anyhow::anyhow!("invalid decision for {method}: {error}"))?;
    serde_json::to_value(typed)
        .map_err(|error| anyhow::anyhow!("failed to serialize decision for {method}: {error}"))
}

/// Accept either a full ACP permission response object or a simple yes/no
/// shaped decision from the TUI, mapping onto the option ids captured when the
/// request arrived.
pub(crate) fn validate_acp_permission_decision(
    decision: &Value,
    allow_option_id: Option<&str>,
    reject_option_id: Option<&str>,
) -> anyhow::Result<Value> {
    if decision.get("outcome").is_some() {
        return validate_typed::<minos_acp_protocol::RequestPermissionResponse>(
            decision,
            "session/request_permission",
        );
    }

    let approved = decision
        .get("approved")
        .and_then(Value::as_bool)
        .or_else(|| {
            decision
                .get("decision")
                .and_then(Value::as_str)
                .map(|s| matches!(s, "accept" | "approved" | "allow" | "yes" | "y"))
        })
        .unwrap_or(false);

    let outcome = if approved {
        let option_id = allow_option_id.ok_or_else(|| {
            anyhow::anyhow!("ACP permission approve selected but no allow option was offered")
        })?;
        minos_acp_protocol::RequestPermissionOutcome::Selected {
            option_id: option_id.to_owned(),
        }
    } else if let Some(option_id) = reject_option_id {
        minos_acp_protocol::RequestPermissionOutcome::Selected {
            option_id: option_id.to_owned(),
        }
    } else {
        minos_acp_protocol::RequestPermissionOutcome::Cancelled
    };

    serde_json::to_value(minos_acp_protocol::RequestPermissionResponse { outcome })
        .map_err(|error| anyhow::anyhow!("failed to serialize ACP permission decision: {error}"))
}

/// Map a TUI decision onto a Grok `ext_method` reply body.
///
/// - `x.ai/exit_plan_mode` → `{ "outcome": "approved"|"cancelled"|"abandoned", "feedback"?: string }`
/// - `x.ai/ask_user_question` → `{ "outcome": "cancelled" }` (or full accepted payload if provided)
pub(crate) fn validate_grok_ext_method_decision(
    nested_method: &str,
    decision: &Value,
) -> anyhow::Result<Value> {
    match nested_method {
        "x.ai/exit_plan_mode" => {
            // Already a full wire response.
            if let Some(outcome) = decision.get("outcome").and_then(Value::as_str) {
                let allowed = matches!(outcome, "approved" | "cancelled" | "abandoned");
                anyhow::ensure!(
                    allowed,
                    "invalid exit_plan_mode outcome: {outcome}"
                );
                return Ok(decision.clone());
            }
            let token = decision
                .get("decision")
                .and_then(Value::as_str)
                .or_else(|| decision.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let feedback = decision
                .get("feedback")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let outcome = match token.as_str() {
                "approve" | "approved" | "yes" | "y" | "a" | "accept" => "approved",
                "abandon" | "abandoned" | "quit" | "q" => "abandoned",
                // revise / request changes / no / cancel
                _ => "cancelled",
            };
            let mut body = serde_json::json!({ "outcome": outcome });
            if outcome == "cancelled" {
                if let Some(feedback) = feedback {
                    body.as_object_mut()
                        .expect("object")
                        .insert("feedback".into(), Value::String(feedback));
                }
            }
            Ok(body)
        }
        "x.ai/ask_user_question" => {
            if decision.get("outcome").is_some() {
                return Ok(decision.clone());
            }
            Ok(serde_json::json!({ "outcome": "cancelled" }))
        }
        other => anyhow::bail!("unsupported Grok ext_method for approval: {other}"),
    }
}

/// Pick the first allow/reject option ids from an ACP permission options array.
#[must_use]
pub(crate) fn acp_permission_option_ids(params: &Value) -> (Option<String>, Option<String>) {
    let mut allow = None;
    let mut reject = None;
    let Some(options) = params.get("options").and_then(Value::as_array) else {
        return (None, None);
    };
    for option in options {
        let option_id = option
            .get("optionId")
            .or_else(|| option.get("option_id"))
            .or_else(|| option.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if option_id.is_empty() {
            continue;
        }
        let kind = option
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = option
            .get("name")
            .or_else(|| option.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let blob = format!("{kind} {name} {option_id}");
        if allow.is_none()
            && (blob.contains("allow")
                || blob.contains("approve")
                || blob.contains("accept")
                || blob.contains("proceed")
                || blob.contains("yes"))
        {
            allow = Some(option_id);
            continue;
        }
        if reject.is_none()
            && (blob.contains("reject")
                || blob.contains("deny")
                || blob.contains("decline")
                || blob.contains("cancel")
                || blob.contains("no"))
        {
            reject = Some(option_id);
        }
    }
    (allow, reject)
}

#[cfg(test)]
mod acp_permission_tests {
    use super::*;

    #[test]
    fn acp_option_ids_prefer_allow_and_reject_kinds() {
        let params = serde_json::json!({
            "options": [
                {"optionId": "proceed_once", "name": "Allow", "kind": "allow_once"},
                {"optionId": "cancel", "name": "Reject", "kind": "reject_once"}
            ]
        });
        let (allow, reject) = acp_permission_option_ids(&params);
        assert_eq!(allow.as_deref(), Some("proceed_once"));
        assert_eq!(reject.as_deref(), Some("cancel"));
    }

    #[test]
    fn acp_decision_maps_yes_to_selected_allow() {
        let reply = validate_acp_permission_decision(
            &serde_json::json!({"approved": true}),
            Some("proceed_once"),
            Some("cancel"),
        )
        .unwrap();
        assert_eq!(
            reply["outcome"]["outcome"].as_str(),
            Some("selected")
        );
        assert_eq!(reply["outcome"]["optionId"].as_str(), Some("proceed_once"));
    }

    #[test]
    fn acp_decision_maps_no_to_reject_option_or_cancelled() {
        let reply = validate_acp_permission_decision(
            &serde_json::json!({"decision": "decline"}),
            Some("proceed_once"),
            Some("cancel"),
        )
        .unwrap();
        assert_eq!(reply["outcome"]["optionId"].as_str(), Some("cancel"));

        let cancelled = validate_acp_permission_decision(
            &serde_json::json!({"approved": false}),
            Some("proceed_once"),
            None,
        )
        .unwrap();
        assert_eq!(cancelled["outcome"]["outcome"].as_str(), Some("cancelled"));
    }
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
    fn grok_exit_plan_decision_maps_tokens_to_outcomes() {
        let approved =
            validate_grok_ext_method_decision("x.ai/exit_plan_mode", &json!({ "decision": "approve" }))
                .unwrap();
        assert_eq!(approved["outcome"], "approved");

        let revise =
            validate_grok_ext_method_decision("x.ai/exit_plan_mode", &json!({ "decision": "revise" }))
                .unwrap();
        assert_eq!(revise["outcome"], "cancelled");

        let abandon =
            validate_grok_ext_method_decision("x.ai/exit_plan_mode", &json!({ "decision": "abandon" }))
                .unwrap();
        assert_eq!(abandon["outcome"], "abandoned");

        let passthrough = validate_grok_ext_method_decision(
            "x.ai/exit_plan_mode",
            &json!({ "outcome": "cancelled", "feedback": "add tests" }),
        )
        .unwrap();
        assert_eq!(passthrough["feedback"], "add tests");
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
    fn request_user_input_is_pending_but_not_rejected() {
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
        assert!(is_approval_request(&req));
    }

    #[test]
    fn auto_resolve_mcp_elicitation_returns_cancel() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "mcpServer/elicitation/request",
            "params": {
                "elicitationId": "elic-1",
                "message": "Open this URL",
                "mode": "url",
                "serverName": "minos_teamwork",
                "threadId": "thr-1",
                "turnId": "turn-1",
                "url": "https://example.com"
            }
        }))
        .expect("mcp elicitation params decode");
        let reply = auto_resolve_non_approval(&req, NonApprovalContext::default())
            .expect("elicitation should auto-cancel");
        assert_eq!(reply, json!({ "action": "cancel" }));
    }

    #[test]
    fn auto_resolve_minos_teamwork_form_elicitation_accepts_default_conversation() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "mcpServer/elicitation/request",
            "params": {
                "message": "Select the Minos conversation to read",
                "mode": "form",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "conversation_id": { "type": "string" }
                    },
                    "required": ["conversation_id"]
                },
                "serverName": "minos_teamwork",
                "threadId": "thr-1",
                "turnId": "turn-1"
            }
        }))
        .expect("mcp elicitation params decode");
        let reply = auto_resolve_non_approval(
            &req,
            NonApprovalContext {
                conversation_id: Some("conversation-main".into()),
            },
        )
        .expect("minos conversation elicitation should auto-accept");
        assert_eq!(
            reply,
            json!({
                "action": "accept",
                "content": { "conversation_id": "conversation-main" }
            })
        );
    }

    #[test]
    fn auto_resolve_tool_request_user_input_returns_none() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "item/tool/requestUserInput",
            "params": {
                "itemId": "item-1",
                "questions": [{
                    "header": "Need input",
                    "id": "q1",
                    "question": "Pick one"
                }],
                "threadId": "thr-1",
                "turnId": "turn-1"
            }
        }))
        .expect("tool/requestUserInput params decode");
        assert!(auto_resolve_non_approval(&req, NonApprovalContext::default()).is_none());
    }


    #[test]
    fn validate_decision_accepts_tool_request_user_input_answer_shape() {
        let req: ServerRequest = serde_json::from_value(json!({
            "method": "item/tool/requestUserInput",
            "params": {
                "itemId": "item-1",
                "questions": [{
                    "header": "Need input",
                    "id": "q1",
                    "question": "Pick one"
                }],
                "threadId": "thr-1",
                "turnId": "turn-1"
            }
        }))
        .expect("tool/requestUserInput params decode");
        let reply = validate_decision(
            &req,
            &json!({ "answers": { "q1": { "answers": ["choice"] } } }),
        )
        .expect("valid tool input response");
        assert_eq!(reply["answers"]["q1"]["answers"][0], json!("choice"));
    }

    #[test]
    fn validate_decision_rejects_wrong_shape() {
        let req = ServerRequest::CommandExecutionRequestApproval(dummy_command_exec_params());
        let err = validate_decision(&req, &json!({ "permissions": {} })).unwrap_err();
        assert!(err.to_string().contains("invalid decision"));
    }
}
