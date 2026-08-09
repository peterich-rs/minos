//! `minos-backend` binary entrypoint.
//!
//! Wires the library modules (`config`, `store`, `pairing`, `session`,
//! `http`) into a running axum server. Plan §10 acceptance:
//!
//! ```sh
//! cargo run -p minos-backend -- --listen 127.0.0.1:8787 --db ./tmp.db
//! ```
//!
//! The binary logs `migrations applied` and `listening` on boot, answers
//! `GET /health/ready` with 200, and tears down cleanly on SIGINT/SIGTERM.
//!
//! ## Tracing
//!
//! Initialised via [`init_tracing`]. mars-xlog writes binary `.xlog` files
//! under `--log-dir`; a fmt layer also sends human-readable records to
//! stdout for dev ergonomics. The `RUST_LOG` env var (or `--log-level`) is
//! parsed with [`tracing_subscriber::EnvFilter`].
//!
//! ## Graceful shutdown
//!
//! Two-phase teardown (see commit history for the shutdown-ordering fix).
//! Phase 1 is the `with_graceful_shutdown` future:
//! [`wait_for_signal`] awaits either `SIGINT` (Ctrl-C) or `SIGTERM`, then
//! we broadcast `Event::ServerShutdown` to every live session and sleep
//! 500ms so clients can drain. Only after `axum::serve` returns — which
//! signals both that the listener has stopped accepting new connections
//! AND that in-flight handlers have finished — do we abort the token GC
//! task and close the backing SQL pool. Closing the store earlier would race
//! handlers still issuing queries and surface `PoolClosed` errors.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig};
use minos_backend::{
    config::{Config, StorageMode},
    http,
    runtime::RuntimeShell,
    store,
};
use minos_protocol::{Envelope, EventKind};
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Default drain window after broadcasting `ServerShutdown` (plan §10 step 8).
const SHUTDOWN_DRAIN: Duration = Duration::from_millis(500);

/// Default shutdown timeout if `MINOS_SHUTDOWN_TIMEOUT_SECS` is not set.
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 30;

/// xlog file prefix. Spec §9.4 reserves `backend` for the server process.
const XLOG_NAME_PREFIX: &str = "backend";

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::parse();

    // Fail fast on invalid CF Access configuration rather than handing out
    // pairing QRs that will be rejected at the CF edge. See spec §13.3.
    if let Err(msg) = cfg.validate() {
        eprintln!("minos-backend: configuration error: {msg}");
        std::process::exit(2);
    }

    init_tracing(&cfg).context("init tracing")?;

    let store = match cfg.storage_mode {
        StorageMode::Sqlite => {
            let db_url = cfg.resolved_database_url();
            tracing::info!(db_url = %db_url, "connecting to sqlite");
            let pool = store::connect_sqlite_with_options(&db_url, cfg.db_max_connections)
                .await
                .with_context(|| format!("store::connect {db_url}"))?;
            tracing::info!(
                runtime_mode = %cfg.runtime_mode.as_str(),
                storage_mode = %cfg.storage_mode.as_str(),
                "migrations applied"
            );

            if cfg.exit_after_migrate {
                tracing::info!("--exit-after-migrate set; exiting after migrations");
                pool.close().await;
                return Ok(());
            }

            pool.into()
        }
        StorageMode::ExternalSql => {
            let db_url = cfg.resolved_database_url();
            tracing::info!(
                max_connections = cfg.db_max_connections,
                "preflighting external postgres store"
            );
            let (store, preflight) =
                store::connect_external_sql_with_options(&db_url, cfg.db_max_connections)
                    .await
                    .context("external_sql::connect")?;
            tracing::info!(
                driver = %preflight.driver,
                database = %preflight.database,
                server_version = %preflight.server_version,
                runtime_mode = %cfg.runtime_mode.as_str(),
                storage_mode = %cfg.storage_mode.as_str(),
                "external SQL preflight complete"
            );

            if cfg.exit_after_migrate {
                tracing::info!("--exit-after-migrate set; exiting after external SQL preflight");
                store.close().await;
                return Ok(());
            }

            tracing::warn!(
                driver = %preflight.driver,
                database = %preflight.database,
                "external SQL runtime is live; any remaining SQLite-only behavior must now be enforced at the specific handler or store boundary"
            );
            store
        }
    };

    // `cfg.validate()` already enforced presence + length above. Unwrap is
    // load-bearing here: a missing secret should never reach BackendState
    // construction, and panicking surfaces the bug loudly in dev runs that
    // somehow skipped validation.
    let jwt_secret = cfg
        .jwt_secret
        .clone()
        .expect("MINOS_JWT_SECRET must be set after Config::validate");
    let shell = RuntimeShell::from_config(
        &cfg,
        store,
        jwt_secret,
        http::parse_cors_origins(&cfg.cors_origins),
    )
    .context("compose runtime shell")?;
    shell
        .hydrate_durable_state()
        .await
        .context("hydrate durable completion watches")?;
    let instance_id = shell.app.instance_id.clone();

    if cfg.runtime_mode.serves_http() {
        let listener = tokio::net::TcpListener::bind(cfg.listen)
            .await
            .with_context(|| format!("bind {}", cfg.listen))?;
        let local_addr = listener.local_addr().context("local_addr")?;
        tracing::info!(
            addr = %local_addr,
            version = %env!("CARGO_PKG_VERSION"),
            instance_id = %instance_id,
            runtime_mode = %cfg.runtime_mode.as_str(),
            storage_mode = %cfg.storage_mode.as_str(),
            "listening"
        );

        let router = http::router(shell.backend_state());

        // Phase 1 of teardown runs inside `with_graceful_shutdown`: await a
        // signal, broadcast `ServerShutdown`, and sleep the drain window.
        // Axum only stops the listener + waits for in-flight handlers AFTER
        // this future resolves, so everything that must happen while handlers
        // are still live (broadcast + drain) belongs here.
        let registry_for_shutdown = Arc::clone(&shell.app.registry);
        let shutdown_timeout = Duration::from_secs(
            std::env::var("MINOS_SHUTDOWN_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT_SECS),
        );
        tracing::info!(
            shutdown_timeout_secs = shutdown_timeout.as_secs(),
            "graceful shutdown configured"
        );
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                wait_for_signal().await;
                tracing::info!("broadcasting ServerShutdown to all sessions");
                registry_for_shutdown.broadcast(Envelope::Event {
                    version: 1,
                    event: EventKind::ServerShutdown,
                });
                // Use the configured shutdown timeout for the drain window,
                // capped at the configured value.
                let drain = SHUTDOWN_DRAIN.min(shutdown_timeout);
                tokio::time::sleep(drain).await;
            })
            .await
            .context("axum::serve")?;
    } else {
        tracing::info!(
            instance_id = %instance_id,
            runtime_mode = %cfg.runtime_mode.as_str(),
            storage_mode = %cfg.storage_mode.as_str(),
            "runtime shell running without HTTP listener"
        );
        wait_for_signal().await;
    }

    // Phase 2: listener has stopped and handlers have drained, so DB
    // resources can go away without racing a query.
    tracing::info!("listener stopped; tearing down runtime store");
    shell.shutdown().await;

    tracing::info!("server exited cleanly");
    // Flush mars-xlog before returning so the teardown info! lines are
    // guaranteed on disk even on fast SIGTERM. `flush_all(true)` is
    // synchronous (see `crates/minos-daemon/src/logging.rs`).
    Xlog::flush_all(true);
    Ok(())
}

/// Install the mars-xlog layer + an `EnvFilter`-gated fmt layer as the
/// global tracing subscriber.
///
/// Mirrors the daemon crate's `logging::init` wiring (spec §9.4). The xlog
/// layer writes `backend_YYYYMMDD.xlog` under `--log-dir`; the fmt layer
/// emits human-readable records to stdout for developer ergonomics.
fn init_tracing(cfg: &Config) -> Result<()> {
    let log_dir = cfg.resolved_log_dir();
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("create log_dir {}", log_dir.display()))?;

    let xlog_cfg = XlogConfig::new(log_dir.to_string_lossy().to_string(), XLOG_NAME_PREFIX);
    // Map the CLI-facing level string onto the mars-xlog enum so
    // `--log-level debug` actually lowers the xlog gate (not just the
    // stdout fmt layer). Full `env_logger`-style directives are supported
    // by taking the first target-less level keyword we find.
    let xlog_level = xlog_level_from_str(&cfg.log_level);
    let logger = Xlog::init(xlog_cfg, xlog_level).context("Xlog::init (mars-xlog)")?;
    let (xlog_layer, _handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(xlog_level).enabled(true));

    // `RUST_LOG` (or --log-level) may carry full directives like
    // "minos_backend=debug,info"; fall back to "info" if parsing fails so a
    // typo'd level never crashes the process.
    let filter = EnvFilter::try_new(&cfg.log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(xlog_layer)
        .with(tracing_subscriber::fmt::layer())
        .try_init()
        .context("install global tracing subscriber")?;

    tracing::info!(
        name_prefix = XLOG_NAME_PREFIX,
        dir = %log_dir.display(),
        "backend logging initialized"
    );
    Ok(())
}

/// Await a shutdown signal (`SIGINT` everywhere, `SIGTERM` on Unix) and
/// return once one has arrived. Side effects are limited to a single
/// `info!` naming which signal fired.
///
/// Kept small so it can be the only thing the `with_graceful_shutdown`
/// future does before broadcasting + draining; teardown that must run
/// AFTER the listener stops (GC abort, pool close) lives in `main`.
async fn wait_for_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("SIGINT received; shutting down"),
        () = terminate => tracing::info!("SIGTERM received; shutting down"),
    }
}

/// Parse `Config::log_level` into a [`mars_xlog::LogLevel`].
///
/// Accepts plain keywords (`trace`/`debug`/`info`/`warn`/`error`) and
/// `env_logger`-style directives like `minos_backend=debug,info`: we take
/// the first comma-segment, strip any `target=` prefix, and match the
/// keyword case-insensitively. Unknown/unmappable input falls back to
/// `Info` with a `debug!` trace so typos don't change the gate silently.
///
/// mars-xlog has no `Trace` variant; `trace` maps to its most verbose
/// level, `Verbose`.
fn xlog_level_from_str(s: &str) -> LogLevel {
    let primary = s
        .split(',')
        .next()
        .unwrap_or(s)
        .split('=')
        .next_back()
        .unwrap_or("info")
        .trim();
    match primary.to_ascii_lowercase().as_str() {
        "trace" => LogLevel::Verbose,
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        other => {
            tracing::debug!(
                input = s,
                parsed = other,
                "xlog level parse fell back to Info"
            );
            LogLevel::Info
        }
    }
}
