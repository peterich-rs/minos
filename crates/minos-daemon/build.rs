// See crates/minos-mobile/build.rs for the rationale. This file mirrors
// the mobile FFI's env-tracking + release fail-fast for the daemon binary.
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
                 .env.local and invoke `just build-daemon release`."
            );
        }
        (_, false) => {
            println!(
                "cargo:warning=MINOS_BACKEND_URL unset (debug build) — \
                 minos-daemon is using the dev-fallback DEV_BACKEND_URL."
            );
        }
        _ => {}
    }
}
