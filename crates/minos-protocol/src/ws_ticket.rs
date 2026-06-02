//! WS ticket request/response types for the realtime gateway.
//!
//! Before upgrading to `/ws/client` or `/ws/host`, the caller must obtain
//! a short-lived ticket via `POST /v1/realtime/ws-ticket` (account) or
//! `POST /v1/host/realtime/ws-ticket` (host).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeWsTicketRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeWsTicketResponse {
    pub ticket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
}

/// Host-side ws-ticket response. The backend wraps this inside a
/// `ResponseEnvelope { data, meta }` so callers should deserialize the
/// outer envelope first, then extract `data` into this struct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostWsTicketResponse {
    pub ticket: String,
    pub gateway_url: String,
    pub expires_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_ws_ticket_response_round_trip() {
        let resp = RealtimeWsTicketResponse {
            ticket: "jwt-token-here".into(),
            gateway_url: Some("wss://minos.example.com/ws/client".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: RealtimeWsTicketResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn host_ws_ticket_response_round_trip() {
        let resp = HostWsTicketResponse {
            ticket: "jwt-host-token".into(),
            gateway_url: "/ws/host?ticket=jwt-host-token".into(),
            expires_at_ms: 1_760_000_060_000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: HostWsTicketResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn realtime_ws_ticket_request_empty_round_trip() {
        let req = RealtimeWsTicketRequest {
            installation_id: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: RealtimeWsTicketRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }
}
