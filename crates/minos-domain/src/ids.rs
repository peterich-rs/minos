//! Identifier newtypes.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable unique identifier for a paired device.
///
/// Newtype over `uuid::Uuid` (v4) so it cannot be confused with other UUIDs
/// in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub Uuid);

impl DeviceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One-shot pairing token: 32 random bytes, presented as base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PairingToken(String);

impl PairingToken {
    /// Generate a fresh token from the OS CSPRNG.
    ///
    /// # Panics
    /// Panics only if `getrandom` cannot supply entropy from the OS, which
    /// indicates an unrecoverable platform fault.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_ne;

    #[test]
    fn device_id_round_trips_through_json() {
        let id = DeviceId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: DeviceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn device_id_default_is_unique() {
        let a = DeviceId::default();
        let b = DeviceId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn pairing_token_is_43_chars_base64url() {
        // 32 bytes base64-encoded with no padding = 43 chars
        let t = PairingToken::generate();
        assert_eq!(t.as_str().len(), 43);
        assert!(t.as_str().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    proptest::proptest! {
        #[test]
        fn pairing_token_uniqueness(_iter in 0u32..1000) {
            // 1000 tokens, no collisions (entropy sanity)
            let mut seen = std::collections::HashSet::new();
            for _ in 0..1000 {
                let t = PairingToken::generate();
                assert!(seen.insert(t.0), "collision");
            }
        }
    }
}
