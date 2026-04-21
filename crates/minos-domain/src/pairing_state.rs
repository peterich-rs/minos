//! Pairing-side state machine state (used both inside the pairing crate and
//! inside `MinosError` for diagnostic context).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Unpaired,
    AwaitingPeer,
    Paired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_peer_serializes_snake_case() {
        let s = serde_json::to_string(&PairingState::AwaitingPeer).unwrap();
        assert_eq!(s, "\"awaiting_peer\"");
    }
}
