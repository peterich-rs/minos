//! `DeviceRole` — classifies which side of the relay a device speaks from.
//!
//! Kebab-case string is the single wire format: it appears in DB rows, in
//! pairing payloads, and in relay envelopes. `Serialize`/`Deserialize`,
//! `Display` and `FromStr` all round-trip through the same set of literals
//! so a DB read-through-RPC-through-store loop is lossless.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceRole {
    /// Agent-host daemon (macOS today, platform-neutral name for future
    /// Linux/Windows ports). One per account at MVP.
    AgentHost,
    /// Mobile client app (iOS today, Android in the future).
    MobileClient,
    /// Browser-based admin console.
    BrowserAdmin,
}

impl fmt::Display for DeviceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AgentHost => "agent-host",
            Self::MobileClient => "mobile-client",
            Self::BrowserAdmin => "browser-admin",
        })
    }
}

impl FromStr for DeviceRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent-host" => Ok(Self::AgentHost),
            "mobile-client" => Ok(Self::MobileClient),
            "browser-admin" => Ok(Self::BrowserAdmin),
            other => Err(format!("unknown device role: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_kebab_case() {
        assert_eq!(DeviceRole::AgentHost.to_string(), "agent-host");
        assert_eq!(DeviceRole::MobileClient.to_string(), "mobile-client");
        assert_eq!(DeviceRole::BrowserAdmin.to_string(), "browser-admin");
    }

    #[test]
    fn from_str_round_trips_display() {
        for role in [
            DeviceRole::AgentHost,
            DeviceRole::MobileClient,
            DeviceRole::BrowserAdmin,
        ] {
            let wire = role.to_string();
            let back = DeviceRole::from_str(&wire).unwrap();
            assert_eq!(back, role, "round-trip failed for {role:?}");
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        let err = DeviceRole::from_str("host").unwrap_err();
        assert!(err.contains("unknown device role"));
        assert!(err.contains("host"));
    }

    #[test]
    fn json_is_kebab_case() {
        // serde kebab-case must agree with Display / FromStr.
        assert_eq!(
            serde_json::to_string(&DeviceRole::AgentHost).unwrap(),
            "\"agent-host\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceRole::MobileClient).unwrap(),
            "\"mobile-client\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceRole::BrowserAdmin).unwrap(),
            "\"browser-admin\""
        );

        let back: DeviceRole = serde_json::from_str("\"mobile-client\"").unwrap();
        assert_eq!(back, DeviceRole::MobileClient);
    }

    #[test]
    fn json_and_display_agree() {
        // Catch any drift between the serde rename and the manual Display.
        for role in [
            DeviceRole::AgentHost,
            DeviceRole::MobileClient,
            DeviceRole::BrowserAdmin,
        ] {
            let from_serde = serde_json::to_string(&role).unwrap();
            let expected = format!("\"{role}\"");
            assert_eq!(from_serde, expected, "mismatch for {role:?}");
        }
    }
}
