use crate::types::{PermissionOption, RequestPermissionOutcome, SessionId, ToolCallUpdate};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RequestPermissionParams {
    pub session_id: SessionId,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RequestPermissionResponse { pub outcome: RequestPermissionOutcome }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsReadTextFileParams {
    pub session_id: SessionId,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsReadTextFileResponse { pub content: String }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsWriteTextFileParams { pub session_id: SessionId, pub path: String, pub content: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsWriteTextFileResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalCreateParams {
    pub session_id: SessionId,
    pub command: String,
    #[serde(default)] pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub cwd: Option<String>,
    #[serde(default)] pub env: Vec<EnvVariable>,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub output_byte_limit: Option<u64>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnvVariable { pub name: String, pub value: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalCreateResponse { pub terminal_id: String }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalOutputParams { pub session_id: SessionId, pub terminal_id: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalOutputResponse {
    pub output: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_status: Option<TerminalExitStatus>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalExitStatus {
    #[serde(skip_serializing_if = "Option::is_none", default)] pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub signal: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalReleaseParams { pub session_id: SessionId, pub terminal_id: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalReleaseResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalKillParams { pub session_id: SessionId, pub terminal_id: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalKillResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalWaitForExitParams { pub session_id: SessionId, pub terminal_id: String }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalWaitForExitResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)] pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub signal: Option<String>,
}
