//! QR payload format (matches spec §6.1).

use chrono::{DateTime, Duration, Utc};
use minos_domain::PairingToken;
use serde::{Deserialize, Serialize};

pub const QR_TOKEN_TTL: Duration = Duration::minutes(5);
pub const PROTOCOL_VERSION: u8 = 1;

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QrPayload {
    pub v: u8,
    pub host: String,
    pub port: u16,
    pub token: PairingToken,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ActiveToken {
    pub token: PairingToken,
    pub issued_at: DateTime<Utc>,
}

impl ActiveToken {
    #[must_use]
    pub fn fresh() -> Self {
        Self {
            token: PairingToken::generate(),
            issued_at: Utc::now(),
        }
    }

    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now - self.issued_at > QR_TOKEN_TTL
    }
}

#[must_use]
pub fn generate_qr_payload(host: String, port: u16, name: String) -> (QrPayload, ActiveToken) {
    let active = ActiveToken::fresh();
    let payload = QrPayload {
        v: PROTOCOL_VERSION,
        host,
        port,
        token: active.token.clone(),
        name,
    };
    (payload, active)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn payload_has_v1_field() {
        let (p, _) = generate_qr_payload("100.64.0.10".into(), 7878, "Mac".into());
        assert_eq!(p.v, 1);
        assert_eq!(p.port, 7878);
    }

    #[test]
    fn payload_round_trips_through_json() {
        let (p, _) = generate_qr_payload("100.64.0.10".into(), 7878, "Mac".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: QrPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn token_expires_after_five_minutes() {
        let issued = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let active = ActiveToken {
            token: PairingToken::generate(),
            issued_at: issued,
        };
        let four_min = issued + Duration::minutes(4);
        let six_min = issued + Duration::minutes(6);
        assert!(!active.is_expired(four_min));
        assert!(active.is_expired(six_min));
    }
}
