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
//! `GET /health` with 200, and tears down cleanly on SIGINT/SIGTERM.
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
//! Two-phase teardown (see commit history for `fix(relay): shutdown
//! ordering...`). Phase 1 is the `with_graceful_shutdown` future:
//! [`wait_for_signal`] awaits either `SIGINT` (Ctrl-C) or `SIGTERM`, then
//! we broadcast `Event::ServerShutdown` to every live session and sleep
//! 500ms so clients can drain. Only after `axum::serve` returns — which
//! signals both that the listener has stopped accepting new connections
//! AND that in-flight handlers have finished — do we abort the token GC
//! task and close the SQLite pool. Closing the pool earlier would race
//! WS handlers still issuing queries and surface `PoolClosed` errors.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig};
use minos_backend::{
    config::Config,
    http::{self, RelayState},
    pairing::PairingService,
    session::SessionRegistry,
    store,
};
use minos_protocol::{Envelope, EventKind};
use sqlx::SqlitePool;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Token GC cadence (plan §10 step 6).
const TOKEN_GC_INTERVAL: Duration = Duration::from_mins(1);

/// Drain window after broadcasting `ServerShutdown` (plan §10 step 8).
const SHUTDOWN_DRAIN: Duration = Duration::from_millis(500);

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

    let db_url = format!("sqlite://{}?mode=rwc", cfg.db.display());
    tracing::info!(db_url = %db_url, "connecting to sqlite");
    let pool = store::connect(&db_url)
        .await
        .with_context(|| format!("store::connect {}", cfg.db.display()))?;
    tracing::info!("migrations applied");

    if cfg.exit_after_migrate {
        tracing::info!("--exit-after-migrate set; exiting after migrations");
        pool.close().await;
        return Ok(());
    }

    let registry = Arc::new(SessionRegistry::new());
    let pairing = Arc::new(PairingService::new(pool.clone()));
    let mut state = RelayState::new(
        registry.clone(),
        pairing.clone(),
        pool.clone(),
        cfg.token_ttl(),
    );
    // Override the default public-cfg with env-sourced values from cfg.
    state.public_cfg = Arc::new(crate::http::BackendPublicConfig {
        public_url: cfg.public_url.clone(),
        cf_access_client_id: cfg.cf_access_client_id.clone(),
        cf_access_client_secret: cfg.cf_access_client_secret.clone(),
    });

    let gc_task = spawn_token_gc(pool.clone());

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("bind {}", cfg.listen))?;
    let local_addr = listener.local_addr().context("local_addr")?;
    tracing::info!(addr = %local_addr, version = %state.version, "listening");

    let router = http::router(state);

    // Phase 1 of teardown runs inside `with_graceful_shutdown`: await a
    // signal, broadcast `ServerShutdown`, and sleep the drain window.
    // Axum only stops the listener + waits for in-flight handlers AFTER
    // this future resolves, so everything that must happen while handlers
    // are still live (broadcast + drain) belongs here.
    let registry_for_shutdown = registry.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            wait_for_signal().await;
            tracing::info!("broadcasting ServerShutdown to all sessions");
            registry_for_shutdown.broadcast(Envelope::Event {
                version: 1,
                event: EventKind::ServerShutdown,
            });
            tokio::time::sleep(SHUTDOWN_DRAIN).await;
        })
        .await
        .context("axum::serve")?;

    // Phase 2: listener has stopped and handlers have drained, so DB
    // resources can go away without racing a query.
    tracing::info!("listener stopped; tearing down GC + pool");
    gc_task.abort();
    pool.close().await;

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

/// Spawn the periodic token-GC background task.
///
/// Ticks every [`TOKEN_GC_INTERVAL`] and calls
/// [`store::tokens::gc_expired`]. Errors are logged at `warn!` — GC is
/// best-effort; a failure here does not take the relay down.
fn spawn_token_gc(pool: SqlitePool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TOKEN_GC_INTERVAL);
        // Skip the immediate-on-start tick so the first GC pass happens
        // one interval after boot; avoids a burst of DB work while the
        // server is still bringing up the listener.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().timestamp_millis();
            match store::tokens::gc_expired(&pool, now).await {
                Ok(n) if n > 0 => {
                    tracing::info!(rows = n, "token GC removed expired rows");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "token GC failed");
                }
            }
        }
    })
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
