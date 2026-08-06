#![forbid(unsafe_code)]

pub mod agent;
pub mod config;
pub mod conversation_completion;
pub mod device_secret_store;
pub mod git;
pub mod handle;
pub mod host_bootstrap_key_store;
pub mod ingest_chunk;
pub mod ingest_coalescer;
pub mod ingest_sync;
pub mod jsonl_recover;
pub mod local_rpc;
pub mod local_state;
pub mod logging;
pub mod media_materialize;
pub mod model_catalog;
mod openwire_trace;
pub mod paths;
pub mod relay_client;
pub mod relay_http;
pub mod relay_pairing;
pub mod roster;
pub mod rpc_server;
pub mod store;
pub mod subscription;

pub use agent::{AgentGlue, AgentSessionSnapshot};
pub use config::{RelayConfig, BACKEND_URL};
pub use handle::*;
pub use local_state::LocalState;
pub use minos_agent_runtime::SessionState;
pub use relay_client::RelayClient;
pub use relay_pairing::PeerRecord;
pub use subscription::{
    AgentStateObserver, ConnectionStateObserver, PeerStateObserver, RelayLinkStateObserver,
    Subscription,
};

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

// `DeviceId` / `DeviceSecret` are registered in `minos-domain` with blanket
// `impl<UT>` coverage under this crate's UniFFI tag.
//
// `DateTime<Utc>` is used by `PeerRecord` and is registered locally.
#[cfg(feature = "uniffi")]
mod uniffi_reexports {
    use chrono::{DateTime, Utc};
    use std::time::SystemTime;

    type DateTimeUtc = DateTime<Utc>;

    uniffi::custom_type!(DateTimeUtc, SystemTime, {
        remote,
        lower: |dt| dt.into(),
        try_lift: |st| Ok(st.into()),
    });
}
