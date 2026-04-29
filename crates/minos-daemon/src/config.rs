//! Compile-time backend URL + runtime Relay configuration. See spec §10.1.

/// Compile-time backend URL. Overridable via `MINOS_BACKEND_URL` env var at build.
/// Fallback is the local dev backend (`cargo run -p minos-backend`).
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => minos_domain::defaults::DEV_BACKEND_URL,
};

/// Runtime relay config. Callers can override the backend URL and optional
/// CF Service Token pair at bootstrap time; blank values fall back to the
/// baked-in defaults.
///
/// Derives `uniffi::Record` so Swift can pass it to
/// `DaemonHandle::start`; the String fields marshal as plain strings.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub backend_url: String,
    pub cf_client_id: String,
    pub cf_client_secret: String,
}

impl RelayConfig {
    pub fn new(backend_url: String, cf_client_id: String, cf_client_secret: String) -> Self {
        Self {
            backend_url,
            cf_client_id,
            cf_client_secret,
        }
    }

    #[must_use]
    pub fn resolved_backend_url(&self) -> &str {
        let trimmed = self.backend_url.trim();
        if trimmed.is_empty() {
            BACKEND_URL
        } else {
            trimmed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_has_a_sane_fallback() {
        // With no MINOS_BACKEND_URL at test-build time, BACKEND_URL must
        // fall back to the shared dev constant from minos-domain.
        assert_eq!(BACKEND_URL, minos_domain::defaults::DEV_BACKEND_URL);
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn relay_config_ctor_stores_fields() {
        let c = RelayConfig::new("wss://backend/devices".into(), "id".into(), "secret".into());
        assert_eq!(c.backend_url, "wss://backend/devices");
        assert_eq!(c.cf_client_id, "id");
        assert_eq!(c.cf_client_secret, "secret");
    }

    #[test]
    fn relay_config_uses_baked_backend_when_runtime_value_is_blank() {
        let c = RelayConfig::new("   ".into(), "id".into(), "secret".into());
        assert_eq!(c.resolved_backend_url(), BACKEND_URL);
    }
}
