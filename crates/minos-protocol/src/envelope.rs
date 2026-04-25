//! WebSocket envelope for the Minos relay.
//!
//! Every text frame over the relay's `/devices` endpoint is exactly one JSON
//! object matching [`Envelope`]. The envelope is intentionally small and
//! server-terminated: the relay routes [`Envelope::Forward`] payloads
//! opaquely between paired devices and terminates everything else locally.
//!
//! # Wire shape
//!
//! - The outer discriminator is `kind` (e.g. `"kind":"local_rpc"`),
//!   `rename_all = "snake_case"`.
//! - Every envelope carries `"v": 1` (the field is named `version` in Rust
//!   but renamed to `v` on the wire). Future breaking changes bump the
//!   version; clients that see an unrecognised `v` are expected to close
//!   the socket with a typed error (spec §6.3).
//! - Local-RPC responses flatten an [`LocalRpcOutcome`] onto the parent
//!   object; the discriminator is `status` (`"ok"` / `"err"`). This choice
//!   documents intent on the wire and keeps the JSON flat enough to read
//!   in logs.
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
    /// Client → Relay. The backend itself handles these and replies with
    /// [`Envelope::LocalRpcResponse`] carrying the same `id`.
    LocalRpc {
        /// Protocol version. Always `1` in MVP.
        #[serde(rename = "v")]
        version: u8,
        /// Client-assigned correlation id, unique per WebSocket connection.
        /// Echoed back in the matching [`Envelope::LocalRpcResponse`].
        id: u64,
        /// The method being invoked. Flattened into the parent object so
        /// the wire shape carries `"method":"ping"` rather than a nested
        /// object; see the module-level docs for why.
        #[serde(flatten)]
        method: LocalRpcMethod,
        /// Method-specific parameters. JSON object by convention; shape
        /// is defined in spec §6.1 per method.
        params: serde_json::Value,
    },
    /// Relay → Client. Reply to a prior [`Envelope::LocalRpc`].
    LocalRpcResponse {
        /// Protocol version. Echoes the request.
        #[serde(rename = "v")]
        version: u8,
        /// Correlation id echoed from the request.
        id: u64,
        /// Success or failure result, flattened onto the parent object
        /// with a `status` discriminator.
        #[serde(flatten)]
        outcome: LocalRpcOutcome,
    },
    /// Client → Relay. Relay forwards `payload` opaquely to the paired
    /// peer as an [`Envelope::Forwarded`]. The relay does not inspect or
    /// mutate `payload`; correlation of request/response is the clients'
    /// responsibility (see spec §6.2).
    Forward {
        /// Protocol version.
        #[serde(rename = "v")]
        version: u8,
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

/// Discriminator for `LocalRpc` requests.
///
/// All variants are unit-like by design — method-specific request data
/// always lives in the sibling [`Envelope::LocalRpc::params`] field. Adding
/// data here would surface it alongside `params` (because we flatten) and
/// break that contract.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum LocalRpcMethod {
    /// Cheap liveness check. Params: `{}`. Result: `{"ok": true}`.
    Ping,
    /// Agent-host role only. Mints a fresh one-shot pairing token and
    /// returns a `PairingQrPayload` for the agent-host to render as a QR
    /// code (spec §6.1). Params: `{"host_display_name": "..."}`.
    RequestPairingQr,
    /// iOS-client role only. Consumes a pairing token and, on success,
    /// the relay emits [`EventKind::Paired`] to the Mac (spec §7.1).
    /// Params: `{"token": "...", "device_name": "..."}`.
    Pair,
    /// Either role. Tears down the pairing for both devices and emits
    /// [`EventKind::Unpaired`] to the peer (spec §7.4).
    ForgetPeer,
    /// Mobile → Backend. List thread summaries for the paired agent-host.
    /// Params: [`messages::ListThreadsParams`]. Result:
    /// [`messages::ListThreadsResponse`].
    ListThreads,
    /// Mobile → Backend. Read a window of translated UI events for one
    /// thread. Params: [`messages::ReadThreadParams`]. Result:
    /// [`messages::ReadThreadResponse`].
    ReadThread,
    /// Host-only helper. Agent-host asks the backend for the last seq it
    /// persisted on a given thread, so the host can decide whether to
    /// re-ingest on startup. Params:
    /// [`messages::GetThreadLastSeqParams`]. Result:
    /// [`messages::GetThreadLastSeqResponse`].
    GetThreadLastSeq,
}

/// Success or failure body for [`Envelope::LocalRpcResponse`].
///
/// Flattened onto the parent envelope with `status` as the discriminator:
///
/// ```json
/// {"kind":"local_rpc_response","v":1,"id":42,"status":"ok","result":{...}}
/// {"kind":"local_rpc_response","v":1,"id":42,"status":"err","error":{"code":"...","message":"..."}}
/// ```
///
/// Using `status` (rather than re-using `type`) documents the intent on
/// the wire and leaves the `type` key free if we ever want to tag the
/// result shape itself. The plan's sketch called for
/// `Ok { result } | Err { code, message }`; we split `code`/`message`
/// into [`RpcError`] so machine-readable error codes can be reused
/// outside of envelope responses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalRpcOutcome {
    /// RPC returned successfully; `result` is method-specific JSON.
    Ok {
        /// Method-specific success payload (shape defined in spec §6.1).
        result: serde_json::Value,
    },
    /// RPC failed; inspect [`RpcError::code`] for the machine-readable
    /// kind and [`RpcError::message`] for the operator-facing reason.
    Err {
        /// Error body.
        error: RpcError,
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
        /// Long-lived bearer secret the Mac should persist and present on
        /// subsequent WS connects. `DeviceSecret` serialises transparently
        /// (no redaction on the wire); redaction is applied only to
        /// `Debug`/`Display` formatters — see `minos_domain::ids`.
        your_device_secret: DeviceSecret,
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
}

/// Error body carried inside [`LocalRpcOutcome::Err`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// Machine-readable error code, e.g. `"pairing_token_invalid"`.
    /// Snake-case, stable across releases; clients match on this.
    pub code: String,
    /// Human-readable message intended for operators' logs. Not
    /// localised; not intended for end-user UI strings.
    pub message: String,
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
    fn local_rpc_ping_round_trips() {
        let env = Envelope::LocalRpc {
            version: 1,
            id: 1,
            method: LocalRpcMethod::Ping,
            params: serde_json::json!({}),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["kind"], "local_rpc");
        assert_eq!(v["v"], 1);
        assert_eq!(v["method"], "ping");
    }

    #[test]
    fn local_rpc_pair_round_trips() {
        let env = Envelope::LocalRpc {
            version: 1,
            id: 7,
            method: LocalRpcMethod::Pair,
            params: serde_json::json!({
                "token": "tok_abc",
                "device_name": "iPhone of fan",
            }),
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["method"], "pair");
        assert_eq!(v["params"]["token"], "tok_abc");
    }

    #[test]
    fn local_rpc_response_ok_round_trips() {
        let env = Envelope::LocalRpcResponse {
            version: 1,
            id: 42,
            outcome: LocalRpcOutcome::Ok {
                result: serde_json::json!({"ok": true}),
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["result"], serde_json::json!({"ok": true}));
    }

    #[test]
    fn local_rpc_response_err_round_trips() {
        let env = Envelope::LocalRpcResponse {
            version: 1,
            id: 42,
            outcome: LocalRpcOutcome::Err {
                error: RpcError {
                    code: "pairing_token_invalid".into(),
                    message: "token expired".into(),
                },
            },
        };
        round_trip(&env);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["status"], "err");
        assert_eq!(v["error"]["code"], "pairing_token_invalid");
    }

    #[test]
    fn forward_round_trips() {
        let env = Envelope::Forward {
            version: 1,
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
                your_device_secret: DeviceSecret("sek".into()),
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
    fn rpc_error_is_plain_struct() {
        let e = RpcError {
            code: "nope".into(),
            message: "because".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"code":"nope","message":"because"}"#);
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
