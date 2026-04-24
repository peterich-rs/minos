//! Pairing persistence port + trusted-device record.

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, MinosError};
use serde::{Deserialize, Serialize};

/// One peer that has successfully paired and may reconnect on its own.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: DeviceId,
    pub name: String,
    /// Tailscale IP captured at pair time. Used by the mobile side to know
    /// where to reconnect; the Mac daemon ignores this field.
    pub host: String,
    pub port: u16,
    pub paired_at: DateTime<Utc>,
}

/// Persistence trait. Implementations:
/// - `minos-mobile::KeychainPairingStore` (FFI callback into iOS Keychain)
/// - test-only in-memory impls
///
/// The Mac daemon used to ship a `FilePairingStore` against a JSON file;
/// plan 05 removed it along with the rest of the Tailscale stack. The
/// Mac now persists `device-secret` to the Keychain and `PeerRecord` to
/// `local-state.json` directly inside `minos-daemon` without going
/// through this trait. iOS still uses the trait until plan 06 ports it
/// onto the relay.
pub trait PairingStore: Send + Sync + 'static {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError>;
    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory store for unit tests in this module and downstream crates.
    pub(crate) struct InMemStore(pub Mutex<Vec<TrustedDevice>>);

    impl PairingStore for InMemStore {
        fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
            *self.0.lock().unwrap() = devices.to_vec();
            Ok(())
        }
    }

    #[test]
    fn round_trip_through_in_mem_store() {
        let store = InMemStore(Mutex::new(vec![]));
        let dev = TrustedDevice {
            device_id: DeviceId::new(),
            name: "fan iPhone".into(),
            host: "100.64.0.42".into(),
            port: 7878,
            paired_at: Utc::now(),
        };
        store.save(&[dev.clone()]).unwrap();
        let back = store.load().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], dev);
    }
}
