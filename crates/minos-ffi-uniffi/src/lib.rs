//! UniFFI surface for Swift. Plan 02 fills in the actual `#[uniffi::export]`
//! annotations on `DaemonHandle`. This crate currently exists only to ensure
//! it compiles under the workspace and reserves its name on disk.

#![allow(unused_imports)]

use minos_daemon::DaemonHandle;
use minos_domain::MinosError;

uniffi::setup_scaffolding!();

/// Sentinel function so the scaffolding has at least one symbol to bind.
/// Removed in plan 02.
#[uniffi::export]
pub fn ping() -> String {
    "minos-ffi-uniffi alive".into()
}
