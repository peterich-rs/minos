//! Compile-time backend URL + runtime Relay configuration. See spec §10.1.

/// Compile-time backend URL. Overridable via `MINOS_BACKEND_URL` env var at build.
/// Fallback is the local dev backend (`cargo run -p minos-backend`).
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => minos_domain::defaults::DEV_BACKEND_URL,
};

/// Runtime relay config. `backend_url` is the only live field.
///
/// Derives `uniffi::Record` so Swift can pass it to
/// `DaemonHandle::start`; the String field marshals as plain text.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub backend_url: String,
}

impl RelayConfig {
    pub fn new(backend_url: String) -> Self {
        Self { backend_url }
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
        // CI/dev builds may bake an explicit MINOS_BACKEND_URL; the only
        // invariant we rely on here is that the constant resolves to a
        // websocket URL.
        assert!(!BACKEND_URL.is_empty());
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn relay_config_ctor_stores_fields() {
        let c = RelayConfig::new("wss://backend/devices".into());
        assert_eq!(c.backend_url, "wss://backend/devices");
    }

    #[test]
    fn relay_config_uses_baked_backend_when_runtime_value_is_blank() {
        let c = RelayConfig::new("   ".into());
        assert_eq!(c.resolved_backend_url(), BACKEND_URL);
    }
}
