//! Same-account host link service (D02).
//!
//! Owns account↔host binding via `host_links` + `host_installation_tokens`.
//! QR pairing was removed; this is the only bind path.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use minos_domain::DeviceId;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres};

use crate::{
    error::BackendError,
    session::{SessionRegistry, SessionRevocation},
    store::{
        host_installation_tokens, host_links, AsStorePool, StoreHandle, StorePoolRef,
    },
};

/// Outcome of a successful [`HostLinkService::link_host`].
#[derive(Debug, Clone)]
pub struct HostLinkOutcome {
    pub host_installation_id: DeviceId,
    pub host_installation_token: String,
    pub link: host_links::PairRow,
}

#[derive(Debug)]
pub enum HostLinkError {
    HostLinkedElsewhere { account_id: String },
    NotFound,
    Internal(BackendError),
}

fn map_host_link_store_err(error: BackendError) -> HostLinkError {
    match error {
        BackendError::HostLinkedElsewhere { .. } => HostLinkError::HostLinkedElsewhere {
            account_id: String::new(),
        },
        other => HostLinkError::Internal(other),
    }
}

/// Stateless facade for host link / unlink.
#[derive(Debug, Clone)]
pub struct HostLinkService {
    pool: StoreHandle,
}

impl HostLinkService {
    #[must_use]
    pub fn new(pool: impl Into<StoreHandle>) -> Self {
        Self { pool: pool.into() }
    }

    /// Same-account host link: upsert `host_links` + mint a host installation token.
    ///
    /// Rejects hosts already linked to a different account (`HostLinkedElsewhere`).
    /// Exclusivity is checked **inside** the write transaction (with
    /// `UNIQUE (host_installation_id)` as belt-and-suspenders). Re-linking the
    /// same account rotates tokens (revoke all then issue a fresh `hit_*`).
    pub async fn link_host(
        &self,
        host_installation_id: DeviceId,
        account_id: &str,
        linked_via_installation_id: DeviceId,
        host_display_name: Option<&str>,
    ) -> Result<HostLinkOutcome, HostLinkError> {
        let now = Utc::now().timestamp_millis();
        let token = generate_host_installation_token();
        let token_hash = sha256_hex(&token);
        let link = match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
                    HostLinkError::Internal(BackendError::StoreQuery {
                        operation: "begin_host_link".into(),
                        message: e.to_string(),
                    })
                })?;
                let result: Result<host_links::PairRow, HostLinkError> = async {
                    host_links::assert_host_available_or_same_account_sqlite(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                    )
                    .await
                    .map_err(map_host_link_store_err)?;
                    let link = host_links::upsert_link_with_executor(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        host_display_name,
                        now,
                    )
                    .await
                    .map_err(map_host_link_store_err)?;
                    // Rotate: revoke prior host tokens then mint a fresh one.
                    host_installation_tokens::revoke_all_for_host_with_executor(
                        &mut *tx,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    host_installation_tokens::insert_token_with_executor(
                        &mut *tx,
                        &token_hash,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    Ok(link)
                }
                .await;
                match result {
                    Ok(link) => {
                        tx.commit().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "commit_host_link".into(),
                                message: e.to_string(),
                            })
                        })?;
                        link
                    }
                    Err(err) => {
                        tx.rollback().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "rollback_host_link".into(),
                                message: e.to_string(),
                            })
                        })?;
                        return Err(err);
                    }
                }
            }
            StorePoolRef::Postgres(pool) => {
                let mut tx = begin_serializable_postgres_tx(pool, "begin_host_link")
                    .await
                    .map_err(HostLinkError::Internal)?;
                let result: Result<host_links::PairRow, HostLinkError> = async {
                    host_links::assert_host_available_or_same_account_postgres(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                    )
                    .await
                    .map_err(map_host_link_store_err)?;
                    let link = host_links::upsert_link_with_postgres_executor(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        host_display_name,
                        now,
                    )
                    .await
                    .map_err(map_host_link_store_err)?;
                    host_installation_tokens::revoke_all_for_host_with_postgres_executor(
                        &mut *tx,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    host_installation_tokens::insert_token_with_postgres_executor(
                        &mut *tx,
                        &token_hash,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    Ok(link)
                }
                .await;
                match result {
                    Ok(link) => {
                        tx.commit().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "commit_host_link".into(),
                                message: e.to_string(),
                            })
                        })?;
                        link
                    }
                    Err(err) => {
                        tx.rollback().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "rollback_host_link".into(),
                                message: e.to_string(),
                            })
                        })?;
                        return Err(err);
                    }
                }
            }
        };

        let _ = crate::ingest::invalidate_peer_targets_for_host(host_installation_id).await;
        let _ = crate::ingest::invalidate_peer_targets_for_account(&self.pool, account_id).await;

        Ok(HostLinkOutcome {
            host_installation_id,
            host_installation_token: token,
            link,
        })
    }

    /// Unlink host for one account: delete link, always revoke host tokens,
    /// kill live `/ws/host`, and invalidate peer-target caches.
    pub async fn unlink_host(
        &self,
        registry: &SessionRegistry,
        host_installation_id: DeviceId,
        account_id: &str,
    ) -> Result<(), HostLinkError> {
        let now = Utc::now().timestamp_millis();
        let deleted = match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
                    HostLinkError::Internal(BackendError::StoreQuery {
                        operation: "begin_host_unlink".into(),
                        message: e.to_string(),
                    })
                })?;
                let result: Result<u64, HostLinkError> = async {
                    let deleted = host_links::delete_pair_with_executor(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    if deleted == 0 {
                        return Ok(0);
                    }
                    host_installation_tokens::revoke_all_for_host_with_executor(
                        &mut *tx,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    Ok(deleted)
                }
                .await;
                match result {
                    Ok(deleted) => {
                        tx.commit().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "commit_host_unlink".into(),
                                message: e.to_string(),
                            })
                        })?;
                        deleted
                    }
                    Err(err) => {
                        tx.rollback().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "rollback_host_unlink".into(),
                                message: e.to_string(),
                            })
                        })?;
                        return Err(err);
                    }
                }
            }
            StorePoolRef::Postgres(pool) => {
                let mut tx = begin_serializable_postgres_tx(pool, "begin_host_unlink")
                    .await
                    .map_err(HostLinkError::Internal)?;
                let result: Result<u64, HostLinkError> = async {
                    let deleted = host_links::delete_pair_with_postgres_executor(
                        &mut *tx,
                        host_installation_id,
                        account_id,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    if deleted == 0 {
                        return Ok(0);
                    }
                    host_installation_tokens::revoke_all_for_host_with_postgres_executor(
                        &mut *tx,
                        host_installation_id,
                        now,
                    )
                    .await
                    .map_err(HostLinkError::Internal)?;
                    Ok(deleted)
                }
                .await;
                match result {
                    Ok(deleted) => {
                        tx.commit().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "commit_host_unlink".into(),
                                message: e.to_string(),
                            })
                        })?;
                        deleted
                    }
                    Err(err) => {
                        tx.rollback().await.map_err(|e| {
                            HostLinkError::Internal(BackendError::StoreQuery {
                                operation: "rollback_host_unlink".into(),
                                message: e.to_string(),
                            })
                        })?;
                        return Err(err);
                    }
                }
            }
        };

        if deleted == 0 {
            return Err(HostLinkError::NotFound);
        }

        let _ = crate::ingest::invalidate_peer_targets_for_host(host_installation_id).await;
        let _ = crate::ingest::invalidate_peer_targets_for_account(&self.pool, account_id).await;
        if let Some(handle) = registry.remove(host_installation_id) {
            handle.revoke(SessionRevocation::AuthRevoked);
        }
        Ok(())
    }
}

async fn begin_serializable_postgres_tx<'a>(
    pool: &'a PgPool,
    operation: &'static str,
) -> Result<sqlx::Transaction<'a, Postgres>, BackendError> {
    let mut tx = pool.begin().await.map_err(|e| BackendError::StoreQuery {
        operation: operation.to_string(),
        message: e.to_string(),
    })?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(|e| BackendError::StoreQuery {
            operation: format!("{operation}.set_isolation"),
            message: e.to_string(),
        })?;
    Ok(tx)
}

/// SHA-256 hex digest of a UTF-8 string.
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String write never fails");
    }
    out
}

fn generate_host_installation_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    format!("hit_{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn sha256_hex_matches_known_vector_and_is_deterministic() {
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex("abc"), want);
        assert_eq!(sha256_hex("abc"), sha256_hex("abc"));
    }

    #[test]
    fn sha256_hex_output_is_64_hex_chars() {
        let d = sha256_hex("any input");
        assert_eq!(d.len(), 64);
        assert!(d
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
