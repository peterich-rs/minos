# Minos · Unified Config Pipeline — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-29 |
| Owner | fannnzhang |
| Repository | `github.com/peterich-rs/minos` |
| Proposed branch | `feature/unified-config-pipeline` (worktree at `../minos-worktrees/unified-config-pipeline` — recommended) |
| Related ADRs | 0013 (compile-time backend URL), 0014 (backend-assembled QR — partially superseded), 0016 (client-side CF Access), proposes 0018 (just-driven config & build pipeline) |
| Companion plan | `docs/superpowers/plans/09-unified-config-pipeline.md` |

---

## 1. Context

The project has accumulated configuration in five different places that don't talk to each other:

1. **Backend runtime config** (`crates/minos-backend/src/config.rs`) — `clap` with `env = "..."` attrs, read from process environment at boot. Five vars: `MINOS_BACKEND_LISTEN`, `MINOS_BACKEND_DB`, `MINOS_BACKEND_LOG_DIR`, `MINOS_BACKEND_TOKEN_TTL`, `MINOS_JWT_SECRET`, plus `RUST_LOG`. Validates JWT secret presence and length at startup.
2. **Daemon compile-time config** (`crates/minos-daemon/src/config.rs`) — `option_env!("MINOS_BACKEND_URL")` baked at build, fallback `ws://127.0.0.1:8787/devices`. Plus `RelayConfig` runtime overrides through the UniFFI surface.
3. **Mobile FFI compile-time config** (`crates/minos-mobile/src/build_config.rs`) — `option_env!` on three vars: `MINOS_BACKEND_URL`, `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`. Same localhost fallback. `build.rs` declares `rerun-if-env-changed` for cache invalidation.
4. **Flutter compile-time config** — `--dart-define=CF_ACCESS_CLIENT_ID=...` per ADR 0016 + `apps/mobile/README.md`. Dart-side reads via `String.fromEnvironment`.
5. **Per-developer shell environment + Xcode scheme** — the user has been hand-rolling env vars in their shell or in `Runner.xcscheme`'s `<EnvironmentVariables>` block. There is no checked-in `.env.example` and no documented entry point.

This setup is broken in three observable ways:

**A. Mobile builds bake `localhost` into the binary even when the developer has set the env var.** Root cause: `Runner.xcscheme` puts env vars under `<LaunchAction>` (active at app **launch**), but cargokit's Rust compile runs during `<BuildAction>` where those vars don't propagate. By the time the app launches with the configured `MINOS_BACKEND_URL`, the Rust constant is already `ws://127.0.0.1:8787/devices` from the localhost fallback. The fallback "succeeds silently"; nothing tells the developer that compilation didn't see their var.

**B. Secrets are checked into the repository.** The current uncommitted change to `Runner.xcscheme` includes the literal Cloudflare Access service-token client id and secret in plaintext (`b631dbf2...access` / `627993fe...e6535`). Once committed, those values live in `git log` forever. Even if rotated later, the historical credentials are recoverable.

**C. Dead vars accumulate.** `MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_CLIENT_ID`, `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` were used by the backend in plan 05 / ADR 0014's "backend-assembled QR" model. ADR 0016 superseded that model and removed the backend's reads. The vars persist as ghosts in the xcscheme, in error message strings (`crates/minos-domain/src/error.rs:751`), in `docs/ops/cloudflare-tunnel-setup.md`, and in old plans/ADRs that are still authoritative-looking. New contributors can't tell which vars are live.

The cumulative cost: configuring a new dev/prod environment requires touching the xcscheme, the developer's shell rcfile, the Cloudflare tunnel runbook, possibly `flutter run --dart-define` flags, and possibly a CI secrets configuration — with no single document that says "set these N values." The same string `ws://127.0.0.1:8787/devices` is hardcoded as a fallback in three Rust source files that don't reference each other; the same `127.0.0.1:8787` listen address is hardcoded in two more.

This spec specifies a single source-of-truth `.env.local` file at the repository root, a `justfile` that is the **only** sanctioned entry point for build and run commands, fail-fast behaviour for missing required variables in release builds, default-string consolidation into the existing `minos-domain` shared crate, removal of dead vars, and a documented secret-rotation flow that does not depend on git-tracked plaintext secrets.

---

## 2. Goals

### 2.1 MVP scope (this spec)

1. **Single `.env.local` at the repository root.** Loaded by the justfile and exported into every subprocess invocation (cargo, flutter, xcodebuild, cargo run -p minos-backend). `.env.example` is committed as the canonical list of supported vars with documentation.
2. **`justfile` as the public entry point plus build-script bootstrap.** Recipes: `just backend`, `just build-daemon`, `just build-macos`, `just build-mobile-rust`, `just build-mobile-ios`, `just build-mobile-android`, `just check`, `just smoke-fake-peer`, `just clean`, `just rotate-cf-access` (documentation-only printer). README and mobile README point readers at `just <recipe>`. Existing `cargo xtask` recipes that wrap cargo continue to work unchanged, and native build scripts call back into `just` for IDE Run paths.
3. **Default-string consolidation.** Three Rust source files currently hardcode `ws://127.0.0.1:8787/devices` as a fallback (`crates/minos-mobile/src/build_config.rs:20`, `crates/minos-daemon/src/config.rs:7`, plus three clap defaults in `crates/minos-mobile/src/bin/fake-peer.rs`). New module `crates/minos-domain/src/defaults.rs` exposes `pub const DEV_BACKEND_URL: &str = "ws://127.0.0.1:8787/devices"`. All five sites import from that module. Tests that asserted the literal string assert via the constant.
4. **Dead-var removal.** Drop `MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_CLIENT_ID`, `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` from: the user's `Runner.xcscheme`, `crates/minos-domain/src/error.rs` test fixtures, `docs/ops/cloudflare-tunnel-setup.md` examples. Old plans (05) and ADRs (0014, 0016) keep the historical references but get a banner pointing at this spec for the current state.
5. **Secret hygiene.** Rotate the Cloudflare Access service token whose secret leaked into the in-flight xcscheme change. Strip CF Access values out of `Runner.xcscheme` entirely; the scheme's `<EnvironmentVariables>` block keeps only non-secret runtime knobs (none, after this work). All secrets live in `.env.local` (gitignored) and the GitHub Actions secret store.
6. **IDE build auto-bootstrap.** Xcode IDE "Build" and "Run" must not bypass env injection. The macOS XcodeGen project invokes `just --command cargo xtask gen-uniffi`, `just --command cargo xtask build-macos`, and patches the built app `Info.plist`; mobile Cargokit re-enters `just` before invoking cargo. The Runner target keeps an early env check for clear missing-`just` / missing-env errors instead of requiring `MINOS_BUILD_VIA_JUST`.
7. **Fail-fast on missing release config.** When building any release artifact (mobile FFI release, daemon release), missing `MINOS_BACKEND_URL` triggers a `compile_error!` at `option_env!` resolution time. Debug builds keep the localhost fallback but emit a `cargo:warning=...` from `build.rs` when no value was injected, surfacing the silent fallback in the build log. Backend's existing `Config::validate()` JWT-secret check is unchanged and remains the runtime equivalent.
8. **Documentation.** Rewrite the "Local setup" section of `README.md` and `apps/mobile/README.md` to point at `cp .env.example .env.local && just <recipe>`. Add ADR 0018 documenting the just entry-point and secret-storage policy. Add `docs/ops/secrets-rotation.md` with the CF Access rotation runbook. Existing ADR 0013 and 0016 get a `Status: Superseded by 0018` banner where applicable.
9. **CI compatibility.** Existing CI workflow uses `secrets.MINOS_BACKEND_URL` exported as an env var to `cargo build`. Justfile recipes work the same way: just exports vars to subprocesses, doesn't care whether they came from `.env.local` or the parent process. CI is updated to invoke `just <recipe>` instead of bare `cargo build` so the failure modes match local.

### 2.2 Non-goals (explicit deferrals)

- **Multi-environment manifests** (`envs/{local,staging,prod}.env` selected by `MINOS_ENV=staging just build-mobile-ios`). Single `.env.local` is sufficient until there are observable workflows that need to switch between named environments without editing one file. Revisit when there's a staging Cloudflare tunnel separate from production.
- **Login-page Server-URL field** (let the user override the baked URL at app startup). Explicitly waived in conversation: the URL is needed for the very first request (`POST /v1/auth/login`), so a runtime override would have to live before authentication, which adds an onboarding step that doesn't exist today. Keep the URL build-time-only. If a forking community develops, revisit.
- **Vault / 1Password / macOS Keychain integration for the developer `.env.local`.** `.env.local` lives on disk, gitignored, with the same protection level as a `~/.zshrc` containing `export OPENAI_API_KEY=...`. Encrypted-at-rest secret storage is out of scope until there's a production-deployment workflow that needs it.
- **Cargo workspace metadata for the JustFile recipes** (e.g. `cargo-make`, `xtask` extension). `just` is a polyglot task runner and the build pipeline crosses cargo + flutter + xcodebuild; xtask is Rust-only. Keep `xtask` for cargo-flavored chores (`xtask check-all`, `xtask gen-frb`, `xtask backend-db-reset`); `just` is the cross-language driver that wraps xtask and the other tools.
- **Removing `Config::validate()` from `minos-backend`.** Backend's runtime validation is correct as-is; this spec only tightens compile-time validation for the client crates.
- **Android product hardening beyond Cargokit env bootstrap.** `just build-mobile-android` exists as a placeholder that runs `flutter build apk` with env passthrough; Cargokit loads `.env.local` before cargo, but no Gradle-specific policy/guard beyond that. Android is not part of the slack.ai-style MVP target surface yet (per `mobile-auth-and-agent-session-design.md`).

### 2.3 Testing philosophy

This is infrastructure work; classical TDD doesn't fit cleanly. The verification rail is:

- **Compile checks** for Rust changes (`cargo xtask check-all` is the gate).
- **Existing unit tests** for `build_config.rs` and `daemon/config.rs` are updated to assert via the new constant rather than the literal string. They become the canary for "did the consolidation break anything."
- **Grep checks** for completion criteria: after consolidation, `rg 'ws://127\.0\.0\.1:8787/devices' crates/` should return only `crates/minos-domain/src/defaults.rs`. After dead-var removal, `rg 'MINOS_BACKEND_PUBLIC_URL|MINOS_BACKEND_CF_ACCESS' crates/ apps/ .github/` should return zero matches.
- **Manual smoke** for justfile recipes: each recipe is invoked with a fresh `.env.local` and the expected outcome is documented in the plan's verification step.
- **End-to-end network smoke** for the mobile localhost bug fix: build a release iOS artifact via `just build-mobile-ios` with `MINOS_BACKEND_URL=wss://minos.fan-nn.top/devices` in `.env.local`, install on a device, watch a packet capture confirm the first WS upgrade targets `minos.fan-nn.top`, not `127.0.0.1`.

No new unit tests are added for shell scripts or the justfile itself; per the user's standing preference, "unit tests cover logic only." The justfile is glue, the verification is the smoke run.

---

## 3. Tech Stack and Defaults

Inherits from the project root and earlier specs. Deltas:

| Area | Change |
|---|---|
| Task runner | New top-level `justfile`. Requires `just` ≥ 1.30 (already on developer machines per `which just`). |
| Env loader | `just`'s built-in `set dotenv-load := true` reads `.env.local` from the recipe's working directory. No external dotenv tool. |
| Shared constants | New module `crates/minos-domain/src/defaults.rs` exporting `pub const DEV_BACKEND_URL: &str` and `pub const DEV_BACKEND_LISTEN: &str`. Re-exported from the crate root. |
| IDE bootstrap | macOS XcodeGen build phases and mobile Cargokit scripts invoke `just` so `.env.local` is loaded before cargo. The Runner target has an early env check for clear diagnostics. |
| Build profile detection | Mobile FFI's `build.rs` learns to inspect `PROFILE` env var (cargo-injected: `debug` / `release`) and emit `compile_error!` via `cargo:rustc-cfg` machinery when `PROFILE=release` and `MINOS_BACKEND_URL` is unset. |

No new crate dependencies. No new Flutter packages.

---

## 4. Design

### 4.1 Single `.env.local`

`.env.local` lives at the repository root. Format is shell-sourceable `KEY=value` lines (`just`'s `dotenv-load` parser is compatible). Committed `.env.example` is the schema:

```sh
# Minos environment configuration.
#
# Copy this file to `.env.local` (gitignored) and fill in values.
# All `just` recipes auto-load `.env.local` and forward vars to subprocesses.
#
# CI uses GitHub Secrets exported as env vars at job time and bypasses
# .env.local entirely (just still works because vars are present in the
# parent environment).

# === REQUIRED ===

# Backend WebSocket URL the mobile and daemon clients dial.
# Local dev:  ws://127.0.0.1:8787/devices
# Production: wss://your-domain.example/devices
# Note: path must end in /devices.
MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices

# HS256 signing secret for backend account-auth bearer tokens.
# Must be ≥32 bytes. Generate: openssl rand -hex 32
# Required by `just backend`. Not needed for `just build-*` recipes.
MINOS_JWT_SECRET=replace-me-with-32-byte-random-string

# === OPTIONAL: Cloudflare Access (set both or neither) ===
# Required when MINOS_BACKEND_URL is wss:// behind a Cloudflare Access app.
# Get from Cloudflare Zero Trust → Access → Service Tokens.
# CF_ACCESS_CLIENT_ID=your-id.access
# CF_ACCESS_CLIENT_SECRET=your-secret-hex

# === OPTIONAL: Backend operational ===
# MINOS_BACKEND_LISTEN=127.0.0.1:8787
# MINOS_BACKEND_DB=./minos-backend.db
# MINOS_BACKEND_TOKEN_TTL=300
# MINOS_BACKEND_LOG_DIR=  # platform default: ~/Library/Logs/Minos on macOS
# RUST_LOG=info
```

`.gitignore` adds `.env.local` and `.env.local.*`. The test suite never reads `.env.local`; backend tests use clap's argv path (`Config::try_parse_from(...)`) and the env-lock pattern already in place at `crates/minos-backend/src/config.rs:146`.

### 4.2 `justfile` as sole entry point

Top of `justfile`:

```just
# Minos task runner. Run `just` to list recipes.
#
# Loads .env.local from the workspace root and exports every defined var
# to recipe subprocesses. CI sets vars in the parent environment instead.
set dotenv-load := true
set positional-arguments := true
set shell := ["bash", "-cu"]

# Default recipe: list available commands.
default:
    @just --list
```

Recipes (one per line in this listing; full bodies in the plan):

| Recipe | What it does |
|---|---|
| `just backend` | `cargo run -p minos-backend -- --listen "$MINOS_BACKEND_LISTEN" --db ./minos-backend.db`. Fails fast if `MINOS_JWT_SECRET` is unset. |
| `just check` | `cargo xtask check-all`. Inherits env passthrough so any compile-time fail-fast triggers correctly. |
| `just build-daemon profile='release'` | `cargo build -p minos-daemon --bin minos-daemon --profile {{profile}}` with `MINOS_BACKEND_URL` and `CF_ACCESS_CLIENT_*` injected. |
| `just build-macos configuration='Debug'` | Regenerates UniFFI/XcodeGen outputs and runs `xcodebuild`; the generated project also calls back into `just` from its build phases. |
| `just build-mobile-rust target='aarch64-apple-ios' profile='release'` | `cargo build -p minos-ffi-frb --target {{target}} --profile {{profile}}` with the three client vars injected. |
| `just build-mobile-ios configuration='Release'` | `(cd apps/mobile && flutter build ios --config-only)` then `xcodebuild -workspace ... build` with the client vars exported; Cargokit also self-bootstraps through `just` for direct IDE/Flutter builds. Variant `just dev-mobile-ios` runs `flutter run --dart-define=...` for hot-reload work. |
| `just build-mobile-android` | Stub: `flutter build apk` with env passthrough; Cargokit still loads `.env.local` before cargo. |
| `just smoke-fake-peer kind='register'` | `cargo run -p minos-mobile --bin fake-peer -- {{kind}} --backend "$MINOS_BACKEND_URL"`. Single dispatch for the two existing fake-peer subcommands. |
| `just clean` | `cargo clean && (cd apps/mobile && flutter clean)`. |
| `just rotate-cf-access` | Pure documentation: prints the rotation runbook from `docs/ops/secrets-rotation.md` so a developer doesn't have to hunt for it. No state mutation. |

The justfile is the documented public surface. Native build systems may invoke
it internally so IDE Run paths behave the same way, but contributors should
still prefer the public recipes for repeatable local and CI workflows.

### 4.3 Default-string consolidation

New file `crates/minos-domain/src/defaults.rs`:

```rust
//! Compile-time default constants shared across crates.
//!
//! These exist so the same dev-fallback string isn't hardcoded in three
//! places that drift independently. Any new fallback that needs to be
//! identical between client crates belongs here.

/// Local backend URL used when `MINOS_BACKEND_URL` is unset at compile time.
/// Matches `--listen 127.0.0.1:8787` plus the `/devices` WebSocket path.
pub const DEV_BACKEND_URL: &str = "ws://127.0.0.1:8787/devices";

/// Default backend listen socket, mirrored by `MINOS_BACKEND_LISTEN`.
/// Used as the fallback in `crates/minos-backend/src/config.rs` clap default.
pub const DEV_BACKEND_LISTEN: &str = "127.0.0.1:8787";
```

`crates/minos-domain/src/lib.rs` adds `pub mod defaults;`. Five callers updated:

- `crates/minos-mobile/src/build_config.rs:20` — `None => minos_domain::defaults::DEV_BACKEND_URL`. Test at `:52` now uses `assert_eq!(BACKEND_URL, minos_domain::defaults::DEV_BACKEND_URL)`.
- `crates/minos-daemon/src/config.rs:7` — same substitution.
- `crates/minos-mobile/src/bin/fake-peer.rs:89,109,124` — clap `default_value_t = minos_domain::defaults::DEV_BACKEND_URL.to_string()` (clap can't take a `&'static str` for an owned `String` field directly, so the `.to_string()` is required).
- `crates/minos-backend/src/config.rs:35` — change `default_value = "127.0.0.1:8787"` to `default_value_t = minos_domain::defaults::DEV_BACKEND_LISTEN.parse::<SocketAddr>().expect("compile-time-valid socket")`. Compile-time-valid; the test at `:178` uses the constant via `parse`.

After this work: `rg 'ws://127\.0\.0\.1:8787/devices' crates/` returns only `crates/minos-domain/src/defaults.rs`. `rg '"127\.0\.0\.1:8787"' crates/` returns the same file plus the test in `minos-backend/src/config.rs` (literal in test scaffolding).

### 4.4 Dead-var removal

| Var | Live usage today | Action |
|---|---|---|
| `MINOS_BACKEND_PUBLIC_URL` | None in source. Mentioned in xcscheme (`Runner.xcscheme:96`), `docs/ops/cloudflare-tunnel-setup.md:185,202`, plan 05, ADR 0014. | Remove from xcscheme. Strike from cloudflare-tunnel-setup.md (replace with a sentence: "Mobile and daemon clients dial the URL baked at build time; the tunnel runbook does not configure it"). Add `Status: Historical (see 0018)` banner to plan 05 §10 and ADR 0014 §2 where the var appears. |
| `MINOS_BACKEND_CF_ACCESS_CLIENT_ID` / `..._SECRET` | None in active source. Referenced in `crates/minos-domain/src/error.rs:751,753` test fixture for the `CfAccessMisconfigured` variant. Mentioned in xcscheme (`Runner.xcscheme:86,91`), plan 05, ADR 0014, ADR 0016. | Remove from xcscheme. Update the error-test fixture string to `"missing CF_ACCESS_CLIENT_ID"` (drop the `MINOS_BACKEND_` prefix; the error message should reflect the live var name, not the dead one). Banner same ADRs. |
| `CF_ACCESS_CLIENT_ID` / `CF_ACCESS_CLIENT_SECRET` | Live in `build_config.rs`, `daemon/config.rs`, `apps/mobile` Dart layer via `--dart-define`. | No change. These are canonical. |
| `MINOS_BACKEND_URL` | Live in `build_config.rs`, `daemon/config.rs`, `build.rs`. | No change; canonical. |

### 4.5 Secret hygiene

The current uncommitted `Runner.xcscheme` change embeds the literal CF Access service-token client secret. Since the change is uncommitted, the secret is **not yet** in `git log` — the rotation requirement is contingent. Two paths:

- **Path α (preferred):** the user does NOT commit the in-flight xcscheme change. We strip the secrets from the file as part of this plan's first phase. The CF Access service token does not need to be rotated.
- **Path β (if already committed before this plan starts):** rotate the CF Access service token in Cloudflare Zero Trust, update `.env.local` and the GitHub Actions secret, scrub the value from `git log` is **not** attempted (rewrites of public history are worse than the leaked-credential exposure once the credential is invalidated).

The plan's Phase 1 starts with `git status` to determine which path applies and bails to ask the user before proceeding. Either way, the end state is: `Runner.xcscheme` `<EnvironmentVariables>` is empty (or removed entirely if it has no remaining keys). All CF Access values come from `.env.local` (dev) or `secrets.CF_ACCESS_CLIENT_*` (CI).

`docs/ops/secrets-rotation.md` (new file) documents the CF rotation runbook:

```
1. In Cloudflare Zero Trust → Access → Service Tokens, click "Rotate" on the Minos token.
2. Cloudflare displays a new client_secret ONCE; copy it immediately.
3. Update .env.local with the new value (do NOT delete the old yet).
4. Update GitHub Actions secret CF_ACCESS_CLIENT_SECRET.
5. Wait for the next CI build of the iOS release artifact (≈5 min) so production
   has a binary signed with the new value.
6. Revoke the old client_secret in Cloudflare. The overlap window must be
   at least one full CI cycle to avoid breaking active sessions.
```

The `just rotate-cf-access` recipe `cat`s this file so it's discoverable.

### 4.6 IDE build bootstrap

The mobile Runner target keeps an early `PBXShellScriptBuildPhase` for
diagnostics, but the load-bearing env injection lives at the actual Rust
compile point: `apps/mobile/rust_builder/cargokit/run_build_tool.sh`
re-execs itself through `just --command` before it invokes cargo. That means
direct `flutter run`, `flutter build`, and Xcode IDE Build/Run all load the
workspace `.env.local` before `option_env!` is evaluated.

The Runner env-check script:

```sh
if ! command -v just >/dev/null 2>&1; then
  echo "error: just is required so Minos can load .env.local during the Rust build."
  exit 1
fi
ROOT="$(cd "$SRCROOT/../../.." && pwd -P)"
just --justfile "$ROOT/justfile" --working-directory "$ROOT" check-env
```

The macOS XcodeGen project follows the same principle. Its pre-build phase
runs `just --command cargo xtask gen-uniffi` and then `just --command cargo
xtask build-macos --configuration "$CONFIGURATION"` so generated bindings are
fresh and `minos-daemon` sees the loaded env during compile. A post-build
phase calls `_patch-macos-info-plist` to write the same relay values into the
built app bundle's `Info.plist` for Finder/Xcode launches.

`Runner.xcscheme` `<EnvironmentVariables>` block is emptied (kept as `<EnvironmentVariables/>` self-closing for diff clarity). The launch-time vars previously listed there were either dead (`MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_*`) or now redundant (`MINOS_BACKEND_URL`, `CF_ACCESS_CLIENT_*` get baked at build time, not consumed at launch).

### 4.7 Fail-fast on missing release config

`crates/minos-mobile/build.rs` and a new `crates/minos-daemon/build.rs` (currently absent — daemon doesn't have one) gain:

```rust
fn main() {
    println!("cargo:rerun-if-env-changed=MINOS_BACKEND_URL");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=CF_ACCESS_CLIENT_SECRET");
    println!("cargo:rerun-if-env-changed=PROFILE");

    let profile = std::env::var("PROFILE").unwrap_or_default();
    let backend_url = std::env::var("MINOS_BACKEND_URL").ok();

    if profile == "release" && backend_url.is_none() {
        // cargo prints this and aborts the build.
        println!(
            "cargo:warning=MINOS_BACKEND_URL is unset in a release build. \
             The binary will dial localhost. Set the var via .env.local + just."
        );
        // Stronger: refuse to compile. Comment in/out per release-mode policy.
        // panic!("release builds require MINOS_BACKEND_URL");
    } else if backend_url.is_none() {
        println!(
            "cargo:warning=MINOS_BACKEND_URL unset (debug build) — using localhost fallback."
        );
    }
}
```

Two-tier: warning-only by default for debug, escalating to `panic!` for release. The `panic!` line is gated behind a Cargo feature `strict-release-config` that CI enables for production artifacts; local release builds can opt out for debugging. The feature is added to `crates/minos-mobile/Cargo.toml` and `crates/minos-daemon/Cargo.toml` and toggled by `just build-mobile-ios --strict` / `just build-daemon --strict`.

(Actually, the simpler path is to skip the cargo feature and always panic on release — local "release for debugging" is rare and the developer can manually set `MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices` in `.env.local` to satisfy it. Decision deferred to the plan; both paths are cheap.)

Backend's existing `Config::validate()` (`crates/minos-backend/src/config.rs:100`) already does runtime fail-fast for `MINOS_JWT_SECRET`. No change.

### 4.8 CI workflow update

`.github/workflows/ci.yml` currently has `cargo build -p minos-daemon --bin minos-daemon` at line 70 and similar for mobile. After this work:

- The `cargo build` invocations get prefixed with the env vars CI provides via `secrets.*`, but to keep the failure modes identical between local and CI, they get rewritten to `just build-daemon` etc.
- The CI's secret-injection step (currently in the workflow's `env:` block at the job level) is unchanged — `just` reads from process env when `.env.local` is absent.
- For PRs from forks where `secrets.MINOS_BACKEND_URL` resolves to empty, CI builds run as `debug` (no `--release`) so the warn-only fail-fast tier applies and the build doesn't break.

---

## 5. Surface inventory

### 5.1 New files

| Path | Purpose |
|---|---|
| `.env.example` | Schema for `.env.local`. Committed. |
| `.env.local` | Per-developer values. **NOT** committed; added to `.gitignore`. Each developer creates their own. |
| `justfile` | Workspace task runner. Committed. |
| `crates/minos-domain/src/defaults.rs` | Shared default constants. |
| `crates/minos-daemon/build.rs` | Cargo build script for env-change tracking + fail-fast. |
| `docs/ops/secrets-rotation.md` | CF Access token rotation runbook. |
| `docs/adr/0018-just-config-pipeline.md` | ADR. |

### 5.2 Modified files

| Path | Change |
|---|---|
| `.gitignore` | Add `.env.local`, `.env.local.*`. |
| `Cargo.toml` (workspace) | Add `crates/minos-mobile/build.rs` workspace warn (no schema change). |
| `crates/minos-domain/src/lib.rs` | `pub mod defaults;`. |
| `crates/minos-mobile/src/build_config.rs` | Replace literal localhost with `minos_domain::defaults::DEV_BACKEND_URL`. Update test. |
| `crates/minos-mobile/Cargo.toml` | Add `minos-domain = { path = "../minos-domain" }` if not already present. |
| `crates/minos-mobile/build.rs` | Add release-tier warn/panic logic. |
| `crates/minos-daemon/src/config.rs` | Replace literal with constant. |
| `crates/minos-daemon/Cargo.toml` | Add `build = "build.rs"` and `minos-domain` dep if absent. |
| `crates/minos-mobile/src/bin/fake-peer.rs` | Replace three clap defaults. |
| `crates/minos-backend/src/config.rs` | Replace listen literal at `:35`; update test scaffolding to use the constant. |
| `crates/minos-domain/src/error.rs:751,753` | Update test fixture strings to drop `MINOS_BACKEND_` prefix on CF var names. |
| `apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme` | Empty `<EnvironmentVariables>` block (or remove if Xcode tolerates absence). |
| `apps/mobile/ios/Runner.xcodeproj/project.pbxproj` | `PBXShellScriptBuildPhase` for the early just/env check. |
| `apps/mobile/rust_builder/cargokit/run_build_tool.sh` | Re-exec through `just --command` before invoking cargo. |
| `apps/macos/project.yml` | Build phases call back into `just`; post-build phase patches runtime Info.plist config. |
| `README.md` | Rewrite "Local setup" section (lines around 127–148 by current count). |
| `apps/mobile/README.md` | Rewrite the Cloudflare Access section to point at just. |
| `docs/ops/cloudflare-tunnel-setup.md:185,202` | Drop `MINOS_BACKEND_PUBLIC_URL` references. |
| `docs/adr/0013-macos-relay-client-cutover.md` | `Status: Superseded by 0018` banner. |
| `docs/adr/0014-backend-assembled-pairing-qr.md` | `Status: Partially superseded (CF and PUBLIC_URL by 0016/0018)` banner. |
| `docs/adr/0016-client-env-cloudflare-access.md` | `Status: Refined by 0018 (entry-point and storage)` banner. |
| `.github/workflows/ci.yml` | Rewrite the `cargo build` invocations to `just <recipe>`. |

### 5.3 Files explicitly NOT touched

- `crates/minos-backend/migrations/**` — no schema impact.
- `xtask/**` — kept as the cargo-flavored chore runner; just wraps it.

---

## 6. Migration / rollout

There is no observable user-facing change. The plan executes in this order to keep the workspace bootable at every checkpoint:

1. **Foundation** (`.env.example`, `.gitignore`, justfile skeleton, defaults module). Repository now has the new files, no behavioural change.
2. **Constant consolidation** (callers swap to the new constant). `cargo xtask check-all` passes; tests still pass with the same string asserted via the constant.
3. **Justfile recipes for low-risk wrappers** (`just check`, `just backend`, `just smoke-fake-peer`). No source change; just shell glue.
4. **Justfile recipes for build wrappers** (`just build-daemon`, `just build-mobile-rust`, `just build-mobile-ios`). End-to-end smoke confirms the localhost bug is fixed.
5. **Xcode/Cargokit integration** (env check + self-bootstrap + xcscheme cleanup + secret strip). After this checkpoint, Xcode IDE Build/Run and direct Flutter builds load `.env.local` before Rust compiles.
6. **Dead-var removal** (xcscheme strip is already done in step 5; this step does the docs/error-message cleanup).
7. **Fail-fast** (`build.rs` updates).
8. **Documentation** (READMEs, ADR 0018, rotation runbook, banners on superseded ADRs).
9. **CI workflow update**.
10. **End-to-end verification**.

If any phase fails, the workspace is left in a state where the previous phase's work stands and only the failing phase is rolled back. There are no destructive migrations (no `git rm` of files that other branches need; no DB schema changes; no secret rotations that block other developers).

The user will need to:

- After the plan lands, run `cp .env.example .env.local` and fill in values once.
- Prefer `just build-mobile-ios` / `just dev-mobile-ios` for repeatable workflows; direct IDE/Flutter builds now self-bootstrap but remain secondary paths.

CI maintainers will need to:

- Confirm GitHub Secrets are populated for `MINOS_BACKEND_URL`, `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`, `MINOS_JWT_SECRET` (presumably already populated).
- Approve the workflow rewrite that swaps `cargo build` → `just <recipe>`.

---

## 7. Validation

Each phase has a checkpoint command and expected output (full bodies in the plan). The end-to-end gate before merge:

1. `cargo xtask check-all` passes with the consolidated constants.
2. `rg 'ws://127\.0\.0\.1:8787/devices' crates/` returns exactly one file.
3. `rg 'MINOS_BACKEND_PUBLIC_URL|MINOS_BACKEND_CF_ACCESS' crates/ apps/ .github/` returns zero matches.
4. `rg 'CF_ACCESS_CLIENT_SECRET=[a-f0-9]{30,}' apps/` returns zero matches (no committed plaintext secret).
5. `MINOS_BACKEND_URL=wss://minos.fan-nn.top/devices just build-mobile-ios` produces an IPA. Installing it on a device and capturing the first WS handshake shows it targets `minos.fan-nn.top`, not `127.0.0.1`.
6. Clicking "Build" in Xcode without first exporting any env still reaches the just/env check; with `.env.local` present it proceeds and Cargokit loads those values before cargo.
7. `just check` is the same as `cargo xtask check-all`.
8. README's "Local setup" section walks a fresh contributor through `cp .env.example .env.local` → `just backend` → `just smoke-fake-peer` and works without any side commands.

---

## 8. Open questions

These are the calls the implementing engineer should make explicitly when executing the plan:

1. **Strict-release-config feature flag** (§4.7). Two options: (a) gate the `panic!` behind a Cargo feature toggleable via `--strict`; (b) always panic in release. The plan starts with (b) for simplicity and adds (a) only if the user pushes back during review.
2. **Xcscheme `<EnvironmentVariables/>` self-close vs. remove the element.** Xcode regenerates the file on save; safer to leave the element in place but empty. The plan does this.
3. **Banner format on superseded ADRs.** Existing ADRs use a `| Status | Accepted |` table row. The plan changes this to `| Status | Refined by 0018 (entry-point and storage) |` rather than removing the original `Status: Accepted` — preserving the historical decision shape.
4. **Whether to add `just build-mobile-android`.** The plan adds the stub recipe so the recipe list has a placeholder; if Android is genuinely out of scope the developer should remove it. Default: keep the stub.
5. **CF Access token rotation scope.** Per §4.5 path α/β. The plan's Phase 1 prompts the implementing engineer for the answer based on `git status`.
