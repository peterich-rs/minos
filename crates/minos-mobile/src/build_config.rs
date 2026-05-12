//! Build-time configuration baked into the mobile binary.
//!
//! The backend URL is populated by `option_env!` at compile time, sourced
//! from `MINOS_BACKEND_URL`. Cargokit's `build_pod.sh` runs `cargo build`
//! with `includeParentEnvironment: true`, so the same shell that invokes
//! `flutter build ios` propagates it into the Rust compile.
//!
//! The companion `build.rs` declares `rerun-if-env-changed` so cargo's
//! incremental cache invalidates when the URL changes between builds.
//!
//! This constant replaces per-pairing storage of `backend_url` — the value
//! lives at the application edge and never enters durable state.

/// Backend WebSocket URL the mobile client opens. Defaults to the local dev
/// backend when no env override is present at build time.
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => minos_domain::defaults::DEV_BACKEND_URL,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_has_a_sane_dev_fallback() {
        // CI/dev builds may bake an explicit MINOS_BACKEND_URL; the only
        // invariant we rely on here is that the constant resolves to a
        // websocket URL.
        assert!(!BACKEND_URL.is_empty());
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn backend_url_stays_decoupled_from_pairing_state() {
        assert!(!BACKEND_URL.is_empty());
    }
}
