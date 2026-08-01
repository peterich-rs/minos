//! Mac-side peer record for linked account viewers (Host Link).

use chrono::{DateTime, Utc};
use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};

/// Linked peer snapshot shown in host status UI after Host Link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeerRecord {
    pub device_id: DeviceId,
    pub name: String,
    pub paired_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
