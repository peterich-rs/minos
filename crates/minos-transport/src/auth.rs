//! Client-side auth header bundle attached to the WS handshake.
//!
//! Headers covered:
//! - `X-Device-Id` (required, UUID string)
//! - `X-Device-Role` (required; kebab-case, see [`minos_domain::DeviceRole`])
//! - `X-Device-Secret` (optional; present only after pairing)
//! - `X-Device-Name` (optional; first-connect display hint)

use minos_domain::{DeviceId, DeviceRole, DeviceSecret};

/// Bundle of headers the client stamps onto the WebSocket upgrade request.
#[derive(Debug, Clone)]
pub struct AuthHeaders {
    pub device_id: DeviceId,
    pub device_role: DeviceRole,
    pub device_secret: Option<DeviceSecret>,
    pub device_name: Option<String>,
}

impl AuthHeaders {
    #[must_use]
    pub fn new(device_id: DeviceId, device_role: DeviceRole) -> Self {
        Self {
            device_id,
            device_role,
            device_secret: None,
            device_name: None,
        }
    }

    #[must_use]
    pub fn with_secret(mut self, secret: DeviceSecret) -> Self {
        self.device_secret = Some(secret);
        self
    }

    #[must_use]
    pub fn with_secret_opt(mut self, secret: Option<DeviceSecret>) -> Self {
        self.device_secret = secret;
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.device_name = Some(name.into());
        self
    }

    /// Render as a lazy iterator of `(header_name, header_value)` tuples in
    /// a stable order: id, role, then any set-optional fields.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, String)> + '_ {
        let secret = self
            .device_secret
            .as_ref()
            .map(|s| ("X-Device-Secret", s.as_str().to_string()));
        let name = self
            .device_name
            .as_deref()
            .map(|n| ("X-Device-Name", n.to_string()));
        std::iter::once(("X-Device-Id", self.device_id.to_string()))
            .chain(std::iter::once((
                "X-Device-Role",
                self.device_role.to_string(),
            )))
            .chain(secret)
            .chain(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn sample() -> (DeviceId, DeviceRole) {
        (DeviceId::new(), DeviceRole::MobileClient)
    }

    #[test]
    fn auth_headers_new_contains_required_pair_only() {
        let (id, role) = sample();
        let headers = AuthHeaders::new(id, role);
        let rendered: Vec<_> = headers.iter().collect();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0].0, "X-Device-Id");
        assert_eq!(rendered[0].1, id.to_string());
        assert_eq!(rendered[1].0, "X-Device-Role");
        assert_eq!(rendered[1].1, "mobile-client");
    }

    #[test]
    fn with_secret_adds_x_device_secret() {
        let (id, role) = sample();
        let secret = DeviceSecret("plain-value-abc".to_owned());
        let headers = AuthHeaders::new(id, role).with_secret(secret);
        let entry = headers
            .iter()
            .find(|(k, _)| *k == "X-Device-Secret")
            .expect("X-Device-Secret present");
        assert_eq!(entry.1, "plain-value-abc");
    }

    #[test]
    fn with_name_adds_x_device_name() {
        let (id, role) = sample();
        let headers = AuthHeaders::new(id, role).with_name("Fan's iPhone");
        let entry = headers
            .iter()
            .find(|(k, _)| *k == "X-Device-Name")
            .expect("X-Device-Name present");
        assert_eq!(entry.1, "Fan's iPhone");
    }

    #[test]
    fn device_role_is_kebab_case_in_header() {
        for (role, expected) in [
            (DeviceRole::AgentHost, "agent-host"),
            (DeviceRole::MobileClient, "mobile-client"),
            (DeviceRole::BrowserAdmin, "browser-admin"),
        ] {
            let headers = AuthHeaders::new(DeviceId::new(), role);
            let entry = headers
                .iter()
                .find(|(k, _)| *k == "X-Device-Role")
                .expect("X-Device-Role present");
            assert_eq!(entry.1, expected);
        }
    }

    #[test]
    fn device_secret_as_header_contains_plaintext() {
        let (id, role) = sample();
        let sentinel = "super-secret-42";
        let headers = AuthHeaders::new(id, role).with_secret(DeviceSecret(sentinel.to_owned()));
        let entry = headers
            .iter()
            .find(|(k, _)| *k == "X-Device-Secret")
            .expect("X-Device-Secret present");
        assert_eq!(entry.1, sentinel);
        assert!(
            !entry.1.contains("redacted"),
            "must not leak Display redaction into header: {}",
            entry.1
        );
    }

    #[test]
    fn with_secret_opt_some_is_equivalent_to_with_secret() {
        let (id, role) = sample();
        let s = DeviceSecret("xyz".into());
        let a = AuthHeaders::new(id, role).with_secret(s.clone());
        let b = AuthHeaders::new(id, role).with_secret_opt(Some(s));
        let av: Vec<_> = a.iter().collect();
        let bv: Vec<_> = b.iter().collect();
        assert_eq!(av, bv);
    }

    #[test]
    fn with_secret_opt_none_omits_x_device_secret() {
        let (id, role) = sample();
        let headers = AuthHeaders::new(id, role).with_secret_opt(None);
        assert!(
            headers.iter().all(|(k, _)| k != "X-Device-Secret"),
            "with_secret_opt(None) must not stamp X-Device-Secret"
        );
    }

    #[test]
    fn full_bundle_has_stable_order() {
        let (id, role) = sample();
        let headers = AuthHeaders::new(id, role)
            .with_secret(DeviceSecret("s".to_owned()))
            .with_name("n");
        let keys: Vec<_> = headers.iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![
                "X-Device-Id",
                "X-Device-Role",
                "X-Device-Secret",
                "X-Device-Name",
            ]
        );
    }
}
