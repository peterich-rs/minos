//! Streaming event payload for `subscribe_events`.
//!
//! Relocated here from `minos-protocol::events` in plan 04 Phase B so that
//! `minos-agent-runtime` — which has no `minos-protocol` dep — can import
//! `AgentEvent` from the same place it imports `AgentName`. `minos-protocol`
//! retains a single-line re-export so downstream crates (daemon, frb adapter)
//! keep their existing `minos_protocol::AgentEvent` imports.
//!
//! The variant set is finalized here so producer crates added later need not
//! migrate consumers. MVP server returns a "not implemented" error from
//! `subscribe_events`; this enum is what *will* be streamed once
//! `minos-agent-runtime` lands in plan-equivalent for P1.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TokenChunk {
        text: String,
    },
    ToolCall {
        name: String,
        args_json: String,
    },
    ToolResult {
        name: String,
        output: String,
    },
    Reasoning {
        text: String,
    },
    Done {
        exit_code: i32,
    },
    /// Forward-compat escape hatch. `kind` is the codex method name
    /// (e.g. `"item/plan/delta"`), `payload_json` is the raw `params` object
    /// as a JSON-encoded string. Consumers may render nothing for unknown
    /// `kind`. See spec §5.2 and ADR 0010.
    Raw {
        kind: String,
        payload_json: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_chunk_serializes_with_type_tag() {
        let s = serde_json::to_string(&AgentEvent::TokenChunk { text: "hi".into() }).unwrap();
        assert_eq!(s, r#"{"type":"token_chunk","text":"hi"}"#);
    }

    #[test]
    fn raw_serializes_with_type_tag() {
        let s = serde_json::to_string(&AgentEvent::Raw {
            kind: "item/plan/delta".into(),
            payload_json: r#"{"step":"compile"}"#.into(),
        })
        .unwrap();
        assert_eq!(
            s,
            r#"{"type":"raw","kind":"item/plan/delta","payload_json":"{\"step\":\"compile\"}"}"#
        );
    }
}
