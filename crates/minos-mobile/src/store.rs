//! Mobile-side `MobilePairingStore`.
//!
//! The phone persists four pieces of state: the backend WS URL scanned from
//! the QR, an optional Cloudflare Access service-token pair (header id + header
//! secret), the client's own `DeviceId`, and the long-lived `DeviceSecret`
//! minted by the backend on successful `pair`. In a real iOS build the durable
//! implementation lives in Dart (`flutter_secure_storage`, plan D5). For Rust
//! unit/integration tests this module offers an in-memory implementation.
//!
//! The trait is mobile-local rather than reused from `minos-pairing` because
//! the backend-assembled QR (spec §8.1) changed the data shape: there is no
//! longer a `TrustedDevice` list, just a single paired-backend descriptor
//! plus credentials.

use async_trait::async_trait;
use minos_domain::{DeviceId, DeviceSecret, MinosError};
use tokio::sync::RwLock;

/// Asynchronous store for the mobile client's durable pairing state.
///
/// Errors surface as `MinosError::StoreIo` / `StoreCorrupt` at the boundary.
/// Implementations must be cheap enough to call on the UI thread — i.e. no
/// blocking disk syncs inside `save_*`.
#[async_trait]
pub trait MobilePairingStore: Send + Sync {
    async fn load_backend_url(&self) -> Result<Option<String>, MinosError>;
    async fn save_backend_url(&self, url: &str) -> Result<(), MinosError>;

    async fn load_cf_access(&self) -> Result<Option<(String, String)>, MinosError>;
    async fn save_cf_access(&self, id: &str, secret: &str) -> Result<(), MinosError>;

    async fn load_device(&self) -> Result<Option<(DeviceId, DeviceSecret)>, MinosError>;
    async fn save_device(&self, id: &DeviceId, secret: &DeviceSecret) -> Result<(), MinosError>;

    async fn clear_all(&self) -> Result<(), MinosError>;
}

/// In-memory [`MobilePairingStore`] for tests and as the default store
/// plumbed through frb (real persistence happens in Dart; see plan D5).
#[derive(Default)]
pub struct InMemoryPairingStore {
    inner: RwLock<InMemoryState>,
}

#[derive(Default, Clone)]
struct InMemoryState {
    backend_url: Option<String>,
    cf_access: Option<(String, String)>,
    device: Option<(DeviceId, DeviceSecret)>,
}

impl InMemoryPairingStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MobilePairingStore for InMemoryPairingStore {
    async fn load_backend_url(&self) -> Result<Option<String>, MinosError> {
        Ok(self.inner.read().await.backend_url.clone())
    }
    async fn save_backend_url(&self, url: &str) -> Result<(), MinosError> {
        self.inner.write().await.backend_url = Some(url.to_string());
        Ok(())
    }

    async fn load_cf_access(&self) -> Result<Option<(String, String)>, MinosError> {
        Ok(self.inner.read().await.cf_access.clone())
    }
    async fn save_cf_access(&self, id: &str, secret: &str) -> Result<(), MinosError> {
        self.inner.write().await.cf_access = Some((id.to_string(), secret.to_string()));
        Ok(())
    }

    async fn load_device(&self) -> Result<Option<(DeviceId, DeviceSecret)>, MinosError> {
        Ok(self.inner.read().await.device.clone())
    }
    async fn save_device(&self, id: &DeviceId, secret: &DeviceSecret) -> Result<(), MinosError> {
        self.inner.write().await.device = Some((*id, secret.clone()));
        Ok(())
    }

    async fn clear_all(&self) -> Result<(), MinosError> {
        let mut guard = self.inner.write().await;
        *guard = InMemoryState::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_every_field_independently() {
        let store = InMemoryPairingStore::new();
        assert!(store.load_backend_url().await.unwrap().is_none());
        store.save_backend_url("wss://x.y/devices").await.unwrap();
        assert_eq!(
            store.load_backend_url().await.unwrap().as_deref(),
            Some("wss://x.y/devices")
        );

        store.save_cf_access("id", "sec").await.unwrap();
        assert_eq!(
            store.load_cf_access().await.unwrap(),
            Some(("id".into(), "sec".into()))
        );

        let id = DeviceId::new();
        let sec = DeviceSecret::generate();
        store.save_device(&id, &sec).await.unwrap();
        let (loaded_id, loaded_sec) = store.load_device().await.unwrap().unwrap();
        assert_eq!(loaded_id, id);
        assert_eq!(loaded_sec.0, sec.0);
    }

    #[tokio::test]
    async fn clear_all_wipes_every_field() {
        let store = InMemoryPairingStore::new();
        store.save_backend_url("x").await.unwrap();
        store.save_cf_access("id", "sec").await.unwrap();
        store
            .save_device(&DeviceId::new(), &DeviceSecret::generate())
            .await
            .unwrap();

        store.clear_all().await.unwrap();
        assert!(store.load_backend_url().await.unwrap().is_none());
        assert!(store.load_cf_access().await.unwrap().is_none());
        assert!(store.load_device().await.unwrap().is_none());
    }
}
