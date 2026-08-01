//! Mobile HTTP smoke tests (Host Link era).
//!
//! QR pairing confirm/redeem flows were removed. Bind is Host Link on Desktop;
//! mobile uses account bearer + `/v1/hosts` list. Remaining coverage lives in
//! auth/session tests.

#![allow(clippy::duration_suboptimal_units)]

// Placeholder: QR pair confirm tests deleted with pairing stack.
// Add Host Link-aware mobile host-list smoke when D05 projection lands.
