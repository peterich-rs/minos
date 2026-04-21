//! Minos build / codegen orchestration.

use std::path::Path;
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
    /// Generate Swift bindings via uniffi-bindgen. (Implemented in plan 02.)
    GenUniffi,
    /// Generate Dart bindings via flutter_rust_bridge_codegen. (Implemented in plan 03.)
    GenFrb,
    /// Build macOS xcframework from minos-ffi-uniffi. (Implemented in plan 02.)
    BuildMacos,
    /// Build iOS staticlib from minos-ffi-frb. (Implemented in plan 03.)
    BuildIos,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckAll => check_all(),
        Cmd::Bootstrap => bootstrap(),
        Cmd::GenUniffi => not_yet("gen-uniffi"),
        Cmd::GenFrb => not_yet("gen-frb"),
        Cmd::BuildMacos => not_yet("build-macos"),
        Cmd::BuildIos => not_yet("build-ios"),
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

    eprintln!("OK: all checks pass.");
    Ok(())
}

fn bootstrap() -> Result<()> {
    let workspace_root = workspace_root()?;
    eprintln!("==> installing cargo-deny + uniffi-bindgen");
    run(
        "cargo",
        &["install", "cargo-deny", "--locked"],
        &workspace_root,
    )?;
    run(
        "cargo",
        &["install", "uniffi-bindgen-cli", "--locked"],
        &workspace_root,
    )?;
    // flutter_rust_bridge_codegen and dart deps come in plan 03.
    Ok(())
}

fn not_yet(name: &str) -> Result<()> {
    bail!("xtask `{name}` not implemented yet (filled in later)")
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
