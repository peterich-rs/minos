//! Compile-time default constants shared across crates.
//!
//! These exist so the same dev-fallback string isn't hardcoded in three
//! places that drift independently. Any new fallback that needs to be
//! identical between client crates belongs here.
//!
//! See `docs/superpowers/specs/unified-config-pipeline-design.md` §4.3.

/// Local backend URL used when `MINOS_BACKEND_URL` is unset at compile time.
/// Matches `--listen 127.0.0.1:8787` plus the `/devices` WebSocket path.
pub const DEV_BACKEND_URL: &str = "ws://127.0.0.1:8787/devices";

/// Default backend listen socket, mirrored by `MINOS_BACKEND_LISTEN`.
/// Used as the fallback in `crates/minos-backend/src/config.rs` clap default.
pub const DEV_BACKEND_LISTEN: &str = "127.0.0.1:8787";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_backend_url_uses_dev_listen_socket() {
        // Pin the relationship: the URL constant must encode the listen
        // constant, otherwise the two will drift.
        assert!(DEV_BACKEND_URL.contains(DEV_BACKEND_LISTEN));
    }

    #[test]
    fn dev_backend_url_is_a_websocket_url() {
        assert!(
            DEV_BACKEND_URL.starts_with("ws://"),
            "DEV_BACKEND_URL must be a ws:// URL for local dev"
        );
        assert!(
            DEV_BACKEND_URL.ends_with("/devices"),
            "DEV_BACKEND_URL path must terminate in /devices per backend route"
        );
    }
}
