//! Request and response payload types.

use minos_domain::{AgentDescriptor, DeviceId, PairingToken};
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
}
