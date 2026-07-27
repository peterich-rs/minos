fn main() {
    println!("cargo:rerun-if-env-changed=MINOS_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=MINOS_UPDATER_ENDPOINT");
    println!("cargo:rustc-check-cfg=cfg(minos_updater_enabled)");

    // Release CI injects both env vars so the updater plugin is compiled in.
    // Local `tauri dev` / unsigned builds leave them unset → no updater plugin.
    let updater_public_key = std::env::var("MINOS_UPDATER_PUBLIC_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let updater_endpoint = std::env::var("MINOS_UPDATER_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if updater_public_key.is_some() && updater_endpoint.is_some() {
        println!("cargo:rustc-cfg=minos_updater_enabled");
    }

    tauri_build::build()
}
