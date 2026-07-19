use crate::client_request::AcpClientNotification;
use crate::types::SessionId;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CancelNotification {
    pub session_id: SessionId,
}

impl AcpClientNotification for CancelNotification {
    const METHOD: &'static str = "session/cancel";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_request::{
        AcpClientRequest, InitializeParams, NewSessionParams, PromptParams, PromptResponse,
    };
    use crate::types::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn initialize_method_constant() {
        assert_eq!(InitializeParams::METHOD, "initialize");
    }

    #[test]
    fn prompt_method_constant() {
        assert_eq!(PromptParams::METHOD, "session/prompt");
    }

    #[test]
    fn cancel_method_constant() {
        assert_eq!(CancelNotification::METHOD, "session/cancel");
    }

    #[test]
    fn new_session_params_serializes() {
        let params = NewSessionParams {
            cwd: "/workspace".into(),
            mcp_servers: vec![],
            additional_directories: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["cwd"], "/workspace");
    }

    #[test]
    fn prompt_response_deserializes_end_turn() {
        let json = r#"{"stopReason":"end_turn"}"#;
        let resp: PromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }
}
