//! Live realtime connection authority for formal `/ws/client` and `/ws/host`.
//!
//! One installation (`DeviceId`) maps to at most one current
//! [`ConnectionState`]. Same-device reconnect replaces the prior connection
//! and signals [`ConnectionRevocation::Superseded`]. Host unlink / auth
//! revoke signals [`ConnectionRevocation::AuthRevoked`].
//!
//! Presence counts and mobile disconnect grace for push also live here:
//! online is live-connection truth, not a durable store row.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::{DeviceId, DeviceRole};

use super::subscription::ConnectionState;

pub use super::subscription::ConnectionRevocation;

/// Concurrent map of `DeviceId →` current formal connection.
#[derive(Debug, Clone, Default)]
pub struct RealtimeConnectionRegistry {
    connections: Arc<DashMap<DeviceId, Arc<ConnectionState>>>,
    /// account_id → wall-clock ms of last "last mobile client left" edge.
    last_mobile_disconnect_at_ms: Arc<DashMap<String, i64>>,
}

impl RealtimeConnectionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `conn` as the current connection for its device.
    ///
    /// Returns the previous connection when the device already had one.
    /// Caller should [`ConnectionState::revoke`] the returned connection
    /// with [`ConnectionRevocation::Superseded`].
    pub fn insert(&self, conn: Arc<ConnectionState>) -> Option<Arc<ConnectionState>> {
        if conn.role == DeviceRole::MobileClient {
            if let Some(account_id) = conn.account_id() {
                self.last_mobile_disconnect_at_ms.remove(account_id);
            }
        }
        let previous = self.connections.insert(conn.device_id, conn);
        crate::telemetry::set_session_registry_size(self.len());
        if let Some(ref prev) = previous {
            self.note_mobile_connection_left(prev);
        }
        previous
    }

    /// Remove and return the connection for `id`, or `None` if none was live.
    pub fn remove(&self, id: DeviceId) -> Option<Arc<ConnectionState>> {
        let removed = self.connections.remove(&id).map(|(_k, v)| v);
        if let Some(ref conn) = removed {
            crate::telemetry::set_session_registry_size(self.len());
            self.note_mobile_connection_left(conn);
        }
        removed
    }

    /// Remove and return `current` only if it is still the live entry.
    ///
    /// ABA-safe disconnect cleanup: an old socket may close after a
    /// reconnect already inserted a replacement for the same `DeviceId`.
    pub fn remove_current(&self, current: &ConnectionState) -> Option<Arc<ConnectionState>> {
        let removed = self
            .connections
            .remove_if(&current.device_id, |_, live| live.same_connection(current))
            .map(|(_k, v)| v);
        if let Some(ref conn) = removed {
            crate::telemetry::set_session_registry_size(self.len());
            self.note_mobile_connection_left(conn);
        }
        removed
    }

    /// Clone the current connection for `id` if live.
    #[must_use]
    pub fn get(&self, id: DeviceId) -> Option<Arc<ConnectionState>> {
        self.connections.get(&id).map(|r| Arc::clone(r.value()))
    }

    /// True when a formal connection is live for `id`.
    #[must_use]
    pub fn is_online(&self, id: DeviceId) -> bool {
        self.connections.contains_key(&id)
    }

    /// Revoke and remove the live connection for `device_id` if present.
    ///
    /// Returns `true` when a connection was revoked.
    pub fn revoke_device(&self, device_id: DeviceId, reason: ConnectionRevocation) -> bool {
        let Some(conn) = self.remove(device_id) else {
            return false;
        };
        conn.revoke(reason);
        true
    }

    /// Revoke every currently-registered connection (process shutdown).
    ///
    /// Does not wait for sockets to drain; the gateway loop reacts to
    /// the revocation watch and closes each formal connection.
    pub fn revoke_all(&self, reason: ConnectionRevocation) {
        let ids: Vec<DeviceId> = self.connections.iter().map(|e| *e.key()).collect();
        for id in ids {
            let _ = self.revoke_device(id, reason);
        }
    }

    /// Wall-clock ms when this account last lost its final live mobile WS.
    #[must_use]
    pub fn last_mobile_disconnect_at_ms(&self, account_id: &str) -> Option<i64> {
        self.last_mobile_disconnect_at_ms
            .get(account_id)
            .map(|v| *v)
    }

    /// Count currently-live account-client connections bound to `account_id`.
    #[must_use]
    pub fn mobile_account_session_count(&self, account_id: &str) -> usize {
        self.connections
            .iter()
            .filter(|entry| {
                let conn = entry.value();
                conn.role.is_account_client() && conn.account_id() == Some(account_id)
            })
            .count()
    }

    /// Count currently-live mobile-client connections bound to `account_id`.
    #[must_use]
    pub fn mobile_client_session_count(&self, account_id: &str) -> usize {
        self.connections
            .iter()
            .filter(|entry| {
                let conn = entry.value();
                conn.role == DeviceRole::MobileClient && conn.account_id() == Some(account_id)
            })
            .count()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// Test / clock-injected stamp for disconnect grace unit tests.
    #[cfg(test)]
    pub fn stamp_mobile_disconnect_for_tests(&self, account_id: &str, at_ms: i64) {
        self.last_mobile_disconnect_at_ms
            .insert(account_id.to_string(), at_ms);
    }

    fn note_mobile_connection_left(&self, conn: &ConnectionState) {
        if conn.role != DeviceRole::MobileClient {
            return;
        }
        let Some(account_id) = conn.account_id() else {
            return;
        };
        if self.mobile_client_session_count(account_id) > 0 {
            self.last_mobile_disconnect_at_ms.remove(account_id);
            return;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.last_mobile_disconnect_at_ms
            .insert(account_id.to_string(), now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_protocol::realtime::ConnectionPrincipal;
    use tokio::sync::mpsc;

    use crate::realtime::subscription::ConnectionState;
    use crate::realtime::wire::ServerFrame;

    fn make_conn(
        device_id: DeviceId,
        role: DeviceRole,
        account_id: Option<&str>,
    ) -> Arc<ConnectionState> {
        let (tx, _rx) = mpsc::channel::<ServerFrame>(8);
        let principal = match role {
            DeviceRole::AgentHost => ConnectionPrincipal::Host {
                host_installation_id: device_id.to_string(),
            },
            _ => ConnectionPrincipal::Account {
                account_id: account_id.unwrap_or("acct-1").to_string(),
            },
        };
        Arc::new(ConnectionState::new(
            principal,
            device_id,
            role,
            tx,
            chrono::Utc::now().timestamp_millis(),
        ))
    }

    #[test]
    fn insert_replace_and_remove_current_are_aba_safe() {
        let reg = RealtimeConnectionRegistry::new();
        let id = DeviceId::new();
        let first = make_conn(id, DeviceRole::MobileClient, Some("a1"));
        assert!(reg.insert(Arc::clone(&first)).is_none());
        assert!(reg.get(id).unwrap().same_connection(&first));

        let second = make_conn(id, DeviceRole::MobileClient, Some("a1"));
        let prev = reg.insert(Arc::clone(&second)).expect("previous");
        assert!(prev.same_connection(&first));
        assert!(reg.get(id).unwrap().same_connection(&second));

        assert!(reg.remove_current(&first).is_none());
        assert!(reg.get(id).unwrap().same_connection(&second));
        assert!(reg.remove_current(&second).is_some());
        assert!(reg.get(id).is_none());
    }

    #[test]
    fn mobile_disconnect_grace_stamps_when_last_client_leaves() {
        let reg = RealtimeConnectionRegistry::new();
        let d1 = DeviceId::new();
        let d2 = DeviceId::new();
        let c1 = make_conn(d1, DeviceRole::MobileClient, Some("acct"));
        let c2 = make_conn(d2, DeviceRole::MobileClient, Some("acct"));
        reg.insert(c1);
        reg.insert(Arc::clone(&c2));
        assert!(reg.last_mobile_disconnect_at_ms("acct").is_none());
        assert_eq!(reg.mobile_client_session_count("acct"), 2);

        reg.remove(d1);
        assert!(reg.last_mobile_disconnect_at_ms("acct").is_none());
        assert_eq!(reg.mobile_client_session_count("acct"), 1);

        reg.remove_current(&c2);
        assert!(reg.last_mobile_disconnect_at_ms("acct").is_some());
        assert_eq!(reg.mobile_client_session_count("acct"), 0);
    }

    #[test]
    fn revoke_device_signals_and_evicts() {
        let reg = RealtimeConnectionRegistry::new();
        let id = DeviceId::new();
        let conn = make_conn(id, DeviceRole::AgentHost, None);
        let mut rx = conn.subscribe_revocation();
        reg.insert(Arc::clone(&conn));
        assert!(reg.revoke_device(id, ConnectionRevocation::AuthRevoked));
        assert!(reg.get(id).is_none());
        assert_eq!(*rx.borrow_and_update(), Some(ConnectionRevocation::AuthRevoked));
    }
}
