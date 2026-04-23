//! Integration-scope mirror of the unit-level translation table in
//! `src/translate.rs`.
//!
//! This file exists as a separate cargo test target so the plan can run
//! `cargo test -p minos-agent-runtime --test translate_table` in isolation
//! during debugging. All assertions duplicate the unit tests by design —
//! if one lane passes and the other fails, the bug is in the crate's public
//! surface rather than in the `translate` module itself.

use minos_agent_runtime::translate_notification;
use minos_domain::AgentEvent;
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
fn reasoning_text_delta_becomes_reasoning() {
    let ev = translate_notification("item/reasoning/textDelta", &json!({"delta": "why"}));
    assert_eq!(ev, AgentEvent::Reasoning { text: "why".into() });
}

#[test]
fn reasoning_summary_text_delta_becomes_reasoning() {
    let ev = translate_notification("item/reasoning/summaryTextDelta", &json!({"delta": "sum"}));
    assert_eq!(ev, AgentEvent::Reasoning { text: "sum".into() });
}

#[test]
fn command_execution_output_delta_becomes_shell_tool_result() {
    let ev = translate_notification(
        "item/commandExecution/outputDelta",
        &json!({"chunk": "out"}),
    );
    assert_eq!(
        ev,
        AgentEvent::ToolResult {
            name: "shell".into(),
            output: "out".into(),
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
fn turn_completed_becomes_done_exit_zero() {
    let ev = translate_notification("turn/completed", &json!({}));
    assert_eq!(ev, AgentEvent::Done { exit_code: 0 });
}

#[test]
fn plan_delta_becomes_raw() {
    let ev = translate_notification("item/plan/delta", &json!({"step": "compile"}));
    assert_eq!(
        ev,
        AgentEvent::Raw {
            kind: "item/plan/delta".into(),
            payload_json: r#"{"step":"compile"}"#.into(),
        }
    );
}

#[test]
fn token_usage_updated_becomes_raw() {
    let ev = translate_notification(
        "thread/tokenUsage/updated",
        &json!({"input_tokens": 1, "output_tokens": 2}),
    );
    assert_eq!(
        ev,
        AgentEvent::Raw {
            kind: "thread/tokenUsage/updated".into(),
            payload_json: r#"{"input_tokens":1,"output_tokens":2}"#.into(),
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
