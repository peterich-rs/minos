//! Round-trip fixtures: each JSON file under `tests/fixtures/<group>/` is
//! parsed into the target typed struct and re-serialised; the result must
//! match the input modulo serde-default field elision.

use minos_codex_protocol::{
    AgentMessageDeltaNotification, CommandExecutionRequestApprovalParams, InitializeParams,
    ThreadStartResponse,
};
use pretty_assertions::assert_eq;
use serde::{de::DeserializeOwned, Serialize};

fn round_trip<T: DeserializeOwned + Serialize>(fixture_path: &str) {
    let raw = std::fs::read_to_string(fixture_path)
        .unwrap_or_else(|e| panic!("read {fixture_path}: {e}"));
    let original: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {fixture_path}: {e}"));
    let typed: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|e| panic!("typed deserialise {fixture_path}: {e}"));
    let re_encoded =
        serde_json::to_value(&typed).unwrap_or_else(|e| panic!("re-serialise {fixture_path}: {e}"));
    // Allow added optional fields (Some(default) appearing in re_encoded that
    // the input omitted) — the typed value must just be a superset of the input.
    let trimmed_re = strip_keys_not_in(re_encoded.clone(), &original);
    assert_eq!(
        trimmed_re, original,
        "round-trip diverged for {fixture_path}"
    );
}

/// Drop keys from `value` that don't appear in `template` at the same path.
/// Lets fixtures omit optional fields without the round-trip flagging the
/// `Some(default)` re-encoding as drift.
fn strip_keys_not_in(value: serde_json::Value, template: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (value, template) {
        (Value::Object(mut v), Value::Object(t)) => {
            v.retain(|k, _| t.contains_key(k));
            for (k, child) in &mut v {
                if let Some(t_child) = t.get(k) {
                    *child = strip_keys_not_in(child.take(), t_child);
                }
            }
            Value::Object(v)
        }
        (v, _) => v,
    }
}

#[test]
fn initialize_params_round_trip() {
    round_trip::<InitializeParams>("tests/fixtures/initialize_params.json");
}

#[test]
fn thread_start_response_round_trip() {
    round_trip::<ThreadStartResponse>("tests/fixtures/thread_start_response.json");
}

#[test]
fn agent_message_delta_notification_round_trip() {
    round_trip::<AgentMessageDeltaNotification>(
        "tests/fixtures/agent_message_delta_notification.json",
    );
}

#[test]
fn command_execution_request_approval_params_round_trip() {
    round_trip::<CommandExecutionRequestApprovalParams>(
        "tests/fixtures/command_execution_request_approval_params.json",
    );
}
