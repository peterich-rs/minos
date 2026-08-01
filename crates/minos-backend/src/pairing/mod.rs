//! Backend-side pairing service: token issuance and token consumption.
//!
//! Sits on top of `store::{device_installations, tokens}` and layers the business
//! rules of spec §6.1 / §7 onto the CRUD:
//!
//! 1. Request — the Mac host asks for a fresh 5-minute token, which is
//!    persisted as a SHA-256 digest (never the plaintext). The plaintext
//!    is returned once for QR rendering and then discarded.
//! 2. Consume — the iOS client presents a candidate token; we atomically
//!    mark the row consumed and mint a `DeviceSecret` for the Mac issuer
//!    (returned in the outcome for legacy event payloads only — **not**
//!    persisted; `secret_hash` was removed). Host steady-state auth uses
//!    `host_installation_tokens` from formal redeem.
//!
//! The account↔host link in `host_links` is inserted by the HTTP handler
//! post-commit — see `http::v1::pairing`. `consume_token` does not see
//! the bearer's `account_id`, so it cannot insert the link itself.
//!
//! # Two hash primitives
//!
//! - `secret::hash_secret` — argon2id PHC string for at-rest `DeviceSecret`.
//!   Tuned for "brute-force resistant if the DB is stolen".
//! - `sha2::Sha256` hex digest for `PairingToken`. Deterministic for PK
//!   lookup; safe because tokens carry 256 bits of entropy and expire in
//!   5 minutes.
//!
//! # Atomicity
//!
//! `consume_token` starts with `BEGIN IMMEDIATE`, then wraps token validation,
//! token consumption, and the issuer's secret-hash upsert in one SQLite
//! transaction. That write lock serializes concurrent consumes before any
//! token lookup. Any failure rolls the whole transaction back so the token
//! is still usable and no partial secret leaks into the store.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, PairingToken};
use minos_protocol::{Envelope, EventKind};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, SqlitePool};

use crate::{
    error::BackendError,
    session::{SessionRegistry, SessionRevocation},
    store::{
        device_installations, host_installation_tokens, host_links, pairing_codes, tokens,
        AsStorePool, StoreHandle, StorePoolRef,
    },
};

pub mod secret;

/// Successful outcome of [`PairingService::consume_token`].
///
/// Carries the Mac issuer's plaintext `DeviceSecret` momentarily so the
/// caller can push it via `EventKind::Paired`. Not persisted (device-secret
/// / `secret_hash` rail removed); formal host auth uses installation tokens.
#[derive(Debug, Clone)]
pub struct PairingOutcome {
    /// `DeviceId` of the side that originally issued the pairing token
    /// (the Mac host).
    pub issuer_device_id: DeviceId,
    /// Plaintext secret minted for the issuer (to be delivered to the Mac).
    pub issuer_secret: DeviceSecret,
}

#[derive(Debug, Clone)]
pub struct PairingCompletion {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
}

#[derive(Debug)]
pub enum ConsumePairingError {
    DeliveryFailed,
    Internal(BackendError),
    IssuerOffline,
    PairingStateMismatch { actual: String },
    PairingTokenInvalid,
}

#[derive(Debug, Clone)]
pub struct FormalPairingConfirm {
    pub host_installation_id: DeviceId,
    pub already_confirmed: bool,
}

#[derive(Debug, Clone)]
pub struct HostInstallationToken {
    pub host_installation_id: DeviceId,
    pub account_id: String,
    pub token: String,
    pub issued_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct FormalRevokeOutcome {
    pub host_installation_id: DeviceId,
    pub remaining_link_count: i64,
    pub revoked_token_count: u64,
}

#[derive(Debug)]
pub enum FormalPairingError {
    Internal(BackendError),
    PairingNotConfirmed,
    PairingCodeInvalid,
    PairingStateMismatch { actual: String },
}

/// Stateless facade around the pairing-related store helpers.
///
/// Cheap to clone — just holds a `SqlitePool` handle. Usually instantiated
/// once in `main.rs` and shared via `Arc`.
#[derive(Debug, Clone)]
pub struct PairingService {
    pool: StoreHandle,
}

impl PairingService {
    /// Construct a service backed by `pool`. The pool must already have
    /// migrations applied (use [`crate::store::connect`]).
    #[must_use]
    pub fn new(pool: impl Into<StoreHandle>) -> Self {
        Self { pool: pool.into() }
    }

    /// Mint a fresh pairing token for `issuer`.
    ///
    /// Returns the plaintext token (for QR rendering) and its absolute
    /// expiry time. Only the SHA-256 digest of the plaintext is persisted,
    /// so the plaintext cannot be recovered from a DB dump.
    ///
    /// # Errors
    ///
    /// - [`BackendError::StoreQuery`] — the underlying `INSERT` failed (for
    ///   example an FK violation if `issuer` has not been inserted yet).
    pub async fn request_token(
        &self,
        issuer: DeviceId, // host no longer bound via account_id
        ttl: Duration,
    ) -> Result<(PairingToken, DateTime<Utc>), BackendError> {
        let result = self.request_token_inner(issuer, ttl).await;
        crate::telemetry::record_pairing_token_issue(pairing_token_issue_outcome(&result));
        result
    }

    async fn request_token_inner(
        &self,
        issuer: DeviceId, // host no longer bound via account_id
        ttl: Duration,
    ) -> Result<(PairingToken, DateTime<Utc>), BackendError> {
        let now = Utc::now();
        // `Duration::from_std` fails only on values beyond i64 nanoseconds
        // (~292 years). 5-minute TTL is nowhere near that.
        let expires = now
            + chrono::Duration::from_std(ttl).map_err(|e| BackendError::PairingHash {
                message: format!("ttl out of range: {e}"),
            })?;

        let plain = PairingToken::generate();
        let digest = sha256_hex(plain.as_str());

        tokens::issue_token(
            &self.pool,
            &digest,
            issuer,
            expires.timestamp_millis(),
            now.timestamp_millis(),
        )
        .await?;

        Ok((plain, expires))
    }

    /// Mint a formal pairing code in the `pending` state.
    pub async fn request_code(
        &self,
        host_installation_id: DeviceId,
        ttl: Duration,
    ) -> Result<(String, DateTime<Utc>), BackendError> {
        let now = Utc::now();
        let expires = now
            + chrono::Duration::from_std(ttl).map_err(|e| BackendError::PairingHash {
                message: format!("ttl out of range: {e}"),
            })?;
        let code = generate_pairing_code();
        let digest = sha256_hex(&code);
        pairing_codes::insert_code(
            &self.pool,
            &digest,
            host_installation_id,
            now.timestamp_millis(),
            expires.timestamp_millis(),
        )
        .await?;
        Ok((code, expires))
    }

    pub async fn confirm_pairing_code(
        &self,
        pairing_code: &str,
        account_id: &str,
        linked_via_installation_id: DeviceId,
        client_request_id: Option<&str>,
    ) -> Result<FormalPairingConfirm, FormalPairingError> {
        match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                self.confirm_pairing_code_sqlite(
                    pool,
                    pairing_code,
                    account_id,
                    linked_via_installation_id,
                    client_request_id,
                )
                .await
            }
            StorePoolRef::Postgres(pool) => {
                self.confirm_pairing_code_postgres(
                    pool,
                    pairing_code,
                    account_id,
                    linked_via_installation_id,
                    client_request_id,
                )
                .await
            }
        }
    }

    async fn confirm_pairing_code_sqlite(
        &self,
        pool: &SqlitePool,
        pairing_code: &str,
        account_id: &str,
        linked_via_installation_id: DeviceId,
        client_request_id: Option<&str>,
    ) -> Result<FormalPairingConfirm, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let digest = sha256_hex(pairing_code);
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
            FormalPairingError::Internal(BackendError::StoreQuery {
                operation: "begin_formal_pairing_confirm".to_string(),
                message: e.to_string(),
            })
        })?;

        let result: Result<FormalPairingConfirm, FormalPairingError> = async {
            let row = pairing_codes::get_code_with_executor(&mut *tx, &digest)
                .await
                .map_err(FormalPairingError::Internal)?
                .ok_or(FormalPairingError::PairingCodeInvalid)?;

            if row.expires_at_ms <= now {
                return Err(FormalPairingError::PairingCodeInvalid);
            }

            match row.status {
                pairing_codes::PairingCodeStatus::Pending => {
                    let updated = pairing_codes::confirm_code_with_executor(
                        &mut *tx,
                        &digest,
                        account_id,
                        linked_via_installation_id,
                        client_request_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    if updated != 1 {
                        return Err(FormalPairingError::PairingCodeInvalid);
                    }
                    host_links::insert_pair_with_executor(
                        &mut *tx,
                        row.host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    Ok(FormalPairingConfirm {
                        host_installation_id: row.host_installation_id,
                        already_confirmed: false,
                    })
                }
                pairing_codes::PairingCodeStatus::Confirmed => {
                    if row.account_id.as_deref() != Some(account_id) {
                        return Err(FormalPairingError::PairingStateMismatch {
                            actual: "confirmed_by_different_account".to_string(),
                        });
                    }
                    host_links::insert_pair_with_executor(
                        &mut *tx,
                        row.host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    Ok(FormalPairingConfirm {
                        host_installation_id: row.host_installation_id,
                        already_confirmed: true,
                    })
                }
                pairing_codes::PairingCodeStatus::Redeemed
                | pairing_codes::PairingCodeStatus::Expired => {
                    Err(FormalPairingError::PairingCodeInvalid)
                }
            }
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_formal_pairing_confirm".to_string(),
                        message: e.to_string(),
                    })
                })?;
                let _ =
                    crate::ingest::invalidate_peer_targets_for_host(outcome.host_installation_id)
                        .await;
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_formal_pairing_confirm".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    async fn confirm_pairing_code_postgres(
        &self,
        pool: &PgPool,
        pairing_code: &str,
        account_id: &str,
        linked_via_installation_id: DeviceId,
        client_request_id: Option<&str>,
    ) -> Result<FormalPairingConfirm, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let digest = sha256_hex(pairing_code);
        let mut tx = begin_serializable_postgres_tx(pool, "begin_formal_pairing_confirm")
            .await
            .map_err(FormalPairingError::Internal)?;

        let result: Result<FormalPairingConfirm, FormalPairingError> = async {
            let row = pairing_codes::get_code_with_postgres_executor(&mut *tx, &digest)
                .await
                .map_err(FormalPairingError::Internal)?
                .ok_or(FormalPairingError::PairingCodeInvalid)?;

            if row.expires_at_ms <= now {
                return Err(FormalPairingError::PairingCodeInvalid);
            }

            match row.status {
                pairing_codes::PairingCodeStatus::Pending => {
                    let updated = pairing_codes::confirm_code_with_postgres_executor(
                        &mut *tx,
                        &digest,
                        account_id,
                        linked_via_installation_id,
                        client_request_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    if updated != 1 {
                        return Err(FormalPairingError::PairingCodeInvalid);
                    }
                    host_links::insert_pair_with_postgres_executor(
                        &mut *tx,
                        row.host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    Ok(FormalPairingConfirm {
                        host_installation_id: row.host_installation_id,
                        already_confirmed: false,
                    })
                }
                pairing_codes::PairingCodeStatus::Confirmed => {
                    if row.account_id.as_deref() != Some(account_id) {
                        return Err(FormalPairingError::PairingStateMismatch {
                            actual: "confirmed_by_different_account".to_string(),
                        });
                    }
                    host_links::insert_pair_with_postgres_executor(
                        &mut *tx,
                        row.host_installation_id,
                        account_id,
                        linked_via_installation_id,
                        now,
                    )
                    .await
                    .map_err(FormalPairingError::Internal)?;
                    Ok(FormalPairingConfirm {
                        host_installation_id: row.host_installation_id,
                        already_confirmed: true,
                    })
                }
                pairing_codes::PairingCodeStatus::Redeemed
                | pairing_codes::PairingCodeStatus::Expired => {
                    Err(FormalPairingError::PairingCodeInvalid)
                }
            }
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_formal_pairing_confirm".to_string(),
                        message: e.to_string(),
                    })
                })?;
                let _ =
                    crate::ingest::invalidate_peer_targets_for_host(outcome.host_installation_id)
                        .await;
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_formal_pairing_confirm".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    pub async fn redeem_host_installation(
        &self,
        pairing_code: &str,
        host_installation_id: DeviceId,
        _client_request_id: Option<&str>,
    ) -> Result<HostInstallationToken, FormalPairingError> {
        match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                self.redeem_host_installation_sqlite(pool, pairing_code, host_installation_id)
                    .await
            }
            StorePoolRef::Postgres(pool) => {
                self.redeem_host_installation_postgres(pool, pairing_code, host_installation_id)
                    .await
            }
        }
    }

    async fn redeem_host_installation_sqlite(
        &self,
        pool: &SqlitePool,
        pairing_code: &str,
        host_installation_id: DeviceId,
    ) -> Result<HostInstallationToken, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let digest = sha256_hex(pairing_code);
        let token = generate_host_installation_token();
        let token_hash = sha256_hex(&token);
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
            FormalPairingError::Internal(BackendError::StoreQuery {
                operation: "begin_host_installation_redeem".to_string(),
                message: e.to_string(),
            })
        })?;

        let result: Result<HostInstallationToken, FormalPairingError> = async {
            let row = pairing_codes::get_code_with_executor(&mut *tx, &digest)
                .await
                .map_err(FormalPairingError::Internal)?
                .ok_or(FormalPairingError::PairingCodeInvalid)?;
            if row.host_installation_id != host_installation_id || row.expires_at_ms <= now {
                return Err(FormalPairingError::PairingCodeInvalid);
            }
            match row.status {
                pairing_codes::PairingCodeStatus::Pending => {
                    return Err(FormalPairingError::PairingNotConfirmed);
                }
                pairing_codes::PairingCodeStatus::Confirmed => {}
                pairing_codes::PairingCodeStatus::Redeemed
                | pairing_codes::PairingCodeStatus::Expired => {
                    return Err(FormalPairingError::PairingCodeInvalid);
                }
            }

            let account_id = pairing_codes::redeem_code_with_executor(
                &mut *tx,
                &digest,
                host_installation_id,
                now,
            )
            .await
            .map_err(FormalPairingError::Internal)?
            .ok_or(FormalPairingError::PairingCodeInvalid)?;
            host_installation_tokens::insert_token_with_executor(
                &mut *tx,
                &token_hash,
                host_installation_id,
                now,
            )
            .await
            .map_err(FormalPairingError::Internal)?;
            Ok(HostInstallationToken {
                host_installation_id,
                account_id,
                token,
                issued_at_ms: now,
            })
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_host_installation_redeem".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_host_installation_redeem".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    async fn redeem_host_installation_postgres(
        &self,
        pool: &PgPool,
        pairing_code: &str,
        host_installation_id: DeviceId,
    ) -> Result<HostInstallationToken, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let digest = sha256_hex(pairing_code);
        let token = generate_host_installation_token();
        let token_hash = sha256_hex(&token);
        let mut tx = begin_serializable_postgres_tx(pool, "begin_host_installation_redeem")
            .await
            .map_err(FormalPairingError::Internal)?;

        let result: Result<HostInstallationToken, FormalPairingError> = async {
            let row = pairing_codes::get_code_with_postgres_executor(&mut *tx, &digest)
                .await
                .map_err(FormalPairingError::Internal)?
                .ok_or(FormalPairingError::PairingCodeInvalid)?;
            if row.host_installation_id != host_installation_id || row.expires_at_ms <= now {
                return Err(FormalPairingError::PairingCodeInvalid);
            }
            match row.status {
                pairing_codes::PairingCodeStatus::Pending => {
                    return Err(FormalPairingError::PairingNotConfirmed);
                }
                pairing_codes::PairingCodeStatus::Confirmed => {}
                pairing_codes::PairingCodeStatus::Redeemed
                | pairing_codes::PairingCodeStatus::Expired => {
                    return Err(FormalPairingError::PairingCodeInvalid);
                }
            }

            let account_id = pairing_codes::redeem_code_with_postgres_executor(
                &mut *tx,
                &digest,
                host_installation_id,
                now,
            )
            .await
            .map_err(FormalPairingError::Internal)?
            .ok_or(FormalPairingError::PairingCodeInvalid)?;
            host_installation_tokens::insert_token_with_postgres_executor(
                &mut *tx,
                &token_hash,
                host_installation_id,
                now,
            )
            .await
            .map_err(FormalPairingError::Internal)?;
            Ok(HostInstallationToken {
                host_installation_id,
                account_id,
                token,
                issued_at_ms: now,
            })
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_host_installation_redeem".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_host_installation_redeem".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    pub async fn revoke_link(
        &self,
        registry: &SessionRegistry,
        host_installation_id: DeviceId,
        account_id: &str,
    ) -> Result<Option<FormalRevokeOutcome>, FormalPairingError> {
        match self.pool.as_store_pool() {
            StorePoolRef::Sqlite(pool) => {
                self.revoke_link_sqlite(pool, registry, host_installation_id, account_id)
                    .await
            }
            StorePoolRef::Postgres(pool) => {
                self.revoke_link_postgres(pool, registry, host_installation_id, account_id)
                    .await
            }
        }
    }

    async fn revoke_link_sqlite(
        &self,
        pool: &SqlitePool,
        registry: &SessionRegistry,
        host_installation_id: DeviceId,
        account_id: &str,
    ) -> Result<Option<FormalRevokeOutcome>, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
            FormalPairingError::Internal(BackendError::StoreQuery {
                operation: "begin_formal_pairing_revoke".to_string(),
                message: e.to_string(),
            })
        })?;

        let result: Result<Option<FormalRevokeOutcome>, FormalPairingError> = async {
            let deleted =
                host_links::delete_pair_with_executor(&mut *tx, host_installation_id, account_id)
                    .await
                    .map_err(FormalPairingError::Internal)?;
            if deleted == 0 {
                return Ok(None);
            }

            let remaining_link_count =
                host_links::count_accounts_for_host_with_executor(&mut *tx, host_installation_id)
                    .await
                    .map_err(FormalPairingError::Internal)?;
            let revoked_token_count = if remaining_link_count == 0 {
                host_installation_tokens::revoke_all_for_host_with_executor(
                    &mut *tx,
                    host_installation_id,
                    now,
                )
                .await
                .map_err(FormalPairingError::Internal)?
            } else {
                0
            };

            Ok(Some(FormalRevokeOutcome {
                host_installation_id,
                remaining_link_count,
                revoked_token_count,
            }))
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_formal_pairing_revoke".to_string(),
                        message: e.to_string(),
                    })
                })?;
                if let Some(outcome) = outcome.as_ref() {
                    let _ =
                        crate::ingest::invalidate_peer_targets_for_host(host_installation_id).await;
                    if outcome.remaining_link_count == 0 {
                        if let Some(handle) = registry.remove(host_installation_id) {
                            handle.revoke(SessionRevocation::AuthRevoked);
                        }
                    } else if let Some(host_handle) = registry.get(host_installation_id) {
                        let _ = registry.try_send_current(
                            &host_handle,
                            Envelope::Event {
                                version: 1,
                                event: EventKind::Unpaired,
                            },
                        );
                    }
                }
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_formal_pairing_revoke".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    async fn revoke_link_postgres(
        &self,
        pool: &PgPool,
        registry: &SessionRegistry,
        host_installation_id: DeviceId,
        account_id: &str,
    ) -> Result<Option<FormalRevokeOutcome>, FormalPairingError> {
        let now = Utc::now().timestamp_millis();
        let mut tx = begin_serializable_postgres_tx(pool, "begin_formal_pairing_revoke")
            .await
            .map_err(FormalPairingError::Internal)?;

        let result: Result<Option<FormalRevokeOutcome>, FormalPairingError> = async {
            let deleted = host_links::delete_pair_with_postgres_executor(
                &mut *tx,
                host_installation_id,
                account_id,
            )
            .await
            .map_err(FormalPairingError::Internal)?;
            if deleted == 0 {
                return Ok(None);
            }

            let remaining_link_count = host_links::count_accounts_for_host_with_postgres_executor(
                &mut *tx,
                host_installation_id,
            )
            .await
            .map_err(FormalPairingError::Internal)?;
            let revoked_token_count = if remaining_link_count == 0 {
                host_installation_tokens::revoke_all_for_host_with_postgres_executor(
                    &mut *tx,
                    host_installation_id,
                    now,
                )
                .await
                .map_err(FormalPairingError::Internal)?
            } else {
                0
            };

            Ok(Some(FormalRevokeOutcome {
                host_installation_id,
                remaining_link_count,
                revoked_token_count,
            }))
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "commit_formal_pairing_revoke".to_string(),
                        message: e.to_string(),
                    })
                })?;
                if let Some(outcome) = outcome.as_ref() {
                    let _ =
                        crate::ingest::invalidate_peer_targets_for_host(host_installation_id).await;
                    if outcome.remaining_link_count == 0 {
                        if let Some(handle) = registry.remove(host_installation_id) {
                            handle.revoke(SessionRevocation::AuthRevoked);
                        }
                    } else if let Some(host_handle) = registry.get(host_installation_id) {
                        let _ = registry.try_send_current(
                            &host_handle,
                            Envelope::Event {
                                version: 1,
                                event: EventKind::Unpaired,
                            },
                        );
                    }
                }
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| {
                    FormalPairingError::Internal(BackendError::StoreQuery {
                        operation: "rollback_formal_pairing_revoke".to_string(),
                        message: e.to_string(),
                    })
                })?;
                Err(err)
            }
        }
    }

    /// Consume a pairing token and mint the Mac's bearer secret.
    ///
    /// Steps:
    /// 1. Hash the candidate and atomically mark the matching row
    ///    consumed (via [`tokens::consume_token_with_executor`]). A
    ///    missing, expired, or already-consumed token surfaces as
    ///    [`BackendError::PairingTokenInvalid`].
    /// 2. Refuse only self-pairing.
    /// 3. Mint one fresh `DeviceSecret` for the issuer (returned only; not
    ///    stored — device-secret rail removed with `secret_hash`).
    /// 4. Ensure the consumer's installation row exists (no-op if present).
    ///
    /// Returns a [`PairingOutcome`] carrying the Mac plaintext secret so
    /// the caller can broadcast `Event::Paired` to the Mac side. The
    /// `host_links` row is inserted by the HTTP handler after this
    /// function returns — `consume_token` does not have the bearer's
    /// `account_id`.
    ///
    /// # Errors
    ///
    /// - [`BackendError::PairingTokenInvalid`] — unknown / expired / already
    ///   consumed candidate.
    /// - [`BackendError::PairingStateMismatch`] — self-pair attempt.
    /// - [`BackendError::StoreQuery`] / [`BackendError::DeviceNotFound`] — any
    ///   underlying store write failed.
    pub async fn consume_token(
        &self,
        candidate: &PairingToken,
        consumer: DeviceId,
        consumer_name: String,
    ) -> Result<PairingOutcome, BackendError> {
        let now = Utc::now().timestamp_millis();
        let digest = sha256_hex(candidate.as_str());

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
            BackendError::StoreQuery {
                operation: "begin_pairing_consume".to_string(),
                message: e.to_string(),
            }
        })?;

        let result: Result<PairingOutcome, BackendError> = async {
            let issuer = tokens::peek_usable_token_with_executor(&mut *tx, &digest, now)
                .await?
                .ok_or(BackendError::PairingTokenInvalid)?
                .issuer_device_id;

            if issuer == consumer {
                return Err(BackendError::PairingStateMismatch {
                    actual: "self".to_string(),
                });
            }

            let issuer_secret = DeviceSecret::generate();

            tokens::consume_token_with_executor(&mut *tx, &digest, now)
                .await?
                .ok_or(BackendError::PairingTokenInvalid)?;

            if device_installations::get_device_with_executor(&mut *tx, consumer)
                .await?
                .is_none()
            {
                device_installations::insert_device_with_executor(
                    &mut *tx,
                    consumer,
                    &consumer_name,
                    DeviceRole::MobileClient,
                    now,
                )
                .await?;
            }

            Ok(PairingOutcome {
                issuer_device_id: issuer,
                issuer_secret,
            })
        }
        .await;

        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(|e| BackendError::StoreQuery {
                    operation: "commit_pairing_consume".to_string(),
                    message: e.to_string(),
                })?;
                Ok(outcome)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| BackendError::StoreQuery {
                    operation: "rollback_pairing_consume".to_string(),
                    message: e.to_string(),
                })?;
                Err(err)
            }
        }
    }

    pub async fn consume_pairing(
        &self,
        registry: &SessionRegistry,
        candidate: &PairingToken,
        consumer: DeviceId,
        consumer_name: String,
        account_id: &str,
    ) -> Result<PairingCompletion, ConsumePairingError> {
        let result = self
            .consume_pairing_inner(registry, candidate, consumer, consumer_name, account_id)
            .await;
        crate::telemetry::record_pairing_consume(pairing_consume_outcome(&result));
        result
    }

    async fn consume_pairing_inner(
        &self,
        registry: &SessionRegistry,
        candidate: &PairingToken,
        consumer: DeviceId,
        consumer_name: String,
        account_id: &str,
    ) -> Result<PairingCompletion, ConsumePairingError> {
        let pairing_outcome = match self
            .consume_token(candidate, consumer, consumer_name.clone())
            .await
        {
            Ok(outcome) => outcome,
            Err(BackendError::PairingTokenInvalid) => {
                return Err(ConsumePairingError::PairingTokenInvalid)
            }
            Err(BackendError::PairingStateMismatch { actual }) => {
                return Err(ConsumePairingError::PairingStateMismatch { actual })
            }
            Err(error) => return Err(ConsumePairingError::Internal(error)),
        };

        let issuer_id = pairing_outcome.issuer_device_id;
        if let Err(error) = host_links::insert_pair(
            &self.pool,
            issuer_id,
            account_id,
            consumer,
            Utc::now().timestamp_millis(),
        )
        .await
        {
            return Err(ConsumePairingError::Internal(error));
        }
        let _ = crate::ingest::invalidate_peer_targets_for_host(issuer_id).await;

        if let Err(error) = self
            .link_account_to_pair_devices(consumer, issuer_id, account_id)
            .await
        {
            self.compensate_pairing(issuer_id, account_id).await;
            return Err(ConsumePairingError::Internal(error));
        }

        let mac_name = match device_installations::get_device(&self.pool, issuer_id).await {
            Ok(Some(row)) => row.display_name,
            _ => "Mac".to_string(),
        };

        let Some(issuer_handle) = registry.get(issuer_id) else {
            self.compensate_pairing(issuer_id, account_id).await;
            return Err(ConsumePairingError::IssuerOffline);
        };

        issuer_handle.set_account_id(account_id.to_string());
        let frame = Envelope::Event {
            version: 1,
            event: EventKind::Paired {
                peer_device_id: consumer,
                peer_name: consumer_name,
                your_device_secret: Some(pairing_outcome.issuer_secret),
            },
        };
        if let Err(error) = registry.try_send_current(&issuer_handle, frame) {
            tracing::warn!(
                target: "minos_backend::pairing",
                error = ?error,
                issuer = %issuer_id,
                consumer = %consumer,
                "failed to deliver pairing event; compensating"
            );
            self.compensate_pairing(issuer_id, account_id).await;
            return Err(ConsumePairingError::DeliveryFailed);
        }

        Ok(PairingCompletion {
            peer_device_id: issuer_id,
            peer_name: mac_name,
        })
    }

    pub async fn forget_pairing(
        &self,
        registry: &SessionRegistry,
        host_device_id: DeviceId,
        account_id: &str,
    ) -> Result<bool, BackendError> {
        let result = self
            .forget_pairing_inner(registry, host_device_id, account_id)
            .await;
        crate::telemetry::record_pairing_forget(pairing_forget_outcome(&result));
        result
    }

    async fn forget_pairing_inner(
        &self,
        registry: &SessionRegistry,
        host_device_id: DeviceId,
        account_id: &str,
    ) -> Result<bool, BackendError> {
        let deleted = host_links::delete_pair(&self.pool, host_device_id, account_id).await?;
        let _ = crate::ingest::invalidate_peer_targets_for_host(host_device_id).await;

        if deleted == 0 {
            return Ok(false);
        }

        if let Some(host_handle) = registry.get(host_device_id) {
            let _ = registry.try_send_current(
                &host_handle,
                Envelope::Event {
                    version: 1,
                    event: EventKind::Unpaired,
                },
            );
        }

        Ok(true)
    }

    async fn link_account_to_pair_devices(
        &self,
        consumer: DeviceId,
        _issuer: DeviceId,
        account_id: &str,
    ) -> Result<(), BackendError> {
        // Host stays account_id NULL (installation_kind CHECK); link via host_links.
        device_installations::set_account_id(&self.pool, &consumer, account_id).await?;
        Ok(())
    }

    async fn compensate_pairing(&self, issuer_id: DeviceId, account_id: &str) {
        let _ = host_links::delete_pair(&self.pool, issuer_id, account_id).await;
        let _ = crate::ingest::invalidate_peer_targets_for_host(issuer_id).await;
    }
}

fn pairing_token_issue_outcome<T>(result: &Result<T, BackendError>) -> &'static str {
    use crate::telemetry as t;
    match result {
        Ok(_) => t::OUTCOME_OK,
        Err(_) => t::OUTCOME_ERROR,
    }
}

fn pairing_consume_outcome<T>(result: &Result<T, ConsumePairingError>) -> &'static str {
    use crate::telemetry as t;
    match result {
        Ok(_) => t::OUTCOME_OK,
        Err(ConsumePairingError::PairingTokenInvalid) => t::OUTCOME_INVALID,
        Err(ConsumePairingError::PairingStateMismatch { .. }) => t::OUTCOME_CONFLICT,
        Err(ConsumePairingError::IssuerOffline) => t::OUTCOME_PEER_OFFLINE,
        Err(ConsumePairingError::DeliveryFailed) => t::OUTCOME_PEER_BACKPRESSURE,
        Err(ConsumePairingError::Internal(_)) => t::OUTCOME_ERROR,
    }
}

fn pairing_forget_outcome<T>(result: &Result<T, BackendError>) -> &'static str {
    use crate::telemetry as t;
    match result {
        Ok(_) => t::OUTCOME_OK,
        Err(_) => t::OUTCOME_ERROR,
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
///
/// Hand-rolled `{:02x}` loop so we don't pull in the `hex` crate just for
/// a 64-char output.
pub(crate) fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String write never fails");
    }
    out
}

fn generate_pairing_code() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_host_installation_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    format!("hit_{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;
    use std::time::Duration as StdDuration;

    const FIVE_MIN: StdDuration = StdDuration::from_mins(5);

    async fn mac_issuer(pool: &SqlitePool) -> DeviceId {
        let id = DeviceId::new();
        device_installations::insert_device(
            pool,
            id,
            "alice's mac",
            DeviceRole::AgentHost,
            Utc::now().timestamp_millis(),
        )
        .await
        .unwrap();
        id
    }

    // ── property: token entropy ────────────────────────────────────────
    //
    // Inlined (no proptest! wrapper) because `PairingToken::generate` takes
    // no inputs — proptest's generator would just drive an iteration count,
    // which a plain loop does more clearly. `minos-domain` already carries a
    // `proptest!` version; this test earns its keep by landing on the backend
    // side too, which is where spec §14's acceptance criterion lives.

    #[test]
    fn token_entropy_no_collisions_in_1000_iterations() {
        let start = std::time::Instant::now();
        let mut seen = std::collections::HashSet::with_capacity(1000);
        for i in 0..1000 {
            let t = PairingToken::generate();
            assert!(seen.insert(t.0), "collision at iteration {i}");
        }
        let elapsed = start.elapsed();
        // Plan §6 acceptance: <1s for 1000 iterations. Loose upper bound of
        // 1s captures regressions while leaving room for slow CI runners.
        assert!(
            elapsed < StdDuration::from_secs(1),
            "property test took {elapsed:?}, expected <1s"
        );
    }

    // ── integration: request + consume happy path ──────────────────────

    #[tokio::test]
    async fn request_then_consume_happy_path_mints_issuer_secret_only() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;

        let (token, expires) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        assert!(expires > Utc::now());

        let consumer = DeviceId::new();
        let outcome = svc
            .consume_token(&token, consumer, "my iPhone".to_string())
            .await
            .unwrap();

        assert_eq!(outcome.issuer_device_id, issuer);
        // Issuer secret is non-empty (Base64URL of 32 bytes → 43 chars).
        assert_eq!(outcome.issuer_secret.as_str().len(), 43);

        // secret_hash column removed; still mint issuer_secret for legacy event payload.
        assert!(device_installations::get_device(&pool, issuer)
            .await
            .unwrap()
            .is_some());
        assert!(device_installations::get_device(&pool, consumer)
            .await
            .unwrap()
            .is_some());
        let _ = outcome.issuer_secret;
    }

    // ── integration: token invalid cases ───────────────────────────────

    #[tokio::test]
    async fn consume_expired_token_returns_pairing_token_invalid() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;

        // 0-second TTL → always expired by the time consume_token sees it.
        let (token, _expires) = svc.request_token(issuer, StdDuration::ZERO).await.unwrap();

        let consumer = DeviceId::new();
        let err = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PairingTokenInvalid));
    }

    #[tokio::test]
    async fn consume_already_consumed_token_returns_pairing_token_invalid() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;

        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer = DeviceId::new();
        svc.consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();

        // A second consumer attempting the same token gets the generic
        // "invalid" error — the token row exists but consumed_at is set.
        let other_consumer = DeviceId::new();
        let err = svc
            .consume_token(&token, other_consumer, "another iphone".into())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PairingTokenInvalid));
    }

    #[tokio::test]
    async fn consume_unknown_token_returns_pairing_token_invalid() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool);

        // A syntactically-plausible token that was never issued.
        let bogus = PairingToken::generate();
        let consumer = DeviceId::new();
        let err = svc
            .consume_token(&bogus, consumer, "iphone".into())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PairingTokenInvalid));
    }

    // ── integration: state-mismatch case ───────────────────────────────

    #[tokio::test]
    async fn consume_self_pair_returns_state_mismatch_without_burning_token() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;

        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let err = svc
            .consume_token(&token, issuer, "alice's mac".into())
            .await
            .unwrap_err();
        match err {
            BackendError::PairingStateMismatch { actual } => assert_eq!(actual, "self"),
            other => panic!("expected PairingStateMismatch, got {other:?}"),
        }

        // Token still usable; secret_hash rail removed (issuer row may or may not exist yet).
        let _ = device_installations::get_device(&pool, issuer)
            .await
            .unwrap();

        let consumer = DeviceId::new();
        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);
        assert!(device_installations::get_device(&pool, issuer)
            .await
            .unwrap()
            .is_some());
    }

    // ── unit: sha256_hex is deterministic + 64 chars ───────────────────

    #[test]
    fn sha256_hex_matches_known_vector_and_is_deterministic() {
        // RFC 6234 test vector: "abc" → ba7816bf...
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex("abc"), want);
        // Determinism: same input always yields same digest.
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
