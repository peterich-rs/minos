//! Mobile-side `PairingStore`. The real implementation lives in Dart and is
//! invoked through frb (plan 03). For tests, an in-memory store is provided.

use std::sync::Mutex;

use minos_domain::MinosError;
use minos_pairing::{PairingStore, TrustedDevice};

pub struct InMemoryPairingStore(pub Mutex<Vec<TrustedDevice>>);

impl InMemoryPairingStore {
    #[must_use]
    pub fn new() -> Self {
        Self(Mutex::new(vec![]))
    }
}

impl Default for InMemoryPairingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingStore for InMemoryPairingStore {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
        *self.0.lock().unwrap() = devices.to_vec();
        Ok(())
    }
}
