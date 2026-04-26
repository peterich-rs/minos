//! Mac-only pairing types for the relay flow. Kept in minos-daemon instead
//! of minos-pairing so iOS's frb bindings (which import the legacy types)
//! are untouched until iOS migrates to relay. See plan divergence note.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, PairingToken};
use serde::{Deserialize, Serialize};

/// QR payload emitted by the Mac when pairing. This mirrors
/// `minos_protocol::PairingQrPayload` so the Mac renders exactly the schema
/// the iOS client scans: backend URL, host display name, one-shot token,
/// expiry, and optional CF Access service-token headers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RelayQrPayload {
    pub v: u8,
    pub backend_url: String,
    pub host_display_name: String,
    pub pairing_token: PairingToken,
    pub expires_at_ms: i64,
    pub cf_access_client_id: Option<String>,
    pub cf_access_client_secret: Option<String>,
}

/// Mac-side peer record (formerly `minos_pairing::TrustedDevice` without
/// the Tailscale IP/port fields). Persisted in local-state.json.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PeerRecord {
    pub device_id: DeviceId,
    pub name: String,
    pub paired_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_qr_payload_round_trip() {
        let qr = RelayQrPayload {
            v: 2,
            backend_url: "wss://minos.fan-nn.top/devices".into(),
            host_display_name: "fannnzhang's MacBook".into(),
            pairing_token: PairingToken("example-32b".into()),
            expires_at_ms: 1_700_000_000_000,
            cf_access_client_id: Some("client-id".into()),
            cf_access_client_secret: Some("client-secret".into()),
        };
        let j = serde_json::to_string(&qr).unwrap();
        let back: RelayQrPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(qr, back);
        assert!(!j.contains("\"host\""));
        assert!(!j.contains("mac_display_name"));
        assert!(!j.contains("\"token\""));
        assert!(!j.contains("port"));
    }

    #[test]
    fn peer_record_round_trip() {
        let pr = PeerRecord {
            device_id: DeviceId::new(),
            name: "fannnzhang's iPhone".into(),
            paired_at: Utc::now(),
        };
        let j = serde_json::to_string(&pr).unwrap();
        let back: PeerRecord = serde_json::from_str(&j).unwrap();
        assert_eq!(pr.device_id, back.device_id);
        assert_eq!(pr.name, back.name);
    }
}
