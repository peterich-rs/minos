//! Lightweight types previously colocated in `runtime.rs`. Phase C task C18
//! retired the single-session `AgentRuntime` along with `runtime.rs`; the
//! configuration value-object and the raw-ingest payload type still need a
//! permanent home for `AgentManager` consumers.

use minos_domain::AgentName;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "test-support")]
use url::Url;

/// Configuration handed to [`crate::manager::AgentManager::new`]. Mirrors the
/// pre-Phase-C `AgentRuntimeConfig` field-for-field so existing daemon wiring
/// keeps compiling.
pub struct AgentRuntimeConfig {
    pub workspace_root: PathBuf,
    pub codex_bin: Option<PathBuf>,
    pub ws_port_range: std::ops::RangeInclusive<u16>,
    pub event_buffer: usize,
    pub handshake_call_timeout: Duration,
    pub subprocess_env: Arc<std::collections::HashMap<String, String>>,
    /// Test-only seam: when `Some`, the manager skips port-probing + codex
    /// spawn + workspace creation and connects directly to this URL.
    /// Production code must leave this as `None`.
    #[cfg(feature = "test-support")]
    pub test_ws_url: Option<Url>,
}

const DEFAULT_HANDSHAKE_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_EVENT_BUFFER: usize = 256;

impl AgentRuntimeConfig {
    /// Minimal constructor that fills in sensible defaults for `ws_port_range`
    /// and `event_buffer`. Callers who need custom values set the fields
    /// afterwards.
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            codex_bin: None,
            ws_port_range: 7879..=7883,
            event_buffer: DEFAULT_EVENT_BUFFER,
            handshake_call_timeout: DEFAULT_HANDSHAKE_CALL_TIMEOUT,
            subprocess_env: Arc::new(std::collections::HashMap::new()),
            #[cfg(feature = "test-support")]
            test_ws_url: None,
        }
    }
}

/// Which codex driver `start_agent` should bring up. The JSONL path is
/// retired post-Phase-C; the `Jsonl` variant is retained only for wire-shape
/// compatibility with pre-Phase-C clients and is silently mapped to `Server`
/// by the daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentLaunchMode {
    /// Retained for backwards-compatible wire shape only. See [`AgentLaunchMode`]
    /// docs.
    Jsonl,
    /// `codex app-server --listen ws://…` long-running, WebSocket-driven.
    #[default]
    Server,
}

impl AgentLaunchMode {
    /// Stable string label suitable for tracing fields and log search.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AgentLaunchMode::Jsonl => "jsonl",
            AgentLaunchMode::Server => "server",
        }
    }
}

/// One raw codex notification, carried verbatim across the manager broadcast.
#[derive(Debug, Clone)]
pub struct RawIngest {
    pub agent: AgentName,
    pub thread_id: String,
    pub payload: Value,
    pub ts_ms: i64,
}
