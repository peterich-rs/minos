#![forbid(unsafe_code)]

pub mod agent;
pub mod config;
pub mod handle;
#[cfg(target_os = "macos")]
pub mod keychain_store;
pub mod local_state;
pub mod logging;
pub mod paths;
pub mod relay_client;
pub mod relay_pairing;
pub mod rpc_server;
pub mod subscription;

pub use agent::AgentGlue;
pub use config::{RelayConfig, BACKEND_URL};
pub use handle::*;
#[cfg(target_os = "macos")]
pub use keychain_store::KeychainTrustedDeviceStore;
pub use local_state::LocalState;
pub use minos_agent_runtime::AgentState;
pub use relay_client::RelayClient;
pub use relay_pairing::{PeerRecord, RelayQrPayload};
pub use subscription::{
    AgentStateObserver, ConnectionStateObserver, PeerStateObserver, RelayLinkStateObserver,
    Subscription,
};

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
    // `DeviceSecret`'s home registration in `minos-domain` uses the
    // `impl<UT>` blanket coverage, so it is already available under this
    // crate's `UniFfiTag` with no extra re-registration needed — same
    // pattern as `DeviceId`.
}
