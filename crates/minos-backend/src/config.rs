//! CLI + env configuration for the `minos-backend` binary.
//!
//! Design: `clap` 4 derive + `env = "..."` attributes so every flag has a
//! paired environment-variable override. Defaults are codified as
//! `default_value`/`default_value_t` literals so `--help` prints the exact
//! values the plan (§10) mandates.
//!
//! The log directory default is platform-dependent (see [`default_log_dir`])
//! and therefore resolved at runtime rather than being a clap literal — the
//! `Option<PathBuf>` field plus the [`Config::resolved_log_dir`] helper
//! captures that without confusing `--help`.
//!
//! # Exit-after-migrate
//!
//! `--exit-after-migrate` is a boot-time flag used by
//! `cargo xtask backend-db-reset` (plan §11). When set, `main.rs` applies
//! migrations and exits with code 0 without binding the axum listener or
//! spawning the GC task. The plan's §10 "steps 1–8" body only runs when
//! this flag is absent.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

/// Default pairing-token TTL (5 minutes) per plan §10.
const DEFAULT_TOKEN_TTL_SECS: u64 = 300;

/// Minos backend: axum WebSocket hub with SQLite state.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Config {
    /// TCP socket to listen on.
    #[arg(long, env = "MINOS_BACKEND_LISTEN", default_value = "127.0.0.1:8787")]
    pub listen: SocketAddr,

    /// SQLite database path. Created on first run via sqlx
    /// `create_if_missing(true)`.
    #[arg(long, env = "MINOS_BACKEND_DB", default_value = "./minos-backend.db")]
    pub db: PathBuf,

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

    /// Optional Cloudflare Access `CF-Access-Client-Id` for QR distribution.
    ///
    /// Current clients normally receive Access credentials from build-time or
    /// host env configuration. If these backend env vars are set, the backend
    /// still embeds them into pairing QR payloads for backwards-compatible
    /// deployments.
    #[arg(long, env = "MINOS_BACKEND_CF_ACCESS_CLIENT_ID")]
    pub cf_access_client_id: Option<String>,

    /// Cloudflare Access `CF-Access-Client-Secret`. Paired with
    /// `cf_access_client_id`; both must be set or both empty.
    #[arg(long, env = "MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET")]
    pub cf_access_client_secret: Option<String>,

    /// Public WebSocket origin the mobile client should connect to.
    /// Embedded into the pairing QR payload so mobile doesn't need to
    /// resolve DNS for the backend. Defaults to the local dev address;
    /// production deploys override via `MINOS_BACKEND_PUBLIC_URL`.
    #[arg(
        long,
        env = "MINOS_BACKEND_PUBLIC_URL",
        default_value = "ws://127.0.0.1:8787/devices"
    )]
    pub public_url: String,

    /// Legacy escape hatch retained for CLI compatibility. Access service
    /// tokens are now client-side configuration, so `wss://` public URLs no
    /// longer require backend-held CF token env vars.
    #[arg(long, env = "MINOS_BACKEND_ALLOW_DEV", default_value_t = false)]
    pub allow_dev: bool,

    /// HS256 secret used to sign account-auth bearer tokens (spec §5.3).
    ///
    /// Required at boot in the binary. Optional at the CLI level so the
    /// crate's own unit tests / `BackendState::new()` can assemble a
    /// state without forcing every test to set the env var.
    /// `validate()` enforces presence + ≥32-byte length when invoked from
    /// `main.rs`.
    #[arg(long, env = "MINOS_JWT_SECRET")]
    pub jwt_secret: Option<String>,
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

    /// Validate optional CF Access QR-distribution configuration at startup.
    ///
    /// Access service tokens are consumed by clients, not the backend. The
    /// backend may optionally carry a token pair only to embed it into QR
    /// payloads for older clients. If one half is set, the other must be set.
    ///
    /// # Errors
    /// Returns a human-readable message suitable for surfacing from main
    /// (`eprintln!` + non-zero exit). Callers shouldn't try to interpret
    /// the string programmatically.
    pub fn validate(&self) -> Result<(), String> {
        let id_set = self.cf_access_client_id.is_some();
        let secret_set = self.cf_access_client_secret.is_some();
        if id_set != secret_set {
            return Err(
                "MINOS_BACKEND_CF_ACCESS_CLIENT_ID and ..._SECRET must be set together or both left unset"
                    .into(),
            );
        }
        let secret = self
            .jwt_secret
            .as_ref()
            .ok_or_else(|| "MINOS_JWT_SECRET is required".to_string())?;
        if secret.as_bytes().len() < 32 {
            return Err("MINOS_JWT_SECRET must be >=32 bytes".into());
        }
        Ok(())
    }
}

/// Platform-specific fallback for the xlog directory.
///
/// On macOS the canonical location is `~/Library/Logs/Minos/` (spec §9.4).
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
    // `set_var` leaks across threads and flakes the defaults assertions.
    //
    // The first element of `try_parse_from`'s iterator is the binary name;
    // subsequent elements are flags.

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for key in [
            "MINOS_BACKEND_LISTEN",
            "MINOS_BACKEND_DB",
            "MINOS_BACKEND_LOG_DIR",
            "MINOS_BACKEND_TOKEN_TTL",
            "MINOS_BACKEND_CF_ACCESS_CLIENT_ID",
            "MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET",
            "MINOS_BACKEND_PUBLIC_URL",
            "MINOS_BACKEND_ALLOW_DEV",
            "MINOS_JWT_SECRET",
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
    fn default_flags_match_plan_defaults() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(
            cfg.listen,
            "127.0.0.1:8787".parse::<SocketAddr>().unwrap(),
            "default --listen must match plan §10"
        );
        assert_eq!(cfg.db, PathBuf::from("./minos-backend.db"));
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.token_ttl_secs, DEFAULT_TOKEN_TTL_SECS);
        assert!(!cfg.exit_after_migrate);
        assert!(cfg.log_dir.is_none());
    }

    #[test]
    fn token_ttl_wraps_seconds_into_duration() {
        let _g = env_scope();

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        // Plan §10 default: 300 seconds. `from_mins(5)` is the same
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
            "--log-dir",
            "/tmp/logs",
            "--log-level",
            "debug",
        ])
        .unwrap();
        assert_eq!(cfg.db, PathBuf::from("/tmp/test.db"));
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

    // ── CF Access validation ──────────────────────────────────────────

    #[test]
    fn default_public_url_is_local_ws() {
        let _g = env_scope();
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        assert_eq!(cfg.public_url, "ws://127.0.0.1:8787/devices");
        assert!(cfg.cf_access_client_id.is_none());
        assert!(cfg.cf_access_client_secret.is_none());
        assert!(!cfg.allow_dev);
    }

    /// Deterministic 32-byte secret for tests that exercise `validate`.
    const TEST_JWT_SECRET: &str = "01234567890123456789012345678901";

    #[test]
    fn validate_ok_for_local_ws_without_cf_tokens() {
        let _g = env_scope();
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);
        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        cfg.validate()
            .expect("local ws:// must not require CF tokens");
    }

    #[test]
    fn validate_ok_for_wss_without_backend_held_cf_tokens() {
        let _g = env_scope();
        std::env::set_var(
            "MINOS_BACKEND_PUBLIC_URL",
            "wss://tunnel.example.com/devices",
        );
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        cfg.validate()
            .expect("clients carry CF tokens; backend does not require them");
    }

    #[test]
    fn validate_allow_dev_keeps_wss_without_cf_tokens_valid() {
        let _g = env_scope();
        std::env::set_var(
            "MINOS_BACKEND_PUBLIC_URL",
            "wss://tunnel.example.com/devices",
        );
        std::env::set_var("MINOS_BACKEND_ALLOW_DEV", "true");
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        cfg.validate()
            .expect("allow-dev remains a no-op compatibility flag");
    }

    #[test]
    fn validate_requires_both_cf_tokens_together() {
        let _g = env_scope();
        std::env::set_var("MINOS_BACKEND_CF_ACCESS_CLIENT_ID", "id-only");
        std::env::set_var("MINOS_JWT_SECRET", TEST_JWT_SECRET);

        let cfg = Config::try_parse_from(["minos-backend"]).unwrap();
        let err = cfg.validate().expect_err("half-set pair must fail");
        assert!(err.contains("must be set together"), "{err}");
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
}
