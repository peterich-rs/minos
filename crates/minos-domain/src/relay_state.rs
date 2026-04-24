//! Relay client-side state axes. Two independent enums — link (to relay)
//! and peer (to paired iPhone). See spec §4.3.

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelayLinkState {
    Disconnected,
    Connecting { attempt: u32 },
    Connected,
}
