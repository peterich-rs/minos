//! codex JSON-RPC notification → [`AgentEvent`] mapping.
//!
//! The mapping table is the **single source of truth** for which codex
//! notifications get typed treatment. Everything outside the table falls
//! through to [`AgentEvent::Raw`] so the bridge is forward-compatible across
//! codex protocol churn (ADR 0010, spec §5.2).
//!
//! This function is intentionally pure: no logging, no I/O, no side effects.
//! Every mapping rule is covered by a unit test below plus the table test in
//! `tests/translate_table.rs`.

use minos_domain::AgentEvent;

/// Translate a codex `method` + `params` pair into an [`AgentEvent`].
///
/// Unknown methods produce [`AgentEvent::Raw`] carrying the method name as
/// `kind` and the stringified `params` as `payload_json`. See spec §5.2 for
/// the full mapping table.
#[must_use]
pub fn translate_notification(method: &str, params: &serde_json::Value) -> AgentEvent {
    match method {
        "item/agentMessage/delta" => AgentEvent::TokenChunk {
            text: params
                .get("delta")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => AgentEvent::Reasoning {
            text: params
                .get("delta")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "item/commandExecution/outputDelta" => AgentEvent::ToolResult {
            name: "shell".to_string(),
            output: params
                .get("chunk")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "item/mcpToolCall/progress" => {
            let phase = params.get("phase").and_then(serde_json::Value::as_str);
            if phase == Some("started") {
                AgentEvent::ToolCall {
                    name: params
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    args_json: serde_json::to_string(
                        params.get("arguments").unwrap_or(&serde_json::Value::Null),
                    )
                    .unwrap_or_default(),
                }
            } else {
                // Stringify the `result` field. If it's a string, emit the
                // unquoted contents; otherwise serialize to JSON text so the
                // consumer has the full payload.
                let result = params.get("result").unwrap_or(&serde_json::Value::Null);
                let output = match result.as_str() {
                    Some(s) => s.to_string(),
                    None => serde_json::to_string(result).unwrap_or_default(),
                };
                AgentEvent::ToolResult {
                    name: params
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    output,
                }
            }
        }
        "turn/completed" => {
            // codex does not expose non-zero exit codes; treat any non-empty
            // `error` field as failure (-1).
            let error_present = params
                .get("error")
                .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.is_empty()));
            AgentEvent::Done {
                exit_code: if error_present { -1 } else { 0 },
            }
        }
        other => AgentEvent::Raw {
            kind: other.to_string(),
            payload_json: serde_json::to_string(params).unwrap_or_default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_message_delta_becomes_token_chunk() {
        let ev = translate_notification("item/agentMessage/delta", &json!({"delta": "Hello"}));
        assert_eq!(
            ev,
            AgentEvent::TokenChunk {
                text: "Hello".into()
            }
        );
    }

    #[test]
    fn agent_message_delta_missing_delta_yields_empty_text() {
        let ev = translate_notification("item/agentMessage/delta", &json!({}));
        assert_eq!(
            ev,
            AgentEvent::TokenChunk {
                text: String::new()
            }
        );
    }

    #[test]
    fn reasoning_text_delta_becomes_reasoning() {
        let ev = translate_notification("item/reasoning/textDelta", &json!({"delta": "because"}));
        assert_eq!(
            ev,
            AgentEvent::Reasoning {
                text: "because".into()
            }
        );
    }

    #[test]
    fn reasoning_summary_text_delta_becomes_reasoning() {
        let ev = translate_notification(
            "item/reasoning/summaryTextDelta",
            &json!({"delta": "summary"}),
        );
        assert_eq!(
            ev,
            AgentEvent::Reasoning {
                text: "summary".into()
            }
        );
    }

    #[test]
    fn command_execution_output_delta_becomes_shell_tool_result() {
        let ev = translate_notification(
            "item/commandExecution/outputDelta",
            &json!({"chunk": "line one\n"}),
        );
        assert_eq!(
            ev,
            AgentEvent::ToolResult {
                name: "shell".into(),
                output: "line one\n".into(),
            }
        );
    }

    #[test]
    fn mcp_tool_call_started_becomes_tool_call() {
        let ev = translate_notification(
            "item/mcpToolCall/progress",
            &json!({"phase": "started", "name": "grep", "arguments": {"pattern": "foo"}}),
        );
        assert_eq!(
            ev,
            AgentEvent::ToolCall {
                name: "grep".into(),
                args_json: r#"{"pattern":"foo"}"#.into(),
            }
        );
    }

    #[test]
    fn mcp_tool_call_finished_becomes_tool_result() {
        let ev = translate_notification(
            "item/mcpToolCall/progress",
            &json!({"phase": "completed", "name": "grep", "result": "42 matches"}),
        );
        assert_eq!(
            ev,
            AgentEvent::ToolResult {
                name: "grep".into(),
                output: "42 matches".into(),
            }
        );
    }

    #[test]
    fn turn_completed_without_error_is_exit_zero() {
        let ev = translate_notification("turn/completed", &json!({}));
        assert_eq!(ev, AgentEvent::Done { exit_code: 0 });
    }

    #[test]
    fn turn_completed_with_error_is_exit_minus_one() {
        let ev = translate_notification("turn/completed", &json!({"error": "boom"}));
        assert_eq!(ev, AgentEvent::Done { exit_code: -1 });
    }

    #[test]
    fn unknown_method_becomes_raw_plan_delta() {
        let params = json!({"step": "compile"});
        let ev = translate_notification("item/plan/delta", &params);
        assert_eq!(
            ev,
            AgentEvent::Raw {
                kind: "item/plan/delta".into(),
                payload_json: r#"{"step":"compile"}"#.into(),
            }
        );
    }

    #[test]
    fn unknown_method_becomes_raw_token_usage() {
        let params = json!({"input_tokens": 42, "output_tokens": 10});
        let ev = translate_notification("thread/tokenUsage/updated", &params);
        assert_eq!(
            ev,
            AgentEvent::Raw {
                kind: "thread/tokenUsage/updated".into(),
                payload_json: r#"{"input_tokens":42,"output_tokens":10}"#.into(),
            }
        );
    }

    #[test]
    fn unknown_method_becomes_raw_foo_bar_baz() {
        let ev = translate_notification("foo/bar/baz", &json!({}));
        assert_eq!(
            ev,
            AgentEvent::Raw {
                kind: "foo/bar/baz".into(),
                payload_json: "{}".into(),
            }
        );
    }
}
