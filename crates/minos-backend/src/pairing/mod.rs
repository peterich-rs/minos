//! Backend-side pairing service: token issuance and token consumption.
//!
//! Sits on top of `store::{devices, tokens}` and layers the business
//! rules of spec §6.1 / §7 onto the CRUD:
//!
//! 1. Request — the Mac host asks for a fresh 5-minute token, which is
//!    persisted as a SHA-256 digest (never the plaintext). The plaintext
//!    is returned once for QR rendering and then discarded.
//! 2. Consume — the iOS client presents a candidate token; we atomically
//!    mark the row consumed, mint a fresh `DeviceSecret` for the Mac
//!    issuer, hash it with argon2id, and persist on the issuer row. iOS
//!    never gets a `DeviceSecret`; its row keeps `secret_hash = NULL` per
//!    ADR-0020 (the iOS rail is bearer-JWT only).
//!
//! The `(mac_device_id, mobile_account_id)` pair row in
//! `account_mac_pairings` is inserted by the HTTP handler post-commit —
//! see `http::v1::pairing::post_consume`. `consume_token` does not see
//! the bearer's `account_id`, so it cannot insert the pair itself.
//!
//! # Two hash primitives
//!
//! - `secret::hash_secret` — argon2id PHC string for at-rest `DeviceSecret`.
//!   Tuned for "brute-force resistant if the DB is stolen".
//! - `sha2::Sha256` hex digest for `PairingToken`. Deterministic for PK
//!   lookup; safe because tokens carry 256 bits of entropy and expire in
//!   5 minutes. See [`migrations/0003_pairing_tokens.sql`] and spec §6.1.
//!
//! # Atomicity
//!
//! `consume_token` starts with `BEGIN IMMEDIATE`, then wraps token validation,
//! token consumption, and the issuer's secret-hash upsert in one SQLite
//! transaction. That write lock serializes concurrent consumes before any
//! token lookup. Any failure rolls the whole transaction back so the token
//! is still usable and no partial secret leaks into the store.

use std::time::Duration;

use chrono::{DateTime, Utc};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret, PairingToken};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::{
    error::BackendError,
    store::{devices, tokens},
};

pub mod secret;

/// Successful outcome of [`PairingService::consume_token`].
///
/// Carries the Mac issuer's plaintext `DeviceSecret` momentarily so the
/// caller can push it via `EventKind::Paired`. Not persisted anywhere
/// — only its argon2id hash was written as part of `consume_token`.
#[derive(Debug, Clone)]
pub struct PairingOutcome {
    /// `DeviceId` of the side that originally issued the pairing token
    /// (the Mac host).
    pub issuer_device_id: DeviceId,
    /// Plaintext secret minted for the issuer (to be delivered to the Mac).
    pub issuer_secret: DeviceSecret,
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

    /// Consume a pairing token and mint the Mac's bearer secret.
    ///
    /// Steps:
    /// 1. Hash the candidate and atomically mark the matching row
    ///    consumed (via [`tokens::consume_token_with_executor`]). A
    ///    missing, expired, or already-consumed token surfaces as
    ///    [`BackendError::PairingTokenInvalid`].
    /// 2. Refuse only self-pairing.
    /// 3. Mint one fresh `DeviceSecret` for the issuer and hash it with
    ///    argon2id.
    /// 4. Upsert the consumer's device row (no-op if already registered).
    ///    iOS rows keep `secret_hash = NULL` per ADR-0020.
    /// 5. Write the issuer's `secret_hash`.
    ///
    /// Returns a [`PairingOutcome`] carrying the Mac plaintext secret so
    /// the caller can broadcast `Event::Paired` to the Mac side. The
    /// `(mac, mobile_account)` pair row is inserted by the HTTP handler
    /// after this function returns — `consume_token` does not have the
    /// bearer's `account_id`.
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
            let issuer_hash = secret::hash_secret(&issuer_secret)?;

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

            // iOS row stays at secret_hash = NULL per ADR-0020.
            devices::upsert_secret_hash_with_executor(&mut *tx, issuer, &issuer_hash).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::memory_pool;
    use minos_domain::DeviceRole;
    use pretty_assertions::assert_eq;
    use std::time::Duration as StdDuration;

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

        // Mac-side: secret_hash IS Some(_) and round-trips through verify.
        let issuer_hash = devices::get_secret_hash(&pool, issuer).await.unwrap();
        assert!(issuer_hash.is_some(), "Mac issuer must have secret_hash");
        assert!(
            secret::verify_secret(outcome.issuer_secret.as_str(), &issuer_hash.unwrap()).unwrap()
        );

        // iOS-side: secret_hash IS NULL per ADR-0020 (bearer-only rail).
        let consumer_hash = devices::get_secret_hash(&pool, consumer).await.unwrap();
        assert!(
            consumer_hash.is_none(),
            "iOS row must keep secret_hash NULL per ADR-0020"
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

        // Token still usable; issuer's secret_hash still NULL.
        assert_eq!(devices::get_secret_hash(&pool, issuer).await.unwrap(), None);

        let consumer = DeviceId::new();
        let outcome = svc
            .consume_token(&token, consumer, "iphone".into())
            .await
            .unwrap();
        assert_eq!(outcome.issuer_device_id, issuer);
        assert!(devices::get_secret_hash(&pool, issuer)
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
