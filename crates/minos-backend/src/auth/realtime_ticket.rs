//! Short-lived, single-use realtime gateway ticket registry.
//!
//! Tickets remain signed JWTs for compact wire compatibility, but a verified
//! JWT is accepted by `/ws/client` and `/ws/host` only if its `jti` is present
//! in this registry. Runtime shells can back the registry with Redis so ticket
//! consume is not process-local; tests and single-process dev continue to use an
//! in-memory fallback.

use std::sync::Arc;

use dashmap::DashMap;
use minos_domain::DeviceRole;
use serde::{Deserialize, Serialize};

use crate::auth::jwt::WsTicketClaims;
use crate::error::BackendError;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RealtimeTicketEntry {
    sub: String,
    did: String,
    role: DeviceRole,
    exp: i64,
}

impl From<&WsTicketClaims> for RealtimeTicketEntry {
    fn from(claims: &WsTicketClaims) -> Self {
        Self {
            sub: claims.sub.clone(),
            did: claims.did.clone(),
            role: claims.role,
            exp: claims.exp,
        }
    }
}

#[derive(Debug, Clone)]
enum RealtimeTicketBackend {
    InMemory(Arc<DashMap<String, RealtimeTicketEntry>>),
    Redis { client: redis::Client },
}

#[derive(Debug, Clone)]
pub struct RealtimeTicketStore {
    backend: RealtimeTicketBackend,
}

impl Default for RealtimeTicketStore {
    fn default() -> Self {
        Self::in_memory()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeTicketConsumeError {
    Missing,
    Expired,
    Mismatch,
    Store(String),
}

impl RealtimeTicketStore {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            backend: RealtimeTicketBackend::InMemory(Arc::new(DashMap::new())),
        }
    }

    pub fn redis(redis_url: &str) -> Result<Self, BackendError> {
        let client = redis::Client::open(redis_url).map_err(|error| BackendError::Cache {
            operation: "realtime_ticket.redis_client".into(),
            message: error.to_string(),
        })?;
        Ok(Self {
            backend: RealtimeTicketBackend::Redis { client },
        })
    }

    pub async fn register(&self, claims: &WsTicketClaims) -> Result<(), BackendError> {
        let entry = RealtimeTicketEntry::from(claims);
        match &self.backend {
            RealtimeTicketBackend::InMemory(entries) => {
                entries.insert(claims.jti.clone(), entry);
                Ok(())
            }
            RealtimeTicketBackend::Redis { client } => {
                let mut conn = client.get_multiplexed_async_connection().await.map_err(|error| {
                    BackendError::Cache {
                        operation: "realtime_ticket.redis_connect".into(),
                        message: error.to_string(),
                    }
                })?;
                let payload = serde_json::to_string(&entry).map_err(|error| BackendError::Cache {
                    operation: "realtime_ticket.redis_encode".into(),
                    message: error.to_string(),
                })?;
                let ttl_secs = u64::try_from(claims.exp.saturating_sub(claims.iat).max(1))
                    .unwrap_or(1);
                let _: () = redis::cmd("SET")
                    .arg(ticket_key(&claims.jti))
                    .arg(payload)
                    .arg("EX")
                    .arg(ttl_secs)
                    .query_async(&mut conn)
                    .await
                    .map_err(|error| BackendError::Cache {
                        operation: "realtime_ticket.redis_set".into(),
                        message: error.to_string(),
                    })?;
                Ok(())
            }
        }
    }

    pub async fn consume(
        &self,
        claims: &WsTicketClaims,
        now_secs: i64,
    ) -> Result<(), RealtimeTicketConsumeError> {
        let entry = match &self.backend {
            RealtimeTicketBackend::InMemory(entries) => entries
                .remove(&claims.jti)
                .map(|(_, entry)| entry),
            RealtimeTicketBackend::Redis { client } => {
                let mut conn = client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|error| RealtimeTicketConsumeError::Store(error.to_string()))?;
                let payload: Option<String> = redis::cmd("GETDEL")
                    .arg(ticket_key(&claims.jti))
                    .query_async(&mut conn)
                    .await
                    .map_err(|error| RealtimeTicketConsumeError::Store(error.to_string()))?;
                payload
                    .map(|payload| {
                        serde_json::from_str::<RealtimeTicketEntry>(&payload).map_err(|error| {
                            RealtimeTicketConsumeError::Store(error.to_string())
                        })
                    })
                    .transpose()?
            }
        };

        let Some(entry) = entry else {
            return Err(RealtimeTicketConsumeError::Missing);
        };
        validate_entry(&entry, claims, now_secs)
    }
}

fn ticket_key(jti: &str) -> String {
    format!("minos:ticket:{jti}")
}

fn validate_entry(
    entry: &RealtimeTicketEntry,
    claims: &WsTicketClaims,
    now_secs: i64,
) -> Result<(), RealtimeTicketConsumeError> {
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

    #[tokio::test]
    async fn ticket_is_single_use() {
        let store = RealtimeTicketStore::default();
        let c = claims(&Uuid::new_v4().to_string());
        store.register(&c).await.unwrap();

        assert_eq!(store.consume(&c, 120).await, Ok(()));
        assert_eq!(
            store.consume(&c, 121).await,
            Err(RealtimeTicketConsumeError::Missing)
        );
    }

    #[tokio::test]
    async fn expired_ticket_is_rejected_and_burned() {
        let store = RealtimeTicketStore::default();
        let c = claims(&Uuid::new_v4().to_string());
        store.register(&c).await.unwrap();

        assert_eq!(
            store.consume(&c, 160).await,
            Err(RealtimeTicketConsumeError::Expired)
        );
        assert_eq!(
            store.consume(&c, 161).await,
            Err(RealtimeTicketConsumeError::Missing)
        );
    }

    #[tokio::test]
    async fn mismatched_ticket_is_rejected_and_burned() {
        let store = RealtimeTicketStore::default();
        let c = claims(&Uuid::new_v4().to_string());
        store.register(&c).await.unwrap();

        let mut mismatched = c.clone();
        mismatched.sub = "different-account".into();

        assert_eq!(
            store.consume(&mismatched, 120).await,
            Err(RealtimeTicketConsumeError::Mismatch)
        );
        assert_eq!(
            store.consume(&c, 121).await,
            Err(RealtimeTicketConsumeError::Missing)
        );
    }
}
