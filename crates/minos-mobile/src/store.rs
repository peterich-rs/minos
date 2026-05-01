//! Mobile-side `MobilePairingStore`.
//!
//! The phone persists multiple pieces of state: the client's own `DeviceId`,
//! the active-Mac `DeviceId` (post ADR-0020 — what we route forwards to),
//! and the slack.ai-style account auth tokens (access_token +
//! access_expires_at_ms + refresh_token) together with the bound account
//! identity (account_id + email). Backend URL and any CF Access service-
//! token headers are NOT persisted: they live in the mobile client's
//! compile-time `build_config` (read by `option_env!` from the shell that
//! drove the cargo build), so transport-edge configuration never leaks into
//! business logic or durable storage.
//!
//! ADR-0020 dropped the `DeviceSecret` from this snapshot — the iOS rail is
//! bearer-only, so the only secret the phone ever holds is the access /
//! refresh token pair. The Mac's display name and the active-Mac id come
//! from the backend's `/v1/me/macs` listing rather than persisted state.
//!
//! In a real iOS build the durable implementation lives in Dart
//! (`flutter_secure_storage`, plan D5). For Rust unit/integration tests
//! this module offers an in-memory implementation.
//!
//! The trait is mobile-local rather than reused from `minos-pairing` because
//! the backend-assembled QR (spec §8.1) changed the data shape: there is no
//! longer a `TrustedDevice` list, just a single paired-backend descriptor
//! plus credentials.

use async_trait::async_trait;
use minos_domain::{DeviceId, MinosError};
use tokio::sync::RwLock;

/// Durable mobile pairing snapshot mirrored into the iOS keychain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedPairingState {
    pub device_id: Option<String>,

    // Phase 4 (auth): account-bound bearer/refresh tokens. All five fields
    // are persisted together — the store's `save_auth` writes the whole
    // tuple atomically, and `clear_auth` wipes all five at once.
    pub access_token: Option<String>,
    pub access_expires_at_ms: Option<i64>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
}

/// Auth half of the persisted state. `load_auth` returns this when ALL five
/// fields are present; otherwise `None` so callers do not have to assemble
/// partial-token states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAuth {
    pub access_token: String,
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
    pub account_id: String,
    pub account_email: String,
}

/// Asynchronous store for the mobile client's durable pairing state.
///
/// Errors surface as `MinosError::StoreIo` / `StoreCorrupt` at the boundary.
/// Implementations must be cheap enough to call on the UI thread — i.e. no
/// blocking disk syncs inside `save_*`.
#[async_trait]
pub trait MobilePairingStore: Send + Sync {
    /// Load the client's own DeviceId (the phone). Returns None pre-register.
    async fn load_device(&self) -> Result<Option<DeviceId>, MinosError>;
    /// Persist the client's own DeviceId (post-register / pre-pair).
    async fn save_device(&self, id: &DeviceId) -> Result<(), MinosError>;

    /// Persist the active-Mac DeviceId — the Mac that subsequent
    /// `Envelope::Forward` frames target. Set by `pair_with_qr_json` after
    /// a successful consume; updated by `set_active_mac` when the user
    /// switches between paired Macs in `/v1/me/macs`.
    async fn save_active_mac(&self, mac: &DeviceId) -> Result<(), MinosError>;
    /// Read the currently-active Mac id, or `None` if no pair has been
    /// completed yet.
    async fn load_active_mac(&self) -> Result<Option<DeviceId>, MinosError>;
    /// Conditionally clear the active-Mac slot — only if it currently
    /// equals `mac`. Used by `forget_mac` to avoid clobbering a
    /// concurrent `set_active_mac` that targeted a different Mac.
    async fn clear_active_if(&self, mac: &DeviceId) -> Result<(), MinosError>;

    /// Persist the slack.ai-style account-auth tuple. Implementations must
    /// store all five fields atomically; readers see either every field or
    /// `None`.
    async fn save_auth(
        &self,
        access: String,
        access_expires_at_ms: i64,
        refresh: String,
        account_id: String,
        account_email: String,
    ) -> Result<(), MinosError>;

    /// Returns `Some(_)` when every auth field is populated, `None` when any
    /// one is missing (i.e. logged-out state).
    async fn load_auth(&self) -> Result<Option<PersistedAuth>, MinosError>;

    /// Clear the auth tuple (logout / refresh-failure path). Leaves the
    /// pairing fields untouched.
    async fn clear_auth(&self) -> Result<(), MinosError>;

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
    device: Option<DeviceId>,
    active_mac: Option<DeviceId>,
    auth: Option<PersistedAuth>,
}

impl InMemoryPairingStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_parts(device: Option<DeviceId>, auth: Option<PersistedAuth>) -> Self {
        Self {
            inner: RwLock::new(InMemoryState {
                device,
                active_mac: None,
                auth,
            }),
        }
    }
}

#[async_trait]
impl MobilePairingStore for InMemoryPairingStore {
    async fn load_device(&self) -> Result<Option<DeviceId>, MinosError> {
        Ok(self.inner.read().await.device)
    }
    async fn save_device(&self, id: &DeviceId) -> Result<(), MinosError> {
        self.inner.write().await.device = Some(*id);
        Ok(())
    }

    async fn save_active_mac(&self, mac: &DeviceId) -> Result<(), MinosError> {
        self.inner.write().await.active_mac = Some(*mac);
        Ok(())
    }

    async fn load_active_mac(&self) -> Result<Option<DeviceId>, MinosError> {
        Ok(self.inner.read().await.active_mac)
    }

    async fn clear_active_if(&self, mac: &DeviceId) -> Result<(), MinosError> {
        let mut guard = self.inner.write().await;
        if guard.active_mac == Some(*mac) {
            guard.active_mac = None;
        }
        Ok(())
    }

    async fn save_auth(
        &self,
        access: String,
        access_expires_at_ms: i64,
        refresh: String,
        account_id: String,
        account_email: String,
    ) -> Result<(), MinosError> {
        self.inner.write().await.auth = Some(PersistedAuth {
            access_token: access,
            access_expires_at_ms,
            refresh_token: refresh,
            account_id,
            account_email,
        });
        Ok(())
    }

    async fn load_auth(&self) -> Result<Option<PersistedAuth>, MinosError> {
        Ok(self.inner.read().await.auth.clone())
    }

    async fn clear_auth(&self) -> Result<(), MinosError> {
        self.inner.write().await.auth = None;
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
    async fn round_trips_device_id() {
        let store = InMemoryPairingStore::new();
        let id = DeviceId::new();
        store.save_device(&id).await.unwrap();
        let loaded_id = store.load_device().await.unwrap().unwrap();
        assert_eq!(loaded_id, id);
    }

    #[tokio::test]
    async fn active_mac_round_trip_and_conditional_clear() {
        let store = InMemoryPairingStore::new();
        assert!(store.load_active_mac().await.unwrap().is_none());

        let mac_a = DeviceId::new();
        let mac_b = DeviceId::new();

        store.save_active_mac(&mac_a).await.unwrap();
        assert_eq!(store.load_active_mac().await.unwrap(), Some(mac_a));

        // clear_active_if must NOT clear when the live value differs.
        store.clear_active_if(&mac_b).await.unwrap();
        assert_eq!(
            store.load_active_mac().await.unwrap(),
            Some(mac_a),
            "non-matching clear must be a no-op"
        );

        // clear_active_if matching value clears it.
        store.clear_active_if(&mac_a).await.unwrap();
        assert!(store.load_active_mac().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn clear_all_wipes_every_field() {
        let store = InMemoryPairingStore::new();
        store.save_device(&DeviceId::new()).await.unwrap();
        store.save_active_mac(&DeviceId::new()).await.unwrap();
        store
            .save_auth(
                "access".into(),
                42,
                "refresh".into(),
                "acct-1".into(),
                "a@b.com".into(),
            )
            .await
            .unwrap();

        store.clear_all().await.unwrap();
        assert!(store.load_device().await.unwrap().is_none());
        assert!(store.load_active_mac().await.unwrap().is_none());
        assert!(store.load_auth().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn save_then_load_auth_round_trips_all_five_fields() {
        let store = InMemoryPairingStore::new();
        assert!(store.load_auth().await.unwrap().is_none());
        store
            .save_auth(
                "access".into(),
                123_456,
                "refresh".into(),
                "acct-1".into(),
                "a@b.com".into(),
            )
            .await
            .unwrap();
        let loaded = store
            .load_auth()
            .await
            .unwrap()
            .expect("auth should be populated");
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.access_expires_at_ms, 123_456);
        assert_eq!(loaded.refresh_token, "refresh");
        assert_eq!(loaded.account_id, "acct-1");
        assert_eq!(loaded.account_email, "a@b.com");
    }

    #[tokio::test]
    async fn clear_auth_wipes_only_auth_fields() {
        let store = InMemoryPairingStore::new();
        let id = DeviceId::new();
        store.save_device(&id).await.unwrap();
        store
            .save_auth(
                "access".into(),
                42,
                "refresh".into(),
                "acct-1".into(),
                "a@b.com".into(),
            )
            .await
            .unwrap();

        store.clear_auth().await.unwrap();
        assert!(store.load_auth().await.unwrap().is_none());
        // Pairing fields preserved.
        let loaded_id = store.load_device().await.unwrap().unwrap();
        assert_eq!(loaded_id, id);
    }

    #[tokio::test]
    async fn from_parts_seeds_every_field() {
        let id = DeviceId::new();
        let store = InMemoryPairingStore::from_parts(
            Some(id),
            Some(PersistedAuth {
                access_token: "access".into(),
                access_expires_at_ms: 42,
                refresh_token: "refresh".into(),
                account_id: "acct-1".into(),
                account_email: "a@b.com".into(),
            }),
        );
        let loaded_id = store.load_device().await.unwrap().unwrap();
        assert_eq!(loaded_id, id);
        let auth = store.load_auth().await.unwrap().unwrap();
        assert_eq!(auth.access_token, "access");
    }
}
