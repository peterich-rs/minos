#![forbid(unsafe_code)]
// `Duration::from_mins` was stabilized in Rust 1.84. To keep the crate's
// MSRV portable for the Flutter / iOS toolchain (which often pins an
// older stable), we deliberately use `Duration::from_secs(N * 60)` style
// throughout; the clippy `duration_suboptimal_units` lint would otherwise
// keep nudging us back to `from_mins`.
#![allow(clippy::duration_suboptimal_units)]

pub mod auth;
pub mod client;
pub mod http;
pub mod log_capture;
pub mod logging;
mod reconnect;
pub mod rpc;
pub mod store;

pub use client::*;
pub(crate) use reconnect::ReconnectController;
pub use store::*;
