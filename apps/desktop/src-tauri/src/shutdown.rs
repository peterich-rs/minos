//! Managed-daemon teardown for Exit / ExitRequested / Unix signals.

use crate::daemon::DaemonBridge;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Outer budget for full managed-daemon stop (agent suspend + provider SIGTERM
/// grace already ≤5s inside `DaemonHandle::stop`). Prevents Cmd+Q / window
/// close / pre-update prepare from hanging forever if a child ignores signals.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Idempotent managed-daemon teardown shared by ExitRequested, Exit, and signals.
///
/// Uses `tauri::async_runtime::block_on` against Tauri's **global multi-thread**
/// Tokio runtime (`async_runtime::default_runtime`). That is safe from non-async
/// threads (exit path, dedicated shutdown thread). It must **not** be called
/// from inside an already-running async task on a worker (would panic).
pub fn shutdown_managed_once(daemon: &Arc<DaemonBridge>, done: &AtomicBool) {
    if done.swap(true, Ordering::SeqCst) {
        return;
    }
    info!(
        timeout_secs = SHUTDOWN_TIMEOUT.as_secs(),
        "desktop host shutting down managed daemon"
    );
    let daemon = Arc::clone(daemon);
    // Block until stop finishes (or times out) — Exit is the last chance before
    // process teardown drops the runtime without group signals to provider children.
    tauri::async_runtime::block_on(async move {
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, daemon.shutdown_managed()).await {
            Ok(Ok(())) => {
                info!("desktop host managed daemon shutdown complete");
            }
            Ok(Err(e)) => {
                warn!(
                    error = %e,
                    "managed daemon shutdown failed; continuing process exit"
                );
            }
            Err(_) => {
                warn!(
                    timeout_secs = SHUTDOWN_TIMEOUT.as_secs(),
                    "managed daemon shutdown timed out; continuing process exit"
                );
            }
        }
    });
}

/// SIGINT / SIGTERM / SIGHUP → stop managed children then exit.
///
/// Without this, Ctrl+C on `tauri dev` can leave opencode serve orphans.
/// Cleanup runs on a **named helper thread** (not the ctrlc callback thread),
/// then we `process::exit` so we do not rely on RunEvent after a hard signal.
#[cfg(unix)]
pub fn install_signal_handler(daemon: Arc<DaemonBridge>, done: Arc<AtomicBool>) {
    if let Err(error) = ctrlc::set_handler(move || {
        warn!("received termination signal; shutting down managed daemon");
        let daemon_for_thread = Arc::clone(&daemon);
        let done_for_thread = Arc::clone(&done);
        // Joinable helper: keeps block_on off the ctrlc internal thread and
        // guarantees stop (or timeout) completes before we hard-exit.
        match std::thread::Builder::new()
            .name("minos-desktop-shutdown".into())
            .spawn(move || {
                shutdown_managed_once(&daemon_for_thread, &done_for_thread);
            }) {
            Ok(join) => {
                if let Err(panic) = join.join() {
                    warn!(?panic, "shutdown helper thread panicked");
                }
            }
            Err(spawn_err) => {
                warn!(%spawn_err, "failed to spawn shutdown helper; running inline");
                shutdown_managed_once(&daemon, &done);
            }
        }
        // 128 + SIGINT(2). RunEvent may not run after a hard signal.
        std::process::exit(130);
    }) {
        warn!(%error, "failed to register signal handler");
    }
}
