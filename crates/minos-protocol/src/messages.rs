//! Request and response payload types.

use minos_domain::{AgentDescriptor, AgentName, DeviceId, PairingToken};
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
    pub ok: bool,
    pub mac_name: String,
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
            ok: true,
            mac_name: "MacBook".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
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
