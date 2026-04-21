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
        run(
            "cargo",
            &[
                "build",
                "-p",
                "minos-ffi-uniffi",
                "--release",
                "--target",
                target,
            ],
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

    let dylib = root.join("target/release").join(format!(
        "libminos_ffi_uniffi.{}",
        host_dylib_suffix()
    ));
    if !dylib.exists() {
        bail!("expected built library at {}", dylib.display());
    }

    eprintln!("==> uniffi-bindgen-swift --swift-sources {}", dylib.display());
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

    for generated in ["MinosCore.swift", "MinosCoreFFI.h", "MinosCoreFFI.modulemap"] {
        let path = out_dir.join(generated);
        if !path.exists() {
            bail!("missing generated UniFFI artifact: {}", path.display());
        }
    }

    eprintln!("OK: {}", out_dir.display());
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
    fs::write(&wrapper, script)
        .with_context(|| format!("writing {}", wrapper.display()))?;

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
    let status = Command::new(program)
        .args(args)
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
