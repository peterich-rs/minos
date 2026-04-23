//! Golden-fixture tests for [`minos_protocol::Envelope`].
//!
//! Each test in this file loads a hand-authored JSON fixture under
//! `tests/golden/envelope/`, parses it into an `Envelope`, re-serialises
//! to a `serde_json::Value`, and asserts the round-trip is byte-for-byte
//! identical (compared as structured JSON, so key-order and whitespace
//! differences don't matter). It also spot-checks one representative
//! field per variant so mis-wired deserialisation is caught.
//!
//! Why a separate integration test rather than inline unit tests: these
//! fixtures are the wire contract. PRs that change the envelope shape
//! MUST update the corresponding fixture — the diff makes the change
//! reviewable. Keeping them in `tests/` signals "here be dragons" more
//! loudly than an inline `#[cfg(test)]` module, and lets us share a
//! single `fixture()` helper without `pub`-exposing it.

use minos_protocol::{Envelope, EventKind, LocalRpcMethod, LocalRpcOutcome};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/envelope")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Parse `name`, re-serialise via `Envelope`, and check round-trip
/// equivalence as `serde_json::Value`. Returns the parsed envelope so
/// callers can add variant-specific spot checks.
fn round_trip(name: &str) -> Envelope {
    let raw = fixture(name);
    let env: Envelope = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("deserialise {name}: {e}\nraw:\n{raw}"));
    let reparsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let reserialised = serde_json::to_value(&env).unwrap();
    assert_eq!(reparsed, reserialised, "round-trip mismatch for {name}");
    env
}

#[test]
fn local_rpc_ping() {
    let env = round_trip("local_rpc_ping.json");
    let Envelope::LocalRpc {
        version,
        id,
        method,
        params,
    } = env
    else {
        panic!("expected LocalRpc");
    };
    assert_eq!(version, 1);
    assert_eq!(id, 1);
    assert_eq!(method, LocalRpcMethod::Ping);
    assert_eq!(params, serde_json::json!({}));
}

#[test]
fn local_rpc_pair() {
    let env = round_trip("local_rpc_pair.json");
    let Envelope::LocalRpc {
        version,
        id,
        method,
        params,
    } = env
    else {
        panic!("expected LocalRpc");
    };
    assert_eq!(version, 1);
    assert_eq!(id, 7);
    assert_eq!(method, LocalRpcMethod::Pair);
    assert_eq!(
        params["device_name"],
        serde_json::Value::String("iPhone of fan".into())
    );
}

#[test]
fn local_rpc_response_ok() {
    let env = round_trip("local_rpc_response_ok.json");
    let Envelope::LocalRpcResponse {
        version,
        id,
        outcome,
    } = env
    else {
        panic!("expected LocalRpcResponse");
    };
    assert_eq!(version, 1);
    assert_eq!(id, 42);
    let LocalRpcOutcome::Ok { result } = outcome else {
        panic!("expected Ok outcome");
    };
    assert_eq!(result["expires_at"], "2026-04-23T12:00:00Z");
}

#[test]
fn local_rpc_response_err() {
    let env = round_trip("local_rpc_response_err.json");
    let Envelope::LocalRpcResponse { outcome, .. } = env else {
        panic!("expected LocalRpcResponse");
    };
    let LocalRpcOutcome::Err { error } = outcome else {
        panic!("expected Err outcome");
    };
    assert_eq!(error.code, "pairing_token_invalid");
    assert_eq!(error.message, "token expired");
}

#[test]
fn forward() {
    let env = round_trip("forward.json");
    let Envelope::Forward { version, payload } = env else {
        panic!("expected Forward");
    };
    assert_eq!(version, 1);
    assert_eq!(payload["method"], "list_clis");
}

#[test]
fn forwarded() {
    let env = round_trip("forwarded.json");
    let Envelope::Forwarded {
        version,
        from,
        payload,
    } = env
    else {
        panic!("expected Forwarded");
    };
    assert_eq!(version, 1);
    // `DeviceId` is transparent over Uuid; fixture uses a deterministic UUID.
    assert_eq!(from.0.to_string(), "11111111-2222-3333-4444-555555555555");
    assert_eq!(payload["jsonrpc"], "2.0");
}

#[test]
fn event_paired() {
    let env = round_trip("event_paired.json");
    let Envelope::Event { version, event } = env else {
        panic!("expected Event");
    };
    assert_eq!(version, 1);
    let EventKind::Paired {
        peer_name,
        your_device_secret,
        ..
    } = event
    else {
        panic!("expected Paired");
    };
    assert_eq!(peer_name, "Mac-mini");
    // DeviceSecret is transparent on the wire (redaction is Debug/Display only).
    assert_eq!(
        your_device_secret.as_str(),
        "Sg3AfM5V0_3Vp1IvGxPzWwXhE-3HXfLQyIJzj6TZAmE"
    );
}

#[test]
fn event_peer_online() {
    let env = round_trip("event_peer_online.json");
    let Envelope::Event { event, .. } = env else {
        panic!("expected Event");
    };
    assert!(matches!(event, EventKind::PeerOnline { .. }));
}

#[test]
fn event_peer_offline() {
    let env = round_trip("event_peer_offline.json");
    let Envelope::Event { event, .. } = env else {
        panic!("expected Event");
    };
    assert!(matches!(event, EventKind::PeerOffline { .. }));
}

#[test]
fn event_unpaired() {
    let env = round_trip("event_unpaired.json");
    let Envelope::Event { event, .. } = env else {
        panic!("expected Event");
    };
    assert!(matches!(event, EventKind::Unpaired));
}

#[test]
fn event_server_shutdown() {
    let env = round_trip("event_server_shutdown.json");
    let Envelope::Event { event, .. } = env else {
        panic!("expected Event");
    };
    assert!(matches!(event, EventKind::ServerShutdown));
}
