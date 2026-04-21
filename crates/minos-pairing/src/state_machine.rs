//! State machine: `Unpaired -> AwaitingPeer -> Paired`.
//!
//! Illegal transitions return `MinosError::PairingStateMismatch`.

use minos_domain::{MinosError, PairingState};

#[derive(Debug, Clone)]
pub struct Pairing {
    state: PairingState,
}

impl Pairing {
    #[must_use]
    pub fn new(initial: PairingState) -> Self {
        Self { state: initial }
    }

    #[must_use]
    pub fn state(&self) -> PairingState {
        self.state
    }

    /// Begin awaiting a peer (i.e., a QR has been displayed).
    pub fn begin_awaiting(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::Unpaired => {
                self.state = PairingState::AwaitingPeer;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }

    /// Accept a peer's pair RPC.
    pub fn accept_peer(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::AwaitingPeer => {
                self.state = PairingState::Paired;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }

    /// Forget current peer (UI "forget device" or corrupt-store reset).
    pub fn forget(&mut self) {
        self.state = PairingState::Unpaired;
    }

    /// Replace current paired peer (user confirmed "replace existing").
    pub fn replace(&mut self) -> Result<(), MinosError> {
        match self.state {
            PairingState::Paired => {
                self.state = PairingState::AwaitingPeer;
                Ok(())
            }
            other => Err(MinosError::PairingStateMismatch { actual: other }),
        }
    }
}
