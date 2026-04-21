//! Minos build / codegen orchestration.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// fmt + clippy + workspace tests + UI lints (UI lints are no-ops until plans 02/03 land).
    CheckAll,
    /// Install developer-side codegen tools.
    Bootstrap,
    /// Generate Swift bindings via uniffi-bindgen-swift.
    GenUniffi,
    /// Generate Dart bindings via flutter_rust_bridge_codegen. (Implemented in plan 03.)
    GenFrb,
    /// Build the universal macOS static library for the Swift app.
    BuildMacos,
    /// Build iOS staticlib from minos-ffi-frb. (Implemented in plan 03.)
    BuildIos,
    /// Generate apps/macos/Minos.xcodeproj from apps/macos/project.yml.
    GenXcode,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckAll => check_all(),
        Cmd::Bootstrap => bootstrap(),
        Cmd::GenUniffi => gen_uniffi(),
        Cmd::GenFrb => not_yet("gen-frb"),
        Cmd::BuildMacos => build_macos(),
        Cmd::BuildIos => not_yet("build-ios"),
        Cmd::GenXcode => gen_xcode(),
    }
}

fn check_all() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> cargo fmt --check");
    run("cargo", &["fmt", "--all", "--check"], &workspace_root)?;

    eprintln!("==> cargo clippy");
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &workspace_root,
    )?;

    eprintln!("==> cargo test");
    run("cargo", &["test", "--workspace"], &workspace_root)?;

    eprintln!("==> cargo deny check (licenses + advisories)");
    if which("cargo-deny").is_some() {
        run("cargo", &["deny", "check"], &workspace_root)?;
    } else {
        eprintln!("    (skipped: cargo-deny not installed; run `cargo xtask bootstrap`)");
    }

    if cfg!(target_os = "macos") {
        eprintln!("==> cargo xtask gen-uniffi");
        gen_uniffi()?;

        eprintln!("==> cargo xtask gen-xcode");
        gen_xcode()?;

        if which("swiftlint").is_none() {
            bail!("swiftlint not installed; run `cargo xtask bootstrap`");
        }

        let macos_root = workspace_root.join("apps/macos");

        eprintln!("==> xcodebuild -scheme Minos build");
        run(
            "xcodebuild",
            &[
                "-project",
                "Minos.xcodeproj",
                "-scheme",
                "Minos",
                "-destination",
                "platform=macOS",
                "-configuration",
                "Debug",
                "build",
            ],
            &macos_root,
        )?;

        eprintln!("==> xcodebuild -scheme MinosTests test");
        run(
            "xcodebuild",
            &[
                "-project",
                "Minos.xcodeproj",
                "-scheme",
                "MinosTests",
                "-destination",
                "platform=macOS",
                "-configuration",
                "Debug",
                "test",
            ],
            &macos_root,
        )?;

        eprintln!("==> swiftlint --strict");
        run("swiftlint", &["--strict"], &macos_root)?;
    } else {
        eprintln!("==> swift leg: skipped (non-macOS host)");
    }

    eprintln!("OK: all checks pass.");
    Ok(())
}

fn bootstrap() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> installing cargo-deny + uniffi (cli feature)");
    run(
        "cargo",
        &["install", "cargo-deny", "--locked"],
        &workspace_root,
    )?;
    run(
        "cargo",
        &["install", "uniffi", "--locked", "--features", "cli"],
        &workspace_root,
    )?;
    ensure_uniffi_bindgen_swift_wrapper()?;

    if cfg!(target_os = "macos") {
        let brewfile = workspace_root.join("apps/macos/Brewfile");
        if brewfile.exists() {
            if which("brew").is_none() {
                bail!("brew not installed; required to install xcodegen and swiftlint");
            }

            eprintln!("==> brew bundle --file {}", brewfile.display());
            run(
                "brew",
                &["bundle", "--file", brewfile.to_str().unwrap()],
                &workspace_root,
            )?;
        } else {
            eprintln!("    (skipped: {} missing)", brewfile.display());
        }
    }

    // flutter_rust_bridge_codegen and dart deps come in plan 03.
    Ok(())
}

fn build_macos() -> Result<()> {
    const MACOS_DEPLOYMENT_TARGET: &str = "13.0";

    if !cfg!(target_os = "macos") {
        bail!("`build-macos` requires a macOS host");
    }

    let root = workspace_root()?;
    if which("lipo").is_none() {
        bail!("lipo not installed; `build-macos` requires Xcode command-line tools");
    }

    eprintln!("==> cargo build-macos: arm64 + x86_64 staticlib -> lipo universal");
    for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        eprintln!("  target: {target}");
        run_env(
            "cargo",
            &[
                "build",
                "-p",
                "minos-ffi-uniffi",
                "--release",
                "--target",
                target,
            ],
            &[("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET)],
            &root,
        )?;
    }

    let out_dir = root.join("target/xcframework");
    fs::create_dir_all(&out_dir).with_context(|| format!("mkdir {}", out_dir.display()))?;

    let out_lib = out_dir.join("libminos_ffi_uniffi.a");
    let arm64 = root.join("target/aarch64-apple-darwin/release/libminos_ffi_uniffi.a");
    let x86_64 = root.join("target/x86_64-apple-darwin/release/libminos_ffi_uniffi.a");

    eprintln!("==> lipo -create -> {}", out_lib.display());
    run(
        "lipo",
        &[
            "-create",
            arm64.to_str().unwrap(),
            x86_64.to_str().unwrap(),
            "-output",
            out_lib.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!("==> lipo -info (verification)");
    run("lipo", &["-info", out_lib.to_str().unwrap()], &root)?;

    eprintln!("OK: {} (universal)", out_lib.display());
    Ok(())
}

fn gen_uniffi() -> Result<()> {
    let root = workspace_root()?;
    let out_dir = root.join("apps/macos/Minos/Generated");
    ensure_macos_scaffold_dirs(&root)?;

    if which("uniffi-bindgen-swift").is_none() {
        bail!("uniffi-bindgen-swift not installed; run `cargo xtask bootstrap`");
    }

    eprintln!("==> cargo build (host arch) -p minos-ffi-uniffi --release");
    run(
        "cargo",
        &["build", "-p", "minos-ffi-uniffi", "--release"],
        &root,
    )?;

    let dylib = root
        .join("target/release")
        .join(format!("libminos_ffi_uniffi.{}", host_dylib_suffix()));
    if !dylib.exists() {
        bail!("expected built library at {}", dylib.display());
    }

    eprintln!(
        "==> uniffi-bindgen-swift --swift-sources {}",
        dylib.display()
    );
    run(
        "uniffi-bindgen-swift",
        &[
            "--swift-sources",
            dylib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!("==> uniffi-bindgen-swift --headers {}", dylib.display());
    run(
        "uniffi-bindgen-swift",
        &[
            "--headers",
            dylib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!("==> uniffi-bindgen-swift --modulemap {}", dylib.display());
    run(
        "uniffi-bindgen-swift",
        &[
            "--modulemap",
            "--xcframework",
            "--module-name",
            "MinosCore",
            "--modulemap-filename",
            "MinosCoreFFI.modulemap",
            dylib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    normalize_generated_uniffi_imports(&out_dir)?;

    for generated in [
        "MinosCore.swift",
        "MinosCoreFFI.h",
        "MinosCoreFFI.modulemap",
    ] {
        let path = out_dir.join(generated);
        if !path.exists() {
            bail!("missing generated UniFFI artifact: {}", path.display());
        }
    }

    eprintln!("OK: {}", out_dir.display());
    Ok(())
}

fn normalize_generated_uniffi_imports(out_dir: &Path) -> Result<()> {
    const MODULE_IMPORT: &str = "#if canImport(MinosCoreFFI)\nimport MinosCoreFFI\n#endif";
    const MODULEMAP_DECL: &str = "framework module MinosCore {";
    const MODULEMAP_DECL_NORMALIZED: &str = "module MinosCoreFFI {";
    const MODULEMAP_DECL_ALREADY_NORMALIZED: &str = "framework module MinosCoreFFI {";
    const DUPLICATE_DAEMON_NEWTYPE_BLOCK: &str = "/**\n * Typealias from the type name used in the UDL file to the builtin type.  This\n * is needed because the UDL type name is used in function/method signatures.\n */\npublic typealias DeviceId = Uuid\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic struct FfiConverterTypeDeviceId: FfiConverter {\n    public static func read(from buf: inout (data: Data, offset: Data.Index)) throws -> DeviceId {\n        return try FfiConverterTypeUuid.read(from: &buf)\n    }\n\n    public static func write(_ value: DeviceId, into buf: inout [UInt8]) {\n        return FfiConverterTypeUuid.write(value, into: &buf)\n    }\n\n    public static func lift(_ value: RustBuffer) throws -> DeviceId {\n        return try FfiConverterTypeUuid_lift(value)\n    }\n\n    public static func lower(_ value: DeviceId) -> RustBuffer {\n        return FfiConverterTypeUuid_lower(value)\n    }\n}\n\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceId_lift(_ value: RustBuffer) throws -> DeviceId {\n    return try FfiConverterTypeDeviceId.lift(value)\n}\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceId_lower(_ value: DeviceId) -> RustBuffer {\n    return FfiConverterTypeDeviceId.lower(value)\n}\n\n\n\n/**\n * Typealias from the type name used in the UDL file to the builtin type.  This\n * is needed because the UDL type name is used in function/method signatures.\n */\npublic typealias Uuid = String\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic struct FfiConverterTypeUuid: FfiConverter {\n    public static func read(from buf: inout (data: Data, offset: Data.Index)) throws -> Uuid {\n        return try FfiConverterString.read(from: &buf)\n    }\n\n    public static func write(_ value: Uuid, into buf: inout [UInt8]) {\n        return FfiConverterString.write(value, into: &buf)\n    }\n\n    public static func lift(_ value: RustBuffer) throws -> Uuid {\n        return try FfiConverterString.lift(value)\n    }\n\n    public static func lower(_ value: Uuid) -> RustBuffer {\n        return FfiConverterString.lower(value)\n    }\n}\n\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeUuid_lift(_ value: RustBuffer) throws -> Uuid {\n    return try FfiConverterTypeUuid.lift(value)\n}\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeUuid_lower(_ value: Uuid) -> RustBuffer {\n    return FfiConverterTypeUuid.lower(value)\n}\n";

    let modulemap_path = out_dir.join("MinosCoreFFI.modulemap");
    if modulemap_path.exists() {
        let original = fs::read_to_string(&modulemap_path)
            .with_context(|| format!("reading {}", modulemap_path.display()))?;
        let updated = original
            .replace(MODULEMAP_DECL, MODULEMAP_DECL_NORMALIZED)
            .replace(MODULEMAP_DECL_ALREADY_NORMALIZED, MODULEMAP_DECL_NORMALIZED);
        if updated != original {
            fs::write(&modulemap_path, updated)
                .with_context(|| format!("writing {}", modulemap_path.display()))?;
        }
    }

    for file_name in [
        "minos_daemon.swift",
        "minos_domain.swift",
        "minos_pairing.swift",
    ] {
        let path = out_dir.join(file_name);
        if !path.exists() {
            continue;
        }

        let original =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        let updated = original
            .replace(
                "#if canImport(minos_daemonFFI)\nimport minos_daemonFFI\n#endif",
                MODULE_IMPORT,
            )
            .replace(
                "#if canImport(minos_domainFFI)\nimport minos_domainFFI\n#endif",
                MODULE_IMPORT,
            )
            .replace(
                "#if canImport(minos_pairingFFI)\nimport minos_pairingFFI\n#endif",
                MODULE_IMPORT,
            );

        let updated = if file_name == "minos_daemon.swift" {
            updated
                .replace(DUPLICATE_DAEMON_NEWTYPE_BLOCK, "")
                .replace(
                    "    static let vtablePtr: UnsafePointer<UniffiVTableCallbackInterfaceConnectionStateObserver> = {",
                    "    nonisolated(unsafe) static let vtablePtr: UnsafePointer<UniffiVTableCallbackInterfaceConnectionStateObserver> = {",
                )
        } else {
            updated
        };

        if updated != original {
            fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
        }
    }

    Ok(())
}

fn gen_xcode() -> Result<()> {
    let root = workspace_root()?;
    ensure_macos_scaffold_dirs(&root)?;

    if which("xcodegen").is_none() {
        bail!("xcodegen not installed; run `cargo xtask bootstrap`");
    }

    let spec = root.join("apps/macos/project.yml");
    if !spec.exists() {
        bail!("{} missing", spec.display());
    }

    eprintln!("==> xcodegen generate --spec {}", spec.display());
    run(
        "xcodegen",
        &["generate", "--spec", spec.to_str().unwrap()],
        &root.join("apps/macos"),
    )?;

    let project = root.join("apps/macos/Minos.xcodeproj");
    if !project.exists() {
        bail!("xcodegen did not produce {}", project.display());
    }

    eprintln!("OK: {}", project.display());
    Ok(())
}

fn not_yet(name: &str) -> Result<()> {
    bail!("xtask `{name}` not implemented yet (filled in later)")
}

fn ensure_macos_scaffold_dirs(root: &Path) -> Result<()> {
    for dir in [
        root.join("apps/macos/Minos"),
        root.join("apps/macos/Minos/Generated"),
        root.join("apps/macos/Minos/Resources/Assets.xcassets"),
        root.join("apps/macos/MinosTests"),
    ] {
        fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    Ok(())
}

fn host_dylib_suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    }
}

fn ensure_uniffi_bindgen_swift_wrapper() -> Result<()> {
    if which("uniffi-bindgen-swift").is_some() {
        return Ok(());
    }

    let cargo_bin = cargo_bin_dir()?;
    let bindgen = cargo_bin.join("uniffi-bindgen");
    if !bindgen.exists() {
        bail!("expected installed uniffi-bindgen at {}", bindgen.display());
    }

    let wrapper = cargo_bin.join("uniffi-bindgen-swift");
    let script = format!(
        "#!/usr/bin/env sh\nexec \"{}\" generate --language swift \"$@\"\n",
        bindgen.display()
    );
    fs::write(&wrapper, script).with_context(|| format!("writing {}", wrapper.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(&wrapper)
            .with_context(|| format!("stat {}", wrapper.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper, perms)
            .with_context(|| format!("chmod {}", wrapper.display()))?;
    }

    Ok(())
}

fn cargo_bin_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(home).join("bin"));
    }

    let home = std::env::var_os("HOME").context("HOME unset and CARGO_HOME unset")?;
    Ok(PathBuf::from(home).join(".cargo/bin"))
}

fn run(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    run_env(program, args, &[], cwd)
}

fn run_env(program: &str, args: &[&str], envs: &[(&str, &str)], cwd: &Path) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .envs(envs.iter().copied())
        .current_dir(cwd)
        .status()
        .with_context(|| format!("spawning `{program} {args:?}`"))?;
    if !status.success() {
        bail!("`{program} {args:?}` exited {status}");
    }
    Ok(())
}

fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR unset")?;
    Ok(Path::new(&manifest).parent().unwrap().to_owned())
}

fn which(bin: &str) -> Option<String> {
    let out = Command::new("which").arg(bin).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
