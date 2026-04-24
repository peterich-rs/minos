#![forbid(unsafe_code)]

pub mod agent;
pub mod config;
pub mod file_store;
pub mod handle;
pub mod local_state;
pub mod logging;
pub mod paths;
pub mod relay_pairing;
pub mod rpc_server;
pub mod subscription;
pub mod tailscale;

pub use agent::AgentGlue;
pub use config::{RelayConfig, BACKEND_URL};
pub use file_store::*;
pub use handle::*;
pub use local_state::LocalState;
pub use minos_agent_runtime::AgentState;
pub use relay_pairing::{PeerRecord, RelayQrPayload};
pub use subscription::{AgentStateObserver, ConnectionStateObserver, Subscription};

/// Module-level wrapper so callers don't need `tailscale::discover_ip` —
/// spec §5.1 #4 calls for this name.
pub use tailscale::discover_ip as discover_tailscale_ip;
pub use tailscale::discover_ip_with_reason as discover_tailscale_ip_with_reason;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

// `DeviceId` is now registered in its home crate `minos-domain` with blanket
// `impl<UT>` coverage, which already applies to this crate's tag — no local
// registration needed here. If the daemon later exposes APIs that need a
// `Uuid` crossing UniFFI, reintroduce a dedicated bridge (and see the
// `minos-pairing` crate for the `remote custom_type!` pattern).
//
// `PairingToken` and `DateTime<Utc>` have their UniFFI custom_type!
// registrations in `minos-pairing` (under the `remote` keyword, which ties
// them to that crate's `UniFfiTag`). The relay-flow types in
// `relay_pairing.rs` use them inside `uniffi::Record` fields and therefore
// need the trait impls under this crate's own tag — pull them in with
// `use_remote_type!` rather than re-registering, to keep the single source
// of truth in `minos-pairing`.
#[cfg(feature = "uniffi")]
mod uniffi_reexports {
    uniffi::use_remote_type!(minos_pairing::minos_domain::PairingToken);
    uniffi::use_remote_type!(minos_pairing::chrono::DateTime<chrono::Utc>);
}
