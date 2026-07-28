//! Desktop auto-update helpers (Tauri updater plugin + process relaunch).

use crate::app_state::AppState;
use crate::daemon::{ConnectionDto, DaemonBridge};
use crate::shutdown::{self, SHUTDOWN_TIMEOUT};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;
use tracing::{info, warn};

/// Process-wide guard so prepare-for-update and ExitRequested do not race.
static UPDATE_SHUTDOWN_DONE: AtomicBool = AtomicBool::new(false);

/// Returns `true` when the running install supports Tauri's auto-updater.
///
/// On Linux, Tauri's updater only works for AppImage bundles. The AppImage
/// runtime sets `APPIMAGE` when the binary is executed from an AppImage.
/// `.deb` / other packages surface a manual-download path instead.
///
/// On macOS and Windows every supported install format is auto-updatable.
#[tauri::command]
pub fn is_auto_update_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var("APPIMAGE").is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// True when this binary was built with updater pubkey + endpoint injected
/// (`MINOS_UPDATER_PUBLIC_KEY` + `MINOS_UPDATER_ENDPOINT` at compile time).
#[tauri::command]
pub fn is_updater_plugin_enabled() -> bool {
    cfg!(minos_updater_enabled) && !cfg!(debug_assertions)
}

/// Stop managed daemon / agent children before applying an in-app update.
///
/// Must run before `update.install()` + process relaunch so the new binary
/// does not fight orphaned OpenCode/Codex processes or a stale daemon port.
///
/// Hard-timeout + error propagation: callers must not install if this fails.
/// On any subsequent install/relaunch failure, call
/// [`restore_after_failed_update`] so the shell is usable again.
#[tauri::command]
pub async fn prepare_for_app_update(state: State<'_, AppState>) -> Result<(), String> {
    if UPDATE_SHUTDOWN_DONE.load(Ordering::SeqCst) {
        info!(target: "minos_desktop::updater", "prepare_for_app_update already completed");
        return Ok(());
    }
    info!(target: "minos_desktop::updater", "stopping managed daemon before app update");
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, state.daemon.shutdown_managed()).await {
        Ok(Ok(())) => {
            UPDATE_SHUTDOWN_DONE.store(true, Ordering::SeqCst);
            info!(target: "minos_desktop::updater", "managed daemon stopped; safe to install update");
            Ok(())
        }
        Ok(Err(e)) => {
            // stop may have partially torn down children — leave guard false so
            // restore / exit can still act, and surface the error to the UI.
            warn!(
                target: "minos_desktop::updater",
                error = %e,
                "prepare_for_app_update failed"
            );
            Err(e)
        }
        Err(_) => {
            warn!(
                target: "minos_desktop::updater",
                timeout_secs = SHUTDOWN_TIMEOUT.as_secs(),
                "prepare_for_app_update timed out"
            );
            // Timeout leaves teardown in an unknown state; do not set the done
            // guard. Frontend should call restore_after_failed_update.
            Err(format!(
                "managed daemon shutdown timed out after {}s",
                SHUTDOWN_TIMEOUT.as_secs()
            ))
        }
    }
}

/// Undo prepare teardown after a failed install/relaunch (or failed prepare).
///
/// Resets the prepare guard and restarts a managed daemon so the desktop shell
/// is not left without local RPC for the rest of the process lifetime.
#[tauri::command]
pub async fn restore_after_failed_update(
    state: State<'_, AppState>,
) -> Result<ConnectionDto, String> {
    UPDATE_SHUTDOWN_DONE.store(false, Ordering::SeqCst);
    info!(
        target: "minos_desktop::updater",
        "restoring managed daemon after failed app update"
    );
    let connection = state.daemon.connect(None).await;
    if connection.connected {
        info!(
            target: "minos_desktop::updater",
            source = %connection.source,
            managed = connection.managed,
            "managed daemon restored after failed update"
        );
        Ok(connection)
    } else {
        let msg = connection
            .error
            .clone()
            .unwrap_or_else(|| "daemon restore failed after update".into());
        warn!(
            target: "minos_desktop::updater",
            error = %msg,
            "daemon restore after failed update did not connect"
        );
        Err(msg)
    }
}

/// Reset the prepare guard after a failed install so the next attempt re-runs teardown.
/// Prefer [`restore_after_failed_update`] from the UI failure path.
#[tauri::command]
pub fn reset_update_shutdown_guard() {
    UPDATE_SHUTDOWN_DONE.store(false, Ordering::SeqCst);
    info!(target: "minos_desktop::updater", "update shutdown guard reset");
}

/// Idempotent sync teardown for Exit / signals (shares guard with prepare).
pub fn shutdown_for_exit(daemon: &Arc<DaemonBridge>, done: &AtomicBool) {
    if UPDATE_SHUTDOWN_DONE.load(Ordering::SeqCst) {
        // prepare_for_app_update already stopped children; mark exit done too.
        done.store(true, Ordering::SeqCst);
        return;
    }
    shutdown::shutdown_managed_once(daemon, done);
    if done.load(Ordering::SeqCst) {
        UPDATE_SHUTDOWN_DONE.store(true, Ordering::SeqCst);
    }
}

/// Reset guard — only for tests.
#[cfg(test)]
pub fn reset_update_shutdown_guard_for_tests() {
    UPDATE_SHUTDOWN_DONE.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_update_supported_is_bool() {
        // Smoke: function is callable on every host OS.
        let _ = is_auto_update_supported();
        assert!(!is_updater_plugin_enabled() || cfg!(minos_updater_enabled));
    }

    #[test]
    fn plugin_enabled_false_in_debug_without_cfg() {
        // Local dev builds never enable the plugin even if cfg is set.
        if cfg!(debug_assertions) {
            assert!(!is_updater_plugin_enabled());
        }
    }

    #[test]
    fn shutdown_guard_is_idempotent_flag() {
        reset_update_shutdown_guard_for_tests();
        assert!(!UPDATE_SHUTDOWN_DONE.load(Ordering::SeqCst));
        UPDATE_SHUTDOWN_DONE.store(true, Ordering::SeqCst);
        assert!(UPDATE_SHUTDOWN_DONE.load(Ordering::SeqCst));
        reset_update_shutdown_guard_for_tests();
    }

    #[test]
    fn reset_command_clears_guard() {
        UPDATE_SHUTDOWN_DONE.store(true, Ordering::SeqCst);
        reset_update_shutdown_guard();
        assert!(!UPDATE_SHUTDOWN_DONE.load(Ordering::SeqCst));
    }
}
