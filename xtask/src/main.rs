//! Minos build / codegen orchestration.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use fs2::FileExt;
use minos_agent_runtime::{AgentRuntime, AgentRuntimeConfig, RawIngest};
use minos_domain::AgentName;
use tempfile::TempDir;
use tokio::runtime::Builder;
use tokio::sync::broadcast::error::RecvError;

mod gen_codex;

#[derive(Parser)]
#[command(name = "xtask", about = "Minos build & codegen orchestration")]
struct Cli {
    /// Opt in to the real-codex smoke leg during `check-all`.
    #[arg(long, global = true)]
    with_codex: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// fmt + clippy + workspace tests + Swift + Flutter legs + frb codegen drift guard.
    CheckAll,
    /// Install developer-side codegen tools (cargo-deny, uniffi, frb codegen,
    /// iOS rustup targets, and Flutter deps for apps/mobile).
    Bootstrap,
    /// Generate Swift bindings via uniffi-bindgen-swift.
    GenUniffi,
    /// Generate Dart bindings via flutter_rust_bridge_codegen.
    GenFrb,
    /// Build the universal macOS static library for the Swift app.
    BuildMacos {
        /// Xcode build configuration. Configurations containing "debug" use
        /// Cargo dev profile; all others use Cargo release profile.
        #[arg(long)]
        configuration: Option<String>,
    },
    /// Build iOS release staticlibs from minos-ffi-frb (arm64 device + arm64 sim).
    BuildIos,
    /// Generate apps/macos/Minos.xcodeproj from apps/macos/project.yml.
    GenXcode,
    /// Wipe and recreate the backend SQLite DB at ./minos-backend.db.
    BackendDbReset,
    /// Run the backend binary with dev-friendly defaults.
    BackendRun,
    /// Regenerate `crates/minos-codex-protocol/src/generated/{types,methods}.rs`
    /// from the JSON schemas in `/schemas`. Run after editing `/schemas`.
    GenCodexProtocol,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let with_codex = codex_smoke_requested(cli.with_codex)?;
    match cli.cmd {
        Cmd::CheckAll => check_all(with_codex),
        Cmd::Bootstrap => bootstrap(),
        Cmd::GenUniffi => gen_uniffi(),
        Cmd::GenFrb => gen_frb(),
        Cmd::BuildMacos { configuration } => build_macos(configuration.as_deref()),
        Cmd::BuildIos => build_ios(),
        Cmd::GenXcode => gen_xcode(),
        Cmd::BackendDbReset => backend_db_reset(),
        Cmd::BackendRun => backend_run(),
        Cmd::GenCodexProtocol => gen_codex::run(&workspace_root()?),
    }
}

fn check_all(with_codex: bool) -> Result<()> {
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

        eprintln!("==> cargo xtask build-macos --configuration Debug");
        build_macos(Some("Debug"))?;

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
                "CODE_SIGNING_ALLOWED=NO",
                "CODE_SIGNING_REQUIRED=NO",
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
                "CODE_SIGNING_ALLOWED=NO",
                "CODE_SIGNING_REQUIRED=NO",
                "test",
            ],
            &macos_root,
        )?;

        eprintln!("==> swiftlint --strict");
        run("swiftlint", &["--strict"], &macos_root)?;
    } else {
        eprintln!("==> swift leg: skipped (non-macOS host)");
    }

    // Flutter leg. Runs only when the mobile package exists and `fvm` (the
    // pinned Flutter launcher) is available. Ubuntu CI's `linux` lane doesn't
    // install Flutter — skipping keeps that lane green while still exercising
    // the full suite on the macOS lane / local dev where fvm is present.
    flutter_leg(&workspace_root)?;

    frb_drift_guard(&workspace_root)?;

    if with_codex {
        codex_smoke_leg()?;
    }

    eprintln!("OK: all checks pass.");
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

    let mut cfg = AgentRuntimeConfig::new(workspace_root);
    cfg.codex_bin = Some(codex_bin);
    cfg.handshake_call_timeout = Duration::from_secs(30);
    let runtime = AgentRuntime::new(cfg);

    let outcome = runtime
        .start(AgentName::Codex)
        .await
        .context("codex smoke: start_agent failed")?;
    let session_id = outcome.session_id;
    let watcher = tokio::spawn(wait_for_codex_ok_token(runtime.ingest_stream()));

    let result = async {
        runtime
            .send_user_message(&session_id, "reply with the word ok")
            .await
            .context("codex smoke: send_user_message failed")?;
        watcher
            .await
            .context("codex smoke: event watcher task failed")??;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let stop_result = runtime
        .stop()
        .await
        .context("codex smoke: stop_agent failed");

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
                Ok(RawIngest { payload, .. }) => {
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

/// Run the Flutter checks (`pub get`, `dart format --set-exit-if-changed`,
/// `flutter analyze`, `flutter test`) from `apps/mobile`. Skips the whole leg
/// (with a clear log line) when Flutter is not set up on the host — e.g. on
/// the Ubuntu `linux` CI lane, which doesn't bootstrap Flutter.
///
/// `flutter test` transitively loads `libminos_ffi_frb.{dylib,so}` via the
/// frb runtime, so we must `cargo build -p minos-ffi-frb` before invoking
/// it. That build is cheap (already cached for the preceding `cargo test`
/// leg) and gives the Dart host tests a predictable artifact path.
fn flutter_leg(workspace_root: &Path) -> Result<()> {
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
        // the Ubuntu CI `linux` lane), it is fine to skip.
        if which("flutter").is_some() || which("dart").is_some() {
            bail!(
                "flutter leg: fvm not found but Flutter/Dart are on PATH. \
                 This project pins Flutter via apps/mobile/.fvmrc; install \
                 fvm (https://fvm.app) so `fvm flutter` resolves to 3.41.6."
            );
        }
        eprintln!("==> flutter leg: skipped (no Flutter toolchain detected on this host)");
        return Ok(());
    }

    eprintln!("==> cargo build -p minos-ffi-frb (host dylib for flutter test)");
    run("cargo", &["build", "-p", "minos-ffi-frb"], workspace_root)?;

    eprintln!("==> fvm flutter pub get (apps/mobile)");
    run("fvm", &["flutter", "pub", "get"], &mobile_root)?;

    eprintln!("==> fvm dart format --set-exit-if-changed lib test (apps/mobile)");
    // Scope explicitly to the project's own Dart sources.  `dart format .`
    // would also walk `rust_builder/cargokit/**` (vendored upstream) and
    // `build/**` (ephemeral generator output) — neither of which we want
    // CI to enforce style on.  `analyzer.exclude` handles `dart analyze`'s
    // side; this arg list handles `dart format`, which has no exclude
    // flag and ignores `analysis_options.yaml`.
    run(
        "fvm",
        &["dart", "format", "--set-exit-if-changed", "lib", "test"],
        &mobile_root,
    )?;

    eprintln!("==> fvm flutter analyze --fatal-infos (apps/mobile)");
    run(
        "fvm",
        &["flutter", "analyze", "--fatal-infos"],
        &mobile_root,
    )?;

    eprintln!("==> fvm flutter test (apps/mobile)");
    run("fvm", &["flutter", "test"], &mobile_root)?;

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

    eprintln!("==> installing flutter_rust_bridge_codegen (v2)");
    // `cargo install` is idempotent for matching versions — if the binary is
    // already present at a compatible version, cargo prints `already
    // installed` and exits 0. `--locked` keeps the transitive graph pinned
    // for reproducibility.
    run(
        "cargo",
        &[
            "install",
            "flutter_rust_bridge_codegen",
            "--version",
            "^2.11",
            "--locked",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoProfile {
    Debug,
    Release,
}

impl CargoProfile {
    fn artifact_dir(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    fn cargo_args(self) -> &'static [&'static str] {
        match self {
            Self::Debug => &[],
            Self::Release => &["--release"],
        }
    }
}

#[derive(Debug)]
struct MacosBuildConfiguration {
    xcode_name: String,
    cargo_profile: CargoProfile,
}

impl MacosBuildConfiguration {
    fn from_xcode(configuration: Option<&str>) -> Self {
        let xcode_name = configuration
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Release")
            .to_string();
        let cargo_profile = if xcode_name.to_ascii_lowercase().contains("debug") {
            CargoProfile::Debug
        } else {
            CargoProfile::Release
        };

        Self {
            xcode_name,
            cargo_profile,
        }
    }
}

fn build_macos(configuration: Option<&str>) -> Result<()> {
    const MACOS_DEPLOYMENT_TARGET: &str = "14.0";

    if !cfg!(target_os = "macos") {
        bail!("`build-macos` requires a macOS host");
    }

    let root = workspace_root()?;
    let build_config = MacosBuildConfiguration::from_xcode(configuration);
    let _build_lock = acquire_build_macos_lock(&root, &build_config.xcode_name)?;
    if which("lipo").is_none() {
        bail!("lipo not installed; `build-macos` requires Xcode command-line tools");
    }

    eprintln!(
        "==> cargo build-macos: {} Xcode config -> {:?} Rust profile",
        build_config.xcode_name, build_config.cargo_profile
    );
    eprintln!("==> cargo build-macos: arm64 + x86_64 staticlib -> lipo universal");
    for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        eprintln!("  target: {target}");
        let mut args = vec!["build", "-p", "minos-ffi-uniffi", "--target", target];
        args.extend_from_slice(build_config.cargo_profile.cargo_args());
        run_env(
            "cargo",
            &args,
            &[("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET)],
            &root,
        )?;
    }

    let out_dir = root
        .join("target/xcframework")
        .join(&build_config.xcode_name);
    fs::create_dir_all(&out_dir).with_context(|| format!("mkdir {}", out_dir.display()))?;

    let out_lib = out_dir.join("libminos_ffi_uniffi.a");
    let artifact_dir = build_config.cargo_profile.artifact_dir();
    let arm64 = root
        .join("target/aarch64-apple-darwin")
        .join(artifact_dir)
        .join("libminos_ffi_uniffi.a");
    let x86_64 = root
        .join("target/x86_64-apple-darwin")
        .join(artifact_dir)
        .join("libminos_ffi_uniffi.a");

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

    if build_config.xcode_name == "Release" {
        let legacy_out_lib = root.join("target/xcframework/libminos_ffi_uniffi.a");
        fs::copy(&out_lib, &legacy_out_lib).with_context(|| {
            format!(
                "copying {} to legacy path {}",
                out_lib.display(),
                legacy_out_lib.display()
            )
        })?;
        eprintln!("OK: {} (legacy release path)", legacy_out_lib.display());
    }

    eprintln!("OK: {} (universal)", out_lib.display());
    Ok(())
}

fn acquire_build_macos_lock(root: &Path, configuration: &str) -> Result<std::fs::File> {
    let lock_dir = root.join("target/locks");
    fs::create_dir_all(&lock_dir).with_context(|| format!("mkdir {}", lock_dir.display()))?;

    let lock_name = configuration
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let lock_path = lock_dir.join(format!("build-macos-{lock_name}.lock"));
    let lock = std::fs::File::create(&lock_path)
        .with_context(|| format!("create {}", lock_path.display()))?;

    eprintln!("==> acquiring build-macos lock {}", lock_path.display());
    lock.lock_exclusive()
        .with_context(|| format!("lock {}", lock_path.display()))?;
    Ok(lock)
}

fn gen_uniffi() -> Result<()> {
    let root = workspace_root()?;
    let out_dir = root.join("apps/macos/Minos/Generated");
    ensure_macos_scaffold_dirs(&root)?;

    if which("uniffi-bindgen-swift").is_none() {
        bail!("uniffi-bindgen-swift not installed; run `cargo xtask bootstrap`");
    }

    let host_target = host_macos_rust_target();
    eprintln!("==> cargo build (host arch) -p minos-ffi-uniffi --target {host_target}");
    run(
        "cargo",
        &["build", "-p", "minos-ffi-uniffi", "--target", host_target],
        &root,
    )?;

    let staticlib = root
        .join("target")
        .join(host_target)
        .join(CargoProfile::Debug.artifact_dir())
        .join("libminos_ffi_uniffi.a");
    if !staticlib.exists() {
        bail!("expected built library at {}", staticlib.display());
    }

    eprintln!(
        "==> uniffi-bindgen-swift --swift-sources {}",
        staticlib.display()
    );
    run(
        "uniffi-bindgen-swift",
        &[
            "--swift-sources",
            staticlib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!("==> uniffi-bindgen-swift --headers {}", staticlib.display());
    run(
        "uniffi-bindgen-swift",
        &[
            "--headers",
            staticlib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    eprintln!(
        "==> uniffi-bindgen-swift --modulemap {}",
        staticlib.display()
    );
    run(
        "uniffi-bindgen-swift",
        &[
            "--modulemap",
            "--xcframework",
            "--module-name",
            "MinosCore",
            "--modulemap-filename",
            "MinosCoreFFI.modulemap",
            staticlib.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
        &root,
    )?;

    normalize_generated_uniffi_imports(&out_dir)?;
    prune_unexpected_uniffi_outputs(&out_dir);

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

    verify_generated_uniffi_surface(&out_dir)?;

    eprintln!("OK: {}", out_dir.display());
    Ok(())
}

fn verify_generated_uniffi_surface(out_dir: &Path) -> Result<()> {
    require_generated_text(
        &out_dir.join("minos_agent_runtime.swift"),
        "public enum AgentState",
        "generated Swift enum for runtime-owned AgentState",
    )?;
    require_generated_text(
        &out_dir.join("minos_daemon.swift"),
        "public protocol AgentStateObserver",
        "generated Swift callback protocol for agent-state updates",
    )?;
    require_generated_text(
        &out_dir.join("minos_daemon.swift"),
        "open func startAgent(req: StartAgentRequest)",
        "generated DaemonHandle.startAgent binding",
    )?;
    require_generated_text(
        &out_dir.join("minos_daemon.swift"),
        "open func subscribeAgentState(observer: AgentStateObserver)",
        "generated DaemonHandle.subscribeAgentState binding",
    )?;
    require_generated_text(
        &out_dir.join("minos_protocol.swift"),
        "public struct StartAgentRequest",
        "generated Swift record for StartAgentRequest",
    )?;
    require_generated_text(
        &out_dir.join("minos_protocol.swift"),
        "public struct StartAgentResponse",
        "generated Swift record for StartAgentResponse",
    )?;
    require_generated_text(
        &out_dir.join("minos_protocol.swift"),
        "public struct SendUserMessageRequest",
        "generated Swift record for SendUserMessageRequest",
    )?;

    Ok(())
}

fn require_generated_text(path: &Path, needle: &str, context: &str) -> Result<()> {
    let buf = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if !buf.contains(needle) {
        bail!(
            "codegen drift: {} missing {} (expected substring: {:?})",
            path.display(),
            context,
            needle,
        );
    }
    Ok(())
}

fn prune_unexpected_uniffi_outputs(out_dir: &Path) {
    let _ = out_dir;
}

#[allow(clippy::too_many_lines)] // Sequential drift guards; splitting them would obscure the per-needle context.
fn normalize_generated_uniffi_imports(out_dir: &Path) -> Result<()> {
    // All `.replace()` calls here rewrite `uniffi-bindgen-swift`'s exact output
    // text. If upstream tweaks whitespace, naming, or pragma layout, a replacement
    // silently becomes a no-op and the resulting Swift sources fail to compile
    // with confusing errors. Each load-bearing replacement is therefore gated by
    // `replace_required`, which bails with a clear drift message when the needle
    // cannot be found. Cross-crate imports are the only optional replacements —
    // a file may not need them if it doesn't reference sibling-crate types.

    const MODULE_IMPORT: &str = "#if canImport(MinosCoreFFI)\nimport MinosCoreFFI\n#endif";
    const MODULEMAP_DECL: &str = "framework module MinosCore {";
    const MODULEMAP_DECL_NORMALIZED: &str = "module MinosCoreFFI {";
    const MODULEMAP_DECL_ALREADY_NORMALIZED: &str = "framework module MinosCoreFFI {";
    // Each cross-crate newtype that minos-daemon and minos-pairing both
    // register as a `remote` UniFFI custom_type emits an identical typealias
    // + FfiConverter block in `minos_daemon.swift` and `minos_pairing.swift`.
    // The two files share a Swift module, so we strip the daemon copy and
    // let pairing's stay as the canonical definition.
    //
    // Three independent strips (one per type) are intentional: uniffi-bindgen
    // -swift sorts these blocks alphabetically, so a future `remote` newtype
    // landing between two existing ones (e.g. between `DeviceSecret` and
    // `Uuid`) would silently break a single monolithic needle.
    const DUPLICATE_DAEMON_DEVICE_ID_BLOCK: &str = "/**\n * Typealias from the type name used in the UDL file to the builtin type.  This\n * is needed because the UDL type name is used in function/method signatures.\n */\npublic typealias DeviceId = Uuid\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic struct FfiConverterTypeDeviceId: FfiConverter {\n    public static func read(from buf: inout (data: Data, offset: Data.Index)) throws -> DeviceId {\n        return try FfiConverterTypeUuid.read(from: &buf)\n    }\n\n    public static func write(_ value: DeviceId, into buf: inout [UInt8]) {\n        return FfiConverterTypeUuid.write(value, into: &buf)\n    }\n\n    public static func lift(_ value: RustBuffer) throws -> DeviceId {\n        return try FfiConverterTypeUuid_lift(value)\n    }\n\n    public static func lower(_ value: DeviceId) -> RustBuffer {\n        return FfiConverterTypeUuid_lower(value)\n    }\n}\n\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceId_lift(_ value: RustBuffer) throws -> DeviceId {\n    return try FfiConverterTypeDeviceId.lift(value)\n}\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceId_lower(_ value: DeviceId) -> RustBuffer {\n    return FfiConverterTypeDeviceId.lower(value)\n}\n";
    const DUPLICATE_DAEMON_DEVICE_SECRET_BLOCK: &str = "/**\n * Typealias from the type name used in the UDL file to the builtin type.  This\n * is needed because the UDL type name is used in function/method signatures.\n */\npublic typealias DeviceSecret = String\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic struct FfiConverterTypeDeviceSecret: FfiConverter {\n    public static func read(from buf: inout (data: Data, offset: Data.Index)) throws -> DeviceSecret {\n        return try FfiConverterString.read(from: &buf)\n    }\n\n    public static func write(_ value: DeviceSecret, into buf: inout [UInt8]) {\n        return FfiConverterString.write(value, into: &buf)\n    }\n\n    public static func lift(_ value: RustBuffer) throws -> DeviceSecret {\n        return try FfiConverterString.lift(value)\n    }\n\n    public static func lower(_ value: DeviceSecret) -> RustBuffer {\n        return FfiConverterString.lower(value)\n    }\n}\n\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceSecret_lift(_ value: RustBuffer) throws -> DeviceSecret {\n    return try FfiConverterTypeDeviceSecret.lift(value)\n}\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeDeviceSecret_lower(_ value: DeviceSecret) -> RustBuffer {\n    return FfiConverterTypeDeviceSecret.lower(value)\n}\n";
    const DUPLICATE_DAEMON_UUID_BLOCK: &str = "/**\n * Typealias from the type name used in the UDL file to the builtin type.  This\n * is needed because the UDL type name is used in function/method signatures.\n */\npublic typealias Uuid = String\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic struct FfiConverterTypeUuid: FfiConverter {\n    public static func read(from buf: inout (data: Data, offset: Data.Index)) throws -> Uuid {\n        return try FfiConverterString.read(from: &buf)\n    }\n\n    public static func write(_ value: Uuid, into buf: inout [UInt8]) {\n        return FfiConverterString.write(value, into: &buf)\n    }\n\n    public static func lift(_ value: RustBuffer) throws -> Uuid {\n        return try FfiConverterString.lift(value)\n    }\n\n    public static func lower(_ value: Uuid) -> RustBuffer {\n        return FfiConverterString.lower(value)\n    }\n}\n\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeUuid_lift(_ value: RustBuffer) throws -> Uuid {\n    return try FfiConverterTypeUuid.lift(value)\n}\n\n#if swift(>=5.8)\n@_documentation(visibility: private)\n#endif\npublic func FfiConverterTypeUuid_lower(_ value: Uuid) -> RustBuffer {\n    return FfiConverterTypeUuid.lower(value)\n}\n";
    const VTABLE_DECL_OLD: &str =
        "    static let vtablePtr: UnsafePointer<UniffiVTableCallbackInterfaceConnectionStateObserver> = {";
    const VTABLE_DECL_NEW: &str =
        "    nonisolated(unsafe) static let vtablePtr: UnsafePointer<UniffiVTableCallbackInterfaceConnectionStateObserver> = {";

    let modulemap_path = out_dir.join("MinosCoreFFI.modulemap");
    if modulemap_path.exists() {
        let original = fs::read_to_string(&modulemap_path)
            .with_context(|| format!("reading {}", modulemap_path.display()))?;
        let updated = original
            .replace(MODULEMAP_DECL, MODULEMAP_DECL_NORMALIZED)
            .replace(MODULEMAP_DECL_ALREADY_NORMALIZED, MODULEMAP_DECL_NORMALIZED);
        if !updated.contains(MODULEMAP_DECL_NORMALIZED) {
            bail!(
                "codegen drift: {} contains no recognizable module declaration \
                 (expected one of '{}', '{}', or '{}'). upstream uniffi-bindgen-swift \
                 output may have changed — update normalize_generated_uniffi_imports.",
                modulemap_path.display(),
                MODULEMAP_DECL,
                MODULEMAP_DECL_ALREADY_NORMALIZED,
                MODULEMAP_DECL_NORMALIZED,
            );
        }
        if updated != original {
            fs::write(&modulemap_path, updated)
                .with_context(|| format!("writing {}", modulemap_path.display()))?;
        }
    }

    for file_name in [
        "minos_agent_runtime.swift",
        "minos_daemon.swift",
        "minos_domain.swift",
        "minos_protocol.swift",
        "minos_pairing.swift",
    ] {
        let path = out_dir.join(file_name);
        if !path.exists() {
            continue;
        }

        let original =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        let mut updated = original
            .replace(
                "#if canImport(minos_agent_runtimeFFI)\nimport minos_agent_runtimeFFI\n#endif",
                MODULE_IMPORT,
            )
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
            )
            .replace(
                "#if canImport(minos_protocolFFI)\nimport minos_protocolFFI\n#endif",
                MODULE_IMPORT,
            );

        // Every per-crate file self-imports at least its own FFI submodule, so after
        // the replacements above at least one `import MinosCoreFFI` block
        // must be present. If none is, the `#if canImport(..._FFI)` pragma layout
        // upstream has changed and we need to refresh the needles.
        if !updated.contains(MODULE_IMPORT) {
            bail!(
                "codegen drift: {} contains no recognizable `import minos_*FFI` block \
                 to rewrite, nor an existing `import MinosCoreFFI` marker. upstream \
                 uniffi-bindgen-swift output may have changed — update \
                 normalize_generated_uniffi_imports.",
                path.display()
            );
        }

        if file_name == "minos_daemon.swift" {
            // After the plan-05 refactor, `DeviceId` / `DeviceSecret` are
            // registered only in their home crate `minos-domain` (see
            // minos-domain::ids::uniffi_bridges), so the per-newtype
            // typealias blocks should appear exactly once across all
            // bindgen outputs. The per-type scrubs are split (rather
            // than one monolithic needle) so a future remote-newtype
            // landing between existing entries surfaces as targeted
            // drift rather than a silently-failed monolithic match;
            // each is `optional` because the canonical world has them
            // already absent — they only need scrubbing if a future
            // uniffi-bindgen-swift release reintroduces them.
            updated = replace_optional(updated, DUPLICATE_DAEMON_DEVICE_ID_BLOCK, "");
            updated = replace_optional(updated, DUPLICATE_DAEMON_DEVICE_SECRET_BLOCK, "");
            updated = replace_optional(updated, DUPLICATE_DAEMON_UUID_BLOCK, "");
            updated = replace_required(
                updated,
                VTABLE_DECL_OLD,
                VTABLE_DECL_NEW,
                "minos_daemon.swift: ConnectionStateObserver vtable needs \
                 `nonisolated(unsafe)` for SWIFT_STRICT_CONCURRENCY=complete",
            )?;
        }

        if updated != original {
            fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
        }
    }

    Ok(())
}

/// Replace `needle` with `replacement` in `buf` if present. Unlike
/// `replace_required`, silently no-ops when the needle is absent. Use when
/// the fixup is conditional — e.g. stripping a duplicate block that may or
/// may not have been emitted depending on how the Rust side registered its
/// UniFFI bridges.
fn replace_optional(buf: String, needle: &str, replacement: &str) -> String {
    if buf.contains(needle) {
        buf.replace(needle, replacement)
    } else {
        buf
    }
}

/// Replace `needle` with `replacement` in `buf` and bail if `needle` is not
/// present. Used for load-bearing fixups to `uniffi-bindgen-swift` output where
/// a silent no-op would produce Swift that fails to compile further down the
/// pipeline with a confusing error.
fn replace_required(buf: String, needle: &str, replacement: &str, context: &str) -> Result<String> {
    if !buf.contains(needle) {
        bail!(
            "codegen drift: expected needle not found — {context}. upstream \
             uniffi-bindgen-swift output may have changed. Head of missing needle: {}",
            needle.lines().next().unwrap_or("<empty>")
        );
    }
    Ok(buf.replace(needle, replacement))
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

fn gen_frb() -> Result<()> {
    let root = workspace_root()?;
    if which("flutter_rust_bridge_codegen").is_none() {
        bail!(
            "flutter_rust_bridge_codegen not found on PATH. Run `cargo xtask bootstrap` \
             to install it (cargo install flutter_rust_bridge_codegen --version ^2.11 \
             --locked)."
        );
    }

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
        "==> flutter_rust_bridge_codegen generate --config-file {config_display}",
        config_display = config.display()
    );
    run(
        "flutter_rust_bridge_codegen",
        &["generate", "--config-file", config.to_str().unwrap()],
        &mobile_root,
    )
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

fn host_macos_rust_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64-apple-darwin"
    } else {
        panic!("unsupported macOS host arch for UniFFI codegen")
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

/// Wipe and recreate the backend SQLite DB at ./minos-backend.db.
///
/// Removes the db file (plus `-shm` / `-wal` sidecars if SQLite is in WAL mode)
/// and then re-runs migrations via `--exit-after-migrate`. Idempotent — missing
/// files are ignored.
fn backend_db_reset() -> Result<()> {
    let root = workspace_root()?;

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
        &root,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_required_performs_replacement_when_needle_present() {
        let result =
            replace_required("hello world".into(), "world", "there", "test context").unwrap();
        assert_eq!(result, "hello there");
    }

    #[test]
    fn replace_required_bails_with_drift_message_when_needle_missing() {
        let err = replace_required("hello world".into(), "MISSING", "x", "vtable decl")
            .expect_err("expected drift error when needle is absent");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("codegen drift"),
            "error should tag itself as codegen drift: {msg}"
        );
        assert!(
            msg.contains("vtable decl"),
            "error should include caller-supplied context: {msg}"
        );
    }

    #[test]
    fn replace_optional_replaces_when_needle_present() {
        let result = replace_optional("hello world".into(), "world", "there");
        assert_eq!(result, "hello there");
    }

    #[test]
    fn replace_optional_returns_buf_unchanged_when_needle_missing() {
        let result = replace_optional("hello world".into(), "MISSING", "x");
        assert_eq!(result, "hello world");
    }
}
