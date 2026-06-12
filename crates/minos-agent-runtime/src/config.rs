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
    pub gemini_bin: Option<PathBuf>,
    pub opencode_bin: Option<PathBuf>,
    pub opencode_port_range: std::ops::RangeInclusive<u16>,
    pub ws_port_range: std::ops::RangeInclusive<u16>,
    pub event_buffer: usize,
    pub handshake_call_timeout: Duration,
    pub approval_request_timeout: Duration,
    pub subprocess_env: Arc<std::collections::HashMap<String, String>>,
    pub mcp: Option<McpConfig>,
    #[cfg(feature = "test-support")]
    pub test_ws_url: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    pub server_bin: PathBuf,
    pub server_args: Vec<String>,
    pub socket_path: PathBuf,
    pub db_path: PathBuf,
    pub permissions: minos_chat_store::mcp_server::McpToolPermissions,
}

const DEFAULT_HANDSHAKE_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_APPROVAL_REQUEST_TIMEOUT: Duration = Duration::from_mins(2);
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
            gemini_bin: None,
            opencode_bin: None,
            opencode_port_range: 4096..=4106,
            ws_port_range: 7879..=7883,
            event_buffer: DEFAULT_EVENT_BUFFER,
            handshake_call_timeout: DEFAULT_HANDSHAKE_CALL_TIMEOUT,
            approval_request_timeout: DEFAULT_APPROVAL_REQUEST_TIMEOUT,
            subprocess_env: Arc::new(std::collections::HashMap::new()),
            mcp: None,
            #[cfg(feature = "test-support")]
            test_ws_url: None,
        }
    }

    pub fn enable_default_mcp(&mut self) -> anyhow::Result<()> {
        let db_path = minos_chat_store::default_db_path()?;
        let minos_home = db_path.parent().expect("db_path parent").to_path_buf();
        let socket_path = minos_home
            .join("run")
            .join(format!("mcp-daemon-{}.sock", uuid::Uuid::new_v4()));
        self.mcp = Some(McpConfig {
            server_bin: PathBuf::from("minos-teamwork-mcp"),
            server_args: Vec::new(),
            socket_path,
            db_path,
            permissions: minos_chat_store::mcp_server::McpToolPermissions::default(),
        });
        Ok(())
    }

    pub fn enable_mcp_with_command(
        &mut self,
        server_bin: PathBuf,
        server_args: Vec<String>,
        socket_path: PathBuf,
    ) -> anyhow::Result<()> {
        self.mcp = Some(McpConfig {
            server_bin,
            server_args,
            socket_path,
            db_path: minos_chat_store::default_db_path()?,
            permissions: minos_chat_store::mcp_server::McpToolPermissions::default(),
        });
        Ok(())
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
