//! Minos desktop shell — Tauri host + local daemon JSON-RPC bridge.

mod app_state;
mod commands;
mod daemon;
mod shutdown;
mod window_reveal;

use app_state::AppState;
use commands::*;
use daemon::DaemonBridge;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{Manager, RunEvent};
use tauri_plugin_window_state::StateFlags;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Attach `tauri-plugin-updater` only for release binaries built with updater secrets.
fn maybe_register_updater(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    #[cfg(minos_updater_enabled)]
    {
        if !cfg!(debug_assertions) {
            return builder.plugin(tauri_plugin_updater::Builder::new().build());
        }
    }
    builder
}

fn init_tracing() {
    // Host is the tracing SSOT for this process. Managed in-process daemon does
    // **not** call `minos_daemon::logging::init` (that path is only for the
    // standalone `minos-daemon` binary / mars-xlog). RUST_LOG, when set, wins
    // for desktop + embedded daemon crates together — intentional.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "minos_desktop_lib=info,minos_daemon=info,minos_agent_runtime=info,minos_chat_store=info",
        )
    });
    // try_init: ignore if a test harness (or re-entrant path) already set a subscriber.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    info!("starting Minos desktop host");

    let daemon = Arc::new(DaemonBridge::new());
    let daemon_for_exit = Arc::clone(&daemon);
    let shutdown_done = Arc::new(AtomicBool::new(false));

    #[cfg(unix)]
    shutdown::install_signal_handler(Arc::clone(&daemon), Arc::clone(&shutdown_done));

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Focus the existing window when a duplicate instance launches so
            // we never start a second managed daemon from a second process.
            info!("second instance launch; focusing existing main window");
            if let Some(w) = app.get_webview_window("main") {
                if let Err(error) = w.unminimize() {
                    warn!(%error, "failed to unminimize main window for single-instance focus");
                }
                if let Err(error) = w.show() {
                    warn!(%error, "failed to show main window for single-instance focus");
                }
                if let Err(error) = w.set_focus() {
                    warn!(%error, "failed to focus main window for single-instance");
                }
            } else {
                warn!("single-instance callback: main window missing");
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility is excluded: the initial-window-reveal plugin
                // shows the window after saved geometry has been restored.
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(window_reveal::plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Always register process so release builds can relaunch after install.
        .plugin(tauri_plugin_process::init());

    // Register the updater only in configured release builds; omit it locally.
    // Requires MINOS_UPDATER_PUBLIC_KEY + MINOS_UPDATER_ENDPOINT at compile time
    // (see build.rs) and a non-debug binary.
    let builder = maybe_register_updater(builder);

    builder
        .manage(AppState {
            daemon: Arc::clone(&daemon),
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let daemon = Arc::clone(&daemon);
            // Attach AppHandle so connect() can start JSON-RPC subscription pumps.
            tauri::async_runtime::spawn(async move {
                daemon.attach_app(handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            daemon_connect,
            daemon_status,
            daemon_list_projects,
            daemon_create_project,
            daemon_list_conversations,
            daemon_list_messages,
            daemon_toggle_message_reaction,
            daemon_list_sessions,
            daemon_list_project_sessions,
            daemon_read_transcript,
            daemon_create_conversation,
            daemon_git_get_status,
            daemon_update_conversation,
            daemon_remove_conversation_agent,
            daemon_append_user_message,
            daemon_list_clis,
            daemon_start_agent_in_conversation,
            daemon_list_models,
            daemon_list_agent_profiles,
            daemon_create_agent_profile,
            daemon_delete_agent_profile,
            daemon_send_user_message,
            daemon_resume_session,
            daemon_resolve_approval,
            daemon_respond_opencode_permission,
            daemon_respond_opencode_question,
            daemon_host_prepare_link,
            daemon_host_sign_link_proof,
            daemon_host_apply_link_token,
            is_auto_update_supported,
            is_updater_plugin_enabled,
            prepare_for_app_update,
            restore_after_failed_update,
            reset_update_shutdown_guard,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Minos desktop")
        .run(move |_app, event| {
            // Kill provider children (OpenCode serve, Codex, …) on exit.
            // Without this, `opencode serve` is reparented to launchd and
            // exhausts ports 4096..=4106 across Desktop restarts.
            // prepare_for_app_update shares the same teardown; skip double-stop.
            match event {
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    commands::shutdown_for_exit(&daemon_for_exit, &shutdown_done);
                }
                _ => {}
            }
        });
}
