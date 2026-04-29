// Cargo's incremental cache pins `option_env!` outputs to the env snapshot at
// the time of the last successful build. Without these declarations, changing
// MINOS_BACKEND_URL or CF_ACCESS_CLIENT_{ID,SECRET} between builds (e.g.
// dev → release) silently reuses the previously baked-in values. Declaring
// `rerun-if-env-changed` forces cargo to mark the crate dirty and recompile
// `build_config.rs` whenever any of the three change.
//
// Additionally surfaces missing-env diagnostics: a debug build with no
// MINOS_BACKEND_URL emits a cargo:warning so the silent localhost fallback
// is visible in the build log; a release build with no MINOS_BACKEND_URL
// panics, preventing the localhost-baking bug from shipping in a release
// artifact.
fn main() {
    println!("cargo:rerun-if-env-changed=MINOS_BACKEND_URL");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_SECRET");
    println!("cargo:rerun-if-env-changed=PROFILE");

    let profile = std::env::var("PROFILE").unwrap_or_default();
    let backend_url = std::env::var("MINOS_BACKEND_URL").ok();

    match (profile.as_str(), backend_url.is_some()) {
        ("release", false) => {
            panic!(
                "MINOS_BACKEND_URL is unset for a release build. Set it via \
                 .env.local and invoke `just build-mobile-rust ... release` \
                 or `just build-mobile-ios Release`."
            );
        }
        (_, false) => {
            println!(
                "cargo:warning=MINOS_BACKEND_URL unset (debug build) — \
                 minos-mobile is using the dev-fallback DEV_BACKEND_URL."
            );
        }
        _ => {}
    }
}
