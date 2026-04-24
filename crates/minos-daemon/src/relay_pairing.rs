//! Mac-only pairing types for the relay flow. Kept in minos-daemon instead
//! of minos-pairing so iOS's frb bindings (which import the legacy types)
//! are untouched until iOS migrates to relay. See plan divergence note.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, PairingToken};
use serde::{Deserialize, Serialize};

/// QR payload emitted by the Mac when pairing. Encodes where and what —
/// the relay backend URL, a one-shot pairing token, and the Mac's display
/// name. No IP/port: the backend is Cloudflare-fronted, addresses are
/// invariant across deployments (baked at compile time).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RelayQrPayload {
    pub v: u8,
    pub backend_url: String,
    pub token: PairingToken,
    pub mac_display_name: String,
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
            v: 1,
            backend_url: "wss://minos.fan-nn.top/devices".into(),
            token: PairingToken("example-32b".into()),
            mac_display_name: "fannnzhang's MacBook".into(),
        };
        let j = serde_json::to_string(&qr).unwrap();
        let back: RelayQrPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(qr, back);
        assert!(!j.contains("host"));
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
