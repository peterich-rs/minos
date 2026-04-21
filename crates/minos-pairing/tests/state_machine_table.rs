use minos_domain::{MinosError, PairingState};
use minos_pairing::Pairing;
use rstest::rstest;

#[rstest]
#[case::ok_unpaired_to_awaiting(PairingState::Unpaired, true)]
#[case::reject_awaiting(PairingState::AwaitingPeer, false)]
#[case::reject_paired(PairingState::Paired, false)]
fn begin_awaiting(#[case] from: PairingState, #[case] should_succeed: bool) {
    let mut p = Pairing::new(from);
    let r = p.begin_awaiting();
    assert_eq!(r.is_ok(), should_succeed, "from {from:?}");
    if should_succeed {
        assert_eq!(p.state(), PairingState::AwaitingPeer);
    } else {
        assert!(matches!(r, Err(MinosError::PairingStateMismatch { .. })));
        assert_eq!(p.state(), from);
    }
}

#[rstest]
#[case::ok_awaiting_to_paired(PairingState::AwaitingPeer, true)]
#[case::reject_unpaired(PairingState::Unpaired, false)]
#[case::reject_paired(PairingState::Paired, false)]
fn accept_peer(#[case] from: PairingState, #[case] should_succeed: bool) {
    let mut p = Pairing::new(from);
    let r = p.accept_peer();
    assert_eq!(r.is_ok(), should_succeed, "from {from:?}");
}

#[test]
fn forget_resets_to_unpaired_from_any_state() {
    for from in [
        PairingState::Unpaired,
        PairingState::AwaitingPeer,
        PairingState::Paired,
    ] {
        let mut p = Pairing::new(from);
        p.forget();
        assert_eq!(p.state(), PairingState::Unpaired);
    }
}

#[test]
fn replace_paired_returns_to_awaiting() {
    let mut p = Pairing::new(PairingState::Paired);
    p.replace().unwrap();
    assert_eq!(p.state(), PairingState::AwaitingPeer);
}

#[test]
fn replace_when_not_paired_errors() {
    let mut p = Pairing::new(PairingState::Unpaired);
    assert!(matches!(
        p.replace(),
        Err(MinosError::PairingStateMismatch { .. })
    ));
}
