#![forbid(unsafe_code)]

pub mod file_store;
pub mod handle;
pub mod logging;
pub mod rpc_server;
pub mod subscription;
pub mod tailscale;

pub use file_store::*;
pub use handle::*;
pub use subscription::{ConnectionStateObserver, Subscription};

/// Module-level wrapper so callers don't need `tailscale::discover_ip` —
/// spec §5.1 #4 calls for this name.
pub use tailscale::discover_ip as discover_tailscale_ip;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
