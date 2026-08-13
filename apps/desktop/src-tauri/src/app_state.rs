//! Shared Tauri managed state for command handlers.

use crate::account_realtime::AccountRealtime;
use crate::daemon::DaemonBridge;
use crate::im_outbox_store::ImOutboxStore;
use std::sync::Arc;

pub struct AppState {
    pub daemon: Arc<DaemonBridge>,
    pub im_outbox: Arc<ImOutboxStore>,
    pub account_ws: Arc<AccountRealtime>,
}
