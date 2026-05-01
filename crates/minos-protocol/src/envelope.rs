//! WebSocket envelope for the Minos relay.
//!
//! Every text frame over the relay's `/devices` endpoint is exactly one JSON
//! object matching [`Envelope`]. The envelope is intentionally small and
//! server-terminated: the relay routes [`Envelope::Forward`] payloads
//! opaquely between paired devices and terminates everything else locally.
//!
//! # Wire shape
//!
//! - The outer discriminator is `kind` (e.g. `"kind":"forward"`),
//!   `rename_all = "snake_case"`.
//! - Every envelope carries `"v": 1` (the field is named `version` in Rust
//!   but renamed to `v` on the wire). Future breaking changes bump the
//!   version; clients that see an unrecognised `v` are expected to close
//!   the socket with a typed error (spec §6.3).
//! - `EventKind` flattens into [`Envelope::Event`] with a `type`
//!   discriminator matching spec §6.
//!
//! See `docs/superpowers/specs/minos-relay-backend-design.md` §6 for the
//! authoritative protocol definition. Any change to the Rust types here
//! MUST be accompanied by an update to the golden JSON fixtures under
//! `tests/golden/envelope/` — those fixtures are how we freeze the wire
//! format across refactors.

use minos_domain::{DeviceId, DeviceSecret};
use serde::{Deserialize, Serialize};

/// One WebSocket frame on the relay's `/devices` endpoint.
///
/// Serialised as a tagged JSON object with `kind` as the discriminator.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    /// Client → Relay. Relay forwards `payload` opaquely to the paired
    /// peer as an [`Envelope::Forwarded`]. The relay does not inspect or
    /// mutate `payload`; correlation of request/response is the clients'
    /// responsibility (see spec §6.2).
    Forward {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// The Mac device this forward should be routed to. Backend
        /// validates against the caller's account_host_pairings rows.
        /// Mismatch → routing error (PeerOffline).
        target_device_id: DeviceId,
        /// Opaque payload (JSON-RPC 2.0 by convention between Minos
        /// clients, but the relay does not read it).
        payload: serde_json::Value,
    },
    /// Relay → Client. Delivery of a peer's [`Envelope::Forward`].
    Forwarded {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// Sender's `DeviceId`. Serialised as a bare UUID string because
        /// `DeviceId` is `#[serde(transparent)]`.
        from: DeviceId,
        /// The payload the peer sent, verbatim.
        payload: serde_json::Value,
    },
    /// Relay → Client. Server-side state push; carries a typed
    /// [`EventKind`] flattened with a `type` discriminator.
    Event {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
        /// The event body; see [`EventKind`] variants.
        #[serde(flatten)]
        event: EventKind,
    },
    /// Agent-host → Backend. Raw native event from a CLI for persistence
    /// and fan-out. No response expected. (seq, thread_id) must be unique
    /// server-side; the host treats conflicts as a no-op.
    Ingest {
        #[serde(rename = "v")]
        version: u8,
        agent: minos_domain::AgentName,
        thread_id: String,
        seq: u64,
        payload: serde_json::Value,
        ts_ms: i64,
    },
}

/// Server-initiated state change pushed to the client, body of
/// [`Envelope::Event`].
///
/// On the wire, the `type` key carries the variant name in `snake_case`
/// (spec §6). Payload fields sit alongside `type` thanks to the
/// `#[serde(flatten)]` in [`Envelope::Event`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// Emitted only to the Mac side after an iPhone successfully consumes
    /// a pairing token (spec §7.1). Delivers the iPhone's identity plus
    /// the long-lived `DeviceSecret` the Mac will use for future
    /// WebSocket auth (spec §9.4).
    Paired {
        /// The iPhone's `DeviceId`.
        peer_device_id: DeviceId,
        /// Display name the iPhone registered during `pair`.
        peer_name: String,
        /// Long-lived bearer secret for the Mac recipient. `None` when this
        /// event is delivered to an iOS recipient (iOS rail is bearer-only;
        /// see ADR-0020).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        your_device_secret: Option<DeviceSecret>,
    },
    /// The paired peer's WebSocket came online.
    PeerOnline {
        /// Peer's `DeviceId`.
        peer_device_id: DeviceId,
    },
    /// The paired peer's WebSocket dropped (clean close or failure).
    PeerOffline {
        /// Peer's `DeviceId`.
        peer_device_id: DeviceId,
    },
    /// The peer called `forget_peer`, or an admin revoked the pairing
    /// server-side. Clients should clear local pair state.
    Unpaired,
    /// Relay is shutting down; clients should reconnect with backoff.
    ServerShutdown,
    /// Backend → Mobile. One translated UI event from backend's live
    /// fan-out. `seq` matches the underlying `raw_events` row so mobile
    /// can dedupe against its per-thread watermark.
    UiEventMessage {
        thread_id: String,
        seq: u64,
        ui: minos_ui_protocol::UiEventMessage,
        ts_ms: i64,
    },
    /// Backend → Daemon. Sent as the first frame after the agent-host
    /// `/v1/devices/ws` upgrade authenticates. Carries the backend's
    /// per-thread `MAX(seq)` so the daemon can detect gaps between its
    /// local DB watermark and what the backend has durably persisted, and
    /// replay missing rows (or fall back to JSONL recovery).
    ///
    /// Only emitted to agent-hosts; mobile clients ingest no events and
    /// receive `UiEventMessage` instead.
    IngestCheckpoint {
        last_seq_per_thread: std::collections::HashMap<String, u64>,
    },
}

#[cfg(test)]
mod tests {
    //! Inline round-trip tests. The separate
    //! `tests/envelope_golden.rs` integration test freezes the exact wire
    //! shape via hand-authored JSON fixtures.

    use super::*;
    use pretty_assertions::assert_eq;

    fn round_trip(env: &Envelope) {
        let json = serde_json::to_string(env).unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap();
        let reserialised = serde_json::to_value(&back).unwrap();
        let expected: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(reserialised, expected);
    }

    #[test]
    fn forward_round_trips() {
        let env = Envelope::Forward {
            version: 1,
            target_device_id: DeviceId::new(),
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "method": "list_clis",
                "id": 1,
            }),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "forward");
    }

    #[test]
    fn forward_with_target_round_trips() {
        let target = DeviceId::new();
        let env = Envelope::Forward {
            version: 1,
            target_device_id: target,
            payload: serde_json::json!({"jsonrpc": "2.0", "method": "ping", "id": 1}),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "forward");
        assert_eq!(
            v["target_device_id"].as_str().unwrap(),
            target.0.to_string()
        );
    }

    #[test]
    fn forwarded_round_trips_with_transparent_device_id() {
        let id = DeviceId::new();
        let env = Envelope::Forwarded {
            version: 1,
            from: id,
            payload: serde_json::json!({"jsonrpc": "2.0", "result": [], "id": 1}),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        // DeviceId must serialise as a bare UUID string (transparent).
        assert!(v["from"].is_string());
        assert_eq!(v["from"].as_str().unwrap(), id.0.to_string());
    }

    #[test]
    fn event_paired_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: DeviceId::new(),
                peer_name: "Mac-mini".into(),
                your_device_secret: Some(DeviceSecret("sek".into())),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "event");
        assert_eq!(v["type"], "paired");
        // Plaintext secret MUST appear on the wire (no redaction via serde).
        assert_eq!(v["your_device_secret"], "sek");
    }

    #[test]
    fn paired_event_with_no_secret_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: DeviceId::new(),
                peer_name: "iPhone".into(),
                your_device_secret: None,
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "paired");
        assert!(v.get("your_device_secret").is_none() || v["your_device_secret"].is_null());
    }

    #[test]
    fn event_peer_online_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::PeerOnline {
                peer_device_id: DeviceId::new(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "peer_online");
    }

    #[test]
    fn event_peer_offline_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::PeerOffline {
                peer_device_id: DeviceId::new(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "peer_offline");
    }

    #[test]
    fn event_unpaired_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::Unpaired,
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "unpaired");
    }

    #[test]
    fn event_server_shutdown_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::ServerShutdown,
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "server_shutdown");
    }

    #[test]
    fn envelope_ingest_round_trip() {
        let e = Envelope::Ingest {
            version: 1,
            agent: minos_domain::AgentName::Codex,
            thread_id: "thr_1".into(),
            seq: 42,
            payload: serde_json::json!({"method":"item/agentMessage/delta","params":{"delta":"Hi"}}),
            ts_ms: 1_714_000_000_000,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""kind":"ingest""#));
        assert!(s.contains(r#""agent":"codex""#));
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn event_ingest_checkpoint_round_trips() {
        let mut map = std::collections::HashMap::new();
        map.insert("thr_a".to_string(), 7u64);
        map.insert("thr_b".to_string(), 3u64);
        let env = Envelope::Event {
            version: 1,
            event: EventKind::IngestCheckpoint {
                last_seq_per_thread: map.clone(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "event");
        assert_eq!(v["type"], "ingest_checkpoint");
        let lsp = &v["last_seq_per_thread"];
        assert_eq!(lsp["thr_a"], 7);
        assert_eq!(lsp["thr_b"], 3);
    }

    #[test]
    fn event_ingest_checkpoint_empty_map_round_trips() {
        let env = Envelope::Event {
            version: 1,
            event: EventKind::IngestCheckpoint {
                last_seq_per_thread: std::collections::HashMap::new(),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "ingest_checkpoint");
        assert!(v["last_seq_per_thread"].is_object());
        assert_eq!(v["last_seq_per_thread"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn envelope_event_ui_event_message_round_trip() {
        let e = Envelope::Event {
            version: 1,
            event: EventKind::UiEventMessage {
                thread_id: "thr_1".into(),
                seq: 42,
                ui: minos_ui_protocol::UiEventMessage::TextDelta {
                    message_id: "msg_1".into(),
                    text: "Hi".into(),
                },
                ts_ms: 1_714_000_000_000,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""type":"ui_event_message""#));
        assert!(s.contains(r#""kind":"text_delta""#));
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }
}
