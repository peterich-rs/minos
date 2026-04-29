// Cargo's incremental cache pins `option_env!` outputs to the env snapshot at
// the time of the last successful build. Without these declarations, changing
// MINOS_BACKEND_URL or CF_ACCESS_CLIENT_{ID,SECRET} between builds (e.g.
// dev → release) silently reuses the previously baked-in values. Declaring
// `rerun-if-env-changed` forces cargo to mark the crate dirty and recompile
// `build_config.rs` whenever any of the three change.
fn main() {
    println!("cargo:rerun-if-env-changed=MINOS_BACKEND_URL");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_SECRET");
}
