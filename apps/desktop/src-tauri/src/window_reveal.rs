//! Initial window reveal — wait for stable geometry + React first paint before show.
//!
//! Inspired by block/buzz `initial-window-reveal` (without vibrancy/transparent chrome).
//! Window is created with `visible: false` so the webview never white-flashes.

use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Listener, Manager, Runtime,
};
use tracing::{info, warn};

const INITIAL_RENDER_READY_EVENT: &str = "initial-render-ready";
const MAIN_WINDOW_LABEL: &str = "main";

fn reveal_initial_window<R: Runtime>(window: &tauri::Window<R>) {
    if let Err(error) = window.show() {
        warn!(%error, "failed to reveal main window");
        return;
    }
    if let Err(error) = window.set_focus() {
        warn!(%error, "failed to focus main window after reveal");
    }
    info!("main window revealed");
}

/// macOS (and others) may apply window-state restore asynchronously. Wait for
/// consecutive identical outer bounds so we don't flash mid-resize.
async fn wait_for_stable_initial_window_geometry<R: Runtime>(window: &tauri::Window<R>) {
    const MAX_POLLS: usize = 120;
    const REQUIRED_STABLE_POLLS: usize = 4;

    let mut previous_bounds = None;
    let mut stable_polls = 0;

    for _ in 0..MAX_POLLS {
        let bounds = match (window.outer_position(), window.outer_size()) {
            (Ok(position), Ok(size)) => Some((position.x, position.y, size.width, size.height)),
            _ => None,
        };

        if bounds.is_some() && bounds == previous_bounds {
            stable_polls += 1;
            if stable_polls >= REQUIRED_STABLE_POLLS {
                return;
            }
        } else {
            stable_polls = 0;
        }
        previous_bounds = bounds;

        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
    }

    warn!("initial window geometry did not settle before reveal timeout");
}

/// Register the inline plugin that reveals the main window after geometry + first paint.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::<R, ()>::new("initial-window-reveal")
        .on_webview_ready(|webview| {
            if webview.label() != MAIN_WINDOW_LABEL {
                return;
            }

            let window = webview.window();
            let (initial_render_tx, initial_render_rx) = tokio::sync::oneshot::channel();
            let app = window.app_handle().clone();
            app.once(INITIAL_RENDER_READY_EVENT, move |_| {
                let _ = initial_render_tx.send(());
            });

            tauri::async_runtime::spawn(async move {
                wait_for_stable_initial_window_geometry(&window).await;

                if tokio::time::timeout(std::time::Duration::from_secs(5), initial_render_rx)
                    .await
                    .is_err()
                {
                    warn!("initial render did not commit before reveal timeout");
                }

                reveal_initial_window(&window);
            });
        })
        .build()
}
