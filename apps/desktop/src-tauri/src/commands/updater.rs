//! Desktop auto-update helpers (Tauri updater plugin + process relaunch).

use crate::app_state::AppState;
use crate::daemon::DaemonBridge;
use crate::shutdown;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;
use tracing::info;

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
#[tauri::command]
pub async fn prepare_for_app_update(state: State<'_, AppState>) -> Result<(), String> {
    if UPDATE_SHUTDOWN_DONE.swap(true, Ordering::SeqCst) {
        info!(target: "minos_desktop::updater", "prepare_for_app_update already completed");
        return Ok(());
    }
    info!(target: "minos_desktop::updater", "stopping managed daemon before app update");
    // Prefer the async path so we do not block_on inside an async command.
    state.daemon.shutdown_managed().await;
    info!(target: "minos_desktop::updater", "managed daemon stopped; safe to install update");
    Ok(())
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
}
