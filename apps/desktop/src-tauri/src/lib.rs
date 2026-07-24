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

fn init_tracing() {
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

    tauri::Builder::default()
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
            daemon_update_conversation,
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building Minos desktop")
        .run(move |_app, event| {
            // Kill provider children (OpenCode serve, Codex, …) on exit.
            // Without this, `opencode serve` is reparented to launchd and
            // exhausts ports 4096..=4106 across Desktop restarts.
            match event {
                RunEvent::ExitRequested { .. } => {
                    shutdown::shutdown_managed_once(&daemon_for_exit, &shutdown_done);
                }
                RunEvent::Exit => {
                    shutdown::shutdown_managed_once(&daemon_for_exit, &shutdown_done);
                }
                _ => {}
            }
        });
}
