//! Client-side auth header bundle attached to the WS handshake.
//!
//! Plan 04 §14 introduces this shape **add-only**: the existing
//! [`crate::client::WsClient`] is deliberately left untouched. Plan 05 will
//! rewire `WsClient::connect` to accept an `AuthHeaders` and stamp the
//! returned `(name, value)` pairs onto the tungstenite upgrade request.
//!
//! Headers covered (per spec §2.1 / §4.3 and plan §9):
//! - `X-Device-Id` (required, UUID string)
//! - `X-Device-Role` (required; kebab-case, see [`minos_domain::DeviceRole`])
//! - `X-Device-Secret` (optional; present only after pairing)
//! - `X-Device-Name` (optional; first-connect display hint)
//! - `CF-Access-Client-Id` + `CF-Access-Client-Secret` (optional; the
//!   Cloudflare Access service-token pair — validated at the edge, never
//!   reaches the relay process).

use minos_domain::{DeviceId, DeviceRole, DeviceSecret};

/// Cloudflare Access Service Token pair shipped to clients via config.
///
/// The two values are validated at the Cloudflare Access edge; the relay
/// itself never observes either field. Stored as plain `String`s because
/// they are public-per-client identifiers, not user secrets — rotating a
/// leaked token is a tenant-admin action in the Cloudflare dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfAccessToken {
    pub client_id: String,
    pub client_secret: String,
}

impl CfAccessToken {
    #[must_use]
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }
}

/// Bundle of headers the client stamps onto the WebSocket upgrade request.
///
/// Construct with [`AuthHeaders::new`] (just device id + role), then layer
/// on optional fields via the `with_*` builders. The resulting bundle is
/// rendered to `(header_name, header_value)` tuples by [`AuthHeaders::iter`];
/// plan 05's `WsClient::connect` will consume that iterator directly.
#[derive(Debug, Clone)]
pub struct AuthHeaders {
    pub device_id: DeviceId,
    pub device_role: DeviceRole,
    pub device_secret: Option<DeviceSecret>,
    pub device_name: Option<String>,
    pub cf_access: Option<CfAccessToken>,
}

impl AuthHeaders {
    /// Minimum shape: device id + role, no secret, no display name, no CF
    /// Access token. Suitable for a fresh unpaired device on the local
    /// network (where Cloudflare Access is not in front of the relay).
    #[must_use]
    pub fn new(device_id: DeviceId, device_role: DeviceRole) -> Self {
        Self {
            device_id,
            device_role,
            device_secret: None,
            device_name: None,
            cf_access: None,
        }
    }

    /// Attach the long-lived bearer secret minted at pair time. Mac-side
    /// callers continue to use this — the host rail still flashes
    /// `X-Device-Secret` post-ADR-0020.
    #[must_use]
    pub fn with_secret(mut self, secret: DeviceSecret) -> Self {
        self.device_secret = Some(secret);
        self
    }

    /// Like [`Self::with_secret`] but accepts an optional secret. iOS
    /// callers post-ADR-0020 pass `None` here so no `X-Device-Secret`
    /// header is stamped onto the upgrade request. Calling with `None`
    /// is functionally equivalent to omitting the call.
    #[must_use]
    pub fn with_secret_opt(mut self, secret: Option<DeviceSecret>) -> Self {
        self.device_secret = secret;
        self
    }

    /// Attach a display-name hint (first-connect only; ignored by the relay
    /// after the device row exists).
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.device_name = Some(name.into());
        self
    }

    /// Attach a Cloudflare Access service-token pair; validated at the edge
    /// so the relay process never sees either value.
    #[must_use]
    pub fn with_cf_access(mut self, token: CfAccessToken) -> Self {
        self.cf_access = Some(token);
        self
    }

    /// Render as a lazy iterator of `(header_name, header_value)` tuples in
    /// a stable order: id, role, then any set-optional fields.
    ///
    /// Note: `X-Device-Secret` is rendered via [`DeviceSecret::as_str`],
    /// **not** `Display` — `Display` is redacted by design. Callers therefore
    /// must not log the returned values directly.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, String)> + '_ {
        let secret = self
            .device_secret
            .as_ref()
            .map(|s| ("X-Device-Secret", s.as_str().to_string()));
        let name = self
            .device_name
            .as_deref()
            .map(|n| ("X-Device-Name", n.to_string()));
        let cf_id = self
            .cf_access
            .as_ref()
            .map(|c| ("CF-Access-Client-Id", c.client_id.clone()));
        let cf_sec = self
            .cf_access
            .as_ref()
            .map(|c| ("CF-Access-Client-Secret", c.client_secret.clone()));
        std::iter::once(("X-Device-Id", self.device_id.to_string()))
            .chain(std::iter::once((
                "X-Device-Role",
                self.device_role.to_string(),
            )))
            .chain(secret)
            .chain(name)
            .chain(cf_id)
            .chain(cf_sec)
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
    fn with_cf_access_adds_both_cf_headers() {
        let (id, role) = sample();
        let token = CfAccessToken::new("client-id.access", "client-secret-opaque");
        let headers = AuthHeaders::new(id, role).with_cf_access(token);
        let cf_id = headers
            .iter()
            .find(|(k, _)| *k == "CF-Access-Client-Id")
            .expect("CF-Access-Client-Id present");
        let cf_sec = headers
            .iter()
            .find(|(k, _)| *k == "CF-Access-Client-Secret")
            .expect("CF-Access-Client-Secret present");
        assert_eq!(cf_id.1, "client-id.access");
        assert_eq!(cf_sec.1, "client-secret-opaque");
    }

    #[test]
    fn device_role_is_kebab_case_in_header() {
        // Covers all three variants — the header value is the Display impl.
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
        // Guard against accidentally stamping Debug/Display (both redact).
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
    fn iter_includes_cf_access_only_when_set() {
        let (id, role) = sample();
        let without = AuthHeaders::new(id, role);
        assert!(without
            .iter()
            .all(|(k, _)| k != "CF-Access-Client-Id" && k != "CF-Access-Client-Secret"));

        let with = AuthHeaders::new(id, role).with_cf_access(CfAccessToken::new("id", "sec"));
        let keys: Vec<_> = with.iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"CF-Access-Client-Id"));
        assert!(keys.contains(&"CF-Access-Client-Secret"));
    }

    #[test]
    fn full_bundle_has_stable_order() {
        // Plan 05 expects id, role, secret, name, cf-id, cf-sec in that order.
        let (id, role) = sample();
        let headers = AuthHeaders::new(id, role)
            .with_secret(DeviceSecret("s".to_owned()))
            .with_name("n")
            .with_cf_access(CfAccessToken::new("cid", "csec"));
        let keys: Vec<_> = headers.iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![
                "X-Device-Id",
                "X-Device-Role",
                "X-Device-Secret",
                "X-Device-Name",
                "CF-Access-Client-Id",
                "CF-Access-Client-Secret",
            ]
        );
    }
}
