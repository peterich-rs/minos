//! High-level connection state visible to the UI.

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    /// Reconnect attempt in progress; `attempt` starts at 1 for the first retry.
    Reconnecting {
        attempt: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_serializes_as_string() {
        assert_eq!(
            serde_json::to_string(&ConnectionState::Disconnected).unwrap(),
            "\"disconnected\""
        );
    }

    #[test]
    fn reconnecting_carries_attempt() {
        let s = serde_json::to_string(&ConnectionState::Reconnecting { attempt: 3 }).unwrap();
        assert_eq!(s, r#"{"reconnecting":{"attempt":3}}"#);
    }
}
