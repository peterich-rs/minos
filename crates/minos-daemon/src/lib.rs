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
pub use tailscale::discover_ip_with_reason as discover_tailscale_ip_with_reason;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[cfg(feature = "uniffi")]
mod uniffi_bridges {
    use minos_domain::DeviceId;
    use uuid::Uuid;

    uniffi::custom_type!(Uuid, String, {
        remote,
        lower: |uuid| uuid.to_string(),
        try_lift: |text| Uuid::parse_str(&text).map_err(Into::into),
    });

    uniffi::custom_type!(DeviceId, Uuid, {
        remote,
        lower: |device_id| device_id.0,
        try_lift: |uuid| Ok(DeviceId(uuid)),
    });
}
