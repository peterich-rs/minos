//! Lightweight types previously colocated in `runtime.rs`. Phase C task C18
//! retired the single-session `AgentRuntime` along with `runtime.rs`; the
//! configuration value-object and the raw-ingest payload type still need a
//! permanent home for `AgentManager` consumers.

use minos_domain::AgentName;
use minos_ui_protocol::{ArtifactRef, DisplayPayload, MessageRole};
use serde::{Deserialize, Serialize};
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
    pub grok_bin: Option<PathBuf>,
    pub opencode_bin: Option<PathBuf>,
    pub opencode_port_range: std::ops::RangeInclusive<u16>,
    pub ws_port_range: std::ops::RangeInclusive<u16>,
    pub event_buffer: usize,
    pub handshake_call_timeout: Duration,
    pub thread_start_timeout: Duration,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedMcpCommand {
    pub server_bin: PathBuf,
    pub server_args: Vec<String>,
}

const DEFAULT_HANDSHAKE_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_THREAD_START_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_EVENT_BUFFER: usize = 256;
const TEAMWORK_MCP_ENV: &str = "MINOS_TEAMWORK_MCP_BIN";
pub const TEAMWORK_MCP_SIDECAR_ARG: &str = "__minos-teamwork-mcp";

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
    locate_teamwork_mcp_command().map(|command| command.server_bin)
}

#[must_use]
pub fn locate_teamwork_mcp_command() -> Option<LocatedMcpCommand> {
    locate_teamwork_mcp_command_from(
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
            grok_bin: None,
            opencode_bin: None,
            opencode_port_range: 4096..=4106,
            ws_port_range: 7879..=7883,
            event_buffer: DEFAULT_EVENT_BUFFER,
            handshake_call_timeout: DEFAULT_HANDSHAKE_CALL_TIMEOUT,
            thread_start_timeout: DEFAULT_THREAD_START_TIMEOUT,
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
        let Some(command) = locate_teamwork_mcp_command() else {
            tracing::warn!(
                target: "minos_agent_runtime::config",
                env = TEAMWORK_MCP_ENV,
                filename = teamwork_mcp_filename(),
                "minos teamwork MCP sidecar not found; skipping MCP injection"
            );
            self.mcp = None;
            return Ok(());
        };
        tracing::info!(
            target: "minos_agent_runtime::config",
            command = %command.server_bin.display(),
            args = ?command.server_args,
            socket_path = %socket_path.display(),
            "enabled Minos teamwork MCP injection"
        );
        self.mcp = Some(McpConfig {
            server_bin: command.server_bin,
            server_args: command.server_args,
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

fn locate_teamwork_mcp_command_from(
    env_override: Option<PathBuf>,
    current_exe: Option<PathBuf>,
    path_env: Option<OsString>,
) -> Option<LocatedMcpCommand> {
    if let Some(candidate) = env_override {
        if is_executable_file(&candidate) {
            return Some(LocatedMcpCommand {
                server_bin: candidate,
                server_args: Vec::new(),
            });
        }
    }

    if let Some(current_exe) = current_exe {
        let Some(dir) = current_exe.parent().map(Path::to_path_buf) else {
            return find_executable_on_path(teamwork_mcp_filename(), path_env);
        };
        let candidate = dir.join(teamwork_mcp_filename());
        if is_executable_file(&candidate) {
            return Some(LocatedMcpCommand {
                server_bin: candidate,
                server_args: Vec::new(),
            });
        }
        if let Some(command) = current_exe_sidecar_command(&current_exe) {
            return Some(command);
        }
    }

    find_executable_on_path(teamwork_mcp_filename(), path_env)
}

fn current_exe_sidecar_command(current_exe: &Path) -> Option<LocatedMcpCommand> {
    let stem = current_exe.file_stem()?.to_string_lossy();
    if !is_teamwork_mcp_sidecar_host(stem.as_ref()) {
        return None;
    }
    is_executable_file(current_exe).then(|| LocatedMcpCommand {
        server_bin: current_exe.to_path_buf(),
        server_args: vec![TEAMWORK_MCP_SIDECAR_ARG.to_owned()],
    })
}

/// Host binaries that implement the hidden `__minos-teamwork-mcp` subcommand
/// (stdio MCP server for conversation-bound agents).
///
/// Desktop ships as Tauri productName `Minos` / cargo package `minos-desktop`
/// and embeds the daemon in-process. Without this match, `enable_default_mcp`
/// skips injection and no agent gets conversation-bound teamwork MCP.
fn is_teamwork_mcp_sidecar_host(stem: &str) -> bool {
    matches!(
        stem.to_ascii_lowercase().as_str(),
        "minos-tui" | "minos-daemon" | "minos-desktop" | "minos"
    )
}

fn find_executable_on_path(
    filename: &str,
    path_env: Option<OsString>,
) -> Option<LocatedMcpCommand> {
    let path_env = path_env?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(filename))
        .find(|path| is_executable_file(path))
        .map(|server_bin| LocatedMcpCommand {
            server_bin,
            server_args: Vec::new(),
        })
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

pub const INLINE_RAW_BODY_THRESHOLD: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawBody {
    InlineBytes { bytes: Vec<u8>, media_type: String },
    Artifact { artifact: ArtifactRef },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEventProjection {
    MessageStarted {
        message_id: String,
        role: MessageRole,
    },
    MessageDelta {
        message_id: String,
        lane: TextLane,
        text: DisplayPayload,
    },
    MessageCompleted {
        message_id: String,
    },
    ToolCallStarted {
        message_id: String,
        tool_call_id: String,
        name: String,
        args: DisplayPayload,
    },
    ToolOutput {
        tool_call_id: String,
        stream: ToolStream,
        output: DisplayPayload,
    },
    ToolCallCompleted {
        tool_call_id: String,
        status: ToolStatus,
    },
    Raw {
        event_type: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextLane {
    Assistant,
    Reasoning,
    User,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStream {
    Stdout,
    Stderr,
    Result,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Succeeded,
    Failed,
    Cancelled,
}

/// One raw agent event carried across the runtime ingest plane.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawIngest {
    pub agent: AgentName,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    pub body: RawBody,
    pub ts_ms: i64,
}

impl RawIngest {
    #[must_use]
    pub fn from_json(agent: AgentName, session_id: String, payload: Value, ts_ms: i64) -> Self {
        let provider_session_id = provider_session_id_from_value(agent, &payload);
        let event_type = event_type_from_value(agent, &payload);
        let provider_event_id = provider_event_id_from_value(&payload);
        let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"null".to_vec());
        Self::from_bytes_with_meta(
            agent,
            session_id,
            bytes,
            "application/json".to_string(),
            ts_ms,
            provider_session_id,
            provider_event_id,
            event_type,
        )
    }

    #[must_use]
    pub fn from_bytes(
        agent: AgentName,
        session_id: String,
        bytes: Vec<u8>,
        media_type: impl Into<String>,
        ts_ms: i64,
    ) -> Self {
        Self::from_bytes_with_meta(
            agent,
            session_id,
            bytes,
            media_type.into(),
            ts_ms,
            None,
            None,
            None,
        )
    }

    #[must_use]
    pub fn from_bytes_with_meta(
        agent: AgentName,
        session_id: String,
        bytes: Vec<u8>,
        media_type: String,
        ts_ms: i64,
        provider_session_id: Option<String>,
        provider_event_id: Option<String>,
        event_type: Option<String>,
    ) -> Self {
        Self {
            agent,
            session_id,
            provider_session_id,
            provider_event_id,
            event_type,
            body: RawBody::InlineBytes { bytes, media_type },
            ts_ms,
        }
    }

    #[must_use]
    pub fn inline_bytes(&self) -> Option<&[u8]> {
        match &self.body {
            RawBody::InlineBytes { bytes, .. } => Some(bytes.as_slice()),
            RawBody::Artifact { .. } => None,
        }
    }

    #[must_use]
    pub fn media_type(&self) -> &str {
        match &self.body {
            RawBody::InlineBytes { media_type, .. } => media_type,
            RawBody::Artifact { artifact } => &artifact.media_type,
        }
    }

    pub fn json_value(&self) -> Option<Value> {
        let bytes = self.inline_bytes()?;
        serde_json::from_slice(bytes).ok()
    }

    #[must_use]
    pub fn body_len(&self) -> u64 {
        match &self.body {
            RawBody::InlineBytes { bytes, .. } => bytes.len() as u64,
            RawBody::Artifact { artifact } => artifact.size_bytes,
        }
    }
}

fn provider_session_id_from_value(agent: AgentName, payload: &Value) -> Option<String> {
    match agent {
        AgentName::Claude => payload
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        AgentName::Gemini | AgentName::Grok => payload
            .get("params")
            .and_then(|params| params.get("sessionId"))
            .and_then(Value::as_str)
            .or_else(|| payload.get("sessionId").and_then(Value::as_str))
            .map(str::to_string),
        AgentName::Opencode => {
            let properties = payload.get("properties").unwrap_or(payload);
            properties
                .get("sessionID")
                .and_then(Value::as_str)
                .or_else(|| {
                    properties
                        .get("info")
                        .and_then(|info| info.get("sessionID").or_else(|| info.get("id")))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    properties
                        .get("part")
                        .and_then(|part| part.get("sessionID"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    payload
                        .get("session")
                        .and_then(|session| session.get("id"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    payload
                        .get("message")
                        .and_then(|message| message.get("sessionID"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    payload
                        .get("part")
                        .and_then(|part| part.get("sessionID"))
                        .and_then(Value::as_str)
                })
                .or_else(|| payload.get("sessionID").and_then(Value::as_str))
                .map(str::to_string)
        }
        AgentName::Codex => None,
    }
}

fn event_type_from_value(agent: AgentName, payload: &Value) -> Option<String> {
    match agent {
        AgentName::Codex => payload
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string),
        AgentName::Claude => payload
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
        AgentName::Gemini | AgentName::Grok => payload
            .get("kind")
            .and_then(Value::as_str)
            .or_else(|| payload.get("method").and_then(Value::as_str))
            .map(str::to_string),
        AgentName::Opencode => payload
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn provider_event_id_from_value(payload: &Value) -> Option<String> {
    payload
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| payload.get("event_id").and_then(Value::as_str))
        .or_else(|| {
            payload
                .get("params")
                .and_then(|params| params.get("id"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
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

        let found = locate_teamwork_mcp_command_from(
            Some(env_bin.clone()),
            Some(sibling_dir.join("minos-daemon")),
            std::env::join_paths([path_dir]).ok(),
        );

        assert_eq!(
            found,
            Some(LocatedMcpCommand {
                server_bin: env_bin,
                server_args: Vec::new(),
            })
        );
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

        let found = locate_teamwork_mcp_command_from(
            None,
            Some(sibling_dir.join("minos-daemon")),
            std::env::join_paths([path_dir]).ok(),
        );

        assert_eq!(
            found,
            Some(LocatedMcpCommand {
                server_bin: sibling_bin,
                server_args: Vec::new(),
            })
        );
    }

    #[test]
    fn teamwork_mcp_locator_uses_current_tui_as_hidden_sidecar_before_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exe_dir = tmp.path().join("exe");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&exe_dir).expect("mkdir exe");
        std::fs::create_dir_all(&path_dir).expect("mkdir path");

        let current_exe = exe_dir.join("minos-tui");
        let path_bin = path_dir.join(teamwork_mcp_filename());
        make_executable(&current_exe);
        make_executable(&path_bin);

        let found = locate_teamwork_mcp_command_from(
            None,
            Some(current_exe.clone()),
            std::env::join_paths([path_dir]).ok(),
        );

        assert_eq!(
            found,
            Some(LocatedMcpCommand {
                server_bin: current_exe,
                server_args: vec![TEAMWORK_MCP_SIDECAR_ARG.to_owned()],
            })
        );
    }

    #[test]
    fn teamwork_mcp_locator_uses_desktop_exe_as_hidden_sidecar() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exe_dir = tmp.path().join("exe");
        std::fs::create_dir_all(&exe_dir).expect("mkdir exe");

        // Tauri productName "Minos" and cargo package "minos-desktop".
        for name in ["Minos", "minos-desktop"] {
            let current_exe = exe_dir.join(name);
            make_executable(&current_exe);
            let found = locate_teamwork_mcp_command_from(None, Some(current_exe.clone()), None);
            assert_eq!(
                found,
                Some(LocatedMcpCommand {
                    server_bin: current_exe,
                    server_args: vec![TEAMWORK_MCP_SIDECAR_ARG.to_owned()],
                }),
                "stem {name}"
            );
        }
    }

    #[test]
    fn is_teamwork_mcp_sidecar_host_covers_desktop_names() {
        assert!(is_teamwork_mcp_sidecar_host("minos-tui"));
        assert!(is_teamwork_mcp_sidecar_host("minos-daemon"));
        assert!(is_teamwork_mcp_sidecar_host("minos-desktop"));
        assert!(is_teamwork_mcp_sidecar_host("Minos"));
        assert!(is_teamwork_mcp_sidecar_host("minos"));
        assert!(!is_teamwork_mcp_sidecar_host("node"));
        assert!(!is_teamwork_mcp_sidecar_host("opencode"));
    }

    #[test]
    fn teamwork_mcp_locator_falls_back_to_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&path_dir).expect("mkdir path");

        let path_bin = path_dir.join(teamwork_mcp_filename());
        make_executable(&path_bin);

        let found =
            locate_teamwork_mcp_command_from(None, None, std::env::join_paths([path_dir]).ok());

        assert_eq!(
            found,
            Some(LocatedMcpCommand {
                server_bin: path_bin,
                server_args: Vec::new(),
            })
        );
    }

    #[test]
    fn teamwork_mcp_locator_returns_none_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path_env = std::env::join_paths([tmp.path().join("path")]).ok();

        let found = locate_teamwork_mcp_command_from(
            Some(tmp.path().join("missing")),
            Some(tmp.path().join("sibling").join("minos-daemon")),
            path_env,
        );

        assert_eq!(found, None);
    }
}
