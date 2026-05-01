//! Request and response payload types.

use minos_domain::{AgentDescriptor, AgentName, DeviceId, PairingToken};
use minos_ui_protocol::{ThreadEndReason, UiEventMessage};
use serde::{Deserialize, Serialize};

/// Response body for `GET /v1/me/peer` — the backend's view of the
/// caller's currently paired peer. Returned by the authenticated
/// pairing-rail (`X-Device-Id` + `X-Device-Secret`) so a freshly
/// reconnected daemon can refresh its in-memory peer mirror without
/// reading anything from local disk.
///
/// On `200`, the body carries the peer's `device_id`, the peer-side
/// display name, and the pairing's `created_at` timestamp (epoch ms).
/// On `404` with `error.code == "not_paired"`, the caller has no row
/// in the `pairings` table — the response body uses the standard
/// `{ "error": { "code": ..., "message": ... } }` envelope shared by
/// every other `/v1/*` route.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MePeerResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
    pub paired_at_ms: i64,
}

/// Response body for `GET /v1/me/macs`. iOS callers receive every Mac
/// paired to their `account_id`. `paired_via_device_id` is the mobile
/// device that performed the scan — recorded for audit; not used for
/// routing.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MeMacsResponse {
    pub macs: Vec<MacSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MacSummary {
    pub mac_device_id: DeviceId,
    pub mac_display_name: String,
    pub paired_at_ms: i64,
    pub paired_via_device_id: DeviceId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairRequest {
    pub device_id: DeviceId,
    pub name: String,
    /// One-shot pairing token presented in the QR. Validated server-side
    /// against the daemon's currently-active token before any state is
    /// mutated; spec §6.4. Required by MVP.
    pub token: PairingToken,
}

/// Result of `POST /v1/pairings` (consume). iOS no longer receives a
/// device secret — the rail is bearer-only post ADR-0020. Mac-side
/// pair state is delivered separately via `EventKind::Paired`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairResponse {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
}

/// Request body for `POST /v1/pairing/consume`. Distinct from
/// [`PairRequest`] because the HTTP route derives `device_id` from the
/// `X-Device-Id` header, not the body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairConsumeRequest {
    pub token: PairingToken,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub version: String,
    pub uptime_secs: u64,
}

pub type ListClisResponse = Vec<AgentDescriptor>;

/// Parameters for the `start_agent` RPC. See spec §5.2.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentRequest {
    pub agent: AgentName,
}

/// Result of a successful `start_agent` RPC — carries the codex `thread_id`
/// as `session_id` and the resolved workspace path. See spec §5.2.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAgentResponse {
    pub session_id: String,
    pub cwd: String,
}

/// Parameters for the `send_user_message` RPC. `session_id` must match the
/// active session's id; see spec §5.2 and §5.4.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendUserMessageRequest {
    pub session_id: String,
    pub text: String,
}

/// Deep-link QR payload minted by the Mac and scanned by iOS. Carries a
/// display name for the host, a short-lived one-shot `pairing_token`, and
/// its RFC-3339-ish epoch-ms expiry. The backend URL and any Cloudflare
/// Access service-token headers live in the mobile client's compile-time
/// build config (see `minos_mobile::build_config`); they are not transit
/// values and never enter the QR payload, durable storage, or the
/// post-pair business logic.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairingQrPayload {
    #[serde(default = "default_pairing_qr_version")]
    pub v: u8,
    #[serde(alias = "mac_display_name")]
    pub host_display_name: String,
    #[serde(alias = "token")]
    pub pairing_token: String,
    #[serde(default)]
    pub expires_at_ms: i64,
}

const fn default_pairing_qr_version() -> u8 {
    2
}

/// Parameters for `request_pairing_qr` — the Mac tells the backend which
/// display name to embed so the iPhone UI can say "Pair with <name>?".
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrParams {
    pub host_display_name: String,
}

/// Response from `request_pairing_qr`; wraps a [`PairingQrPayload`] for
/// the Mac to render directly as a QR code.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RequestPairingQrResponse {
    pub qr_payload: PairingQrPayload,
}

/// Compact summary of one persisted thread, returned by `list_threads`
/// for the mobile history list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
}

/// Parameters for `list_threads`. `before_ts_ms` paginates older entries;
/// `agent` filters by CLI kind.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListThreadsParams {
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_ts_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentName>,
}

/// Response from `list_threads`; `next_before_ts_ms` is set iff there is
/// a strictly older page the caller can request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListThreadsResponse {
    pub threads: Vec<ThreadSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before_ts_ms: Option<i64>,
}

/// Parameters for `read_thread`. `from_seq` resumes from after the given
/// sequence; if omitted, the backend returns the oldest `limit` events.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadThreadParams {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<u64>,
    pub limit: u32,
}

/// Response from `read_thread`. `next_seq` is set iff more events exist
/// past the returned window. `thread_end_reason` is set iff the thread is
/// closed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReadThreadResponse {
    pub ui_events: Vec<UiEventMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_end_reason: Option<ThreadEndReason>,
}

/// Parameters for `get_thread_last_seq` (host-only helper).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetThreadLastSeqParams {
    pub thread_id: String,
}

/// Response from `get_thread_last_seq`; `last_seq` is `0` when the thread
/// is unknown or empty.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GetThreadLastSeqResponse {
    pub last_seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pair_request_round_trip() {
        let req = PairRequest {
            device_id: DeviceId::new(),
            name: "iPhone of fan".into(),
            token: PairingToken::generate(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: PairRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn pair_response_round_trip() {
        let resp = PairResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "MacBook".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["peer_device_id"],
            serde_json::to_value(resp.peer_device_id).unwrap()
        );
        assert_eq!(value["peer_name"], serde_json::json!("MacBook"));
        assert!(value.get("your_device_secret").is_none());
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn pair_response_no_secret_field_round_trip() {
        let resp = PairResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "iPhone".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            value.get("your_device_secret").is_none(),
            "secret must not appear"
        );
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn me_macs_response_round_trips() {
        let macs = MeMacsResponse {
            macs: vec![MacSummary {
                mac_device_id: DeviceId::new(),
                mac_display_name: "Mac-mini".into(),
                paired_at_ms: 1_714_000_000_000,
                paired_via_device_id: DeviceId::new(),
            }],
        };
        let json = serde_json::to_string(&macs).unwrap();
        let back: MeMacsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, macs);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["macs"].is_array());
    }

    #[test]
    fn health_response_round_trip() {
        let resp = HealthResponse {
            version: "0.1.0".into(),
            uptime_secs: 42,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: HealthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn start_agent_request_round_trip() {
        let req = StartAgentRequest {
            agent: AgentName::Codex,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: StartAgentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn start_agent_response_round_trip() {
        let resp = StartAgentResponse {
            session_id: "thread-abc12".into(),
            cwd: "/Users/fan/.minos/workspaces".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: StartAgentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn send_user_message_request_round_trip() {
        let req = SendUserMessageRequest {
            session_id: "thread-abc12".into(),
            text: "ping".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SendUserMessageRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }
}

#[cfg(test)]
mod new_type_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pairing_qr_payload_accepts_legacy_mac_field_names() {
        // Legacy QR payloads may still carry `backend_url` and CF Access
        // fields — `serde` ignores unknown fields by default, so this is
        // a forward-compat read of older Mac builds. The struct itself no
        // longer carries them.
        let back: PairingQrPayload = serde_json::from_value(serde_json::json!({
            "backend_url": "wss://minos.fan-nn.top/devices",
            "mac_display_name": "Mac",
            "token": "tok"
        }))
        .unwrap();

        assert_eq!(back.v, 2);
        assert_eq!(back.host_display_name, "Mac");
        assert_eq!(back.pairing_token, "tok");
        assert_eq!(back.expires_at_ms, 0);
    }

    #[test]
    fn request_pairing_qr_params_round_trip() {
        let p = RequestPairingQrParams {
            host_display_name: "Fan's Mac".into(),
        };
        let back: RequestPairingQrParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn request_pairing_qr_response_round_trip() {
        let r = RequestPairingQrResponse {
            qr_payload: PairingQrPayload {
                v: 1,
                host_display_name: "Mac".into(),
                pairing_token: "tok".into(),
                expires_at_ms: 42,
            },
        };
        let back: RequestPairingQrResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn thread_summary_round_trip_with_end_reason() {
        let s = ThreadSummary {
            thread_id: "thr_1".into(),
            agent: AgentName::Codex,
            title: Some("A thread".into()),
            first_ts_ms: 100,
            last_ts_ms: 200,
            message_count: 3,
            ended_at_ms: Some(300),
            end_reason: Some(ThreadEndReason::AgentDone),
        };
        let back: ThreadSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn thread_summary_round_trip_open_thread() {
        let s = ThreadSummary {
            thread_id: "thr_2".into(),
            agent: AgentName::Claude,
            title: None,
            first_ts_ms: 100,
            last_ts_ms: 200,
            message_count: 1,
            ended_at_ms: None,
            end_reason: None,
        };
        let back: ThreadSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn list_threads_params_round_trip_filters() {
        let p = ListThreadsParams {
            limit: 50,
            before_ts_ms: Some(1_000),
            agent: Some(AgentName::Gemini),
        };
        let back: ListThreadsParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn list_threads_params_round_trip_omits_none_fields() {
        let p = ListThreadsParams {
            limit: 10,
            before_ts_ms: None,
            agent: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(!s.contains("before_ts_ms"));
        assert!(!s.contains("agent"));
    }

    #[test]
    fn list_threads_response_round_trip() {
        let r = ListThreadsResponse {
            threads: vec![ThreadSummary {
                thread_id: "thr_1".into(),
                agent: AgentName::Codex,
                title: None,
                first_ts_ms: 1,
                last_ts_ms: 2,
                message_count: 0,
                ended_at_ms: None,
                end_reason: None,
            }],
            next_before_ts_ms: Some(1),
        };
        let back: ListThreadsResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn read_thread_params_round_trip() {
        let p = ReadThreadParams {
            thread_id: "thr_1".into(),
            from_seq: Some(10),
            limit: 100,
        };
        let back: ReadThreadParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn read_thread_response_round_trip() {
        let r = ReadThreadResponse {
            ui_events: vec![UiEventMessage::TextDelta {
                message_id: "msg_1".into(),
                text: "Hi".into(),
            }],
            next_seq: Some(2),
            thread_end_reason: Some(ThreadEndReason::AgentDone),
        };
        let back: ReadThreadResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn get_thread_last_seq_params_round_trip() {
        let p = GetThreadLastSeqParams {
            thread_id: "thr_1".into(),
        };
        let back: GetThreadLastSeqParams =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn get_thread_last_seq_response_round_trip() {
        let r = GetThreadLastSeqResponse { last_seq: 42 };
        let back: GetThreadLastSeqResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn me_peer_response_round_trip() {
        let r = MePeerResponse {
            peer_device_id: DeviceId::new(),
            peer_name: "fan's iPhone".into(),
            paired_at_ms: 1_726_500_000_000,
        };
        let json = serde_json::to_string(&r).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["peer_device_id"],
            serde_json::to_value(r.peer_device_id).unwrap()
        );
        assert_eq!(value["peer_name"], serde_json::json!("fan's iPhone"));
        assert_eq!(
            value["paired_at_ms"],
            serde_json::json!(1_726_500_000_000_i64)
        );
        let back: MePeerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
