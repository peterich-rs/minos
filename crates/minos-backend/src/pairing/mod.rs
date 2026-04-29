//! Backend-side pairing service: token issuance, token consumption, and
//! pair dismissal.
//!
//! Sits on top of `store::{devices, pairings, tokens}` and layers the
//! business rules of spec §6.1 / §7 onto the CRUD:
//!
//! 1. Request — the Mac host asks for a fresh 5-minute token, which is
//!    persisted as a SHA-256 digest (never the plaintext). The plaintext
//!    is returned once for QR rendering and then discarded.
//! 2. Consume — the iOS client presents a candidate token; we atomically
//!    mark the row consumed, mint two fresh `DeviceSecret`s (one for each
//!    side), hash them with argon2id, and insert the pairing row.
//! 3. Forget — either side can dissolve one pair; the caller learns the
//!    peer's `DeviceId` so it can broadcast the `Unpaired` event. iOS-side
//!    secrets are revoked when the iOS device has no remaining pair; Mac
//!    host secrets are kept because one host may stay paired with other
//!    mobile devices.
//!
//! # Two hash primitives
//!
//! - `secret::hash_secret` — argon2id PHC string for at-rest `DeviceSecret`.
//!   Tuned for "brute-force resistant if the DB is stolen".
//! - `sha2::Sha256` hex digest for `PairingToken`. Deterministic for PK
//!   lookup; safe because tokens carry 256 bits of entropy and expire in
//!   5 minutes. See [`migrations/0003_pairing_tokens.sql`] and spec §6.1.
//!
//! # Replacement policy
//!
//! Pairing is multi-device: a mobile client may add multiple runtime devices,
//! and a runtime may be visible to multiple mobile clients. The exact same
//! pair remains idempotent via the DB uniqueness constraint.
//!
//! # Atomicity
//!
//! `consume_token` starts with `BEGIN IMMEDIATE`, then wraps token validation,
//! token consumption, secret-hash updates, and pairing insertion in one SQLite
//! transaction. That write lock serializes concurrent consumes before any token
//! or pairing lookup, so two valid outstanding tokens for the same issuer
//! cannot both clear the prechecks. Any failure in the flow rolls the whole
//! transaction back so the token is still usable and no partial secrets or
//! pair rows leak into the store.

use std::time::Duration;

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, PairingToken};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::{
    error::BackendError,
    store::{devices, pairings, tokens},
};

pub mod secret;

/// Successful outcome of [`PairingService::consume_token`].
///
/// Both plaintext secrets live in this struct momentarily — just long
/// enough for the caller to push each one to its owning device over the
/// envelope. Neither value is persisted anywhere in the backend; only their
/// argon2id hashes were written as part of `consume_token` itself.
#[derive(Debug, Clone)]
pub struct PairingOutcome {
    /// `DeviceId` of the side that originally issued the pairing token.
    pub issuer_device_id: DeviceId,
    /// Plaintext secret minted for the issuer (to be delivered to the Mac).
    pub issuer_secret: DeviceSecret,
    /// Plaintext secret minted for the consumer (to be returned to the
    /// iOS client as the `pair` RPC result).
    pub consumer_secret: DeviceSecret,
}

/// Stateless facade around the pairing-related store helpers.
///
/// Cheap to clone — just holds a `SqlitePool` handle. Usually instantiated
/// once in `main.rs` and shared via `Arc`.
#[derive(Debug, Clone)]
pub struct PairingService {
    pool: SqlitePool,
}

impl PairingService {
    /// Construct a service backed by `pool`. The pool must already have
    /// migrations applied (use [`crate::store::connect`]).
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
        issuer: DeviceId,
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

    /// Consume a pairing token and complete a pair.
    ///
    /// Steps:
    /// 1. Hash the candidate and atomically mark the matching row
    ///    consumed (via [`tokens::consume_token`]). A missing, expired, or
    ///    already-consumed token surfaces as [`BackendError::PairingTokenInvalid`].
    /// 2. Refuse only self-pairing. Existing pair rows for either side are
    ///    allowed; the exact same pair remains idempotent at the DB layer.
    /// 3. Mint two fresh `DeviceSecret`s, hash each with argon2id.
    /// 4. Upsert the consumer's device row (no-op if already registered),
    ///    write both `secret_hash` columns, insert the pairing row.
    ///
    /// Returns a [`PairingOutcome`] carrying both plaintext secrets so the
    /// caller can broadcast `Event::Paired` to each side.
    ///
    /// # Errors
    ///
    /// - [`BackendError::PairingTokenInvalid`] — unknown / expired / already
    ///   consumed candidate.
    /// - [`BackendError::PairingStateMismatch`] — self-pair attempt.
    /// - [`BackendError::PairingHash`] — argon2 reported an internal error.
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
            let consumer_secret = DeviceSecret::generate();
            let issuer_hash = secret::hash_secret(&issuer_secret)?;
            let consumer_hash = secret::hash_secret(&consumer_secret)?;

            tokens::consume_token_with_executor(&mut *tx, &digest, now)
                .await?
                .ok_or(BackendError::PairingTokenInvalid)?;

            if devices::get_device_with_executor(&mut *tx, consumer)
                .await?
                .is_none()
            {
                devices::insert_device_with_executor(
                    &mut *tx,
                    consumer,
                    &consumer_name,
                    DeviceRole::IosClient,
                    now,
                )
                .await?;
            }

            devices::upsert_secret_hash_with_executor(&mut *tx, consumer, &consumer_hash).await?;
            devices::upsert_secret_hash_with_executor(&mut *tx, issuer, &issuer_hash).await?;
            pairings::insert_pairing_with_executor(&mut *tx, issuer, consumer, now)
                .await
                .map_err(normalize_pairing_insert_error)?;

            Ok(PairingOutcome {
                issuer_device_id: issuer,
                issuer_secret,
                consumer_secret,
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

    /// Dissolve the pair that includes `either_side`.
    ///
    /// Returns `Some(peer)` when a pair existed (so the caller can
    /// broadcast `Event::Unpaired` to the other side), or `Ok(None)` if
    /// `either_side` was unpaired to begin with.
    ///
    /// Idempotent at the store level: calling twice in a row is safe.
    ///
    /// # Errors
    ///
    /// - [`BackendError::StoreQuery`] / [`BackendError::StoreDecode`] — any
    ///   underlying store op failed.
    pub async fn forget_pair(
        &self,
        either_side: DeviceId,
    ) -> Result<Option<DeviceId>, BackendError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(|e| {
            BackendError::StoreQuery {
                operation: "begin_forget_pair".to_string(),
                message: e.to_string(),
            }
        })?;

        let result: Result<Option<DeviceId>, BackendError> = async {
            let Some(peer) = pairings::get_pair_with_executor(&mut *tx, either_side).await? else {
                return Ok(None);
            };

            pairings::delete_pair_with_executor(&mut *tx, either_side, peer).await?;
            clear_secret_if_unpaired_non_host(&mut tx, either_side).await?;
            clear_secret_if_unpaired_non_host(&mut tx, peer).await?;

            Ok(Some(peer))
        }
        .await;

        match result {
            Ok(peer) => {
                tx.commit().await.map_err(|e| BackendError::StoreQuery {
                    operation: "commit_forget_pair".to_string(),
                    message: e.to_string(),
                })?;
                Ok(peer)
            }
            Err(err) => {
                tx.rollback().await.map_err(|e| BackendError::StoreQuery {
                    operation: "rollback_forget_pair".to_string(),
                    message: e.to_string(),
                })?;
                Err(err)
            }
        }
    }
}

async fn clear_secret_if_unpaired_non_host(
    tx: &mut sqlx::SqliteConnection,
    device_id: DeviceId,
) -> Result<(), BackendError> {
    if pairings::get_pair_with_executor(&mut *tx, device_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let role = devices::get_device_with_executor(&mut *tx, device_id)
        .await?
        .map(|row| row.role);
    if role != Some(DeviceRole::AgentHost) {
        devices::clear_secret_hash_with_executor(&mut *tx, device_id).await?;
    }
    Ok(())
}

/// SHA-256 hex digest of a UTF-8 string.
///
/// Hand-rolled `{:02x}` loop so we don't pull in the `hex` crate just for
/// a 64-char output.
fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String write never fails");
    }
    out
}

fn normalize_pairing_insert_error(err: BackendError) -> BackendError {
    match err {
        BackendError::StoreQuery { operation, message }
            if operation == "insert_pairing"
                && message.contains(pairings::SINGLE_PAIR_VIOLATION_MARKER) =>
        {
            BackendError::PairingStateMismatch {
                actual: "paired".to_string(),
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use std::time::Duration as StdDuration;
    use tempfile::tempdir;
    use tokio::sync::Barrier;

    const FIVE_MIN: StdDuration = StdDuration::from_mins(5);

    async fn mac_issuer(pool: &SqlitePool) -> DeviceId {
        let id = DeviceId::new();
        devices::insert_device(
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
    async fn request_then_consume_happy_path_returns_outcome_with_secrets() {
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
        // Secrets are distinct and non-empty.
        assert_ne!(
            outcome.issuer_secret.as_str(),
            outcome.consumer_secret.as_str()
        );
        assert_eq!(outcome.issuer_secret.as_str().len(), 43);
        assert_eq!(outcome.consumer_secret.as_str().len(), 43);

        // Pair row + both secret hashes are persisted.
        assert_eq!(
            pairings::get_pair(&pool, issuer).await.unwrap(),
            Some(consumer)
        );
        assert_eq!(
            pairings::get_pair(&pool, consumer).await.unwrap(),
            Some(issuer)
        );
        let issuer_hash = devices::get_secret_hash(&pool, issuer).await.unwrap();
        let consumer_hash = devices::get_secret_hash(&pool, consumer).await.unwrap();
        assert!(issuer_hash.is_some());
        assert!(consumer_hash.is_some());
        // Hashes round-trip through secret::verify_secret.
        assert!(
            secret::verify_secret(outcome.issuer_secret.as_str(), &issuer_hash.unwrap()).unwrap()
        );
        assert!(
            secret::verify_secret(outcome.consumer_secret.as_str(), &consumer_hash.unwrap())
                .unwrap()
        );
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

    // ── integration: state-mismatch cases ──────────────────────────────

    #[tokio::test]
    async fn consume_when_consumer_already_paired_allows_second_mac_pair() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());

        // Pre-seed a pairing: consumer_id ↔ some third device.
        let third = DeviceId::new();
        devices::insert_device(&pool, third, "third", DeviceRole::AgentHost, 0)
            .await
            .unwrap();
        let consumer = DeviceId::new();
        devices::insert_device(&pool, consumer, "iphone", DeviceRole::IosClient, 0)
            .await
            .unwrap();
        pairings::insert_pairing(&pool, third, consumer, 0)
            .await
            .unwrap();

        // Now a fresh issuer tries to pair with the already-paired consumer.
        let issuer = mac_issuer(&pool).await;
        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();

        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);

        let peers = pairings::get_peers(&pool, consumer).await.unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&third));
        assert!(peers.contains(&issuer));
    }

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

        assert_eq!(pairings::get_pair(&pool, issuer).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, issuer).await.unwrap(), None);

        let consumer = DeviceId::new();
        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);
        assert_eq!(
            pairings::get_pair(&pool, issuer).await.unwrap(),
            Some(consumer)
        );
    }

    #[tokio::test]
    async fn consume_when_issuer_already_paired_allows_second_ios_pair() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());

        let issuer = mac_issuer(&pool).await;
        // Seed issuer with an existing pair.
        let prior_peer = DeviceId::new();
        devices::insert_device(&pool, prior_peer, "prior", DeviceRole::IosClient, 0)
            .await
            .unwrap();
        pairings::insert_pairing(&pool, issuer, prior_peer, 0)
            .await
            .unwrap();

        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer = DeviceId::new();

        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);

        let peers = pairings::get_peers(&pool, issuer).await.unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&prior_peer));
        assert!(peers.contains(&consumer));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_two_outstanding_tokens_for_same_issuer_can_pair_two_ios_devices() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("pairing-race.db");
        let url = format!("sqlite://{}?mode=rwc", db.display());
        let pool = crate::store::connect(&url).await.unwrap();
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;

        let (token_a, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let (token_b, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer_a = DeviceId::new();
        let consumer_b = DeviceId::new();
        let barrier = Arc::new(Barrier::new(3));

        let first = {
            let svc = svc.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                svc.consume_token(&token_a, consumer_a, "iphone-a".into())
                    .await
                    .map(|outcome| (consumer_a, outcome))
            })
        };
        let second = {
            let svc = svc.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                svc.consume_token(&token_b, consumer_b, "iphone-b".into())
                    .await
                    .map(|outcome| (consumer_b, outcome))
            })
        };

        barrier.wait().await;

        let first = first.await.unwrap();
        let second = second.await.unwrap();

        let (first_consumer, first_outcome) = first.expect("first consume should succeed");
        let (second_consumer, second_outcome) = second.expect("second consume should succeed");
        assert_eq!(first_outcome.issuer_device_id, issuer);
        assert_eq!(second_outcome.issuer_device_id, issuer);

        let peers = pairings::get_peers(&pool, issuer).await.unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&first_consumer));
        assert!(peers.contains(&second_consumer));
        assert_eq!(
            pairings::get_pair(&pool, first_consumer).await.unwrap(),
            Some(issuer)
        );
        assert_eq!(
            pairings::get_pair(&pool, second_consumer).await.unwrap(),
            Some(issuer)
        );
    }

    #[tokio::test]
    async fn consume_rolls_back_token_and_secret_hashes_when_pairing_insert_fails() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;
        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer = DeviceId::new();

        sqlx::query(
            "
            CREATE TRIGGER fail_pairing_insert
            BEFORE INSERT ON pairings
            BEGIN
                SELECT RAISE(ABORT, 'pairing insert failed');
            END;
            ",
        )
        .execute(&pool)
        .await
        .unwrap();

        let err = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap_err();
        match err {
            BackendError::StoreQuery { operation, message } => {
                assert_eq!(operation, "insert_pairing");
                assert!(message.contains("pairing insert failed"));
            }
            other => panic!("expected StoreQuery, got {other:?}"),
        }

        assert_eq!(pairings::get_pair(&pool, issuer).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, consumer).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, issuer).await.unwrap(), None);
        assert_eq!(
            devices::get_secret_hash(&pool, consumer).await.unwrap(),
            None
        );
        assert_eq!(devices::get_device(&pool, consumer).await.unwrap(), None);

        let consumed_at = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at FROM pairing_tokens WHERE token_hash = ?",
        )
        .bind(sha256_hex(token.as_str()))
        .fetch_optional(&pool)
        .await
        .unwrap()
        .flatten();
        assert_eq!(consumed_at, None);

        sqlx::query("DROP TRIGGER fail_pairing_insert")
            .execute(&pool)
            .await
            .unwrap();

        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);
        assert_eq!(
            pairings::get_pair(&pool, issuer).await.unwrap(),
            Some(consumer)
        );
    }

    // ── integration: forget ────────────────────────────────────────────

    #[tokio::test]
    async fn forget_pair_returns_peer_and_deletes_row() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;
        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer = DeviceId::new();
        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();

        let peer = svc.forget_pair(outcome.issuer_device_id).await.unwrap();
        assert_eq!(peer, Some(consumer));
        // Row is gone from both directions.
        assert_eq!(pairings::get_pair(&pool, issuer).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, consumer).await.unwrap(), None);
    }

    #[tokio::test]
    async fn forget_pair_from_consumer_side_returns_issuer() {
        // Symmetry check: forget called on the consumer must return the
        // issuer's DeviceId.
        let pool = memory_pool().await;
        let svc = PairingService::new(pool.clone());
        let issuer = mac_issuer(&pool).await;
        let (token, _) = svc.request_token(issuer, FIVE_MIN).await.unwrap();
        let consumer = DeviceId::new();
        svc.consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();

        let peer = svc.forget_pair(consumer).await.unwrap();
        assert_eq!(peer, Some(issuer));
    }

    #[tokio::test]
    async fn forget_pair_unpaired_returns_none() {
        let pool = memory_pool().await;
        let svc = PairingService::new(pool);
        let lonely = DeviceId::new();
        assert_eq!(svc.forget_pair(lonely).await.unwrap(), None);
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
