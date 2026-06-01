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
    fn realtime_ws_ticket_request_empty_round_trip() {
        let req = RealtimeWsTicketRequest {
            installation_id: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: RealtimeWsTicketRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }
}
