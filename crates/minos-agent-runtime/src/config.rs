//! Lightweight types previously colocated in `runtime.rs`. Phase C task C18
//! retired the single-session `AgentRuntime` along with `runtime.rs`; the
//! configuration value-object and the raw-ingest payload type still need a
//! permanent home for `AgentManager` consumers.

use minos_domain::AgentName;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
const TEAMWORK_MCP_ENV: &str = "MINOS_TEAMWORK_MCP_BIN";

#[must_use]
pub fn teamwork_mcp_filename() -> &'static str {
    if cfg!(windows) {
        "minos-teamwork-mcp.exe"
    } else {
        "minos-teamwork-mcp"
    }
}

#[must_use]
pub fn locate_teamwork_mcp_binary() -> Option<PathBuf> {
    locate_teamwork_mcp_binary_from(
        std::env::var_os(TEAMWORK_MCP_ENV).map(PathBuf::from),
        std::env::current_exe().ok(),
        std::env::var_os("PATH"),
    )
}

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
        self.enable_default_mcp_with_socket_path(default_mcp_socket_path()?)
    }

    pub fn enable_default_mcp_with_socket_path(
        &mut self,
        socket_path: PathBuf,
    ) -> anyhow::Result<()> {
        let db_path = minos_chat_store::default_db_path()?;
        let Some(server_bin) = locate_teamwork_mcp_binary() else {
            tracing::warn!(
                target: "minos_agent_runtime::config",
                env = TEAMWORK_MCP_ENV,
                filename = teamwork_mcp_filename(),
                "minos teamwork MCP sidecar not found; skipping MCP injection"
            );
            self.mcp = None;
            return Ok(());
        };
        self.mcp = Some(McpConfig {
            server_bin,
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

fn default_mcp_socket_path() -> anyhow::Result<PathBuf> {
    let db_path = minos_chat_store::default_db_path()?;
    let minos_home = db_path.parent().expect("db_path parent").to_path_buf();
    Ok(minos_home
        .join("run")
        .join(format!("mcp-daemon-{}.sock", uuid::Uuid::new_v4())))
}

fn locate_teamwork_mcp_binary_from(
    env_override: Option<PathBuf>,
    current_exe: Option<PathBuf>,
    path_env: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(candidate) = env_override {
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    if let Some(dir) = current_exe.and_then(|path| path.parent().map(Path::to_path_buf)) {
        let candidate = dir.join(teamwork_mcp_filename());
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    find_executable_on_path(teamwork_mcp_filename(), path_env)
}

fn find_executable_on_path(filename: &str, path_env: Option<OsString>) -> Option<PathBuf> {
    let path_env = path_env?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(filename))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executable(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\n").expect("write test executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("chmod test executable");
        }
    }

    #[test]
    fn teamwork_mcp_locator_prefers_env_override() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env_dir = tmp.path().join("env");
        let sibling_dir = tmp.path().join("sibling");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&env_dir).expect("mkdir env");
        std::fs::create_dir_all(&sibling_dir).expect("mkdir sibling");
        std::fs::create_dir_all(&path_dir).expect("mkdir path");

        let env_bin = env_dir.join(teamwork_mcp_filename());
        let sibling_bin = sibling_dir.join(teamwork_mcp_filename());
        let path_bin = path_dir.join(teamwork_mcp_filename());
        make_executable(&env_bin);
        make_executable(&sibling_bin);
        make_executable(&path_bin);

        let found = locate_teamwork_mcp_binary_from(
            Some(env_bin.clone()),
            Some(sibling_dir.join("minos-daemon")),
            std::env::join_paths([path_dir]).ok(),
        );

        assert_eq!(found, Some(env_bin));
    }

    #[test]
    fn teamwork_mcp_locator_uses_sibling_before_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sibling_dir = tmp.path().join("sibling");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&sibling_dir).expect("mkdir sibling");
        std::fs::create_dir_all(&path_dir).expect("mkdir path");

        let sibling_bin = sibling_dir.join(teamwork_mcp_filename());
        let path_bin = path_dir.join(teamwork_mcp_filename());
        make_executable(&sibling_bin);
        make_executable(&path_bin);

        let found = locate_teamwork_mcp_binary_from(
            None,
            Some(sibling_dir.join("minos-daemon")),
            std::env::join_paths([path_dir]).ok(),
        );

        assert_eq!(found, Some(sibling_bin));
    }

    #[test]
    fn teamwork_mcp_locator_falls_back_to_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&path_dir).expect("mkdir path");

        let path_bin = path_dir.join(teamwork_mcp_filename());
        make_executable(&path_bin);

        let found =
            locate_teamwork_mcp_binary_from(None, None, std::env::join_paths([path_dir]).ok());

        assert_eq!(found, Some(path_bin));
    }

    #[test]
    fn teamwork_mcp_locator_returns_none_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path_env = std::env::join_paths([tmp.path().join("path")]).ok();

        let found = locate_teamwork_mcp_binary_from(
            Some(tmp.path().join("missing")),
            Some(tmp.path().join("sibling").join("minos-daemon")),
            path_env,
        );

        assert_eq!(found, None);
    }
}
