#![forbid(unsafe_code)]

pub mod state_machine;
pub mod store;
pub mod token;

pub use state_machine::*;
pub use store::*;
pub use token::*;

// UniFFI 0.31 per-crate scaffolding: every crate that carries `uniffi::*`
// derives must define `UniFfiTag` locally via `setup_scaffolding!()`; the
// derive expansions reference `crate::UniFfiTag`. Feature-gated so the
// non-UniFFI build path (plan-03 Dart/frb consumers) pays nothing.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
