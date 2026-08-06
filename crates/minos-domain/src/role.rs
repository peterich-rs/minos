//! `DeviceRole` — classifies which side of the relay a device speaks from.
//!
//! Kebab-case string is the single **wire** format (headers, JWT, envelopes).
//! Storage uses a shorter `installation_kind` vocabulary (`mobile` / `browser` /
//! `desktop` / `host`); map via [`DeviceRole::to_installation_kind`] /
//! [`DeviceRole::from_installation_kind`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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
    /// Desktop console (Tauri shell).
    DesktopConsole,
}

impl fmt::Display for DeviceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AgentHost => "agent-host",
            Self::MobileClient => "mobile-client",
            Self::BrowserAdmin => "browser-admin",
            Self::DesktopConsole => "desktop-console",
        })
    }
}

impl DeviceRole {
    #[must_use]
    pub const fn is_account_client(self) -> bool {
        matches!(
            self,
            Self::MobileClient | Self::BrowserAdmin | Self::DesktopConsole
        )
    }

    /// Map wire role → `device_installations.kind` / `installation_kind`.
    #[must_use]
    pub const fn to_installation_kind(self) -> &'static str {
        match self {
            Self::AgentHost => "host",
            Self::MobileClient => "mobile",
            Self::BrowserAdmin => "browser",
            Self::DesktopConsole => "desktop",
        }
    }

    /// Map storage `kind` → wire role.
    pub fn from_installation_kind(kind: &str) -> Result<Self, String> {
        match kind {
            "host" => Ok(Self::AgentHost),
            "mobile" => Ok(Self::MobileClient),
            "browser" => Ok(Self::BrowserAdmin),
            "desktop" => Ok(Self::DesktopConsole),
            other => Err(format!("unknown installation kind: {other}")),
        }
    }
}

impl FromStr for DeviceRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent-host" => Ok(Self::AgentHost),
            "mobile-client" => Ok(Self::MobileClient),
            "browser-admin" => Ok(Self::BrowserAdmin),
            "desktop-console" => Ok(Self::DesktopConsole),
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
        assert_eq!(DeviceRole::DesktopConsole.to_string(), "desktop-console");
    }

    #[test]
    fn from_str_round_trips_display() {
        for role in [
            DeviceRole::AgentHost,
            DeviceRole::MobileClient,
            DeviceRole::BrowserAdmin,
            DeviceRole::DesktopConsole,
        ] {
            let wire = role.to_string();
            let back = DeviceRole::from_str(&wire).unwrap();
            assert_eq!(back, role, "round-trip failed for {role:?}");
        }
    }

    #[test]
    fn installation_kind_round_trips() {
        for role in [
            DeviceRole::AgentHost,
            DeviceRole::MobileClient,
            DeviceRole::BrowserAdmin,
            DeviceRole::DesktopConsole,
        ] {
            let kind = role.to_installation_kind();
            let back = DeviceRole::from_installation_kind(kind).unwrap();
            assert_eq!(back, role, "kind round-trip failed for {role:?}");
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
        assert_eq!(
            serde_json::to_string(&DeviceRole::DesktopConsole).unwrap(),
            "\"desktop-console\""
        );

        let back: DeviceRole = serde_json::from_str("\"mobile-client\"").unwrap();
        assert_eq!(back, DeviceRole::MobileClient);
    }

    #[test]
    fn json_and_display_agree() {
        for role in [
            DeviceRole::AgentHost,
            DeviceRole::MobileClient,
            DeviceRole::BrowserAdmin,
            DeviceRole::DesktopConsole,
        ] {
            let from_serde = serde_json::to_string(&role).unwrap();
            let expected = format!("\"{role}\"");
            assert_eq!(from_serde, expected, "mismatch for {role:?}");
        }
    }

    #[test]
    fn desktop_is_account_client() {
        assert!(DeviceRole::DesktopConsole.is_account_client());
        assert!(!DeviceRole::AgentHost.is_account_client());
    }
}
