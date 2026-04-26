//! Request and response payload types.

use minos_domain::{AgentDescriptor, AgentName, DeviceId, DeviceSecret, PairingToken};
use minos_ui_protocol::{ThreadEndReason, UiEventMessage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairRequest {
    pub device_id: DeviceId,
    pub name: String,
    /// One-shot pairing token presented in the QR. Validated server-side
    /// against the daemon's currently-active token before any state is
    /// mutated; spec §6.4. Required by MVP.
    pub token: PairingToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairResponse {
    /// Mirrors the envelope/local-RPC pair result naming so the legacy typed
    /// jsonrpsee surface exposes the same contract.
    pub peer_device_id: DeviceId,
    pub peer_name: String,
    pub your_device_secret: DeviceSecret,
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

/// Deep-link QR payload minted by the Mac and scanned by iOS. Carries the
/// backend URL, a display name for the host, a short-lived one-shot
/// `pairing_token`, its RFC-3339-ish epoch-ms expiry, and — when the
/// backend sits behind a Cloudflare Access service-token — the two
/// service-token headers so the iPhone can authenticate against the edge
/// before the bearer-secret WebSocket handshake.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PairingQrPayload {
    #[serde(default = "default_pairing_qr_version")]
    pub v: u8,
    pub backend_url: String,
    #[serde(alias = "mac_display_name")]
    pub host_display_name: String,
    #[serde(alias = "token")]
    pub pairing_token: String,
    #[serde(default)]
    pub expires_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cf_access_client_secret: Option<String>,
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
            your_device_secret: DeviceSecret::generate(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["peer_device_id"],
            serde_json::to_value(resp.peer_device_id).unwrap()
        );
        assert_eq!(value["peer_name"], serde_json::json!("MacBook"));
        assert_eq!(
            value["your_device_secret"],
            serde_json::json!(resp.your_device_secret.as_str())
        );
        let back: PairResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
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
    fn pairing_qr_payload_round_trip_with_cf() {
        let p = PairingQrPayload {
            v: 2,
            backend_url: "wss://minos.fan-nn.top/devices".into(),
            host_display_name: "Mac".into(),
            pairing_token: "tok".into(),
            expires_at_ms: 1,
            cf_access_client_id: Some("id".into()),
            cf_access_client_secret: Some("sec".into()),
        };
        let back: PairingQrPayload =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn pairing_qr_payload_without_cf_omits_fields() {
        let p = PairingQrPayload {
            v: 2,
            backend_url: "x".into(),
            host_display_name: "x".into(),
            pairing_token: "t".into(),
            expires_at_ms: 0,
            cf_access_client_id: None,
            cf_access_client_secret: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(!s.contains("cf_access_client_id"));
        assert!(!s.contains("cf_access_client_secret"));
    }

    #[test]
    fn pairing_qr_payload_accepts_legacy_mac_field_names() {
        let back: PairingQrPayload = serde_json::from_value(serde_json::json!({
            "backend_url": "wss://minos.fan-nn.top/devices",
            "mac_display_name": "Mac",
            "token": "tok"
        }))
        .unwrap();

        assert_eq!(back.v, 2);
        assert_eq!(back.backend_url, "wss://minos.fan-nn.top/devices");
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
                backend_url: "wss://example.com/devices".into(),
                host_display_name: "Mac".into(),
                pairing_token: "tok".into(),
                expires_at_ms: 42,
                cf_access_client_id: None,
                cf_access_client_secret: None,
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
}
