//! Streaming event payload (placeholder for P1).
//!
//! The variant set is finalized here so producer crates added later need not
//! migrate consumers. MVP server returns a "not implemented" error from
//! `subscribe_events`; this enum is what *will* be streamed once
//! `minos-agent-runtime` lands in plan-equivalent for P1.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TokenChunk { text: String },
    ToolCall { name: String, args_json: String },
    ToolResult { name: String, output: String },
    Reasoning { text: String },
    Done { exit_code: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_chunk_serializes_with_type_tag() {
        let s = serde_json::to_string(&AgentEvent::TokenChunk { text: "hi".into() }).unwrap();
        assert_eq!(s, r#"{"type":"token_chunk","text":"hi"}"#);
    }
}
