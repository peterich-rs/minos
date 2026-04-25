//! Compile-time backend URL + runtime Relay configuration. See spec §10.1.

/// Compile-time backend URL. Overridable via `MINOS_BACKEND_URL` env var at build.
/// Fallback is the local dev backend (`cargo run -p minos-backend`).
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};

/// Runtime relay config (optional CF Service Token pair). Backend URL is
/// BACKEND_URL (compile-time).
///
/// Derives `uniffi::Record` so Swift can pass it to
/// `DaemonHandle::start`; the two String fields marshal as plain strings.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub cf_client_id: String,
    pub cf_client_secret: String,
}

impl RelayConfig {
    pub fn new(cf_client_id: String, cf_client_secret: String) -> Self {
        Self {
            cf_client_id,
            cf_client_secret,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_has_a_sane_fallback() {
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn relay_config_ctor_stores_fields() {
        let c = RelayConfig::new("id".into(), "secret".into());
        assert_eq!(c.cf_client_id, "id");
        assert_eq!(c.cf_client_secret, "secret");
    }
}
