//! Golden-file checks. Failure here means the JSON shape exposed across the
//! UniFFI / frb boundary would silently change, breaking Swift / Dart consumers.

use minos_domain::{AgentDescriptor, AgentEvent, AgentName, AgentStatus, ConnectionState};

#[test]
fn agent_descriptor_matches_golden() {
    let golden = include_str!("golden/agent_descriptor.json");
    let parsed: AgentDescriptor = serde_json::from_str(golden).unwrap();
    assert_eq!(
        parsed,
        AgentDescriptor {
            name: AgentName::Codex,
            path: Some("/usr/local/bin/codex".into()),
            version: Some("0.18.2".into()),
            status: AgentStatus::Ok,
        }
    );
    let reserialized = serde_json::to_value(parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
    assert_eq!(reserialized, expected);
}

#[test]
fn connection_state_reconnecting_matches_golden() {
    let golden = include_str!("golden/connection_state.json");
    let parsed: ConnectionState = serde_json::from_str(golden).unwrap();
    assert_eq!(parsed, ConnectionState::Reconnecting { attempt: 7 });
    let reserialized = serde_json::to_value(parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
    assert_eq!(reserialized, expected);
}

#[test]
fn agent_event_raw_matches_golden() {
    let golden = include_str!("golden/agent_event_raw.json");
    let parsed: AgentEvent = serde_json::from_str(golden).unwrap();
    assert_eq!(
        parsed,
        AgentEvent::Raw {
            kind: "item/plan/delta".into(),
            payload_json: r#"{"step":"compile"}"#.into(),
        }
    );
    let reserialized = serde_json::to_value(parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
    assert_eq!(reserialized, expected);
}
