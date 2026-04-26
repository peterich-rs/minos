# Mobile Auth + Agent Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the slack.ai-style account auth, mobile-driven agent dispatch, auto-reconnect, and production chat UI in one cohesive change, per `docs/superpowers/specs/mobile-auth-and-agent-session-design.md`.

**Architecture:** Three-tier change. (1) Backend grows account model, JWT bearer rail, account-aware WS routing while keeping the device-secret rail intact. (2) Rust mobile core grows JSON-RPC outbound dispatch with id correlation, auto-reconnect with token refresh, lifecycle hooks, and token persistence. (3) Flutter app grows auth state + login UI, account-gated routing, full Remodex-style chat with bubbles/streaming/input, lifecycle observer.

**Tech Stack:**
- Backend: Rust + axum + sqlx (SQLite STRICT) + `argon2` (already in workspace) + `jsonwebtoken` (new)
- Mobile core: Rust + tokio + `tokio-tungstenite` + `reqwest` + `dashmap` (already in workspace)
- frb: flutter_rust_bridge 2.12 + `cargo xtask gen-frb` codegen
- Flutter: Riverpod 3 + shadcn_ui + `flutter_secure_storage` + `flutter_markdown_plus` + `flutter_highlight`

**Critical clarifications (read before starting):**
- `Envelope::Forward { version, payload }` carries JSON-RPC `{id, method, params}` *inside* `payload`. Correlation id is on the inner JSON-RPC object, NOT on the envelope.
- `stop_agent` takes `()`. `StopAgentRequest` does **not** exist.
- `StartAgentRequest { agent: AgentName }` carries no prompt. The frb `start_agent` wrapper internally calls `minos_start_agent` then `minos_send_user_message`.
- `StartAgentResponse.session_id` is the thread_id (per `crates/minos-protocol/src/messages.rs:50` doc comment).
- `MinosError` lives in `crates/minos-domain/src/error.rs`, not `minos-protocol`. Uses field-init variants (e.g. `Disconnected { reason }`), no `ErrorKind` enum to nest into.
- `MobileClient.outbox` already uses `mpsc::Sender<Envelope>`. Reuse it; don't introduce `mpsc::UnboundedSender`.
- `crates/minos-mobile/src/client.rs::handle_text_frame` has **no** `Forwarded` arm today — adding one is mandatory for dispatch to work.
- `auth.rs` is per-handler call, not axum middleware. Mirror that pattern for the bearer rail.
- Workspace gate is `cargo xtask check-all`. Per memory, run it before every commit. The frb drift guard regenerates `apps/mobile/lib/src/rust/` and `crates/minos-ffi-frb/src/frb_generated.rs` and fails if they aren't committed.

**Worktree:** Execute in a dedicated worktree per CLAUDE.md §3:
```bash
cd /Users/zhangfan/develop/github.com/minos
git worktree add ../minos-worktrees/mobile-auth-and-agent-session -b feature/mobile-auth-and-agent-session main
cd ../minos-worktrees/mobile-auth-and-agent-session
```

**Phase map:**
1. Backend foundation (DB + crypto + auth endpoints)
2. Backend integration (account-aware routing)
3. Protocol + error variants
4. Mobile Rust HTTP auth
5. Mobile Rust RPC dispatch
6. Mobile Rust auto-reconnect + lifecycle
7. frb surface
8. Flutter domain + state
9. Flutter UI: login
10. Flutter UI: chat rework
11. Flutter UI: account settings + lifecycle wiring
12. Tooling + verification

---

## Phase 1: Backend Foundation

### Task 1.1: Add `jsonwebtoken` to workspace + minos-backend

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/minos-backend/Cargo.toml`

- [ ] **Step 1: Add to workspace dependency table**

In `Cargo.toml` workspace `[workspace.dependencies]` block, add (alphabetically):

```toml
jsonwebtoken = "9"
```

- [ ] **Step 2: Add to backend crate**

In `crates/minos-backend/Cargo.toml` `[dependencies]`, add:

```toml
jsonwebtoken = { workspace = true }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p minos-backend`
Expected: PASS (no usage yet, just resolves the dep tree).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/minos-backend/Cargo.toml
git commit -m "build(backend): add jsonwebtoken dep for JWT auth"
```

---

### Task 1.2: Migration `0007_accounts.sql` + `accounts` store

**Files:**
- Create: `crates/minos-backend/migrations/0007_accounts.sql`
- Create: `crates/minos-backend/src/store/accounts.rs`
- Modify: `crates/minos-backend/src/store/mod.rs`

- [ ] **Step 1: Write the migration**

Content for `crates/minos-backend/migrations/0007_accounts.sql`:

```sql
CREATE TABLE accounts (
    account_id     TEXT PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
```

- [ ] **Step 2: Write the store module**

Create `crates/minos-backend/src/store/accounts.rs`:

```rust
//! `accounts` table CRUD. Account ids are UUIDv4 strings; emails are
//! lowercased before lookup (the table is `COLLATE NOCASE` for defence).

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::BackendError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

pub async fn create(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
) -> Result<AccountRow, BackendError> {
    let account_id = Uuid::new_v4().to_string();
    let email_norm = email.to_lowercase();
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        r#"INSERT INTO accounts (account_id, email, password_hash, created_at)
           VALUES (?, ?, ?, ?)"#,
    )
    .bind(&account_id)
    .bind(&email_norm)
    .bind(password_hash)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => BackendError::EmailTaken,
        _ => BackendError::from(e),
    })?;
    Ok(AccountRow {
        account_id,
        email: email_norm,
        password_hash: password_hash.into(),
        created_at: now,
        last_login_at: None,
    })
}

pub async fn find_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<AccountRow>, BackendError> {
    let email_norm = email.to_lowercase();
    let row = sqlx::query_as::<_, AccountRow>(
        r#"SELECT account_id, email, password_hash, created_at, last_login_at
           FROM accounts WHERE email = ?"#,
    )
    .bind(&email_norm)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn touch_last_login(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<(), BackendError> {
    let now = Utc::now().timestamp_millis();
    sqlx::query("UPDATE accounts SET last_login_at = ? WHERE account_id = ?")
        .bind(now)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Add `EmailTaken` variant to `BackendError`**

Read `crates/minos-backend/src/error.rs` first to see the existing variants. Add:

```rust
#[error("email already registered")]
EmailTaken,
```

(Place it alphabetically near other auth-shaped errors.)

- [ ] **Step 4: Wire the store module**

In `crates/minos-backend/src/store/mod.rs`, add:

```rust
pub mod accounts;
```

- [ ] **Step 5: Run sqlx prepare + cargo check**

```bash
cargo check -p minos-backend
DATABASE_URL=sqlite::memory: cargo sqlx prepare --workspace
```

Expected: PASS. New `.sqlx/*.json` files appear under repo root (commit them).

- [ ] **Step 6: Commit**

```bash
git add crates/minos-backend/migrations/0007_accounts.sql \
        crates/minos-backend/src/store/accounts.rs \
        crates/minos-backend/src/store/mod.rs \
        crates/minos-backend/src/error.rs \
        .sqlx/
git commit -m "feat(backend): add accounts table + store module"
```

---

### Task 1.3: Migration `0008_refresh_tokens.sql` + store

**Files:**
- Create: `crates/minos-backend/migrations/0008_refresh_tokens.sql`
- Create: `crates/minos-backend/src/store/refresh_tokens.rs`
- Modify: `crates/minos-backend/src/store/mod.rs`

- [ ] **Step 1: Write the migration**

`crates/minos-backend/migrations/0008_refresh_tokens.sql`:

```sql
CREATE TABLE refresh_tokens (
    token_hash     TEXT PRIMARY KEY,
    account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    device_id      TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    issued_at      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    revoked_at     INTEGER
) STRICT;

CREATE INDEX idx_refresh_tokens_account ON refresh_tokens(account_id) WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_tokens_device ON refresh_tokens(device_id) WHERE revoked_at IS NULL;
```

- [ ] **Step 2: Write the store**

Create `crates/minos-backend/src/store/refresh_tokens.rs`:

```rust
//! `refresh_tokens` table. Tokens are stored as SHA-256 hex of the
//! 32-byte random plaintext; plaintext is only ever in transit. Same
//! pattern as `pairing_tokens`.

use chrono::Utc;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::error::BackendError;

pub const REFRESH_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefreshTokenRow {
    pub token_hash: String,
    pub account_id: String,
    pub device_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

pub fn generate_plaintext() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_plaintext(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    hex::encode(digest)
}

pub async fn insert(
    pool: &SqlitePool,
    plaintext: &str,
    account_id: &str,
    device_id: &str,
) -> Result<RefreshTokenRow, BackendError> {
    let now = Utc::now().timestamp_millis();
    let row = RefreshTokenRow {
        token_hash: hash_plaintext(plaintext),
        account_id: account_id.into(),
        device_id: device_id.into(),
        issued_at: now,
        expires_at: now + REFRESH_TTL_MS,
        revoked_at: None,
    };
    sqlx::query(
        r#"INSERT INTO refresh_tokens (token_hash, account_id, device_id, issued_at, expires_at)
           VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(&row.token_hash)
    .bind(&row.account_id)
    .bind(&row.device_id)
    .bind(row.issued_at)
    .bind(row.expires_at)
    .execute(pool)
    .await?;
    Ok(row)
}

pub async fn find_active(
    pool: &SqlitePool,
    plaintext: &str,
) -> Result<Option<RefreshTokenRow>, BackendError> {
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    let row = sqlx::query_as::<_, RefreshTokenRow>(
        r#"SELECT token_hash, account_id, device_id, issued_at, expires_at, revoked_at
           FROM refresh_tokens
           WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?"#,
    )
    .bind(&hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn revoke_one(pool: &SqlitePool, plaintext: &str) -> Result<(), BackendError> {
    let hash = hash_plaintext(plaintext);
    let now = Utc::now().timestamp_millis();
    sqlx::query("UPDATE refresh_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(&hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Revoke every active refresh token for an account. Used on login to
/// enforce single-active-iPhone (spec §5.2).
pub async fn revoke_all_for_account(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<u64, BackendError> {
    let now = Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
```

- [ ] **Step 3: Add `rand`, `sha2`, `hex` deps if missing**

Check `crates/minos-backend/Cargo.toml`. `sha2` is already present. Add `hex = "0.4"` (workspace) and `rand = "0.8"` if missing. Verify with: `cargo check -p minos-backend`.

- [ ] **Step 4: Wire module**

In `crates/minos-backend/src/store/mod.rs`:

```rust
pub mod refresh_tokens;
```

- [ ] **Step 5: sqlx prepare + cargo check**

```bash
cargo check -p minos-backend
DATABASE_URL=sqlite::memory: cargo sqlx prepare --workspace
```

- [ ] **Step 6: Commit**

```bash
git add crates/minos-backend/migrations/0008_refresh_tokens.sql \
        crates/minos-backend/src/store/refresh_tokens.rs \
        crates/minos-backend/src/store/mod.rs \
        crates/minos-backend/Cargo.toml Cargo.toml \
        .sqlx/
git commit -m "feat(backend): add refresh_tokens table + store"
```

---

### Task 1.4: Migration `0009_devices_account_link.sql` + extend `devices` store

**Files:**
- Create: `crates/minos-backend/migrations/0009_devices_account_link.sql`
- Modify: `crates/minos-backend/src/store/devices.rs`

- [ ] **Step 1: Write migration**

`crates/minos-backend/migrations/0009_devices_account_link.sql`:

```sql
ALTER TABLE devices ADD COLUMN account_id TEXT REFERENCES accounts(account_id);
CREATE INDEX idx_devices_account ON devices(account_id) WHERE account_id IS NOT NULL;
```

- [ ] **Step 2: Extend `DeviceRow` and queries**

Read `crates/minos-backend/src/store/devices.rs` first to find `DeviceRow` and update the SELECT lists. Add `account_id: Option<String>` to the struct and to every SELECT in the file.

Add a helper:

```rust
pub async fn set_account_id(
    pool: &SqlitePool,
    device_id: &DeviceId,
    account_id: &str,
) -> Result<(), BackendError> {
    sqlx::query("UPDATE devices SET account_id = ? WHERE device_id = ?")
        .bind(account_id)
        .bind(device_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: sqlx prepare + cargo check**

```bash
cargo check -p minos-backend --tests
DATABASE_URL=sqlite::memory: cargo sqlx prepare --workspace
```

- [ ] **Step 4: Commit**

```bash
git add crates/minos-backend/migrations/0009_devices_account_link.sql \
        crates/minos-backend/src/store/devices.rs \
        .sqlx/
git commit -m "feat(backend): link devices.account_id to accounts(account_id)"
```

---

### Task 1.5: `passwords.rs` (argon2id wrappers)

**Files:**
- Create: `crates/minos-backend/src/auth/mod.rs`
- Create: `crates/minos-backend/src/auth/passwords.rs`
- Modify: `crates/minos-backend/src/lib.rs`

- [ ] **Step 1: Create `auth/mod.rs`**

```rust
//! Account-auth (bearer-token) rail. Coexists with the device-secret
//! rail (`crate::http::auth`). Spec §5.3–5.4.

pub mod bearer;
pub mod jwt;
pub mod passwords;
```

- [ ] **Step 2: Implement `passwords.rs`**

Mirror the style of `crates/minos-backend/src/pairing/secret.rs` (uses `Argon2::default()`):

```rust
//! Argon2id password hashing. Reuses the workspace's existing default
//! parameters (`m=19456, t=2, p=1`) — see `pairing/secret.rs`.

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::rngs::OsRng;

use crate::error::BackendError;

pub fn hash(password: &str) -> Result<String, BackendError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| BackendError::PasswordHash {
            message: e.to_string(),
        })
}

pub fn verify(password: &str, encoded: &str) -> Result<bool, BackendError> {
    let parsed = PasswordHash::new(encoded).map_err(|e| BackendError::PasswordHash {
        message: e.to_string(),
    })?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let h = hash("hunter22").unwrap();
        assert!(verify("hunter22", &h).unwrap());
        assert!(!verify("wrong", &h).unwrap());
    }
}
```

- [ ] **Step 3: Add `PasswordHash` error variant**

In `crates/minos-backend/src/error.rs`:

```rust
#[error("password hash error: {message}")]
PasswordHash { message: String },
```

- [ ] **Step 4: Wire `auth` mod**

In `crates/minos-backend/src/lib.rs`:

```rust
pub mod auth;
```

- [ ] **Step 5: Run unit tests**

```bash
cargo test -p minos-backend auth::passwords
```

Expected: 1 PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-backend/src/auth/ \
        crates/minos-backend/src/lib.rs \
        crates/minos-backend/src/error.rs
git commit -m "feat(backend): argon2id password hashing helpers"
```

---

### Task 1.6: `jwt.rs` (sign + verify)

**Files:**
- Create: `crates/minos-backend/src/auth/jwt.rs`

- [ ] **Step 1: Write the module**

```rust
//! HS256 JWT helpers. Spec §5.3.
//!
//! Claims: { sub: account_id, did: device_id, iat, exp, jti }. The
//! `did` claim binds the access token to a specific device — replay
//! from another device is rejected at verify time.

use chrono::Utc;
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::BackendError;

pub const ACCESS_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub did: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

pub fn sign(secret: &[u8], account_id: &str, device_id: &str) -> Result<String, BackendError> {
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: account_id.into(),
        did: device_id.into(),
        iat: now,
        exp: now + ACCESS_TTL_SECS,
        jti: Uuid::new_v4().to_string(),
    };
    encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret))
        .map_err(|e| BackendError::JwtSign { message: e.to_string() })
}

/// Parse + verify (signature + exp). Caller is responsible for
/// `did == X-Device-Id` check (`bearer.rs` does it).
pub fn verify(secret: &[u8], token: &str) -> Result<Claims, BackendError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 5;
    let data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map_err(|e| BackendError::JwtVerify { message: e.to_string() })?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let secret = b"a".repeat(32);
        let tok = sign(&secret, "acct-1", "dev-1").unwrap();
        let claims = verify(&secret, &tok).unwrap();
        assert_eq!(claims.sub, "acct-1");
        assert_eq!(claims.did, "dev-1");
    }

    #[test]
    fn verify_with_wrong_secret_fails() {
        let tok = sign(&b"a".repeat(32), "acct-1", "dev-1").unwrap();
        assert!(verify(&b"b".repeat(32), &tok).is_err());
    }
}
```

- [ ] **Step 2: Add `JwtSign` / `JwtVerify` error variants**

In `crates/minos-backend/src/error.rs`:

```rust
#[error("jwt sign error: {message}")]
JwtSign { message: String },
#[error("jwt verify error: {message}")]
JwtVerify { message: String },
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p minos-backend auth::jwt
```

Expected: 2 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-backend/src/auth/jwt.rs \
        crates/minos-backend/src/error.rs
git commit -m "feat(backend): HS256 JWT sign + verify"
```

---

### Task 1.7: `bearer.rs` extractor

**Files:**
- Create: `crates/minos-backend/src/auth/bearer.rs`

- [ ] **Step 1: Implement**

```rust
//! Bearer-token extractor (spec §5.4). Pattern mirrors
//! `crate::http::auth::authenticate` — handler-level call, not axum
//! middleware, so handlers can opt in per route.

use axum::http::{HeaderMap, StatusCode};

use crate::auth::jwt::{self, Claims};
use crate::error::BackendError;
use crate::http::auth::extract_device_id;
use crate::state::BackendState;

pub struct AccountAuthOutcome {
    pub account_id: String,
    pub device_id: String,
    pub claims: Claims,
}

pub enum BearerError {
    Missing,
    Invalid(String),
    DeviceMismatch,
}

impl BearerError {
    pub fn into_response_tuple(self) -> (StatusCode, String) {
        match self {
            Self::Missing => (StatusCode::UNAUTHORIZED, "missing bearer".into()),
            Self::Invalid(m) => (StatusCode::UNAUTHORIZED, format!("invalid bearer: {m}")),
            Self::DeviceMismatch => (StatusCode::UNAUTHORIZED, "device mismatch".into()),
        }
    }
}

pub fn require(
    state: &BackendState,
    headers: &HeaderMap,
) -> Result<AccountAuthOutcome, BearerError> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(BearerError::Missing)?;
    let tok = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or(BearerError::Missing)?;
    let claims = jwt::verify(state.jwt_secret.as_bytes(), tok)
        .map_err(|e| match e {
            BackendError::JwtVerify { message } => BearerError::Invalid(message),
            _ => BearerError::Invalid("verify failed".into()),
        })?;
    let device_id = extract_device_id(headers)
        .map_err(|_| BearerError::DeviceMismatch)?;
    if claims.did != device_id.to_string() {
        return Err(BearerError::DeviceMismatch);
    }
    Ok(AccountAuthOutcome {
        account_id: claims.sub.clone(),
        device_id: device_id.to_string(),
        claims,
    })
}
```

> **Note:** This file references `state.jwt_secret` (Task 1.8) and `BackendState` location. Check the actual import path — it may be `crate::http::state::BackendState` or similar; adapt the `use` line to match.

- [ ] **Step 2: Cargo check**

```bash
cargo check -p minos-backend
```

Expected: PASS once Task 1.8 lands; until then, `state.jwt_secret` will fail. Either land Task 1.8 first or temporarily comment out the `jwt::verify` call.

- [ ] **Step 3: Commit (after Task 1.8)**

```bash
git add crates/minos-backend/src/auth/bearer.rs
git commit -m "feat(backend): Bearer token extractor with device-id binding"
```

---

### Task 1.8: Add `MINOS_JWT_SECRET` to config

**Files:**
- Modify: `crates/minos-backend/src/config.rs`
- Modify: wherever `BackendState` is constructed (likely `src/state.rs` or `src/lib.rs`)

- [ ] **Step 1: Add config field**

In `crates/minos-backend/src/config.rs`, after the existing `#[arg(...)]` block for an existing string field, add:

```rust
#[arg(long, env = "MINOS_JWT_SECRET")]
pub jwt_secret: Option<String>,
```

- [ ] **Step 2: Validate in `validate(&self)`**

In the existing `validate` method, append:

```rust
let secret = self.jwt_secret.as_ref()
    .ok_or_else(|| "MINOS_JWT_SECRET is required".to_string())?;
if secret.as_bytes().len() < 32 {
    return Err("MINOS_JWT_SECRET must be ≥32 bytes".into());
}
```

- [ ] **Step 3: Thread into `BackendState`**

Find `BackendState`, add `pub jwt_secret: Arc<String>` field. In its constructor, take and store the secret. Confirm `bearer.rs::require` resolves.

- [ ] **Step 4: Set test fixture**

Find the test-support helper that builds an in-memory backend (per exploration: `crates/minos-backend/src/test_support` or similar). Set `MINOS_JWT_SECRET` to a deterministic 32-byte string in the fixture.

- [ ] **Step 5: Cargo check**

```bash
cargo check -p minos-backend --tests
```

- [ ] **Step 6: Commit**

```bash
git add crates/minos-backend/src/config.rs \
        crates/minos-backend/src/state.rs \
        crates/minos-backend/src/test_support
git commit -m "feat(backend): require MINOS_JWT_SECRET via config"
```

---

### Task 1.9–1.12: Auth endpoints (`/v1/auth/{register,login,refresh,logout}`)

**Files:**
- Create: `crates/minos-backend/src/http/v1/auth.rs`
- Modify: `crates/minos-backend/src/http/v1/mod.rs`

> Bundle these four endpoints in one file. They share request/response types and helpers.

- [ ] **Step 1: Define request/response types**

```rust
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::{bearer, jwt, passwords};
use crate::error::BackendError;
use crate::state::BackendState;
use crate::store::{accounts, refresh_tokens};
use crate::http::auth::{authenticate, extract_device_id};

#[derive(Deserialize)]
pub struct RegisterReq { pub email: String, pub password: String }
#[derive(Deserialize)]
pub struct LoginReq { pub email: String, pub password: String }
#[derive(Deserialize)]
pub struct RefreshReq { pub refresh_token: String }
#[derive(Deserialize)]
pub struct LogoutReq { pub refresh_token: String }

#[derive(Serialize)]
pub struct AuthResp {
    pub account: AccountSummary,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Serialize)]
pub struct AccountSummary {
    pub account_id: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct RefreshResp {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Serialize)]
pub struct ErrorBody { pub kind: &'static str }
```

- [ ] **Step 2: Register handler**

```rust
pub async fn post_register(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Result<(StatusCode, Json<AuthResp>), (StatusCode, Json<ErrorBody>)> {
    if req.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorBody { kind: "weak_password" })));
    }
    let outcome = authenticate(&state.store, &headers).await
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "unauthorized" })))?;
    let device_id = outcome.device_id;

    let hash = passwords::hash(&req.password)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    let account = accounts::create(&state.store, &req.email, &hash).await
        .map_err(|e| match e {
            BackendError::EmailTaken => (StatusCode::CONFLICT, Json(ErrorBody { kind: "email_taken" })),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })),
        })?;

    crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;

    let access = jwt::sign(state.jwt_secret.as_bytes(), &account.account_id, &device_id.to_string())
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    let refresh_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(&state.store, &refresh_plain, &account.account_id, &device_id.to_string()).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;

    Ok((StatusCode::OK, Json(AuthResp {
        account: AccountSummary { account_id: account.account_id, email: account.email },
        access_token: access,
        refresh_token: refresh_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    })))
}
```

- [ ] **Step 3: Login handler**

```rust
pub async fn post_login(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Json<AuthResp>, (StatusCode, Json<ErrorBody>)> {
    let outcome = authenticate(&state.store, &headers).await
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "unauthorized" })))?;
    let device_id = outcome.device_id;

    let account = accounts::find_by_email(&state.store, &req.email).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?
        .ok_or((StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "invalid_credentials" })))?;
    let ok = passwords::verify(&req.password, &account.password_hash)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    if !ok {
        return Err((StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "invalid_credentials" })));
    }

    // Single-active-iPhone: revoke prior refresh tokens for this account.
    let _ = refresh_tokens::revoke_all_for_account(&state.store, &account.account_id).await;
    // Forcibly close any WS sessions on other devices for this account.
    state.session_registry.close_account_sessions(
        &account.account_id,
        Some(&device_id.to_string()),
    );

    crate::store::accounts::touch_last_login(&state.store, &account.account_id).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    crate::store::devices::set_account_id(&state.store, &device_id, &account.account_id).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;

    let access = jwt::sign(state.jwt_secret.as_bytes(), &account.account_id, &device_id.to_string())
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    let refresh_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(&state.store, &refresh_plain, &account.account_id, &device_id.to_string()).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;

    Ok(Json(AuthResp {
        account: AccountSummary { account_id: account.account_id, email: account.email },
        access_token: access,
        refresh_token: refresh_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    }))
}
```

- [ ] **Step 4: Refresh handler**

```rust
pub async fn post_refresh(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<RefreshReq>,
) -> Result<Json<RefreshResp>, (StatusCode, Json<ErrorBody>)> {
    let outcome = authenticate(&state.store, &headers).await
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "unauthorized" })))?;
    let device_id = outcome.device_id;

    let row = refresh_tokens::find_active(&state.store, &req.refresh_token).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?
        .ok_or((StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "invalid_refresh" })))?;

    if row.device_id != device_id.to_string() {
        return Err((StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "invalid_refresh" })));
    }

    refresh_tokens::revoke_one(&state.store, &req.refresh_token).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    let new_plain = refresh_tokens::generate_plaintext();
    refresh_tokens::insert(&state.store, &new_plain, &row.account_id, &row.device_id).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    let access = jwt::sign(state.jwt_secret.as_bytes(), &row.account_id, &row.device_id)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;

    Ok(Json(RefreshResp {
        access_token: access,
        refresh_token: new_plain,
        expires_in: jwt::ACCESS_TTL_SECS,
    }))
}
```

- [ ] **Step 5: Logout handler**

```rust
pub async fn post_logout(
    State(state): State<BackendState>,
    headers: HeaderMap,
    Json(req): Json<LogoutReq>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let _ = authenticate(&state.store, &headers).await
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(ErrorBody { kind: "unauthorized" })))?;
    let _ = bearer::require(&state, &headers).map_err(|e| {
        let (s, _) = e.into_response_tuple();
        (s, Json(ErrorBody { kind: "unauthorized" }))
    })?;
    refresh_tokens::revoke_one(&state.store, &req.refresh_token).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { kind: "internal" })))?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 6: Build router**

At the bottom of `auth.rs`:

```rust
pub fn router() -> Router<BackendState> {
    Router::new()
        .route("/v1/auth/register", post(post_register))
        .route("/v1/auth/login",    post(post_login))
        .route("/v1/auth/refresh",  post(post_refresh))
        .route("/v1/auth/logout",   post(post_logout))
}
```

- [ ] **Step 7: Wire into `v1/mod.rs`**

```rust
pub fn router() -> Router<BackendState> {
    Router::new()
        .merge(pairing::router())
        .merge(threads::router())
        .merge(auth::router())
}
```

Add `mod auth;` at top of file.

- [ ] **Step 8: Cargo check + commit**

```bash
cargo check -p minos-backend --tests
git add crates/minos-backend/src/http/v1/auth.rs crates/minos-backend/src/http/v1/mod.rs
git commit -m "feat(backend): /v1/auth/{register,login,refresh,logout} endpoints"
```

---

### Task 1.13: Backend integration tests for auth endpoints

**Files:**
- Create: `crates/minos-backend/tests/auth_endpoints.rs`

- [ ] **Step 1: Write happy-path test**

Mirror the existing `tests/http_smoke.rs` style — use `tower::ServiceExt` to drive the router. Add at minimum these 8 tests (one per scenario in spec §9.2):

```rust
//! Account-auth HTTP integration tests. Each test runs against a fresh
//! in-memory SQLite via `#[sqlx::test]`-style fixtures.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use minos_backend::http;
use minos_backend::test_support::{backend_state, with_jwt_secret};
use serde_json::json;
use tower::ServiceExt;

async fn post(router: &axum::Router, path: &str, headers: &[(&str, &str)], body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let mut req = Request::post(path).header("content-type", "application/json");
    for (k, v) in headers { req = req.header(*k, *v); }
    let resp = router.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn auth_register_login_refresh_logout_happy_path() {
    let state = with_jwt_secret(backend_state().await);
    let router = http::router(state);
    let did = uuid::Uuid::new_v4().to_string();

    let (s, body) = post(&router, "/v1/auth/register",
        &[("x-device-id", &did), ("x-device-role", "ios-client")],
        json!({"email": "a@b.com", "password": "testpass1"})).await;
    assert_eq!(s, StatusCode::OK);
    let access = body["access_token"].as_str().unwrap().to_string();
    let refresh = body["refresh_token"].as_str().unwrap().to_string();
    assert!(!access.is_empty() && !refresh.is_empty());

    let (s, body) = post(&router, "/v1/auth/login",
        &[("x-device-id", &did), ("x-device-role", "ios-client")],
        json!({"email": "a@b.com", "password": "testpass1"})).await;
    assert_eq!(s, StatusCode::OK);
    let new_refresh = body["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, refresh);

    let (s, body) = post(&router, "/v1/auth/refresh",
        &[("x-device-id", &did), ("x-device-role", "ios-client")],
        json!({"refresh_token": new_refresh})).await;
    assert_eq!(s, StatusCode::OK);
    let new_access = body["access_token"].as_str().unwrap().to_string();

    let (s, _) = post(&router, "/v1/auth/logout",
        &[("x-device-id", &did),
          ("x-device-role", "ios-client"),
          ("authorization", &format!("Bearer {new_access}"))],
        json!({"refresh_token": body["refresh_token"]})).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}
```

(Add `auth_register_duplicate_email_returns_409`, `auth_login_wrong_password_returns_401`, `auth_login_revokes_existing_refresh_tokens`, `auth_refresh_with_revoked_token_returns_401`, `auth_refresh_rotation_old_token_invalidated`, `auth_logout_revokes_only_current_refresh_token` following the same skeleton.)

- [ ] **Step 2: Add `with_jwt_secret` test helper if missing**

In `crates/minos-backend/src/test_support/mod.rs`, add a helper that injects a deterministic 32-byte secret into `BackendState`.

- [ ] **Step 3: Run tests**

```bash
cargo test -p minos-backend --test auth_endpoints
```

Expected: 8 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-backend/tests/auth_endpoints.rs \
        crates/minos-backend/src/test_support/
git commit -m "test(backend): integration tests for /v1/auth/*"
```

---

### Task 1.14: Rate limiting (per-IP, per-email)

**Files:**
- Create: `crates/minos-backend/src/auth/rate_limit.rs`
- Modify: `crates/minos-backend/src/http/v1/auth.rs`
- Modify: `crates/minos-backend/src/auth/mod.rs`

> **Note:** Spec §5.6 says "tower-governor or hand-rolled in-memory token bucket". Given the dep churn risk flagged in spec §12.1, write a minimal hand-rolled bucket — the rate limits are coarse and a tower middleware is overkill.

- [ ] **Step 1: Implement bucket**

```rust
//! Coarse in-memory rate limiter for auth endpoints. Per-key sliding
//! window with `permits` slots over `window`. Spec §5.6.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
    permits: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(permits: usize, window: Duration) -> Self {
        Self { inner: Mutex::new(HashMap::new()), permits, window }
    }

    /// Returns `Ok(())` if a permit was granted, `Err(retry_after_secs)`
    /// if the bucket is full.
    pub fn check(&self, key: &str) -> Result<(), u32> {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        let entries = map.entry(key.into()).or_default();
        entries.retain(|t| now.duration_since(*t) < self.window);
        if entries.len() >= self.permits {
            let oldest = entries[0];
            let retry = self.window
                .saturating_sub(now.duration_since(oldest))
                .as_secs() as u32;
            return Err(retry.max(1));
        }
        entries.push(now);
        Ok(())
    }
}
```

- [ ] **Step 2: Wire instances into `BackendState`**

```rust
pub auth_login_per_email: Arc<RateLimiter>,   // 10 / minute
pub auth_login_per_ip:    Arc<RateLimiter>,   //  5 / minute
pub auth_register_per_ip: Arc<RateLimiter>,   //  3 / hour
pub auth_refresh_per_acc: Arc<RateLimiter>,   // 60 / hour
```

- [ ] **Step 3: Apply at handler entry**

In each `post_*` handler in `auth.rs`, call the appropriate limiter at the very top:

```rust
let ip = headers.get("x-forwarded-for")
    .and_then(|v| v.to_str().ok())
    .unwrap_or("unknown")
    .to_string();
if let Err(retry) = state.auth_login_per_ip.check(&ip) {
    let mut hdrs = HeaderMap::new();
    hdrs.insert("Retry-After", retry.to_string().parse().unwrap());
    return Err((StatusCode::TOO_MANY_REQUESTS, Json(ErrorBody { kind: "rate_limited" })));
}
```

- [ ] **Step 4: Add a test**

Append `auth_rate_limit_login_returns_429_with_retry_after` to `tests/auth_endpoints.rs` — fire login 6× from the same IP; assert the 6th returns 429.

- [ ] **Step 5: Run tests + commit**

```bash
cargo test -p minos-backend --test auth_endpoints
git add crates/minos-backend/src/auth/rate_limit.rs \
        crates/minos-backend/src/auth/mod.rs \
        crates/minos-backend/src/http/v1/auth.rs \
        crates/minos-backend/src/state.rs \
        crates/minos-backend/tests/auth_endpoints.rs
git commit -m "feat(backend): hand-rolled rate limiter for /v1/auth/*"
```

---

## Phase 2: Backend Integration (Account-Aware Routing)

### Task 2.1: Extend `SessionHandle` with `account_id`

**Files:**
- Modify: `crates/minos-backend/src/session/registry.rs`

- [ ] **Step 1: Add field**

In `SessionHandle` struct, add:

```rust
pub account_id: Mutex<Option<String>>,
```

(Use `Mutex` because the value gets seeded after upgrade succeeds, the same way `paired_with` is seeded.)

- [ ] **Step 2: Initialize in `SessionHandle::new`**

```rust
account_id: Mutex::new(None),
```

- [ ] **Step 3: Add setter**

```rust
impl SessionHandle {
    pub fn set_account_id(&self, id: String) {
        *self.account_id.lock().unwrap() = Some(id);
    }
    pub fn account_id(&self) -> Option<String> {
        self.account_id.lock().unwrap().clone()
    }
}
```

- [ ] **Step 4: Cargo check + commit**

```bash
cargo check -p minos-backend
git add crates/minos-backend/src/session/registry.rs
git commit -m "feat(backend): SessionHandle carries account_id"
```

---

### Task 2.2: Require Bearer for ios-client WS upgrade

**Files:**
- Modify: `crates/minos-backend/src/http/ws_devices.rs`

- [ ] **Step 1: Branch on role after `authenticate`**

In the `upgrade` handler, after the existing `auth::authenticate(...)` call (per exploration around line 97), branch:

```rust
let outcome = auth::authenticate(&state.store, &headers).await
    .map_err(|e| e.into_response_tuple())?;

let account_id = if outcome.role == DeviceRole::IosClient {
    let bearer = bearer::require(&state, &headers)
        .map_err(|e| e.into_response_tuple())?;
    Some(bearer.account_id)
} else {
    None
};
```

- [ ] **Step 2: Seed `SessionHandle.account_id` after construction**

Right after `SessionHandle::new(...)`, call:

```rust
if let Some(aid) = account_id.as_ref() { handle.set_account_id(aid.clone()); }
```

- [ ] **Step 3: Add test for unauthorized ios upgrade**

In an existing or new integration test file, add a test that an ios-client WS upgrade without `Authorization: Bearer` returns 401.

- [ ] **Step 4: Run tests + commit**

```bash
cargo test -p minos-backend
git add crates/minos-backend/src/http/ws_devices.rs
git commit -m "feat(backend): require Bearer on iOS WS upgrade"
```

---

### Task 2.3: Pairing/consume requires Bearer for iOS, writes account_id

**Files:**
- Modify: `crates/minos-backend/src/http/v1/pairing.rs`
- Possibly: `crates/minos-backend/src/pairing/mod.rs`

- [ ] **Step 1: Require Bearer in `post_consume`**

After the existing `auth::authenticate_role(..., DeviceRole::IosClient)` call:

```rust
let bearer = bearer::require(&state, &headers).map_err(|e| {
    let (s, m) = e.into_response_tuple();
    (s, Json(ErrorEnvelope::new(m)))
})?;
let account_id = bearer.account_id;
```

- [ ] **Step 2: After `consume_token` succeeds, copy account_id to Mac device**

Right after the consume call:

```rust
crate::store::devices::set_account_id(
    &state.store,
    &pairing_outcome.issuer_device_id,  // the Mac
    &account_id,
).await.map_err(|e| ...)?;
```

(Also re-write `account_id` on the iOS device row in case the bearer's account differs from a stale value.)

- [ ] **Step 3: Cargo check + commit**

```bash
cargo check -p minos-backend
git add crates/minos-backend/src/http/v1/pairing.rs
git commit -m "feat(backend): pairing/consume copies account_id to Mac side"
```

---

### Task 2.4: Account-aware Mac→iOS routing query

**Files:**
- Modify: `crates/minos-backend/src/session/registry.rs`

- [ ] **Step 1: Update `route()` to filter by account when sender is Mac**

Inside `route(from, to, payload)`, prior to looking up `to` in the live session map, if the `from` session's role is `agent-host`, restrict `to` to a device whose `account_id` matches `from`'s account_id:

```rust
let from_handle = self.0.get(&from).ok_or(BackendError::NotPaired)?;
if from_handle.role == DeviceRole::AgentHost {
    let from_account = from_handle.account_id().ok_or(BackendError::NotPaired)?;
    let to_handle = self.0.get(&to).ok_or(BackendError::NotPaired)?;
    if to_handle.account_id() != Some(from_account) {
        return Err(BackendError::NotPaired);
    }
}
```

- [ ] **Step 2: Add integration test**

Test that a Mac forwarding to a paired iPhone whose `account_id` differs returns NotPaired.

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p minos-backend
git add crates/minos-backend/src/session/registry.rs
git commit -m "feat(backend): Mac→iOS route filters by account_id"
```

---

### Task 2.5: `SessionRegistry::close_account_sessions`

**Files:**
- Modify: `crates/minos-backend/src/session/registry.rs`

- [ ] **Step 1: Implement**

```rust
impl SessionRegistry {
    /// Revoke + drop every session owned by `account_id` except `except`.
    /// Returns the count closed (used in tracing). Spec §5.5.
    pub fn close_account_sessions(
        &self,
        account_id: &str,
        except: Option<&str>,
    ) -> usize {
        let to_close: Vec<DeviceId> = self.0.iter()
            .filter(|e| {
                let h = e.value();
                let role_ok = h.role == DeviceRole::IosClient;
                let account_ok = h.account_id().as_deref() == Some(account_id);
                let not_except = except.map(|s| s != e.key().to_string()).unwrap_or(true);
                role_ok && account_ok && not_except
            })
            .map(|e| *e.key())
            .collect();
        let mut closed = 0;
        for id in to_close {
            if let Some((_, handle)) = self.0.remove(&id) {
                handle.revoke();
                closed += 1;
            }
        }
        closed
    }
}
```

- [ ] **Step 2: Test**

Spin two iOS WS sessions for the same account; call `close_account_sessions(account_id, except: device_a)`; assert `device_b` is dropped from the registry and its `revoked` watch fired.

- [ ] **Step 3: Cargo check + commit**

```bash
cargo test -p minos-backend
git add crates/minos-backend/src/session/registry.rs
git commit -m "feat(backend): close_account_sessions for single-active-iPhone"
```

---

### Task 2.6: Account-scoped thread queries

**Files:**
- Modify: `crates/minos-backend/src/http/v1/threads.rs`
- Possibly: `crates/minos-backend/src/store/threads.rs`

- [ ] **Step 1: Modify `require_paired_session` to also produce account_id**

Make it return `(owner: DeviceId, account_id: String)` and call `bearer::require` in addition to the existing device-secret check.

- [ ] **Step 2: Filter store queries by account**

In `store/threads.rs::list`, accept an optional `account_id: Option<&str>` param and add a JOIN+WHERE to restrict to threads whose `owner_device.account_id` matches.

- [ ] **Step 3: Test**

Add `routing_threads_filtered_by_account` integration test: two accounts, two pairings, list_threads on account A only sees A's threads.

- [ ] **Step 4: Run tests + commit**

```bash
cargo test -p minos-backend
git add crates/minos-backend/src/http/v1/threads.rs \
        crates/minos-backend/src/store/threads.rs \
        .sqlx/
git commit -m "feat(backend): thread queries scoped by bearer account"
```

---

## Phase 3: Protocol + Error Variants

### Task 3.1: New `MinosError` variants

**Files:**
- Modify: `crates/minos-domain/src/error.rs`

- [ ] **Step 1: Read the file**

Read `crates/minos-domain/src/error.rs` lines 24-263. The header comment at lines 60-66 lists the six places that must be touched per new variant. Honor it.

- [ ] **Step 2: Add `ErrorKind` variants**

After the existing variants in `ErrorKind`:

```rust
Timeout,
NotConnected,
RequestDropped,
AuthRefreshFailed,
EmailTaken,
WeakPassword,
RateLimited,
InvalidCredentials,
AgentStartFailed,
PairingTokenExpired,
```

- [ ] **Step 3: Add `MinosError` variants**

In `MinosError`:

```rust
#[error("request timed out")]
Timeout,
#[error("not connected to backend")]
NotConnected,
#[error("request dropped (connection closed)")]
RequestDropped,
#[error("auth refresh failed: {message}")]
AuthRefreshFailed { message: String },
#[error("email already registered")]
EmailTaken,
#[error("password too weak (min 8 chars)")]
WeakPassword,
#[error("rate limited (retry after {retry_after_s}s)")]
RateLimited { retry_after_s: u32 },
#[error("invalid credentials")]
InvalidCredentials,
#[error("agent start failed: {reason}")]
AgentStartFailed { reason: String },
#[error("pairing token expired")]
PairingTokenExpired,
```

- [ ] **Step 4: Update `kind()` mapping**

Add mappings for each new variant in the `kind()` method.

- [ ] **Step 5: Update localized message helpers**

If there is a `kind_message_zh` / `kind_message_en` table, add Chinese + English strings for each new kind. Spec §8.1 specifies a few of the user-facing strings (e.g. "另一台设备登录,请重新登录" for refresh-revoked).

- [ ] **Step 6: Run tests**

```bash
cargo test -p minos-domain
```

Expected: existing tests still pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-domain/src/error.rs
git commit -m "feat(domain): MinosError variants for auth + dispatch + lifecycle"
```

---

### Task 3.2: Auth API DTOs in `minos-protocol`

**Files:**
- Create: `crates/minos-protocol/src/auth.rs`
- Modify: `crates/minos-protocol/src/lib.rs`

- [ ] **Step 1: Define request/response types matching backend**

```rust
//! HTTP DTOs for the /v1/auth/* endpoints. Spec §5.2.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSummary {
    pub account_id: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthResponse {
    pub account: AuthSummary,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefreshRequest { pub refresh_token: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogoutRequest { pub refresh_token: String }
```

- [ ] **Step 2: Re-export**

In `crates/minos-protocol/src/lib.rs`, add `pub mod auth;` and `pub use auth::*;`.

- [ ] **Step 3: Roundtrip test**

In `auth.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_response_round_trip() {
        let r = AuthResponse {
            account: AuthSummary { account_id: "a".into(), email: "a@b".into() },
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_in: 900,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: AuthResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p minos-protocol
git add crates/minos-protocol/src/auth.rs crates/minos-protocol/src/lib.rs
git commit -m "feat(protocol): auth DTOs (Auth/Refresh/Logout)"
```

---

## Checkpoint: Phase 1+2+3 → backend ships

At this point the backend can register / login / refresh / logout, account-scope its routing, and the protocol surface knows about the new error and DTO shapes. Verify the workspace gate before moving on:

```bash
cargo xtask check-all
```

Expected: green. If not, fix before continuing.

---

> **Plan continues in subsequent files** — see `08a-mobile-rust-and-frb.md` for Phases 4–7 and `08b-flutter-and-verification.md` for Phases 8–12.

This file (Phases 1–3) is self-contained: backend ships and is testable end-to-end with `curl` / `cargo test -p minos-backend`. The mobile and Flutter phases land in companion files to keep each plan document under reading-comfort length.
