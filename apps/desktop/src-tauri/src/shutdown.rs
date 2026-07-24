//! Managed-daemon teardown for Exit / ExitRequested / Unix signals.

use crate::daemon::DaemonBridge;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

/// Idempotent managed-daemon teardown shared by ExitRequested, Exit, and signals.
pub fn shutdown_managed_once(daemon: &Arc<DaemonBridge>, done: &AtomicBool) {
    if done.swap(true, Ordering::SeqCst) {
        return;
    }
    info!("desktop host shutting down managed daemon");
    let daemon = Arc::clone(daemon);
    // Block until stop finishes — Exit is the last chance before process
    // teardown drops the runtime without group signals to provider children.
    tauri::async_runtime::block_on(async move {
        daemon.shutdown_managed().await;
    });
    info!("desktop host managed daemon shutdown complete");
}

/// SIGINT / SIGTERM / SIGHUP → stop managed children then exit.
/// Without this, Ctrl+C on `tauri dev` can leave opencode serve orphans.
#[cfg(unix)]
pub fn install_signal_handler(daemon: Arc<DaemonBridge>, done: Arc<AtomicBool>) {
    if let Err(error) = ctrlc::set_handler(move || {
        warn!("received termination signal; shutting down managed daemon");
        shutdown_managed_once(&daemon, &done);
        std::process::exit(0);
    }) {
        warn!(%error, "failed to register signal handler");
    }
}
