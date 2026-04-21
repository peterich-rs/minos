//! Minos build / codegen orchestration.
//!
//! Subcommands are filled in across phases of plan 01:
//! - `check-all`: phase K
//! - `gen-uniffi` / `gen-frb` / `build-macos` / `build-ios`: phase K stubs only;
//!   real implementations land in plans 02 and 03.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run fmt + clippy + workspace tests + (later) UI lints.
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
        Cmd::CheckAll => not_yet("check-all"),
        Cmd::Bootstrap => not_yet("bootstrap"),
        Cmd::GenUniffi => not_yet("gen-uniffi"),
        Cmd::GenFrb => not_yet("gen-frb"),
        Cmd::BuildMacos => not_yet("build-macos"),
        Cmd::BuildIos => not_yet("build-ios"),
    }
}

fn not_yet(name: &str) -> Result<()> {
    anyhow::bail!("xtask `{name}` not implemented yet (filled in later in plan 01)")
}
