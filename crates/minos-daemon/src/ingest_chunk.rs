use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use minos_agent_runtime::{RawBody, RawIngest};
use minos_domain::{AgentName, DeviceId};
use minos_protocol::realtime::HostIngestChunk;
use minos_ui_protocol::{ArtifactRef, UiEventMessage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::{EventRow, SessionRow};

#[derive(Debug, Clone)]
pub struct IngestChunk {
    pub ingest: RawIngest,
    pub seq: u64,
    /// Local projection for Desktop/TUI; not uploaded as cloud SSOT.
    pub projection: Vec<UiEventMessage>,
    /// Local conversation id for hub formal session auto-registration.
    pub conversation_id: Option<String>,
    pub byte_len: u64,
    pub checksum_sha256: String,
}

impl IngestChunk {
    #[must_use]
    pub fn new(
        ingest: RawIngest,
        seq: u64,
        projection: Vec<UiEventMessage>,
        conversation_id: Option<String>,
    ) -> Self {
        let byte_len = ingest.body_len();
        let checksum_sha256 = checksum_raw_body(&ingest.body);
        Self {
            ingest,
            seq,
            projection,
            conversation_id,
            byte_len,
            checksum_sha256,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.ingest.session_id
    }

    #[must_use]
    pub fn ts_ms(&self) -> i64 {
        self.ingest.ts_ms
    }

    #[must_use]
    pub fn to_wire(&self, host_id: DeviceId) -> HostIngestChunk {
        HostIngestChunk {
            event_id: event_id(host_id, &self.ingest.session_id, self.seq),
            session_id: self.ingest.session_id.clone(),
            seq: self.seq,
            agent: self.ingest.agent,
            kind: self
                .ingest
                .event_type
                .clone()
                .unwrap_or_else(|| "agent_event".to_string()),
            payload: payload_from_raw_body(&self.ingest.body),
            conversation_id: self.conversation_id.clone(),
            first_ts_ms: self.ingest.ts_ms,
            last_ts_ms: self.ingest.ts_ms,
            byte_len: self.byte_len,
            checksum_sha256: self.checksum_sha256.clone(),
        }
    }
}

pub fn wire_chunk_from_event_row(
    host_id: DeviceId,
    thread: &SessionRow,
    row: &EventRow,
) -> anyhow::Result<HostIngestChunk> {
    let seq = row.seq.max(0) as u64;
    let payload = payload_from_event_row(row);
    let payload_bytes = serde_json::to_vec(&payload)?;
    let checksum_sha256 = if let Some(sha256) = &row.artifact_sha256 {
        sha256.clone()
    } else {
        hex_sha256(&payload_bytes)
    };
    Ok(HostIngestChunk {
        event_id: event_id(host_id, &row.session_id, seq),
        session_id: row.session_id.clone(),
        seq,
        agent: agent_from_thread(thread),
        kind: "agent_event".to_string(),
        payload,
        conversation_id: Some(thread.conversation_id.clone()).filter(|s| !s.is_empty()),
        first_ts_ms: row.ts_ms,
        last_ts_ms: row.ts_ms,
        byte_len: event_row_body_len(row),
        checksum_sha256,
    })
}

fn agent_from_thread(thread: &SessionRow) -> AgentName {
    match thread.agent.as_str() {
        "claude" => AgentName::Claude,
        "gemini" => AgentName::Gemini,
        "opencode" => AgentName::Opencode,
        "grok" => AgentName::Grok,
        _ => AgentName::Codex,
    }
}

fn event_id(host_id: DeviceId, session_id: &str, seq: u64) -> String {
    format!("{}:{session_id}:{seq}", host_id.0)
}

fn checksum_raw_body(body: &RawBody) -> String {
    match body {
        RawBody::InlineBytes { bytes, .. } => hex_sha256(bytes),
        RawBody::Artifact { artifact } => artifact.sha256.clone(),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn payload_from_raw_body(body: &RawBody) -> Value {
    match body {
        RawBody::InlineBytes { bytes, media_type } if media_type == "application/json" => {
            serde_json::from_slice(bytes).unwrap_or(Value::Null)
        }
        RawBody::InlineBytes { bytes, media_type } => json!({
            "kind": "inline_bytes",
            "media_type": media_type,
            "base64": BASE64_STANDARD.encode(bytes),
        }),
        RawBody::Artifact { artifact } => payload_from_artifact(artifact),
    }
}

fn payload_from_event_row(row: &EventRow) -> Value {
    if let Some(bytes) = row.body_inline.as_deref() {
        return serde_json::from_slice(bytes).unwrap_or_else(|_| {
            json!({
                "kind": "inline_bytes",
                "media_type": row.artifact_media_type.as_deref().unwrap_or("application/octet-stream"),
                "base64": BASE64_STANDARD.encode(bytes),
            })
        });
    }
    json!({
        "kind": "artifact",
        "artifact_id": row.artifact_id,
        "size_bytes": row.artifact_size_bytes.unwrap_or_default(),
        "sha256": row.artifact_sha256,
        "media_type": row.artifact_media_type,
    })
}

fn payload_from_artifact(artifact: &ArtifactRef) -> Value {
    json!({
        "kind": "artifact",
        "artifact_id": artifact.artifact_id,
        "size_bytes": artifact.size_bytes,
        "sha256": artifact.sha256,
        "media_type": artifact.media_type,
    })
}

fn event_row_body_len(row: &EventRow) -> u64 {
    if let Some(bytes) = row.body_inline.as_ref() {
        return bytes.len() as u64;
    }
    row.artifact_size_bytes.unwrap_or_default().max(0) as u64
}
