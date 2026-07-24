//! Shared Tauri managed state for command handlers.

use crate::daemon::DaemonBridge;
use std::sync::Arc;

pub struct AppState {
    pub daemon: Arc<DaemonBridge>,
}
