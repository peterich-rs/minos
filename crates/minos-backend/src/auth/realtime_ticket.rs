//! Short-lived, single-use realtime gateway ticket registry.
//!
//! Tickets remain signed JWTs for compact wire compatibility, but a verified
//! JWT is accepted by `/ws/client` and `/ws/host` only if its `jti` is present
//! in this registry. Consuming removes the row, so reconnects must ask the
//! control API for a fresh ticket.

use dashmap::DashMap;
use minos_domain::DeviceRole;

use crate::auth::jwt::WsTicketClaims;

#[derive(Debug, Clone)]
struct RealtimeTicketEntry {
    sub: String,
    did: String,
    role: DeviceRole,
    exp: i64,
}

#[derive(Debug, Default)]
pub struct RealtimeTicketStore {
    entries: DashMap<String, RealtimeTicketEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeTicketConsumeError {
    Missing,
    Expired,
    Mismatch,
}

impl RealtimeTicketStore {
    pub fn register(&self, claims: &WsTicketClaims) {
        self.entries.insert(
            claims.jti.clone(),
            RealtimeTicketEntry {
                sub: claims.sub.clone(),
                did: claims.did.clone(),
                role: claims.role,
                exp: claims.exp,
            },
        );
    }

    pub fn consume(
        &self,
        claims: &WsTicketClaims,
        now_secs: i64,
    ) -> Result<(), RealtimeTicketConsumeError> {
        let Some((_, entry)) = self.entries.remove(&claims.jti) else {
            return Err(RealtimeTicketConsumeError::Missing);
        };
        if entry.exp <= now_secs {
            return Err(RealtimeTicketConsumeError::Expired);
        }
        if entry.sub != claims.sub
            || entry.did != claims.did
            || entry.role != claims.role
            || entry.exp != claims.exp
        {
            return Err(RealtimeTicketConsumeError::Mismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn claims(jti: &str) -> WsTicketClaims {
        WsTicketClaims {
            sub: "acct".into(),
            did: "device".into(),
            role: DeviceRole::BrowserAdmin,
            iat: 100,
            exp: 160,
            jti: jti.into(),
        }
    }

    #[test]
    fn ticket_is_single_use() {
        let store = RealtimeTicketStore::default();
        let c = claims(&Uuid::new_v4().to_string());
        store.register(&c);

        assert_eq!(store.consume(&c, 120), Ok(()));
        assert_eq!(
            store.consume(&c, 121),
            Err(RealtimeTicketConsumeError::Missing)
        );
    }

    #[test]
    fn expired_ticket_is_rejected_and_burned() {
        let store = RealtimeTicketStore::default();
        let c = claims(&Uuid::new_v4().to_string());
        store.register(&c);

        assert_eq!(
            store.consume(&c, 160),
            Err(RealtimeTicketConsumeError::Expired)
        );
        assert_eq!(
            store.consume(&c, 161),
            Err(RealtimeTicketConsumeError::Missing)
        );
    }
}
