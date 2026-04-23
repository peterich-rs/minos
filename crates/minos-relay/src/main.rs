//! `minos-relay` binary entrypoint.
//!
//! Wires the library modules (`config`, `store`, `pairing`, `session`,
//! `http`) into a running axum server. Plan §10 acceptance:
//!
//! ```sh
//! cargo run -p minos-relay -- --listen 127.0.0.1:8787 --db ./tmp.db
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
//! [`shutdown_signal`] awaits either `SIGINT` (Ctrl-C) or `SIGTERM`, then
//! broadcasts `Event::ServerShutdown` to every live session and sleeps
//! 500ms to give clients time to drain. The token GC task is aborted
//! afterwards, and the SQLite pool closes in-place via `SqlitePool::close`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig};
use minos_protocol::{Envelope, EventKind};
use minos_relay::{
    config::Config,
    http::{self, RelayState},
    pairing::PairingService,
    session::SessionRegistry,
    store,
};
use sqlx::SqlitePool;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Token GC cadence (plan §10 step 6).
const TOKEN_GC_INTERVAL: Duration = Duration::from_mins(1);

/// Drain window after broadcasting `ServerShutdown` (plan §10 step 8).
const SHUTDOWN_DRAIN: Duration = Duration::from_millis(500);

/// xlog file prefix. Spec §9.4 reserves `relay` for the server process.
const XLOG_NAME_PREFIX: &str = "relay";

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::parse();

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
    let state = RelayState {
        registry: registry.clone(),
        pairing: pairing.clone(),
        store: pool.clone(),
        version: env!("CARGO_PKG_VERSION"),
    };

    let gc_task = spawn_token_gc(pool.clone());

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("bind {}", cfg.listen))?;
    let local_addr = listener.local_addr().context("local_addr")?;
    tracing::info!(addr = %local_addr, version = %state.version, "listening");

    let router = http::router(state);
    let shutdown = shutdown_signal(registry.clone(), pool.clone(), gc_task);

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum::serve")?;

    tracing::info!("server exited cleanly");
    Ok(())
}

/// Install the mars-xlog layer + an `EnvFilter`-gated fmt layer as the
/// global tracing subscriber.
///
/// Mirrors the daemon crate's `logging::init` wiring (spec §9.4). The xlog
/// layer writes `relay_YYYYMMDD.xlog` under `--log-dir`; the fmt layer
/// emits human-readable records to stdout for developer ergonomics.
fn init_tracing(cfg: &Config) -> Result<()> {
    let log_dir = cfg.resolved_log_dir();
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("create log_dir {}", log_dir.display()))?;

    let xlog_cfg = XlogConfig::new(log_dir.to_string_lossy().to_string(), XLOG_NAME_PREFIX);
    let logger = Xlog::init(xlog_cfg, LogLevel::Info).context("Xlog::init (mars-xlog)")?;
    let (xlog_layer, _handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(LogLevel::Info).enabled(true));

    // `RUST_LOG` (or --log-level) may carry full directives like
    // "minos_relay=debug,info"; fall back to "info" if parsing fails so a
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
        "relay logging initialized"
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

/// Await a shutdown signal, then fan out `Event::ServerShutdown` and
/// tear down background tasks + the pool.
///
/// - `Ctrl-C` (`SIGINT`) on every platform.
/// - `SIGTERM` on Unix (POSIX service managers / `kill <pid>`).
///
/// The returned future is handed to `axum::serve(...).with_graceful_shutdown`.
async fn shutdown_signal(
    registry: Arc<SessionRegistry>,
    pool: SqlitePool,
    gc_task: tokio::task::JoinHandle<()>,
) {
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

    let frame = Envelope::Event {
        version: 1,
        event: EventKind::ServerShutdown,
    };
    registry.broadcast(frame);

    tokio::time::sleep(SHUTDOWN_DRAIN).await;

    gc_task.abort();
    pool.close().await;
    tracing::info!("drain window elapsed; pool closed");
}
