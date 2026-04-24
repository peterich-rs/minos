use minos_domain::RelayLinkState;

#[test]
fn relay_link_state_disconnected_serde_round_trip() {
    let state = RelayLinkState::Disconnected;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#""disconnected""#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn relay_link_state_connecting_carries_attempt() {
    let state = RelayLinkState::Connecting { attempt: 3 };
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#"{"connecting":{"attempt":3}}"#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn relay_link_state_connected_serde_round_trip() {
    let state = RelayLinkState::Connected;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#""connected""#);
    let back: RelayLinkState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}
