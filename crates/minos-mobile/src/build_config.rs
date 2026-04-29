//! Build-time configuration baked into the mobile binary.
//!
//! All three values are populated by `option_env!` at compile time, sourced
//! from shell env vars (`MINOS_BACKEND_URL`, `CF_ACCESS_CLIENT_ID`,
//! `CF_ACCESS_CLIENT_SECRET`). Cargokit's `build_pod.sh` runs `cargo build`
//! with `includeParentEnvironment: true`, so the same shell that invokes
//! `flutter build ios` propagates these into the Rust compile.
//!
//! The companion `build.rs` declares `rerun-if-env-changed` for all three so
//! cargo's incremental cache invalidates when values change between builds.
//!
//! These constants replace per-pairing storage of `backend_url` and CF Access
//! tokens — the values live at the application edge (transport headers, WS
//! upgrade target) and never enter business logic or durable state.

/// Backend WebSocket URL the mobile client opens. Defaults to the local dev
/// backend when no env override is present at build time.
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};

/// Optional Cloudflare Access service-token client id. `Some(..)` only when
/// `CF_ACCESS_CLIENT_ID` was set at compile time.
pub const CF_ACCESS_CLIENT_ID: Option<&str> = option_env!("CF_ACCESS_CLIENT_ID");

/// Optional Cloudflare Access service-token client secret. Paired with
/// [`CF_ACCESS_CLIENT_ID`].
pub const CF_ACCESS_CLIENT_SECRET: Option<&str> = option_env!("CF_ACCESS_CLIENT_SECRET");

/// Owned `(client_id, client_secret)` tuple ready for `MobileHttpClient::new`
/// and the WS upgrade header path. Returns `None` unless BOTH halves are
/// populated; a half-set pair is treated as misconfiguration and ignored
/// rather than silently sending one header.
#[must_use]
pub fn cf_access() -> Option<(String, String)> {
    match (CF_ACCESS_CLIENT_ID, CF_ACCESS_CLIENT_SECRET) {
        (Some(id), Some(sec)) => Some((id.to_string(), sec.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_has_a_sane_dev_fallback() {
        // The test build doesn't set MINOS_BACKEND_URL, so we expect the
        // local dev fallback. Asserting the literal here also pins the
        // contract: any future tweak to the fallback breaks this test.
        assert_eq!(BACKEND_URL, "ws://127.0.0.1:8787/devices");
        assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
    }

    #[test]
    fn cf_access_helper_matches_const_pair_state() {
        // Whether the env vars are set is environment-dependent (CI may set
        // them, dev shells may not). We test only the invariant: `cf_access()`
        // is `Some` iff BOTH halves are populated; a half-set pair must be
        // ignored as misconfiguration.
        match (CF_ACCESS_CLIENT_ID, CF_ACCESS_CLIENT_SECRET) {
            (Some(_), Some(_)) => {
                let pair = cf_access().expect("both halves set => Some");
                assert!(!pair.0.is_empty());
                assert!(!pair.1.is_empty());
            }
            _ => assert!(cf_access().is_none()),
        }
    }
}
