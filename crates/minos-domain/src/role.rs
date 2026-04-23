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
    /// macOS host daemon (one per account at MVP).
    MacHost,
    /// iOS client app.
    IosClient,
    /// Browser-based admin console.
    BrowserAdmin,
}

impl fmt::Display for DeviceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MacHost => "mac-host",
            Self::IosClient => "ios-client",
            Self::BrowserAdmin => "browser-admin",
        })
    }
}

impl FromStr for DeviceRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mac-host" => Ok(Self::MacHost),
            "ios-client" => Ok(Self::IosClient),
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
        assert_eq!(DeviceRole::MacHost.to_string(), "mac-host");
        assert_eq!(DeviceRole::IosClient.to_string(), "ios-client");
        assert_eq!(DeviceRole::BrowserAdmin.to_string(), "browser-admin");
    }

    #[test]
    fn from_str_round_trips_display() {
        for role in [
            DeviceRole::MacHost,
            DeviceRole::IosClient,
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
            serde_json::to_string(&DeviceRole::MacHost).unwrap(),
            "\"mac-host\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceRole::IosClient).unwrap(),
            "\"ios-client\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceRole::BrowserAdmin).unwrap(),
            "\"browser-admin\""
        );

        let back: DeviceRole = serde_json::from_str("\"ios-client\"").unwrap();
        assert_eq!(back, DeviceRole::IosClient);
    }

    #[test]
    fn json_and_display_agree() {
        // Catch any drift between the serde rename and the manual Display.
        for role in [
            DeviceRole::MacHost,
            DeviceRole::IosClient,
            DeviceRole::BrowserAdmin,
        ] {
            let from_serde = serde_json::to_string(&role).unwrap();
            let expected = format!("\"{role}\"");
            assert_eq!(from_serde, expected, "mismatch for {role:?}");
        }
    }
}
