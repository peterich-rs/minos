use std::io::{self, Read, Write};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const FRAME_HEADER_LEN: usize = 4;
pub const MAX_FRAME_LEN: u32 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SocketRequest {
    ListConversationMessages {
        conversation_id: String,
        before_seq: Option<u64>,
        limit: Option<u32>,
    },
    ListConversationRoster {
        conversation_id: String,
    },
    DelegateToAgent {
        conversation_id: String,
        source_agent: Option<String>,
        source_session_id: Option<String>,
        /// Runtime agent name (codex/claude/…). Optional when `profile_id` or
        /// `target_profile` is set; daemon may also apply newest-profile convenience.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_agent: Option<String>,
        /// Explicit host agent profile id (stable). Prefer over name when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_id: Option<String>,
        /// Profile display name; daemon resolves to a unique profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_profile: Option<String>,
        prompt: String,
    },
    GetDelegationStatus {
        conversation_id: String,
        delegation_id: String,
    },
    WaitDelegation {
        conversation_id: String,
        delegation_id: String,
        timeout_ms: i64,
    },
    CancelDelegation {
        conversation_id: String,
        delegation_id: String,
        reason: Option<String>,
    },
    PostConversationUpdate {
        conversation_id: String,
        source_agent: Option<String>,
        source_session_id: Option<String>,
        message: String,
    },
    PostGitUpdate {
        conversation_id: String,
        source_agent: Option<String>,
        source_session_id: Option<String>,
        /// JSON object matching minos_protocol::GitActivity (tagged `kind`).
        activity: serde_json::Value,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SocketResponse {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
    Pong,
}

pub fn encode_frame(value: &SocketResponse) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(value).context("failed to serialize socket frame")?;
    let len = u32::try_from(payload.len()).context("socket frame payload too large")?;
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<SocketRequest>> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header);
    anyhow::ensure!(len <= MAX_FRAME_LEN, "socket frame too large: {len} bytes");
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    let request: SocketRequest =
        serde_json::from_slice(&payload).context("failed to deserialize socket frame")?;
    Ok(Some(request))
}

pub fn read_response_frame<R: Read>(reader: &mut R) -> Result<Option<SocketResponse>> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header);
    anyhow::ensure!(len <= MAX_FRAME_LEN, "socket frame too large: {len} bytes");
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    let response: SocketResponse =
        serde_json::from_slice(&payload).context("failed to deserialize socket response frame")?;
    Ok(Some(response))
}

pub fn write_response<W: Write>(writer: &mut W, response: &SocketResponse) -> Result<()> {
    let frame = encode_frame(response)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}
