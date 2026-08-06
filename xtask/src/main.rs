//! Minos build / codegen orchestration.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use minos_agent_runtime::{AgentManager, AgentRuntimeConfig, InstanceCaps, RawIngest};
use minos_domain::AgentName;
use tempfile::TempDir;
use tokio::runtime::Builder;
use tokio::sync::broadcast::error::RecvError;

mod backend_platform_schemas;
mod gen_codex;
mod lint_contract;
mod lint_conventions;
mod lint_docs;
mod lint_metrics;
mod lint_naming;
mod lint_route_inventory;
mod lint_schema_parity;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum BackendDbDriver {
    Sqlite,
    Postgres,
}

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    /// Opt in to the real-codex smoke leg during `check-all`.
    #[arg(long, global = true)]
    with_codex: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

/// How much of the Flutter suite to run.
enum FlutterMode {
    /// format + analyze + full `flutter test` (local `check-all`).
    Full,
    /// Host dylib + `--tags ffi` only (CI macOS; dart lane owns the rest).
    #[allow(dead_code)]
    FfiOnly,
}

#[derive(Subcommand)]
enum Cmd {
    /// Full local gate: check-rust + (macOS) desktop cargo check + Flutter + frb drift.
    CheckAll,
    /// Rust-only quality gate used by the CI `backend` job and `just check-rust`.
    ///
    /// fmt + clippy + real lints + workspace tests + daemon `test-support`
    /// integration tests + backend platform schema drift. Does not run Swift,
    /// Flutter, FRB, or `minos-desktop` (GUI system deps).
    CheckRust,
    /// Install developer-side codegen tools (frb codegen, mobile rust targets,
    /// and Flutter deps for apps/mobile).
    Bootstrap,
    /// Generate Dart bindings via flutter_rust_bridge_codegen.
    GenFrb,
    /// Build iOS release staticlibs from minos-ffi-frb (arm64 device + arm64 sim).
    BuildIos,
    /// Wipe and recreate the backend database using the selected driver.
    BackendDbReset {
        #[arg(long, value_enum, default_value = "sqlite")]
        driver: BackendDbDriver,
    },
    /// Run the backend binary with dev-friendly defaults.
    BackendRun,
    /// Regenerate `crates/minos-codex-protocol/src/generated/{types,methods}.rs`
    /// from the JSON schemas in `/schemas`. Run after editing `/schemas`.
    GenCodexProtocol,
    /// Generate the backend platform artifacts (runtime contract, OpenAPI,
    /// and websocket schema), or verify they are up to date with `--check`.
    GenBackendPlatformContract {
        #[arg(long)]
        check: bool,
    },
    /// Scan protocol/FFI/HTTP/SQL surfaces for `mac_*` / `ios_*` identifiers
    /// (Phase B naming-sweep guard). Fails if any are found.
    LintNaming,
    /// Check docs files exist and topic/path consistency is valid.
    LintDocs,
    /// Check for the "transaction triple" pattern in service code.
    LintConventions,
    /// Check metric registry drift.
    LintMetrics,
    /// Check OpenAPI contract drift against baseline.
    LintContract,
    /// Check route inventory completeness.
    LintRouteInventory,
    /// Logical schema parity between SQLite and Postgres migrations.
    LintSchemaParity,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let with_codex = codex_smoke_requested(cli.with_codex)?;
    match cli.cmd {
        Cmd::CheckAll => check_all(with_codex),
        Cmd::CheckRust => check_rust(),
        Cmd::Bootstrap => bootstrap(),
        Cmd::GenFrb => gen_frb(),
        Cmd::BuildIos => build_ios(),
        Cmd::BackendDbReset { driver } => backend_db_reset(driver),
        Cmd::BackendRun => backend_run(),
        Cmd::GenCodexProtocol => gen_codex::run(&workspace_root()?),
        Cmd::GenBackendPlatformContract { check } => {
            backend_platform_schemas::generate(&workspace_root()?, check)
        }
        Cmd::LintNaming => lint_naming::run(&workspace_root()?),
        Cmd::LintDocs => lint_docs::run(&workspace_root()?),
        Cmd::LintConventions => lint_conventions::run(&workspace_root()?),
        Cmd::LintMetrics => lint_metrics::run(&workspace_root()?),
        Cmd::LintContract => lint_contract::run(&workspace_root()?),
        Cmd::LintRouteInventory => lint_route_inventory::run(&workspace_root()?),
        Cmd::LintSchemaParity => lint_schema_parity::run(&workspace_root()?),
    }
}

/// Rust-only gate shared by CI `backend` and the first stage of local `check-all`.
///
/// Phases run cheapest-first so simple failures surface before long compiles
/// or the test suite:
/// 1. static  — fmt, text lints, schema drift (no cargo build graph)
/// 2. compile — clippy (`-D warnings`; also typechecks the workspace)
/// 3. test    — workspace tests, then daemon `test-support` integration tests
fn check_rust() -> Result<()> {
    let workspace_root = workspace_root()?;

    // --- phase 1: static ---------------------------------------------------
    eprintln!("==> [static] cargo fmt --check");
    run("cargo", &["fmt", "--all", "--check"], &workspace_root)?;

    // Only real (non-stub) lints run here. `lint-conventions` / `lint-metrics`
    // remain available as standalone commands but are not enforced until
    // implemented — running stubs inside the gate created false confidence.
    eprintln!("==> [static] cargo xtask lint-naming");
    lint_naming::run(&workspace_root)?;

    eprintln!("==> [static] cargo xtask lint-docs");
    lint_docs::run(&workspace_root)?;

    // In-process against the already-linked minos-backend; fails fast on
    // OpenAPI / WS schema drift before we pay for clippy or tests.
    eprintln!("==> [static] backend platform schema drift");
    backend_platform_schemas::generate(&workspace_root, true)?;

    eprintln!("==> [static] cargo xtask lint-schema-parity");
    lint_schema_parity::run(&workspace_root)?;

    // --- phase 2: compile --------------------------------------------------
    eprintln!("==> [compile] cargo clippy");
    // Exclude Tauri host shell: it needs platform GUI system deps (GTK/WebKit
    // on Linux, etc.) that plain Linux CI runners do not install. The crate is
    // compiled on the desktop CI job via `cargo check -p minos-desktop`
    // and locally via `just check-desktop` / Tauri builds.
    //
    // `--keep-going` is required: without it, cargo stops at the first crate
    // that fails to compile under `-D warnings`, so later crates only surface
    // their clippy debt on subsequent CI rounds after the earlier ones are fixed.
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--exclude",
            "minos-desktop",
            "--keep-going",
            "--",
            "-D",
            "warnings",
        ],
        &workspace_root,
    )?;

    // --- phase 3: test -----------------------------------------------------
    eprintln!("==> [test] cargo test --workspace (exclude minos-desktop)");
    run(
        "cargo",
        &["test", "--workspace", "--exclude", "minos-desktop"],
        &workspace_root,
    )?;

    // Daemon integration tests (`tests/*.rs`) are gated on the `test-support`
    // feature and are invisible to plain `cargo test -p minos-daemon`. Fold
    // them into the rust gate so local `just check` and CI share one path.
    eprintln!("==> [test] cargo test -p minos-daemon --features test-support");
    run(
        "cargo",
        &["test", "-p", "minos-daemon", "--features", "test-support"],
        &workspace_root,
    )?;

    eprintln!("OK: check-rust passed.");
    Ok(())
}


fn check_all(with_codex: bool) -> Result<()> {
    let workspace_root = workspace_root()?;

    // Same cheapest-first rule: finish static/codegen drift before Flutter/smoke.
    check_rust()?;

    // FRB drift is a codegen consistency check (regenerate + git diff). Run it
    // before the Flutter leg so a forgotten `gen-frb` fails fast. Self-skips
    // when flutter_rust_bridge_codegen is absent. CI mobile job owns this too.
    frb_drift_guard(&workspace_root)?;

    if cfg!(target_os = "macos") {
        // Tauri host shell compile-check (excluded from Linux workspace clippy).
        desktop_rust_check(&workspace_root)?;
    } else {
        eprintln!("==> desktop rust check: skipped (non-macOS host)");
    }

    // Full Flutter suite when fvm is present. CI mobile job owns format/analyze/
    // non-ffi tests + frb drift; local check-all runs the full suite when ready.
    flutter_leg(&workspace_root, FlutterMode::Full)?;

    if with_codex {
        codex_smoke_leg()?;
    }

    eprintln!("OK: all checks pass.");
    Ok(())
}


/// Compile the Tauri host shell on macOS (excluded from workspace clippy/test
/// on Linux because of GUI system deps).
fn desktop_rust_check(workspace_root: &Path) -> Result<()> {
    eprintln!("==> cargo check -p minos-desktop");
    run("cargo", &["check", "-p", "minos-desktop"], workspace_root)?;
    Ok(())
}

fn codex_smoke_requested(with_codex_flag: bool) -> Result<bool> {
    let Some(value) = std::env::var_os("MINOS_XTASK_WITH_CODEX") else {
        return Ok(with_codex_flag);
    };
    let value = value.to_string_lossy();
    if value == "1" {
        return Ok(true);
    }
    bail!(
        "MINOS_XTASK_WITH_CODEX must be set to `1` when present; got {:?}",
        value.as_ref()
    )
}

fn codex_smoke_leg() -> Result<()> {
    let codex_bin = if let Some(path) = which(AgentName::Codex.bin_name()) {
        PathBuf::from(path)
    } else {
        eprintln!("==> codex smoke: skipped (codex not found on PATH)");
        return Ok(());
    };

    eprintln!("==> codex smoke (real codex app-server)");
    Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for codex smoke")?
        .block_on(codex_smoke_leg_async(codex_bin))
}

async fn codex_smoke_leg_async(codex_bin: PathBuf) -> Result<()> {
    let tempdir = TempDir::new().context("creating tempdir for codex smoke")?;
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(&workspace_root)
        .with_context(|| format!("mkdir {}", workspace_root.display()))?;

    let mut cfg = AgentRuntimeConfig::new(workspace_root.clone());
    cfg.codex_bin = Some(codex_bin);
    cfg.handshake_call_timeout = Duration::from_secs(30);
    let manager = std::sync::Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

    let outcome = manager
        .start_agent(AgentName::Codex, workspace_root)
        .await
        .context("codex smoke: start_agent failed")?;
    let session_id = outcome.session_id;
    let watcher = tokio::spawn(wait_for_codex_ok_token(manager.ingest_stream()));

    let result = async {
        manager
            .send_user_message(&session_id, "reply with the word ok".into())
            .await
            .context("codex smoke: send_user_message failed")?;
        watcher
            .await
            .context("codex smoke: event watcher task failed")??;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let stop_result = manager
        .close_session(&session_id)
        .await
        .context("codex smoke: close_session failed");

    match (result, stop_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) => Err(err),
        (Ok(()), Err(stop_err)) => Err(stop_err),
        (Err(err), Err(stop_err)) => {
            Err(err.context(format!("codex smoke cleanup also failed: {stop_err:#}")))
        }
    }
}

async fn wait_for_codex_ok_token(
    mut events: tokio::sync::broadcast::Receiver<RawIngest>,
) -> Result<()> {
    tokio::time::timeout(Duration::from_mins(1), async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let Some(payload) = event.json_value() else {
                        continue;
                    };
                    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
                    if method == "item/agentMessage/delta" {
                        let delta = payload
                            .get("params")
                            .and_then(|p| p.get("delta"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if delta.to_ascii_lowercase().contains("ok") {
                            return Ok(());
                        }
                    } else if method == "thread/archived" {
                        bail!(
                            "codex smoke: thread archived before emitting an `ok` agent-message delta"
                        );
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    eprintln!(
                        "    codex smoke: event subscriber lagged by {skipped} messages; continuing"
                    );
                }
                Err(RecvError::Closed) => {
                    bail!("codex smoke: event stream closed before receiving an `ok` token");
                }
            }
        }
    })
    .await
    .context("codex smoke: timed out waiting up to 60s for an `ok` agent-message delta")?
}

/// Regenerate the Dart/Rust bridge and fail on any drift — tracked diffs OR
/// untracked new files. Gates on `flutter_rust_bridge_codegen` being on PATH
/// so contributors who haven't run `cargo xtask bootstrap` (and hosts without
/// Flutter, e.g. the Ubuntu `linux` CI lane) skip silently rather than fail.
fn frb_drift_guard(workspace_root: &Path) -> Result<()> {
    if !workspace_root.join("apps/mobile/pubspec.yaml").exists()
        || which("flutter_rust_bridge_codegen").is_none()
    {
        eprintln!(
            "==> frb codegen drift: skipped (flutter_rust_bridge_codegen not found or apps/mobile missing)"
        );
        return Ok(());
    }

    eprintln!("==> frb codegen drift (gen-frb + git diff + untracked check)");
    gen_frb()?;
    run(
        "git",
        &[
            "diff",
            "--exit-code",
            "--",
            "apps/mobile/lib/src/rust",
            "crates/minos-ffi-frb/src/frb_generated.rs",
        ],
        workspace_root,
    )?;
    // `git diff` only surfaces modifications to tracked files; a new frb API
    // that emits a fresh .dart file would be invisible without this. Close
    // the loophole by also failing on any untracked file under either
    // generated-artifact root.
    let untracked = Command::new("git")
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            "apps/mobile/lib/src/rust",
            "crates/minos-ffi-frb/src/frb_generated.rs",
        ])
        .current_dir(workspace_root)
        .output()
        .context("git ls-files --others for drift guard")?;
    if !untracked.stdout.is_empty() {
        let listing = String::from_utf8_lossy(&untracked.stdout);
        bail!("frb codegen produced untracked files. Commit these and re-run:\n{listing}");
    }
    Ok(())
}

/// Run Flutter checks from `apps/mobile`.
///
/// - [`FlutterMode::Full`]: format + analyze + all tests (local `check-all`).
/// - [`FlutterMode::FfiOnly`]: host dylib + `--tags ffi` (optional local).
///
/// Skips the whole leg when Flutter is not set up — e.g. the Ubuntu `rust`
/// CI job does not install Flutter. `flutter test` loads
/// `libminos_ffi_frb.{dylib,so}` via the frb runtime, so we `cargo build -p
/// minos-ffi-frb` before invoking tests that need the host dylib.
fn flutter_leg(workspace_root: &Path, mode: FlutterMode) -> Result<()> {
    let mobile_root = workspace_root.join("apps/mobile");
    if !mobile_root.join("pubspec.yaml").exists() {
        eprintln!("==> flutter leg: skipped (apps/mobile/pubspec.yaml missing)");
        return Ok(());
    }
    if which("fvm").is_none() {
        // Distinguish three situations to avoid silently green-lighting a
        // misconfigured workstation.  If Flutter or Dart is on PATH but fvm
        // is not, the developer has Flutter installed but hasn't adopted the
        // project's version pin — fail loudly so they install fvm rather
        // than run a mismatched SDK.  Otherwise (no Flutter at all, e.g.
        // the Ubuntu CI `rust` lane), it is fine to skip.
        if which("flutter").is_some() || which("dart").is_some() {
            bail!(
                "flutter leg: fvm not found but Flutter/Dart are on PATH. \
                 This project pins Flutter via apps/mobile/.fvmrc; install \
                 fvm (https://fvm.app) so `fvm flutter` resolves to 3.44.0."
            );
        }
        eprintln!("==> flutter leg: skipped (no Flutter toolchain detected on this host)");
        return Ok(());
    }

    // Prefer offline when pubspec.lock is present so hosts with a flaky/hung
    // pub.dev path still gate. Fall back to online only if offline fails.
    eprintln!("==> fvm flutter pub get (apps/mobile)");
    let has_lock = mobile_root.join("pubspec.lock").is_file();
    if has_lock {
        if let Err(offline_err) = run("fvm", &["flutter", "pub", "get", "--offline"], &mobile_root)
        {
            eprintln!("==> flutter pub get --offline failed ({offline_err}); retrying online");
            run("fvm", &["flutter", "pub", "get"], &mobile_root)?;
        }
    } else {
        run("fvm", &["flutter", "pub", "get"], &mobile_root)?;
    }

    match mode {
        FlutterMode::Full => flutter_leg_full(&mobile_root, workspace_root)?,
        FlutterMode::FfiOnly => flutter_leg_ffi_only(&mobile_root, workspace_root)?,
    }

    Ok(())
}

fn flutter_leg_full(mobile_root: &Path, workspace_root: &Path) -> Result<()> {
    // static → compile host dylib → test
    eprintln!("==> [static] fvm dart format --set-exit-if-changed lib test (apps/mobile)");
    // Scope explicitly to the project's own Dart sources.  `dart format .`
    // would also walk `rust_builder/cargokit/**` (vendored upstream) and
    // `build/**` (ephemeral generator output) — neither of which we want
    // CI to enforce style on.  `analyzer.exclude` handles `dart analyze`'s
    // side; this arg list handles `dart format`, which has no exclude
    // flag and ignores `analysis_options.yaml`.
    run(
        "fvm",
        &["dart", "format", "--set-exit-if-changed", "lib", "test"],
        mobile_root,
    )?;

    // Match the CI `dart` lane: warnings/errors are fatal, info-level lints
    // (e.g. prefer_shorthands on new SDK releases) stay advisory so unrelated
    // PRs are not blocked. `--no-pub`: deps already resolved above.
    eprintln!("==> [static] fvm flutter analyze --no-pub (apps/mobile)");
    run("fvm", &["flutter", "analyze", "--no-pub"], mobile_root)?;

    eprintln!("==> [compile] cargo build -p minos-ffi-frb (host dylib for flutter test)");
    run("cargo", &["build", "-p", "minos-ffi-frb"], workspace_root)?;

    eprintln!("==> [test] fvm flutter test --no-pub (apps/mobile)");
    run("fvm", &["flutter", "test", "--no-pub"], mobile_root)?;
    Ok(())
}

fn flutter_leg_ffi_only(mobile_root: &Path, workspace_root: &Path) -> Result<()> {
    // Format/analyze/non-ffi tests + frb drift are owned by the CI `dart` job.
    // This leg only proves the host dylib loads under Flutter's test runner.
    eprintln!("==> cargo build -p minos-ffi-frb (host dylib for flutter ffi tests)");
    run("cargo", &["build", "-p", "minos-ffi-frb"], workspace_root)?;

    if !mobile_root.join("test").is_dir() {
        eprintln!("==> flutter ffi tests: skipped (apps/mobile/test missing)");
        return Ok(());
    }

    eprintln!("==> fvm flutter test --no-pub --tags ffi (apps/mobile)");
    // package:test exits 79 when the tag filter matches zero tests ("No tests
    // ran"). Until an ffi-tagged host-dylib test lands, treat that as success
    // so the macOS lane stays green and becomes meaningful on first add.
    let status = run_status(
        "fvm",
        &["flutter", "test", "--no-pub", "--tags", "ffi"],
        mobile_root,
    )?;
    if flutter_ffi_tag_filter_ok(status.success(), status.code()) {
        if !status.success() {
            eprintln!("==> flutter ffi tests: no tests tagged ffi (ok)");
        }
        return Ok(());
    }
    bail!("`fvm [\"flutter\", \"test\", \"--no-pub\", \"--tags\", \"ffi\"]` exited {status}");
}

/// Accept a successful `flutter test --tags ffi` run, or exit 79 (no matches).
fn flutter_ffi_tag_filter_ok(success: bool, code: Option<i32>) -> bool {
    success || code == Some(79)
}

fn bootstrap() -> Result<()> {
    let workspace_root = workspace_root()?;

    eprintln!("==> installing flutter_rust_bridge_codegen {FRB_CODEGEN_VERSION}");
    // Must stay lockstep with `flutter_rust_bridge = "=2.12.0"` (Cargo) and
    // `flutter_rust_bridge: 2.12.0` (apps/mobile pubspec). `--force` replaces
    // a newer/older binary already on PATH (e.g. a beta) so bootstrap is the
    // single pin path. `--locked` keeps the transitive graph reproducible.
    run(
        "cargo",
        &[
            "install",
            "flutter_rust_bridge_codegen",
            "--version",
            FRB_CODEGEN_VERSION,
            "--locked",
            "--force",
        ],
        &workspace_root,
    )?;

    // iOS rustup targets are required for `cargo xtask build-ios` and the
    // Phase F real-device path. On non-macOS hosts `rustup target add` still
    // succeeds (rustup just records the target as available for future
    // cross-compiles), but the targets are never actually used there. We
    // attempt the add unconditionally to keep one happy path.
    if cfg!(target_os = "macos") {
        eprintln!("==> rustup target add (aarch64-apple-ios, aarch64-apple-ios-sim)");
        run(
            "rustup",
            &[
                "target",
                "add",
                "aarch64-apple-ios",
                "aarch64-apple-ios-sim",
            ],
            &workspace_root,
        )?;
    }

    // Prime the Flutter + Dart side so a fresh clone's first
    // `cargo xtask check-all` does not fail for missing `pub get` or
    // `build_runner`-generated files. Gate on the pubspec existing so this
    // crate still bootstraps cleanly before plan 03's `apps/mobile` scaffold
    // lands.
    let mobile_root = workspace_root.join("apps/mobile");
    if mobile_root.join("pubspec.yaml").exists() {
        if which("fvm").is_none() {
            bail!(
                "fvm not installed; required to manage the pinned Flutter version for \
                 apps/mobile. Install via https://fvm.app (macOS: `brew tap leoafarias/fvm \
                 && brew install fvm`)."
            );
        }

        eprintln!("==> fvm flutter pub get (apps/mobile)");
        run("fvm", &["flutter", "pub", "get"], &mobile_root)?;

        eprintln!("==> fvm dart run build_runner build --delete-conflicting-outputs");
        run(
            "fvm",
            &[
                "dart",
                "run",
                "build_runner",
                "build",
                "--delete-conflicting-outputs",
            ],
            &mobile_root,
        )?;
    } else {
        eprintln!(
            "    (skipped Flutter bootstrap: {} missing)",
            mobile_root.join("pubspec.yaml").display()
        );
    }

    Ok(())
}

/// Must match `minos-ffi-frb`'s `flutter_rust_bridge = "=2.12.0"` and
/// `apps/mobile` pubspec `flutter_rust_bridge: 2.12.0`. Drift here is the
/// mobile FRB regen footgun (mismatched encode/decode / `@generated` headers).
const FRB_CODEGEN_VERSION: &str = "2.12.0";




fn gen_frb() -> Result<()> {
    let root = workspace_root()?;
    if which("flutter_rust_bridge_codegen").is_none() {
        bail!(
            "flutter_rust_bridge_codegen not found on PATH. Run `cargo xtask bootstrap` \
             to install it (cargo install flutter_rust_bridge_codegen --version \
             {FRB_CODEGEN_VERSION} --locked --force)."
        );
    }
    ensure_frb_codegen_matches_workspace()?;

    let config = root.join("flutter_rust_bridge.yaml");
    if !config.exists() {
        bail!(
            "{} missing; frb codegen needs the repo-root config",
            config.display()
        );
    }

    // frb's codegen invokes `fvm flutter --version` internally to discover the
    // Dart toolchain, and `fvm` only resolves the pinned version when it's
    // run from a directory containing `.fvmrc` (apps/mobile). We therefore
    // invoke the codegen from `apps/mobile` and point it at the repo-root
    // YAML explicitly — the paths inside the YAML (`rust_root`,
    // `dart_output`, `rust_output`) are interpreted relative to the config
    // file, not CWD, so this works transparently.
    let mobile_root = root.join("apps/mobile");
    if !mobile_root.join("pubspec.yaml").exists() {
        bail!(
            "{} missing; gen-frb needs apps/mobile for fvm to resolve Flutter",
            mobile_root.join("pubspec.yaml").display()
        );
    }

    eprintln!(
        "==> flutter_rust_bridge_codegen generate --config-file {config_display} \
         (pinned {FRB_CODEGEN_VERSION})",
        config_display = config.display()
    );
    run(
        "flutter_rust_bridge_codegen",
        &["generate", "--config-file", config.to_str().unwrap()],
        &mobile_root,
    )
}

fn ensure_frb_codegen_matches_workspace() -> Result<()> {
    let out = Command::new("flutter_rust_bridge_codegen")
        .arg("--version")
        .output()
        .context("running flutter_rust_bridge_codegen --version")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let text = format!("{stdout}{stderr}");
    // CLI prints e.g. `flutter_rust_bridge_codegen 2.12.0`. Exact token match
    // rejects betas (`2.12.0-beta.5`) and other minors.
    if text
        .split_whitespace()
        .any(|tok| tok == FRB_CODEGEN_VERSION)
    {
        return Ok(());
    }
    let list = Command::new("cargo")
        .args(["install", "--list"])
        .output()
        .context("running cargo install --list")?;
    let list_text = String::from_utf8_lossy(&list.stdout);
    let expected = format!("flutter_rust_bridge_codegen v{FRB_CODEGEN_VERSION}:");
    if list_text.lines().any(|line| line.starts_with(&expected)) {
        return Ok(());
    }
    bail!(
        "flutter_rust_bridge_codegen is not version {FRB_CODEGEN_VERSION} (workspace pin). \
         Run `cargo xtask bootstrap` to reinstall. --version output: {text:?}\n\
         cargo install --list flutter_rust_bridge lines:\n{}",
        list_text
            .lines()
            .filter(|line| line.contains("flutter_rust_bridge"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn build_ios() -> Result<()> {
    const IOS_DEPLOYMENT_TARGET: &str = "16.0";

    if !cfg!(target_os = "macos") {
        bail!("`build-ios` requires a macOS host");
    }

    let root = workspace_root()?;

    // Both iOS targets must be registered with rustup before cargo can
    // cross-compile. `rustup target list --installed` is a cheap, stable
    // query; if the needed targets are missing we bail and point the user
    // at `bootstrap` (the single place that mutates rustup state) instead
    // of mutating here.
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .current_dir(&root)
        .output()
        .context("running `rustup target list --installed`")?;
    if !installed.status.success() {
        bail!(
            "`rustup target list --installed` exited {}",
            installed.status
        );
    }
    let installed = String::from_utf8_lossy(&installed.stdout);
    for target in ["aarch64-apple-ios", "aarch64-apple-ios-sim"] {
        if !installed.lines().any(|line| line.trim() == target) {
            bail!(
                "rustup target `{target}` not installed; run `cargo xtask bootstrap` \
                 (or manually: `rustup target add {target}`)"
            );
        }
    }

    for target in ["aarch64-apple-ios", "aarch64-apple-ios-sim"] {
        eprintln!("==> cargo build -p minos-ffi-frb --release --target {target}");
        run_env(
            "cargo",
            &[
                "build",
                "-p",
                "minos-ffi-frb",
                "--release",
                "--target",
                target,
            ],
            &[("IPHONEOS_DEPLOYMENT_TARGET", IOS_DEPLOYMENT_TARGET)],
            &root,
        )?;

        let out = root
            .join("target")
            .join(target)
            .join("release")
            .join("libminos_ffi_frb.a");
        if !out.exists() {
            bail!("expected staticlib at {}", out.display());
        }
        eprintln!("    produced {}", out.display());
    }

    Ok(())
}

/// Run the backend binary with dev-friendly defaults.
///
/// Convenience wrapper for `cargo run -p minos-backend -- --listen 127.0.0.1:8787
/// --db ./minos-backend.db --log-level debug`. Used by plan §11 acceptance for
/// booting the backend during iteration.
fn backend_run() -> Result<()> {
    let root = workspace_root()?;
    eprintln!("==> cargo run -p minos-backend (dev listen 127.0.0.1:8787)");
    run(
        "cargo",
        &[
            "run",
            "-p",
            "minos-backend",
            "--",
            "--listen",
            "127.0.0.1:8787",
            "--db",
            "./minos-backend.db",
            "--log-level",
            "debug",
        ],
        &root,
    )
}

/// Wipe and recreate the backend database for the selected driver.
fn backend_db_reset(driver: BackendDbDriver) -> Result<()> {
    let root = workspace_root()?;
    match driver {
        BackendDbDriver::Sqlite => backend_db_reset_sqlite(&root),
        BackendDbDriver::Postgres => backend_db_reset_postgres(&root),
    }
}

/// Wipe and recreate the backend SQLite DB at ./minos-backend.db.
///
/// Removes the db file (plus `-shm` / `-wal` sidecars if SQLite is in WAL mode)
/// and then re-runs migrations via `--exit-after-migrate`. Idempotent — missing
/// files are ignored.
fn backend_db_reset_sqlite(root: &Path) -> Result<()> {
    for suffix in ["", "-shm", "-wal"] {
        let path = root.join(format!("minos-backend.db{suffix}"));
        if path.exists() {
            eprintln!("==> rm {}", path.display());
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }

    eprintln!("==> cargo run -p minos-backend -- --db ./minos-backend.db --exit-after-migrate");
    run(
        "cargo",
        &[
            "run",
            "-p",
            "minos-backend",
            "--",
            "--db",
            "./minos-backend.db",
            "--exit-after-migrate",
        ],
        root,
    )
}

fn backend_db_reset_postgres(root: &Path) -> Result<()> {
    let database_url = std::env::var("MINOS_DATABASE_URL")
        .or_else(|_| std::env::var("MINOS_BACKEND_POSTGRES_URL"))
        .context(
            "MINOS_DATABASE_URL or MINOS_BACKEND_POSTGRES_URL must be set for `cargo xtask backend-db-reset --driver postgres`",
        )?;

    if which("psql").is_none() {
        bail!("psql is required for `cargo xtask backend-db-reset --driver postgres`");
    }

    eprintln!("==> psql $MINOS_DATABASE_URL -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public'");
    run_owned(
        "psql",
        &[
            database_url.clone(),
            "-v".to_string(),
            "ON_ERROR_STOP=1".to_string(),
            "-c".to_string(),
            "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;".to_string(),
        ],
        root,
    )?;

    eprintln!("==> cargo run -p minos-backend -- --storage-mode external-sql --database-url $MINOS_DATABASE_URL --exit-after-migrate");
    run_owned(
        "cargo",
        &[
            "run".to_string(),
            "-p".to_string(),
            "minos-backend".to_string(),
            "--".to_string(),
            "--storage-mode".to_string(),
            "external-sql".to_string(),
            "--database-url".to_string(),
            database_url,
            "--exit-after-migrate".to_string(),
        ],
        root,
    )
}


fn run(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    run_env(program, args, &[], cwd)
}

fn run_status(program: &str, args: &[&str], cwd: &Path) -> Result<ExitStatus> {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("spawning `{program} {args:?}`"))
}

fn run_owned(program: &str, args: &[String], cwd: &Path) -> Result<()> {
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
