//! `AgentEvent` lives in `minos-domain::events`; this module re-exports it so
//! downstream crates (daemon, frb adapter, mobile) can keep their existing
//! `minos_protocol::AgentEvent` imports unchanged. See `minos_domain::events`
//! for the canonical type definition and serde golden test.

pub use minos_domain::events::*;

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
