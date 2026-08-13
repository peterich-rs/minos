//! Same-account host link service (D02).
//!
//! Owns account↔host binding via `host_links` + `host_tokens`.
//! QR pairing was removed; this is the only bind path.
//!
//! Multi-end roster: every successful link/unlink records `HostLinked` /
//! `HostUnlinked` durable + outbox on `account:{id}` in the **same** write
//! transaction (Realtime Surface R1, T2 digest).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use minos_domain::DeviceId;
use minos_protocol::DurableEvent;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

use crate::{
    app::tx::DbTx,
    error::BackendError,
    realtime::{ConnectionRevocation, RealtimeConnectionRegistry},
    store::{
        durable_event_log, host_links, host_tokens, outbox_events, AsStorePool, StoreHandle,
        StorePoolRef,
    },
};

/// Outcome of a successful [`HostLinkService::link_host`].
#[derive(Debug, Clone)]
pub struct HostLinkOutcome {
    pub host_device_id: DeviceId,
    pub host_installation_token: String,
    pub link: host_links::PairRow,
    /// Deterministic durable event id enqueued for account roster fanout.
    pub durable_event_id: String,
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

/// Deterministic HostLinked event id (idempotent re-link same pair_id).
#[must_use]
pub fn host_linked_event_id(account_id: &str, host_device_id: &str, pair_id: &str) -> String {
    format!("host-linked-{account_id}-{host_device_id}-{pair_id}")
}

/// Deterministic HostUnlinked event id (pair_id from pre-delete row).
#[must_use]
pub fn host_unlinked_event_id(account_id: &str, host_device_id: &str, pair_id: &str) -> String {
    format!("host-unlinked-{account_id}-{host_device_id}-{pair_id}")
}

/// Idempotent on deterministic `event_id`: same-account re-link keeps token
/// rotation but does not re-insert HostLinked / HostUnlinked durable rows.
async fn enqueue_host_roster_durable(
    tx: &mut DbTx<'_>,
    event_id: &str,
    event: &DurableEvent,
    at_ms: i64,
) -> Result<(), BackendError> {
    let topic_kind = event.topic().kind().as_str();
    let exists = match tx {
        DbTx::Sqlite(raw) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
               FROM durable_event_log
              WHERE topic_kind = ?
                AND event_id = ?",
        )
        .bind(topic_kind)
        .bind(event_id)
        .fetch_one(&mut **raw)
        .await
        .map(|n| n > 0),
        DbTx::Postgres(raw) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
               FROM durable_event_log
              WHERE topic_kind = $1
                AND event_id = $2",
        )
        .bind(topic_kind)
        .bind(event_id)
        .fetch_one(&mut **raw)
        .await
        .map(|n| n > 0),
    }
    .map_err(|e| BackendError::StoreQuery {
        operation: "host_link::durable_exists".into(),
        message: e.to_string(),
    })?;
    if exists {
        return Ok(());
    }

    let cursor = durable_event_log::record_in_tx(tx, event_id, event, at_ms).await?;
    let outbox_id = Uuid::new_v4().to_string();
    outbox_events::enqueue_in_tx(
        tx,
        &outbox_id,
        cursor.topic.kind().as_str(),
        &cursor.event_id,
        outbox_events::OutboxLane::SocialDurable,
        at_ms,
    )
    .await?;
    Ok(())
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
    /// `UNIQUE (host_device_id)` as belt-and-suspenders). Re-linking the
    /// same account rotates tokens (revoke all then issue a fresh `hit_*`).
    ///
    /// On success, records `DurableEvent::HostLinked` + outbox in the same tx.
    pub async fn link_host(
        &self,
        host_device_id: DeviceId,
        account_id: &str,
        linked_via_device_id: DeviceId,
        host_display_name: Option<&str>,
    ) -> Result<HostLinkOutcome, HostLinkError> {
        let now = Utc::now().timestamp_millis();
        let token = generate_host_installation_token();
        let token_hash = sha256_hex(&token);
        let (link, durable_event_id) = match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let mut raw_tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
                    HostLinkError::Internal(BackendError::StoreQuery {
                        operation: "begin_host_link".into(),
                        message: e.to_string(),
                    })
                })?;
                host_links::assert_host_available_or_same_account_sqlite(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                )
                .await
                .map_err(map_host_link_store_err)?;
                let link = host_links::upsert_link_with_executor(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                    linked_via_device_id,
                    host_display_name,
                    now,
                )
                .await
                .map_err(map_host_link_store_err)?;
                host_tokens::revoke_all_for_host_with_executor(&mut *raw_tx, host_device_id, now)
                    .await
                    .map_err(HostLinkError::Internal)?;
                host_tokens::insert_token_with_executor(
                    &mut *raw_tx,
                    &token_hash,
                    host_device_id,
                    Some(account_id),
                    now,
                )
                .await
                .map_err(HostLinkError::Internal)?;

                let event_id =
                    host_linked_event_id(account_id, &host_device_id.to_string(), &link.pair_id);
                let display = link
                    .link_display_name
                    .clone()
                    .or_else(|| host_display_name.map(str::to_string));
                let event = DurableEvent::HostLinked {
                    account_id: account_id.to_string(),
                    host_device_id: host_device_id.to_string(),
                    pair_id: link.pair_id.clone(),
                    at_ms: now,
                    host_display_name: display,
                };
                let mut db_tx = DbTx::Sqlite(raw_tx);
                enqueue_host_roster_durable(&mut db_tx, &event_id, &event, now)
                    .await
                    .map_err(HostLinkError::Internal)?;
                db_tx.commit().await.map_err(HostLinkError::Internal)?;
                (link, event_id)
            }
            StorePoolRef::Postgres(pool) => {
                let mut raw_tx = begin_serializable_postgres_tx(pool, "begin_host_link")
                    .await
                    .map_err(HostLinkError::Internal)?;
                host_links::assert_host_available_or_same_account_postgres(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                )
                .await
                .map_err(map_host_link_store_err)?;
                let link = host_links::upsert_link_with_postgres_executor(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                    linked_via_device_id,
                    host_display_name,
                    now,
                )
                .await
                .map_err(map_host_link_store_err)?;
                host_tokens::revoke_all_for_host_with_postgres_executor(
                    &mut *raw_tx,
                    host_device_id,
                    now,
                )
                .await
                .map_err(HostLinkError::Internal)?;
                host_tokens::insert_token_with_postgres_executor(
                    &mut *raw_tx,
                    &token_hash,
                    host_device_id,
                    Some(account_id),
                    now,
                )
                .await
                .map_err(HostLinkError::Internal)?;

                let event_id =
                    host_linked_event_id(account_id, &host_device_id.to_string(), &link.pair_id);
                let display = link
                    .link_display_name
                    .clone()
                    .or_else(|| host_display_name.map(str::to_string));
                let event = DurableEvent::HostLinked {
                    account_id: account_id.to_string(),
                    host_device_id: host_device_id.to_string(),
                    pair_id: link.pair_id.clone(),
                    at_ms: now,
                    host_display_name: display,
                };
                let mut db_tx = DbTx::Postgres(raw_tx);
                enqueue_host_roster_durable(&mut db_tx, &event_id, &event, now)
                    .await
                    .map_err(HostLinkError::Internal)?;
                db_tx.commit().await.map_err(HostLinkError::Internal)?;
                (link, event_id)
            }
        };

        let _ = crate::ingest::invalidate_peer_targets_for_host(host_device_id).await;
        let _ = crate::ingest::invalidate_peer_targets_for_account(&self.pool, account_id).await;

        Ok(HostLinkOutcome {
            host_device_id,
            host_installation_token: token,
            link,
            durable_event_id,
        })
    }

    /// Unlink host for one account: delete link, always revoke host tokens,
    /// kill live `/ws/host`, and invalidate peer-target caches.
    ///
    /// On success, records `DurableEvent::HostUnlinked` + outbox in the same tx.
    /// Returns the durable event id.
    pub async fn unlink_host(
        &self,
        registry: &RealtimeConnectionRegistry,
        host_device_id: DeviceId,
        account_id: &str,
    ) -> Result<String, HostLinkError> {
        let now = Utc::now().timestamp_millis();
        let durable_event_id = match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                let mut raw_tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
                    HostLinkError::Internal(BackendError::StoreQuery {
                        operation: "begin_host_unlink".into(),
                        message: e.to_string(),
                    })
                })?;
                let existing =
                    host_links::get_pair_with_executor(&mut *raw_tx, host_device_id, account_id)
                        .await
                        .map_err(HostLinkError::Internal)?;
                let Some(existing) = existing else {
                    return Err(HostLinkError::NotFound);
                };
                let deleted =
                    host_links::delete_pair_with_executor(&mut *raw_tx, host_device_id, account_id)
                        .await
                        .map_err(HostLinkError::Internal)?;
                if deleted == 0 {
                    return Err(HostLinkError::NotFound);
                }
                host_tokens::revoke_all_for_host_with_executor(&mut *raw_tx, host_device_id, now)
                    .await
                    .map_err(HostLinkError::Internal)?;

                let event_id = host_unlinked_event_id(
                    account_id,
                    &host_device_id.to_string(),
                    &existing.pair_id,
                );
                let event = DurableEvent::HostUnlinked {
                    account_id: account_id.to_string(),
                    host_device_id: host_device_id.to_string(),
                    at_ms: now,
                };
                let mut db_tx = DbTx::Sqlite(raw_tx);
                enqueue_host_roster_durable(&mut db_tx, &event_id, &event, now)
                    .await
                    .map_err(HostLinkError::Internal)?;
                db_tx.commit().await.map_err(HostLinkError::Internal)?;
                event_id
            }
            StorePoolRef::Postgres(pool) => {
                let mut raw_tx = begin_serializable_postgres_tx(pool, "begin_host_unlink")
                    .await
                    .map_err(HostLinkError::Internal)?;
                let existing = host_links::get_pair_with_postgres_executor(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                )
                .await
                .map_err(HostLinkError::Internal)?;
                let Some(existing) = existing else {
                    return Err(HostLinkError::NotFound);
                };
                let deleted = host_links::delete_pair_with_postgres_executor(
                    &mut *raw_tx,
                    host_device_id,
                    account_id,
                )
                .await
                .map_err(HostLinkError::Internal)?;
                if deleted == 0 {
                    return Err(HostLinkError::NotFound);
                }
                host_tokens::revoke_all_for_host_with_postgres_executor(
                    &mut *raw_tx,
                    host_device_id,
                    now,
                )
                .await
                .map_err(HostLinkError::Internal)?;

                let event_id = host_unlinked_event_id(
                    account_id,
                    &host_device_id.to_string(),
                    &existing.pair_id,
                );
                let event = DurableEvent::HostUnlinked {
                    account_id: account_id.to_string(),
                    host_device_id: host_device_id.to_string(),
                    at_ms: now,
                };
                let mut db_tx = DbTx::Postgres(raw_tx);
                enqueue_host_roster_durable(&mut db_tx, &event_id, &event, now)
                    .await
                    .map_err(HostLinkError::Internal)?;
                db_tx.commit().await.map_err(HostLinkError::Internal)?;
                event_id
            }
        };

        let _ = crate::ingest::invalidate_peer_targets_for_host(host_device_id).await;
        let _ = crate::ingest::invalidate_peer_targets_for_account(&self.pool, account_id).await;
        let _ = registry.revoke_device(host_device_id, ConnectionRevocation::AuthRevoked);
        Ok(durable_event_id)
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
    use crate::realtime::RealtimeConnectionRegistry;
    use crate::store::test_support::insert_test_host;
    use crate::store::test_support::{insert_account, insert_ios_device, memory_pool, T0};
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

    #[test]
    fn host_linked_event_id_is_stable() {
        assert_eq!(
            host_linked_event_id("acc", "host", "pair"),
            "host-linked-acc-host-pair"
        );
    }

    #[tokio::test]
    async fn link_host_enqueues_host_linked_durable() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "link-durable@example.com").await;
        let host = DeviceId::new();
        insert_test_host(&pool, host, "Office Mac", T0).await;
        let via = insert_ios_device(&pool, &account).await;
        let svc = HostLinkService::new(pool.clone());
        let outcome = svc
            .link_host(host, &account, via, Some("Office Mac"))
            .await
            .expect("link");

        let topic = format!("account:{account}");
        let rows = durable_event_log::read_topic_after(&pool, "account", &topic, 0, 10)
            .await
            .expect("read durable");
        assert!(
            rows.iter().any(|r| {
                r.event_id == outcome.durable_event_id
                    && r.payload_json.get("kind").and_then(|k| k.as_str()) == Some("host_linked")
            }),
            "expected host_linked durable, got {rows:?}"
        );
    }

    #[tokio::test]
    async fn unlink_host_enqueues_host_unlinked_durable() {
        let pool = memory_pool().await;
        let account = insert_account(&pool, "unlink-durable@example.com").await;
        let host = DeviceId::new();
        insert_test_host(&pool, host, "Laptop", T0).await;
        let via = insert_ios_device(&pool, &account).await;
        let svc = HostLinkService::new(pool.clone());
        svc.link_host(host, &account, via, Some("Laptop"))
            .await
            .expect("link");
        let registry = RealtimeConnectionRegistry::new();
        let event_id = svc
            .unlink_host(&registry, host, &account)
            .await
            .expect("unlink");

        let topic = format!("account:{account}");
        let rows = durable_event_log::read_topic_after(&pool, "account", &topic, 0, 20)
            .await
            .expect("read durable");
        assert!(
            rows.iter().any(|r| {
                r.event_id == event_id
                    && r.payload_json.get("kind").and_then(|k| k.as_str()) == Some("host_unlinked")
            }),
            "expected host_unlinked durable, got {rows:?}"
        );
    }
}
