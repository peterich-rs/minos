//! CLI + env configuration for the `minos-backend` binary.
//!
//! Design: `clap` 4 derive + `env = "..."` attributes so every flag has a
//! paired environment-variable override. Defaults are codified as
//! `default_value`/`default_value_t` literals so `--help` prints the exact
//! values.
//!
//! The log directory default is platform-dependent (see [`default_log_dir`])
//! and therefore resolved at runtime rather than being a clap literal — the
//! `Option<PathBuf>` field plus the [`Config::resolved_log_dir`] helper
//! captures that without confusing `--help`.
//!
//! # Exit-after-migrate
//!
//! `--exit-after-migrate` is a boot-time flag used by
//! `cargo xtask backend-db-reset`. When set, `main.rs` applies
//! migrations and exits with code 0 without binding the axum listener or
//! spawning the GC task. The normal boot body only runs when
//! this flag is absent.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use serde::Serialize;

use crate::realtime::{CacheBackendKind, MessageBusBackendKind};

/// Default pairing-token TTL (5 minutes).
pub(crate) const DEFAULT_TOKEN_TTL_SECS: u64 = 300;
pub(crate) const DEFAULT_DB_MAX_CONNECTIONS: u32 = 32;
pub(crate) const DEFAULT_CLUSTER_CHANNEL: &str = "minos.backend.cluster";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

impl Environment {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Prod => "prod",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
pub enum StorageMode {
    Sqlite,
    ExternalSql,
}

impl StorageMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::ExternalSql => "external-sql",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
pub enum RuntimeMode {
    Monolith,
    HttpOnly,
    WorkerOnly,
}

impl RuntimeMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monolith => "monolith",
            Self::HttpOnly => "http-only",
            Self::WorkerOnly => "worker-only",
        }
    }

    #[must_use]
    pub const fn serves_http(self) -> bool {
        matches!(self, Self::Monolith | Self::HttpOnly)
    }

    #[must_use]
    pub const fn runs_supervised_workers(self) -> bool {
        matches!(self, Self::Monolith | Self::WorkerOnly)
    }
}

/// Minos backend: axum WebSocket hub with SQLite-first state.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Config {
    /// TCP socket to listen on.
    #[arg(
        long,
        env = "MINOS_BACKEND_LISTEN",
        default_value_t = minos_domain::defaults::DEV_BACKEND_LISTEN
            .parse::<SocketAddr>()
            .expect("DEV_BACKEND_LISTEN is a compile-time-valid SocketAddr"),
    )]
    pub listen: SocketAddr,

    /// SQLite database path. Created on first run via sqlx
    /// `create_if_missing(true)`.
    #[arg(long, env = "MINOS_BACKEND_DB", default_value = "./minos-backend.db")]
    pub db: PathBuf,

    /// Storage adapter profile for the runtime shell.
    ///
    /// `sqlite` is the local/dev default. `external-sql` opens a live Postgres
    /// pool (`MINOS_DATABASE_URL`) and serves the full HTTP/WS surface against
    /// it. Logical schema is shared across dialects (see storage-parity spec).
    #[arg(long, env = "MINOS_STORAGE_MODE", value_enum, default_value = "sqlite")]
    pub storage_mode: StorageMode,

    /// Explicit database URL used by the runtime shell.
    ///
    /// In `sqlite` mode this may optionally override `--db` with a sqlite://
    /// URL. In `external-sql` mode it is required and must be a
    /// `postgres://` or `postgresql://` URL.
    #[arg(long, env = "MINOS_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Maximum number of SQLite pool connections.
    #[arg(
        long,
        env = "MINOS_BACKEND_DB_MAX_CONNECTIONS",
        default_value_t = DEFAULT_DB_MAX_CONNECTIONS,
        value_parser = clap::value_parser!(u32).range(1..),
    )]
    pub db_max_connections: u32,

    /// Directory for xlog files. Defaults to `~/Library/Logs/Minos/` on
    /// macOS and `$TMPDIR/minos` elsewhere (resolved at runtime; not shown
    /// in `--help` because the default is platform-dependent).
    #[arg(long, env = "MINOS_BACKEND_LOG_DIR")]
    pub log_dir: Option<PathBuf>,

    /// Log level. Accepts plain levels (`trace`/`debug`/`info`/`warn`/`error`)
    /// and full `env_logger`-style directives (e.g. `minos_backend=debug,info`).
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,

    /// Pairing token TTL in seconds.
    #[arg(long, env = "MINOS_BACKEND_TOKEN_TTL", default_value_t = DEFAULT_TOKEN_TTL_SECS)]
    pub token_ttl_secs: u64,

    /// Run migrations, then exit with code 0. Used by
    /// `cargo xtask backend-db-reset`. When set, no listener is bound and no
    /// background tasks are spawned.
    #[arg(long)]
    pub exit_after_migrate: bool,

    /// HS256 secret used to sign account-auth bearer tokens.
    ///
    /// Required at boot in the binary. Optional at the CLI level so the
    /// crate's own unit tests / `BackendState::new()` can assemble a
    /// state without forcing every test to set the env var.
    /// `validate()` enforces presence + ≥32-byte length when invoked from
    /// `main.rs`.
    #[arg(long, env = "MINOS_JWT_SECRET")]
    pub jwt_secret: Option<String>,

    /// Comma-separated list of allowed CORS origins. When empty or set to
    /// `"*"`, all origins are permitted (dev mode). In production, set to
    /// the frontend URL(s) e.g. `"https://app.minos.dev,https://minos.dev"`.
    #[arg(long, env = "MINOS_CORS_ORIGINS", default_value = "*")]
    pub cors_origins: String,

    /// Deployment environment. Production enables stricter config validation
    /// such as rejecting wildcard CORS.
    #[arg(long, env = "MINOS_ENV", value_enum, default_value = "dev")]
    pub environment: Environment,

    /// Runtime shell topology.
    ///
    /// `monolith` runs the HTTP surface plus supervised background workers in
    /// one process. `http-only` disables worker-plane pollers so API/gateway
    /// instances can run without duplicate timeout GC. `worker-only` skips the
    /// listener and only runs the supervised worker plane.
    #[arg(
        long,
        env = "MINOS_RUNTIME_MODE",
        value_enum,
        default_value = "monolith"
    )]
    pub runtime_mode: RuntimeMode,

    /// Peer-target cache backend. `in-memory` is fine for local dev; use
    /// `redis` for multi-instance deployments so cache invalidation is not
    /// process-local only.
    #[arg(
        long,
        env = "MINOS_CACHE_BACKEND",
        value_enum,
        default_value = "in-memory"
    )]
    pub cache_backend: CacheBackendKind,

    /// Cross-instance event bus backend for realtime fan-out.
    #[arg(
        long,
        env = "MINOS_MESSAGE_BUS_BACKEND",
        value_enum,
        default_value = "inline"
    )]
    pub message_bus_backend: MessageBusBackendKind,

    /// Redis endpoint shared by the cache and message-bus adapters when
    /// either backend is configured as `redis`.
    #[arg(long, env = "MINOS_REDIS_URL")]
    pub redis_url: Option<String>,

    /// Redis pub/sub channel name used for cluster fan-out.
    #[arg(
        long,
        env = "MINOS_CLUSTER_CHANNEL",
        default_value = DEFAULT_CLUSTER_CHANNEL
    )]
    pub cluster_channel: String,

    /// Graceful shutdown timeout in seconds. After receiving SIGTERM/SIGINT,
    /// the server waits up to this duration for in-flight requests to complete
    /// before forcibly terminating.
    #[arg(long, env = "MINOS_SHUTDOWN_TIMEOUT_SECS", default_value_t = 30)]
    pub shutdown_timeout_secs: u64,

    /// Supabase project URL (`https://<ref>.supabase.co`). When set, enables
    /// `POST /v1/auth/supabase` token exchange. Optional in dev so local
    /// password-only flows still boot without an IdP.
    #[arg(long, env = "SUPABASE_URL")]
    pub supabase_url: Option<String>,

    /// Expected JWT `aud` for Supabase access tokens. Defaults to
    /// `"authenticated"` when unset (Supabase default for user sessions).
    #[arg(long, env = "SUPABASE_JWT_AUD")]
    pub supabase_jwt_aud: Option<String>,

    /// Optional legacy HS256 JWT secret (Supabase Dashboard → Settings → API
    /// → JWT Secret). Needed when access tokens are still signed with the
    /// shared secret rather than the ES256 JWKS key.
    #[arg(long, env = "SUPABASE_JWT_SECRET")]
    pub supabase_jwt_secret: Option<String>,
}

impl Config {
    /// Pairing-token TTL as a [`Duration`]. Wraps
    /// [`Config::token_ttl_secs`] so callers don't repeat the
    /// `Duration::from_secs` boilerplate.
    #[must_use]
    pub fn token_ttl(&self) -> Duration {
        Duration::from_secs(self.token_ttl_secs)
    }

    /// Log directory with the platform default applied when `--log-dir` /
    /// `MINOS_BACKEND_LOG_DIR` was not provided. See [`default_log_dir`].
    #[must_use]
    pub fn resolved_log_dir(&self) -> PathBuf {
        self.log_dir.clone().unwrap_or_else(default_log_dir)
    }

    #[must_use]
    pub fn resolved_database_url(&self) -> String {
        match self.storage_mode {
            StorageMode::Sqlite => self
                .database_url
                .clone()
                .filter(|url| url.starts_with("sqlite:"))
                .unwrap_or_else(|| format!("sqlite://{}?mode=rwc", self.db.display())),
            StorageMode::ExternalSql => self.database_url.clone().unwrap_or_default(),
        }
    }

    /// Validate startup configuration. CF Access service tokens and the
    /// public backend URL are now exclusively client-side build config —
    /// they no longer enter QR payloads or backend state — so this only
    /// enforces the JWT-secret invariants.
    ///
    /// # Errors
    /// Returns a human-readable message suitable for surfacing from main
    /// (`eprintln!` + non-zero exit). Callers shouldn't try to interpret
    /// the string programmatically.
    pub fn validate(&self) -> Result<(), String> {
        let secret = self
            .jwt_secret
            .as_ref()
            .ok_or_else(|| "MINOS_JWT_SECRET is required".to_string())?;
        if secret.len() < 32 {
            return Err("MINOS_JWT_SECRET must be >=32 bytes".into());
        }
        if secret.len() > 1024 {
            return Err("MINOS_JWT_SECRET must be <=1024 bytes (unusually long; check for accidental newlines)".into());
        }
        if self.storage_mode == StorageMode::ExternalSql {
            if !crate::store::postgres_backend_enabled() {
                return Err(
                    "MINOS_STORAGE_MODE=external-sql requires the backend-postgres cargo feature"
                        .into(),
                );
            }
            let url = self
                .database_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "MINOS_DATABASE_URL is required when MINOS_STORAGE_MODE=external-sql"
                        .to_string()
                })?;
            if url.starts_with("sqlite:") {
                return Err(
                    "MINOS_STORAGE_MODE=external-sql cannot use a sqlite:// database URL".into(),
                );
            }
            if crate::store::detect_external_sql_driver(url).is_none() {
                return Err(
                    "MINOS_STORAGE_MODE=external-sql currently supports only postgres:// or postgresql:// MINOS_DATABASE_URL values"
                        .into(),
                );
            }
        } else {
            if !crate::store::sqlite_backend_enabled() {
                return Err(
                    "MINOS_STORAGE_MODE=sqlite requires the backend-sqlite cargo feature".into(),
                );
            }
            let database_url = self.resolved_database_url();
            if !database_url.starts_with("sqlite:") {
                return Err(
                    "MINOS_STORAGE_MODE=sqlite requires a sqlite:// database URL or --db path"
                        .into(),
                );
            }
        }
        if self.environment == Environment::Prod {
            if self.runtime_mode.serves_http() {
                let cors = self.cors_origins.trim();
                if cors.is_empty() || cors == "*" {
                    return Err("MINOS_CORS_ORIGINS must not be wildcard in prod".into());
                }
                if self.cache_backend != CacheBackendKind::Redis {
                    return Err("MINOS_CACHE_BACKEND must be redis in prod".into());
                }
                if self.message_bus_backend != MessageBusBackendKind::Redis {
                    return Err("MINOS_MESSAGE_BUS_BACKEND must be redis in prod".into());
                }
            }
            if self.storage_mode == StorageMode::Sqlite {
                return Err(
                    "embedded SQLite is not supported in prod yet; set MINOS_STORAGE_MODE=external-sql with a Postgres-class MINOS_DATABASE_URL before setting MINOS_ENV=prod"
                        .into(),
                );
            }
        }
        if self.cache_backend == CacheBackendKind::Redis
            || self.message_bus_backend == MessageBusBackendKind::Redis
        {
            if self
                .redis_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(
                    "MINOS_REDIS_URL is required when using redis cache/message bus".into(),
                );
            }
        }
        if let Some(url) = self
            .supabase_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            crate::auth::supabase::SupabaseConfig::from_url(
                url,
                self.supabase_jwt_aud.as_deref(),
                self.supabase_jwt_secret.as_deref(),
            )
            .map_err(|e| format!("SUPABASE_URL invalid: {e}"))?;
        }
        Ok(())
    }

    /// Build a Supabase token verifier when `SUPABASE_URL` is configured.
    #[must_use]
    pub fn supabase_verifier(
        &self,
    ) -> Option<std::sync::Arc<crate::auth::supabase::SupabaseTokenVerifier>> {
        let url = self
            .supabase_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let cfg = crate::auth::supabase::SupabaseConfig::from_url(
            url,
            self.supabase_jwt_aud.as_deref(),
            self.supabase_jwt_secret.as_deref(),
        )
        .ok()?;
        Some(crate::auth::supabase::SupabaseTokenVerifier::from_config(
            cfg,
        ))
    }
}

/// Platform-specific fallback for the xlog directory.
///
/// On macOS the canonical location is `~/Library/Logs/Minos/`.
/// On non-Apple targets we fall back to `$TMPDIR/minos` (or `/tmp/minos`
/// when `$TMPDIR` is absent) — CI runners, containers, and developer
/// sandboxes usually honour `TMPDIR` via `tempfile::tempdir`, so this keeps
/// test runs self-cleaning.
fn default_log_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Logs/Minos");
        }
    }
    let base = std::env::var_os("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    base.join("minos")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // `clap::Parser::try_parse_from` drives argv deterministically so tests
    // don't depend on the process's real CLI state. But clap *also* reads
    // env vars at parse time (via `env = "..."` attrs), and Rust runs tests
    // concurrently by default — so every test here must hold `ENV_LOCK`
    // and begin with `clear_env()`. Without that, a sibling test's
    // `set_var` leaks across sessions and flakes the defaults assertions.
    //
    // The first element of `try_parse_from`'s iterator is the binary name;
    // subsequent elements are flags.

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for key in [
            "MINOS_BACKEND_LISTEN",
            "MINOS_BACKEND_DB",
            "MINOS_DATABASE_URL",
            "MINOS_BACKEND_DB_MAX_CONNECTIONS",
            "MINOS_BACKEND_LOG_DIR",
            "MINOS_BACKEND_TOKEN_TTL",
            "MINOS_JWT_SECRET",
            "MINOS_ENV",
            "MINOS_STORAGE_MODE",
            "MINOS_RUNTIME_MODE",
            "MINOS_CACHE_BACKEND",
            "MINOS_MESSAGE_BUS_BACKEND",
            "MINOS_REDIS_URL",
            "MINOS_CLUSTER_CHANNEL",
            "RUST_LOG",
        ] {
            std::env::remove_var(key);
        }
    }

    /// Acquire the shared env lock and reset the five env vars clap reads.
    /// Returns a guard that must be held for the remainder of the test.
    fn env_scope() -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        guard
    }

    #[test]
    fn default_flags_match_defaults() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(
            cfg.listen,
            "127.0.0.1:8787".parse::<SocketAddr>().unwrap(),
            "default --listen must match"
        );
        assert_eq!(cfg.db, PathBuf::from("./minos-backend.db"));
        assert_eq!(cfg.storage_mode, StorageMode::Sqlite);
        assert_eq!(cfg.db_max_connections, 32);
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.token_ttl_secs, DEFAULT_TOKEN_TTL_SECS);
        assert!(!cfg.exit_after_migrate);
        assert!(cfg.log_dir.is_none());
        assert_eq!(cfg.environment, Environment::Dev);
        assert_eq!(cfg.runtime_mode, RuntimeMode::Monolith);
    }

    #[test]
    fn runtime_mode_helpers_reflect_worker_and_http_toggles() {
        assert!(RuntimeMode::Monolith.serves_http());
        assert!(RuntimeMode::Monolith.runs_supervised_workers());
        assert!(RuntimeMode::HttpOnly.serves_http());
        assert!(!RuntimeMode::HttpOnly.runs_supervised_workers());
        assert!(!RuntimeMode::WorkerOnly.serves_http());
        assert!(RuntimeMode::WorkerOnly.runs_supervised_workers());
    }

    #[test]
    fn token_ttl_wraps_seconds_into_duration() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        // Default: 300 seconds. `from_mins(5)` is the same
        // Duration; clippy prefers the larger-unit form.
        assert_eq!(cfg.token_ttl(), Duration::from_mins(5));

        let cfg = Config::try_parse_from(["minos-backend", "--token-ttl-secs", "42"]).unwrap();
        assert_eq!(cfg.token_ttl(), Duration::from_secs(42));
    }

    #[test]
    fn listen_flag_overrides_default() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend", "--listen", "0.0.0.0:9999"]).unwrap();
        assert_eq!(cfg.listen, "0.0.0.0:9999".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn db_and_log_flags_override_defaults() {
        let _g = env_scope();

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--db",
            "/tmp/test.db",
            "--db-max-connections",
            "64",
            "--log-dir",
            "/tmp/logs",
            "--log-level",
            "debug",
        ])
        .unwrap();
        assert_eq!(cfg.db, PathBuf::from("/tmp/test.db"));
        assert_eq!(cfg.db_max_connections, 64);
        assert_eq!(cfg.log_dir, Some(PathBuf::from("/tmp/logs")));
        assert_eq!(cfg.log_level, "debug");
    }

    #[test]
    fn exit_after_migrate_flag_flips_boolean() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend", "--exit-after-migrate"]).unwrap();
        assert!(cfg.exit_after_migrate);
    }

    #[test]
    fn resolved_log_dir_uses_provided_path_when_set() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend", "--log-dir", "/tmp/explicit"]).unwrap();
        assert_eq!(cfg.resolved_log_dir(), PathBuf::from("/tmp/explicit"));
    }

    #[test]
    fn resolved_log_dir_falls_back_to_platform_default() {
        let _g = env_scope();

        // No --log-dir provided: default_log_dir() is invoked. The result
        // is platform-dependent — rather than pin the exact path (and
        // depend on HOME/TMPDIR shape), assert the "Minos"/"minos"
        // convention.
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        let dir = cfg.resolved_log_dir();
        let tail = dir
            .file_name()
            .expect("log dir must have a last component")
            .to_string_lossy()
            .into_owned();
        if cfg!(target_os = "macos") {
            assert_eq!(tail, "Minos");
        } else {
            assert_eq!(tail, "minos");
        }
    }

    // ── env-var wiring ────────────────────────────────────────────────

    #[test]
    fn env_var_overrides_listen_default() {
        let _g = env_scope();
        std::env::set_var("MINOS_BACKEND_LISTEN", "127.0.0.1:4242");

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(cfg.listen, "127.0.0.1:4242".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn env_var_overrides_token_ttl_default() {
        let _g = env_scope();
        std::env::set_var("MINOS_BACKEND_TOKEN_TTL", "600");

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(cfg.token_ttl_secs, 600);
        assert_eq!(cfg.token_ttl(), Duration::from_mins(10));
    }

    #[test]
    fn env_var_overrides_db_max_connections_default() {
        let _g = env_scope();
        std::env::set_var("MINOS_BACKEND_DB_MAX_CONNECTIONS", "48");

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(cfg.db_max_connections, 48);
    }

    // ── JWT-secret validation ─────────────────────────────────────────

    /// Deterministic 32-byte secret for tests that exercise `validate`.
    const TEST_JWT_SECRET: &str = "01234567890123456789012345678901";

    #[test]
    fn validate_ok_with_jwt_secret_set() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        cfg.validate().expect("jwt secret present and long enough");
    }

    #[test]
    fn validate_requires_jwt_secret_to_be_set() {
        let _g = env_scope();
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        let err = cfg
            .validate()
            .expect_err("missing MINOS_JWT_SECRET must fail");
        assert!(err.contains("MINOS_JWT_SECRET"), "{err}");
    }

    #[test]
    fn validate_rejects_short_jwt_secret() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", "tiny");
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        let err = cfg
            .validate()
            .expect_err("short MINOS_JWT_SECRET must fail");
        assert!(err.contains(">=32"), "{err}");
    }

    #[test]
    fn validate_rejects_wildcard_cors_in_prod() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from(["minos-backend", "--environment", "prod"]).unwrap();
        let err = cfg.validate().expect_err("wildcard CORS must fail in prod");
        assert!(err.contains("MINOS_CORS_ORIGINS"), "{err}");
    }

    #[test]
    fn validate_rejects_embedded_sqlite_even_with_explicit_cors_in_prod() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--environment",
            "prod",
            "--cors-origins",
            "https://app.minos.dev",
            "--cache-backend",
            "redis",
            "--message-bus-backend",
            "redis",
            "--redis-url",
            "redis://127.0.0.1:6379/",
        ])
        .unwrap();

        let err = cfg
            .validate()
            .expect_err("embedded sqlite must still fail in prod");
        assert!(err.contains("embedded SQLite"), "{err}");
    }

    #[test]
    fn validate_requires_database_url_when_external_sql_mode_is_selected() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg =
            Config::try_parse_from(["minos-backend", "--storage-mode", "external-sql"]).unwrap();

        let err = cfg
            .validate()
            .expect_err("external sql mode without url must fail");
        assert!(err.contains("MINOS_DATABASE_URL"), "{err}");
    }

    #[test]
    fn validate_rejects_sqlite_url_in_external_sql_mode() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--storage-mode",
            "external-sql",
            "--database-url",
            "sqlite://./wrong.db?mode=rwc",
        ])
        .unwrap();

        let err = cfg
            .validate()
            .expect_err("external sql mode must reject sqlite urls");
        assert!(err.contains("cannot use a sqlite://"), "{err}");
    }

    #[test]
    fn validate_allows_postgres_url_in_external_sql_mode() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--storage-mode",
            "external-sql",
            "--database-url",
            "postgres://minos:secret@localhost/minos",
        ])
        .unwrap();

        cfg.validate()
            .expect("postgres urls should pass config validation");
    }

    #[test]
    fn validate_rejects_unsupported_external_sql_driver_urls() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--storage-mode",
            "external-sql",
            "--database-url",
            "mysql://minos:secret@localhost/minos",
        ])
        .unwrap();

        let err = cfg
            .validate()
            .expect_err("unsupported external sql drivers must fail validation");
        assert!(err.contains("postgres://"), "{err}");
    }

    #[test]
    fn validate_requires_redis_url_when_redis_runtime_is_selected() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from([
            "minos-backend",
            "--cache-backend",
            "redis",
            "--message-bus-backend",
            "redis",
        ])
        .unwrap();

        let err = cfg
            .validate()
            .expect_err("redis runtime without redis url must fail");
        assert!(err.contains("MINOS_REDIS_URL"), "{err}");
    }
}
