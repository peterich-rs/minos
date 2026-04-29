# Unified Config Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse Minos environment configuration into a single `.env.local` + `justfile` entry point, fix the mobile "localhost baked into release" bug, get plaintext CF Access secrets out of the source tree, and consolidate three duplicate localhost fallbacks into one shared constant. Per `docs/superpowers/specs/unified-config-pipeline-design.md`.

**Architecture:** Three layers of change. (1) **Workspace foundation**: new `.env.example` + `justfile` + `crates/minos-domain/src/defaults.rs` + `.gitignore` updates land first; nothing breaks. (2) **Source consolidation**: five Rust callers swap their hardcoded `ws://127.0.0.1:8787/devices` literal to the shared constant; `cargo xtask check-all` is the gate. (3) **Build pipeline**: `just <recipe>` becomes the only sanctioned entry point; `Runner.xcscheme` loses its plaintext secrets; a Pre-Build Run Script Phase fails the Xcode IDE Build with a clear message when not invoked via just; `build.rs` warn/panic on missing release config.

**Tech Stack:**
- Task runner: `just` ≥ 1.30 (already installed at `/Users/fannnzhang/.cargo/bin/just`).
- Env loading: `just`'s built-in `set dotenv-load := true`. No external dotenv tool.
- Shared constants: existing `minos-domain` crate, new `defaults.rs` module.
- Cargo build script: standard `build.rs` per crate; reads `PROFILE` env var injected by cargo.
- Xcode hook: `PBXShellScriptBuildPhase` in `project.pbxproj` (single shell snippet).

**Critical clarifications (read before starting):**

- The `minos_domain::defaults::DEV_BACKEND_URL` constant must be a `&'static str` so it's usable in `const` context (`option_env!`'s match arm) and in clap's `default_value_t` via `.parse()` / `.to_string()`. Don't introduce a function.
- `crates/minos-mobile/src/build_config.rs` already has `build.rs` declaring `rerun-if-env-changed` for the three vars (see `crates/minos-mobile/build.rs`). Don't recreate that file; extend it.
- `crates/minos-daemon` does NOT have a `build.rs` today. Adding one requires a `build = "build.rs"` line in `crates/minos-daemon/Cargo.toml` (otherwise cargo ignores the file).
- Backend `Config::validate()` (`crates/minos-backend/src/config.rs:100`) already enforces `MINOS_JWT_SECRET` presence and length. **Do not modify.** This plan only adds compile-time fail-fast on the client crates.
- `Runner.xcscheme`'s `<EnvironmentVariables>` block is currently in the user's uncommitted change (lines 79–110 of the file). Phase 1 starts with `git status` to confirm whether the secrets are still uncommitted (path α — strip them, no rotation needed) or already committed (path β — rotate first, then strip).
- `apps/mobile/rust_builder/cargokit/build_pod.sh:14` runs `env` at build time; cargo inherits the shell. So setting env vars before `xcodebuild build` correctly propagates them through cargokit to `cargo build -p minos-ffi-frb`. The fix for the localhost bug is purely "set the vars at the right scope" — no cargokit changes.
- The Pre-Build Run Script Phase must be the **first** build phase on the `Runner` target so it fails before cargokit runs (and before any source compiles). Place it at the top of `buildPhases` in the `Runner` PBXNativeTarget block.
- CI's job-level `env:` block at `.github/workflows/ci.yml` already exports the secrets. Replacing `cargo build` with `just <recipe>` does not require touching the env block; just inherits.
- This plan touches `apps/mobile/ios/Runner.xcodeproj/project.pbxproj` — a brittle, generated-looking file. Edits must be surgical (one `PBXShellScriptBuildPhase` block + one entry in the target's `buildPhases` array). After every pbxproj edit, run `xcodebuild -list -workspace apps/mobile/ios/Runner.xcworkspace` to confirm the project still parses.
- `crates/minos-mobile/Cargo.toml` may not currently depend on `minos-domain`. Phase 2 Step 1 verifies and adds the dep before any source-side reference.

**Worktree (recommended):**

Per CLAUDE.md and superpowers:using-git-worktrees, run this plan in an isolated worktree so it doesn't intermix with the in-flight `feature/mobile-auth-and-agent-session` work. The current branch has uncommitted changes including the xcscheme that this plan rewrites — running in a new worktree off `main` keeps those changes safe and lets you cherry-pick or merge after this plan ships.

```bash
cd /Users/fannnzhang/code/github.com/Minos
git worktree add ../minos-worktrees/unified-config-pipeline -b feature/unified-config-pipeline main
cd ../minos-worktrees/unified-config-pipeline
```

If the user prefers to land this on `feature/mobile-auth-and-agent-session` directly (because the bug fix is a blocker for that branch's smoke tests), skip the worktree and work in place. **Confirm with the user before Phase 1.**

**Phase map:**
1. Foundation: `.env.example`, `.gitignore`, `defaults.rs`, justfile skeleton
2. Constant consolidation in Rust source
3. Justfile low-risk recipes (`check`, `backend`, `smoke-fake-peer`, `clean`)
4. Justfile build recipes (`build-daemon`, `build-mobile-rust`, `build-mobile-ios`, `dev-mobile-ios`, `build-mobile-android`)
5. Xcode integration: secret strip + Pre-Build hook
6. Dead-var cleanup: error messages, ops doc
7. Fail-fast: `build.rs` warn/panic
8. Documentation: README rewrite, ADR 0018, rotation runbook, banners on superseded ADRs
9. CI workflow update
10. End-to-end verification

---

## Phase 1: Foundation

### Task 1.1: Confirm secret-leak scope (Path α vs. β)

**Files:** none modified; this is an inspection task.

- [ ] **Step 1: Check uncommitted xcscheme state**

Run: `git status apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme`

If output shows ` M ` (modified, uncommitted): **Path α applies** — the CF secret has not entered git history. Continue with Phase 1 normally.

If output is clean: **Path β applies** — check `git log -p apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme` for the literal string `627993fe39e5b909`. If present, the secret is in history. Stop and ask the user to rotate the CF Access service token first per `docs/ops/secrets-rotation.md` (created in Phase 8 — for now follow the inline instructions in the design doc §4.5).

- [ ] **Step 2: Record decision**

Note in your worktree commit message for Phase 1 which path applies. Example commit body line: `Path α (uncommitted): no rotation required; xcscheme strip in Phase 5.`

### Task 1.2: Add `.env.example`

**Files:**
- Create: `.env.example` (workspace root)

- [ ] **Step 1: Write the file**

Path: `.env.example`

```sh
# Minos environment configuration.
#
# Copy this file to `.env.local` (gitignored) and fill in values.
# All `just` recipes auto-load `.env.local` and forward vars to subprocesses.
#
# CI uses GitHub Secrets exported as env vars at job time and bypasses
# .env.local entirely (just still works because vars are present in the
# parent environment).
#
# Schema reference: docs/superpowers/specs/unified-config-pipeline-design.md §4.1

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

- [ ] **Step 2: Verify it parses as shell-sourceable**

Run: `bash -n .env.example`
Expected: no output, exit code 0.

(`bash -n` does syntax check only; no execution.)

- [ ] **Step 3: Stage**

Run: `git add .env.example`

### Task 1.3: Update `.gitignore`

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Append .env.local patterns**

Add at the end of `.gitignore`:

```
# Per-developer environment configuration. .env.example is the schema.
.env.local
.env.local.*
```

- [ ] **Step 2: Verify pattern works**

Run: `touch .env.local && git status .env.local`
Expected: no output (file is ignored).

Then: `rm .env.local`

- [ ] **Step 3: Stage**

Run: `git add .gitignore`

### Task 1.4: Add `defaults.rs` to `minos-domain`

**Files:**
- Create: `crates/minos-domain/src/defaults.rs`
- Modify: `crates/minos-domain/src/lib.rs`

- [ ] **Step 1: Write the module**

Path: `crates/minos-domain/src/defaults.rs`

```rust
//! Compile-time default constants shared across crates.
//!
//! These exist so the same dev-fallback string isn't hardcoded in three
//! places that drift independently. Any new fallback that needs to be
//! identical between client crates belongs here.
//!
//! See `docs/superpowers/specs/unified-config-pipeline-design.md` §4.3.

/// Local backend URL used when `MINOS_BACKEND_URL` is unset at compile time.
/// Matches `--listen 127.0.0.1:8787` plus the `/devices` WebSocket path.
pub const DEV_BACKEND_URL: &str = "ws://127.0.0.1:8787/devices";

/// Default backend listen socket, mirrored by `MINOS_BACKEND_LISTEN`.
/// Used as the fallback in `crates/minos-backend/src/config.rs` clap default.
pub const DEV_BACKEND_LISTEN: &str = "127.0.0.1:8787";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_backend_url_uses_dev_listen_socket() {
        // Pin the relationship: the URL constant must encode the listen
        // constant, otherwise the two will drift.
        assert!(DEV_BACKEND_URL.contains(DEV_BACKEND_LISTEN));
    }

    #[test]
    fn dev_backend_url_is_a_websocket_url() {
        assert!(
            DEV_BACKEND_URL.starts_with("ws://"),
            "DEV_BACKEND_URL must be a ws:// URL for local dev"
        );
        assert!(
            DEV_BACKEND_URL.ends_with("/devices"),
            "DEV_BACKEND_URL path must terminate in /devices per backend route"
        );
    }
}
```

- [ ] **Step 2: Re-export from the crate root**

Edit `crates/minos-domain/src/lib.rs`. Find the existing `pub mod ...;` block (likely near the top of the file) and add:

```rust
pub mod defaults;
```

- [ ] **Step 3: Verify it compiles and tests pass**

Run: `cargo test -p minos-domain --lib defaults::tests`
Expected: `test result: ok. 2 passed; 0 failed`

- [ ] **Step 4: Commit**

```bash
git add crates/minos-domain/src/defaults.rs crates/minos-domain/src/lib.rs
git commit -m "feat(domain): add defaults module for shared constants

Introduces DEV_BACKEND_URL and DEV_BACKEND_LISTEN so the same dev-fallback
strings aren't hardcoded across mobile, daemon, and backend. Phase 2
swaps the duplicates."
```

### Task 1.5: Create justfile skeleton

**Files:**
- Create: `justfile` (workspace root)

- [ ] **Step 1: Write the skeleton**

Path: `justfile`

```just
# Minos task runner. Run `just` to list recipes.
#
# Loads .env.local from the workspace root and exports every defined var
# to recipe subprocesses. CI sets vars in the parent environment instead;
# this works the same way (just doesn't care where the vars came from).
#
# Reference: docs/superpowers/specs/unified-config-pipeline-design.md §4.2
set dotenv-load := true
set positional-arguments := true
set shell := ["bash", "-cu"]

# Default recipe: list available commands.
default:
    @just --list

# Verify .env.local exists and has the required keys.
# Prints a summary; doesn't print secret values.
check-env:
    @if [ ! -f .env.local ]; then \
        echo "error: .env.local not found. Run: cp .env.example .env.local"; \
        exit 1; \
    fi
    @echo "MINOS_BACKEND_URL = ${MINOS_BACKEND_URL:-<unset>}"
    @echo "MINOS_JWT_SECRET  = ${MINOS_JWT_SECRET:+<set, ${#MINOS_JWT_SECRET} chars>}"
    @echo "CF_ACCESS_CLIENT_ID     = ${CF_ACCESS_CLIENT_ID:-<unset>}"
    @echo "CF_ACCESS_CLIENT_SECRET = ${CF_ACCESS_CLIENT_SECRET:+<set>}"
```

- [ ] **Step 2: Verify just parses it**

Run: `just --list`
Expected output (order may vary):
```
Available recipes:
    check-env
    default
```

- [ ] **Step 3: Verify `check-env` reports missing file when absent**

Run: `just check-env`
Expected: `error: .env.local not found. Run: cp .env.example .env.local` and exit code 1.

- [ ] **Step 4: Smoke test with a real .env.local**

Run:
```bash
cp .env.example .env.local
just check-env
```

Expected output (no quoting issues, no secret leaks):
```
MINOS_BACKEND_URL = ws://127.0.0.1:8787/devices
MINOS_JWT_SECRET  = <set, 39 chars>
CF_ACCESS_CLIENT_ID     = <unset>
CF_ACCESS_CLIENT_SECRET = <unset>
```

(If `MINOS_JWT_SECRET` value is the example placeholder, `<set, 39 chars>` is its length.)

- [ ] **Step 5: Clean up the smoke .env.local**

Run: `rm .env.local`

(The next phase doesn't need it; we'll recreate as needed.)

- [ ] **Step 6: Commit**

```bash
git add justfile
git commit -m "feat(workspace): add justfile with .env.local loader

Single entry point for build and run commands. Loads .env.local from the
workspace root and forwards vars to subprocesses. check-env recipe gives
a no-secret-leak summary of what's loaded.

Subsequent phases add backend, build-*, smoke-fake-peer recipes."
```

### Task 1.6: Phase 1 verification

- [ ] **Step 1: Confirm workspace still builds**

Run: `cargo xtask check-all`
Expected: PASS (no source change beyond `defaults.rs` + `lib.rs` reexport).

- [ ] **Step 2: Stop for review**

Pause here. The user reviews:
- `.env.example` schema captures every var they actually use.
- `justfile` skeleton parses correctly.
- `defaults.rs` constants match what they expect.

Continue to Phase 2 only after approval.

---

## Phase 2: Constant consolidation

### Task 2.1: Confirm `minos-mobile` depends on `minos-domain`

**Files:**
- Inspect: `crates/minos-mobile/Cargo.toml`
- Modify (conditional): `crates/minos-mobile/Cargo.toml`

- [ ] **Step 1: Inspect**

Run: `grep '^minos-domain' crates/minos-mobile/Cargo.toml`
If output is non-empty: dep already present, skip Step 2.
If empty: continue.

- [ ] **Step 2: Add dep (only if Step 1 found nothing)**

In `crates/minos-mobile/Cargo.toml`, under `[dependencies]`, add (alphabetically — likely just before `minos-pairing` or `minos-protocol`):

```toml
minos-domain = { path = "../minos-domain", version = "0.1.0" }
```

- [ ] **Step 3: Verify resolution**

Run: `cargo check -p minos-mobile --no-default-features`
Expected: PASS.

### Task 2.2: Swap `minos-mobile/src/build_config.rs` to constant

**Files:**
- Modify: `crates/minos-mobile/src/build_config.rs`

- [ ] **Step 1: Update the const declaration**

In `crates/minos-mobile/src/build_config.rs`, replace the literal at line 18–21:

Current:
```rust
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};
```

Replace with:
```rust
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => minos_domain::defaults::DEV_BACKEND_URL,
};
```

- [ ] **Step 2: Update the unit test**

In the same file, replace the existing test body at line 47–54:

Current:
```rust
#[test]
fn backend_url_has_a_sane_dev_fallback() {
    assert_eq!(BACKEND_URL, "ws://127.0.0.1:8787/devices");
    assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
}
```

Replace with:
```rust
#[test]
fn backend_url_has_a_sane_dev_fallback() {
    // Note: this test runs in `cargo test -p minos-mobile` with no
    // MINOS_BACKEND_URL set, so we expect the shared dev fallback.
    // The constant lives in `minos-domain` so all client crates point
    // at the same string — see unified-config-pipeline-design.md §4.3.
    assert_eq!(BACKEND_URL, minos_domain::defaults::DEV_BACKEND_URL);
    assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
}
```

- [ ] **Step 3: Verify tests pass**

Run: `cargo test -p minos-mobile --lib build_config::tests`
Expected: `test result: ok. 2 passed; 0 failed` (the second test `cf_access_helper_matches_const_pair_state` is unchanged).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-mobile/src/build_config.rs crates/minos-mobile/Cargo.toml
git commit -m "refactor(mobile): point BACKEND_URL fallback at minos-domain const

Drops the duplicated literal 'ws://127.0.0.1:8787/devices'. Daemon and
fake-peer follow in the next two tasks."
```

### Task 2.3: Swap `minos-daemon/src/config.rs` to constant

**Files:**
- Inspect: `crates/minos-daemon/Cargo.toml`
- Modify (conditional): `crates/minos-daemon/Cargo.toml`
- Modify: `crates/minos-daemon/src/config.rs`

- [ ] **Step 1: Confirm `minos-domain` dep**

Run: `grep '^minos-domain' crates/minos-daemon/Cargo.toml`
If empty, add to `[dependencies]`:
```toml
minos-domain = { path = "../minos-domain", version = "0.1.0" }
```

- [ ] **Step 2: Update the const**

In `crates/minos-daemon/src/config.rs`, replace lines 5–8:

Current:
```rust
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => "ws://127.0.0.1:8787/devices",
};
```

Replace with:
```rust
pub const BACKEND_URL: &str = match option_env!("MINOS_BACKEND_URL") {
    Some(v) => v,
    None => minos_domain::defaults::DEV_BACKEND_URL,
};
```

- [ ] **Step 3: Update the unit test at the bottom of the file**

Replace the existing assertion at line 49–51:

Current:
```rust
#[test]
fn backend_url_has_a_sane_fallback() {
    assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
}
```

Replace with:
```rust
#[test]
fn backend_url_has_a_sane_fallback() {
    // With no MINOS_BACKEND_URL at test-build time, BACKEND_URL must
    // fall back to the shared dev constant from minos-domain.
    assert_eq!(BACKEND_URL, minos_domain::defaults::DEV_BACKEND_URL);
    assert!(BACKEND_URL.starts_with("ws://") || BACKEND_URL.starts_with("wss://"));
}
```

- [ ] **Step 4: Verify**

Run: `cargo test -p minos-daemon --lib config::tests`
Expected: `test result: ok. 3 passed` (the existing 2 + the rewritten 1).

- [ ] **Step 5: Commit**

```bash
git add crates/minos-daemon/src/config.rs crates/minos-daemon/Cargo.toml
git commit -m "refactor(daemon): point BACKEND_URL fallback at minos-domain const"
```

### Task 2.4: Swap `fake-peer.rs` clap defaults

**Files:**
- Modify: `crates/minos-mobile/src/bin/fake-peer.rs`

- [ ] **Step 1: Inspect the three sites**

Run: `grep -n 'default_value = "ws://127.0.0.1:8787/devices"' crates/minos-mobile/src/bin/fake-peer.rs`
Expected: three lines (currently `:89`, `:109`, `:124`).

- [ ] **Step 2: Add a `use` for the constant**

Find the existing `use` block near the top. Add:
```rust
use minos_domain::defaults::DEV_BACKEND_URL;
```

- [ ] **Step 3: Replace each clap default**

For each of the three matched lines, replace:
```rust
#[arg(long, default_value = "ws://127.0.0.1:8787/devices")]
```

With:
```rust
#[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
```

(The `_t` suffix takes a typed value; `String` works because the field type is `String`. The `.to_string()` is needed because clap's `default_value_t` consumes ownership.)

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p minos-mobile --bin fake-peer`
Expected: PASS.

- [ ] **Step 5: Smoke test the bin still has the right default**

Run: `cargo run -p minos-mobile --bin fake-peer -- register --help 2>&1 | grep -A1 -- '--backend'`
Expected output (formatting may vary):
```
      --backend <BACKEND>
          [default: ws://127.0.0.1:8787/devices]
```

(If clap renders the `_t` default differently, the value displayed should still be the expected URL.)

- [ ] **Step 6: Commit**

```bash
git add crates/minos-mobile/src/bin/fake-peer.rs
git commit -m "refactor(fake-peer): point clap defaults at DEV_BACKEND_URL"
```

### Task 2.5: Swap `minos-backend/src/config.rs` listen default

**Files:**
- Inspect: `crates/minos-backend/Cargo.toml`
- Modify (conditional): `crates/minos-backend/Cargo.toml`
- Modify: `crates/minos-backend/src/config.rs`

- [ ] **Step 1: Confirm `minos-domain` dep**

Run: `grep '^minos-domain' crates/minos-backend/Cargo.toml`
If empty, add to `[dependencies]`:
```toml
minos-domain = { path = "../minos-domain", version = "0.1.0" }
```

- [ ] **Step 2: Update clap default**

In `crates/minos-backend/src/config.rs`, find line 35:

Current:
```rust
#[arg(long, env = "MINOS_BACKEND_LISTEN", default_value = "127.0.0.1:8787")]
pub listen: SocketAddr,
```

Replace with:
```rust
#[arg(
    long,
    env = "MINOS_BACKEND_LISTEN",
    default_value_t = minos_domain::defaults::DEV_BACKEND_LISTEN
        .parse::<SocketAddr>()
        .expect("DEV_BACKEND_LISTEN is a compile-time-valid SocketAddr"),
)]
pub listen: SocketAddr,
```

- [ ] **Step 3: Update the test fixture if needed**

The test at `crates/minos-backend/src/config.rs:177-180` asserts:
```rust
assert_eq!(
    cfg.listen,
    "127.0.0.1:8787".parse::<SocketAddr>().unwrap(),
    "default --listen must match plan §10"
);
```

Leave the literal in the test (it is the canary that pins the constant's value). If you prefer, adjust the panic message:
```rust
"default --listen must match minos_domain::defaults::DEV_BACKEND_LISTEN"
```

- [ ] **Step 4: Verify**

Run: `cargo test -p minos-backend --lib config::tests::default_flags_match_plan_defaults`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-backend/src/config.rs crates/minos-backend/Cargo.toml
git commit -m "refactor(backend): point listen default at DEV_BACKEND_LISTEN const"
```

### Task 2.6: Phase 2 verification

- [ ] **Step 1: Workspace check**

Run: `cargo xtask check-all`
Expected: PASS.

- [ ] **Step 2: Grep — only one source of truth left**

Run:
```bash
rg 'ws://127\.0\.0\.1:8787/devices' crates/
```

Expected: exactly one file in the output, `crates/minos-domain/src/defaults.rs:14`. (Plus possibly test scaffolding in `relay_http.rs:225-226` and `subscribe_no_runtime.rs:10` — these are test/doc strings, not fallbacks. Leave them.)

If other crate-level fallbacks remain, fix them now before moving on.

- [ ] **Step 3: Grep — listen address is consolidated**

Run:
```bash
rg '"127\.0\.0\.1:8787"' crates/
```

Expected: `crates/minos-domain/src/defaults.rs` plus the test scaffolding in `crates/minos-backend/src/config.rs` (literal in the test fixture). Both are intentional.

- [ ] **Step 4: Pause for review**

Stop. The user verifies no behavioural change has been introduced — this is purely a refactor.

---

## Phase 3: Justfile low-risk recipes

### Task 3.1: `just backend`

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Append the recipe**

Add to `justfile` after the `check-env` recipe:

```just
# Run minos-backend with values loaded from .env.local.
# Fails fast if MINOS_JWT_SECRET is unset (Config::validate enforces
# presence + ≥32 bytes at startup).
backend:
    @just check-env >/dev/null
    @if [ -z "${MINOS_JWT_SECRET:-}" ]; then \
        echo "error: MINOS_JWT_SECRET is required by minos-backend"; \
        exit 1; \
    fi
    cargo run -p minos-backend -- \
        --listen "${MINOS_BACKEND_LISTEN:-127.0.0.1:8787}" \
        --db "${MINOS_BACKEND_DB:-./minos-backend.db}"
```

- [ ] **Step 2: Verify recipe lists**

Run: `just --list`
Expected: `backend` appears in the list.

- [ ] **Step 3: Verify fail-fast on missing JWT secret**

Run:
```bash
cp .env.example .env.local
# Edit .env.local to remove MINOS_JWT_SECRET line, then:
just backend
```

Expected: `error: MINOS_JWT_SECRET is required by minos-backend` and exit 1.

- [ ] **Step 4: Verify with valid secret**

Run:
```bash
echo 'MINOS_JWT_SECRET=01234567890123456789012345678901' > .env.local
echo 'MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices' >> .env.local
just backend
```

Expected: `cargo run -p minos-backend ...` builds and runs; the backend logs `listening on 127.0.0.1:8787` and `migrations applied`. Hit Ctrl-C to stop.

- [ ] **Step 5: Cleanup and commit**

Run: `rm .env.local`

```bash
git add justfile
git commit -m "feat(just): add 'backend' recipe with JWT-secret precheck"
```

### Task 3.2: `just check`

- [ ] **Step 1: Append**

```just
# Workspace-wide compile + test gate. Wraps cargo xtask check-all.
check:
    cargo xtask check-all
```

- [ ] **Step 2: Verify**

Run: `just check`
Expected: same output as `cargo xtask check-all`. PASS.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "feat(just): add 'check' wrapping cargo xtask check-all"
```

### Task 3.3: `just smoke-fake-peer`

- [ ] **Step 1: Append**

```just
# Run the fake-peer smoke binary with a subcommand (default: register).
# Usage: just smoke-fake-peer [register|smoke-session]
smoke-fake-peer kind='register':
    @just check-env >/dev/null
    cargo run -p minos-mobile --bin fake-peer -- \
        {{kind}} --backend "$MINOS_BACKEND_URL"
```

- [ ] **Step 2: Verify --help reachable**

Run:
```bash
cp .env.example .env.local
just smoke-fake-peer register --help 2>&1 | head -5 || true
```

(The `--help` won't propagate through this recipe shape — fake-peer subcommand args after `kind` aren't supported by the simple recipe. Leave the recipe simple; advanced invocation goes through bare cargo.)

Expected: at minimum the cargo invocation runs and either registers or fails with a clear error if the backend is unreachable.

- [ ] **Step 3: Cleanup and commit**

Run: `rm .env.local`

```bash
git add justfile
git commit -m "feat(just): add 'smoke-fake-peer' for register/session smokes"
```

### Task 3.4: `just clean`

- [ ] **Step 1: Append**

```just
# Remove all build artifacts (cargo target/ + flutter build/).
clean:
    cargo clean
    cd apps/mobile && flutter clean
```

- [ ] **Step 2: Verify**

Run: `just clean`
Expected: `cargo clean` removes `target/`; `flutter clean` removes `apps/mobile/build/`. No errors.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "feat(just): add 'clean' to wipe cargo + flutter artifacts"
```

### Task 3.5: Phase 3 verification

- [ ] **Step 1: Recipe inventory**

Run: `just --list`
Expected (alphabetical by default):
```
Available recipes:
    backend
    check
    check-env
    clean
    default
    smoke-fake-peer kind='register'
```

- [ ] **Step 2: Pause**

Confirm with the user that the recipe set is what they want before adding the heavier build recipes in Phase 4.

---

## Phase 4: Justfile build recipes

### Task 4.1: `just build-daemon`

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Append**

```just
# Build the minos-daemon binary with env vars baked into the Rust compile.
# profile = release | debug
build-daemon profile='release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL must be set in .env.local for build-daemon"; \
        exit 1; \
    fi
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    cargo build -p minos-daemon --bin minos-daemon --profile {{profile}}
```

- [ ] **Step 2: Verify**

```bash
cp .env.example .env.local
# Edit MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices in .env.local
just build-daemon debug
```

Expected: `cargo build -p minos-daemon` runs to completion. The compiled binary at `target/debug/minos-daemon` has `BACKEND_URL` baked from `.env.local`.

- [ ] **Step 3: Verify the value baked in**

Run:
```bash
strings target/debug/minos-daemon | grep -E 'ws://[^"]+/devices' | head -1
```

Expected: shows the URL from `.env.local` (`ws://127.0.0.1:8787/devices` for the example).

If you set `.env.local` `MINOS_BACKEND_URL=wss://test.example/devices` and rebuild, `strings` should show `wss://test.example/devices` instead.

- [ ] **Step 4: Cleanup and commit**

Run: `rm .env.local`

```bash
git add justfile
git commit -m "feat(just): add 'build-daemon' recipe with env-var injection"
```

### Task 4.2: `just build-mobile-rust`

- [ ] **Step 1: Append**

```just
# Build the mobile Rust FFI staticlib for a given target.
# target  = aarch64-apple-ios | aarch64-apple-ios-sim | x86_64-apple-ios | <android targets>
# profile = release | debug
build-mobile-rust target='aarch64-apple-ios' profile='release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-rust"; \
        exit 1; \
    fi
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    cargo build -p minos-ffi-frb --target {{target}} --profile {{profile}}
```

- [ ] **Step 2: Verify the iOS device target builds**

```bash
cp .env.example .env.local
just build-mobile-rust aarch64-apple-ios debug
```

Expected: cargo builds the staticlib for `aarch64-apple-ios`. Output goes to `target/aarch64-apple-ios/debug/`.

(If the iOS toolchain isn't installed: `rustup target add aarch64-apple-ios` first.)

- [ ] **Step 3: Cleanup and commit**

Run: `rm .env.local`

```bash
git add justfile
git commit -m "feat(just): add 'build-mobile-rust' for FFI staticlib builds"
```

### Task 4.3: `just build-mobile-ios` (release)

- [ ] **Step 1: Append**

```just
# Build a Release iOS app via xcodebuild. Sets MINOS_BUILD_VIA_JUST=1
# so the project's Pre-Build Run Script Phase doesn't fail (added in
# Phase 5). Env vars MINOS_BACKEND_URL / CF_ACCESS_CLIENT_* are exported
# into the xcodebuild environment so cargokit's build_pod.sh inherits
# them and cargo build picks them up via option_env!.
#
# configuration = Release | Debug
build-mobile-ios configuration='Release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && flutter build ios --config-only --release
    cd apps/mobile/ios && \
    MINOS_BUILD_VIA_JUST=1 \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    xcodebuild \
        -workspace Runner.xcworkspace \
        -scheme Runner \
        -configuration {{configuration}} \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        build
```

- [ ] **Step 2: Verify the recipe parses**

Run: `just --list | grep build-mobile-ios`
Expected: `build-mobile-ios configuration='Release'` appears.

- [ ] **Step 3: Defer the actual build smoke**

The end-to-end build smoke is deferred to **Phase 5** because the Pre-Build Run Script Phase doesn't exist yet. Without it, this recipe builds successfully but the localhost-baking bug isn't yet fixed (cargokit's env propagation works either way once we set vars at the xcodebuild scope).

For now, do a dry-run that exercises everything except the actual cargo compile:

```bash
cp .env.example .env.local
# Edit MINOS_BACKEND_URL=wss://test.example/devices
cd apps/mobile && flutter build ios --config-only --release && cd ../..
# Confirm the flutter step succeeds. Skip the xcodebuild step until Phase 5.
```

- [ ] **Step 4: Cleanup and commit**

Run: `rm .env.local`

```bash
git add justfile
git commit -m "feat(just): add 'build-mobile-ios' release recipe

The recipe wires env vars into the xcodebuild scope so cargokit
inherits them. The Pre-Build hook that prevents IDE-direct builds
lands in Phase 5; until then both code paths work."
```

### Task 4.4: `just dev-mobile-ios` (debug + flutter run)

- [ ] **Step 1: Append**

```just
# Hot-reload dev workflow. Runs `flutter run` in debug mode with --dart-define
# for Cloudflare Access, and exports MINOS_BACKEND_URL into the parent shell
# so cargokit's Rust compile (triggered by flutter's first build) picks it up.
dev-mobile-ios:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for dev-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BUILD_VIA_JUST=1 \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter run \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"
```

- [ ] **Step 2: Verify the recipe parses**

Run: `just --list | grep dev-mobile-ios`
Expected: `dev-mobile-ios` listed.

- [ ] **Step 3: Defer the smoke until Phase 5 (same reasoning as 4.3)**

- [ ] **Step 4: Commit**

```bash
git add justfile
git commit -m "feat(just): add 'dev-mobile-ios' for flutter run hot-reload"
```

### Task 4.5: `just build-mobile-android` (stub)

- [ ] **Step 1: Append**

```just
# Stub: Android APK build. No Pre-Build hook on Android yet; this recipe
# exists for parity. If Android stops being out-of-scope, harden it.
build-mobile-android:
    @just check-env >/dev/null
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter build apk \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"
```

- [ ] **Step 2: Commit (no smoke — Android may not be set up)**

```bash
git add justfile
git commit -m "feat(just): add 'build-mobile-android' stub recipe"
```

### Task 4.6: Phase 4 verification

- [ ] **Step 1: Recipe inventory**

Run: `just --list`
Expected:
```
Available recipes:
    backend
    build-daemon profile='release'
    build-mobile-android
    build-mobile-ios configuration='Release'
    build-mobile-rust target='aarch64-apple-ios' profile='release'
    check
    check-env
    clean
    dev-mobile-ios
    default
    smoke-fake-peer kind='register'
```

- [ ] **Step 2: Pause**

Confirm with the user. Phase 5 is the load-bearing change (Pre-Build hook). Get a green light first.

---

## Phase 5: Xcode integration — secret strip + Pre-Build hook

### Task 5.1: Strip secrets from `Runner.xcscheme`

**Files:**
- Modify: `apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme`

- [ ] **Step 1: Confirm path α from Phase 1.1**

The xcscheme should still be in your worktree's modified state with the secrets in `<EnvironmentVariables>`. If you're on path β (already committed), the scheme has the same content but the secret is also in git history; the rotation runbook applies (see Phase 1.1).

- [ ] **Step 2: Replace the EnvironmentVariables block**

In `apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme`, find the `<LaunchAction>` block (currently lines 54–111). Replace lines 79–110 (the entire `<EnvironmentVariables>` element with all six children) with:

```xml
      <EnvironmentVariables>
      </EnvironmentVariables>
```

(Self-closing `<EnvironmentVariables/>` works too but Xcode tends to expand it to the open form on next save.)

The `<LaunchAction>` opening (`<LaunchAction ...>`) and closing (`</LaunchAction>`) tags around it stay unchanged. Net effect: lines 79–110 become two lines.

- [ ] **Step 3: Verify Xcode still parses the scheme**

Run:
```bash
xcodebuild -list -workspace apps/mobile/ios/Runner.xcworkspace 2>&1 | head -20
```

Expected: lists the `Runner` scheme without errors. Xcode is tolerant of empty `<EnvironmentVariables>`.

- [ ] **Step 4: Verify no secrets remain in the file**

Run:
```bash
grep -E 'CF_ACCESS|MINOS_BACKEND' apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/ios/Runner.xcodeproj/xcshareddata/xcschemes/Runner.xcscheme
git commit -m "security(ios): strip CF Access secrets from Runner.xcscheme

These were ineffective for fixing the localhost-baking bug (LaunchAction
env vars don't reach BuildAction-time cargo compile) and exposed the CF
Access service-token secret in the source tree.

The just recipes export the same vars at the xcodebuild scope, which
DOES propagate to cargokit's cargo build. Per Path α (Phase 1.1) the
secrets were not yet committed, so no rotation is required."
```

### Task 5.2: Add Pre-Build Run Script Phase to `Runner` target

**Files:**
- Modify: `apps/mobile/ios/Runner.xcodeproj/project.pbxproj`

- [ ] **Step 1: Locate the Runner native target**

Run:
```bash
grep -n '/* Runner */ = {' apps/mobile/ios/Runner.xcodeproj/project.pbxproj | head -3
```

Find the `PBXNativeTarget` block for `Runner` (the one whose `productType = "com.apple.product-type.application"`). Note its `buildPhases = ( ... );` array — that's where the new phase ID goes.

- [ ] **Step 2: Generate a unique 24-char hex ID**

Run:
```bash
openssl rand -hex 12 | tr 'a-f' 'A-F'
```

Example output: `B3F4A1C2D5E60718293A4B5C`. Use this as `<NEW_ID>` in subsequent steps.

- [ ] **Step 3: Add the PBXShellScriptBuildPhase block**

In `project.pbxproj`, find the `/* Begin PBXShellScriptBuildPhase section */` marker. After it, add (using your `<NEW_ID>`):

```
		<NEW_ID> /* MINOS via-just guard */ = {
			isa = PBXShellScriptBuildPhase;
			alwaysOutOfDate = 1;
			buildActionMask = 2147483647;
			files = (
			);
			inputFileListPaths = (
			);
			inputPaths = (
			);
			name = "MINOS via-just guard";
			outputFileListPaths = (
			);
			outputPaths = (
			);
			runOnlyForDeploymentPostprocessing = 0;
			shellPath = /bin/sh;
			shellScript = "if [ \"${MINOS_BUILD_VIA_JUST:-}\" != \"1\" ]; then\n  echo \"error: this build must be invoked via 'just build-mobile-ios' or 'just dev-mobile-ios'.\"\n  echo \"error: direct Xcode IDE Build/Run does not propagate env vars to cargokit's Rust compile,\"\n  echo \"error: which silently bakes localhost into the binary. See docs/superpowers/specs/unified-config-pipeline-design.md §1A.\"\n  exit 1\nfi\necho \"MINOS_BUILD_VIA_JUST=1 → proceeding with build.\"\n";
		};
```

- [ ] **Step 4: Insert the new phase ID at the top of `Runner`'s `buildPhases` array**

Find the `buildPhases = ( ... );` array on the Runner target. The first entry is currently the `[CP] Check Pods Manifest.lock` or similar phase. Insert your new phase **before** all existing entries:

```
			buildPhases = (
				<NEW_ID> /* MINOS via-just guard */,
				<EXISTING_FIRST_PHASE_ID> /* [CP] Check Pods Manifest.lock */,
				<other existing phases>
			);
```

The Pre-Build hook MUST be first so it runs before cargokit. If it's anywhere else, the bake happens before the guard fires.

- [ ] **Step 5: Verify the project still parses**

Run:
```bash
xcodebuild -list -workspace apps/mobile/ios/Runner.xcworkspace
```

Expected: lists schemes without errors. If the pbxproj is malformed, this command emits a parse error and you must revert.

- [ ] **Step 6: Verify the guard fires when invoked outside just**

Run (without `MINOS_BUILD_VIA_JUST=1`):
```bash
cd apps/mobile/ios && \
xcodebuild -workspace Runner.xcworkspace -scheme Runner -configuration Debug \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' build 2>&1 \
    | grep -A3 'MINOS via-just guard\|error:'
```

Expected: the build fails with the three `error:` lines from the guard script. The build does not proceed to cargokit.

- [ ] **Step 7: Verify the guard passes when invoked through just**

```bash
cp .env.example .env.local
just build-mobile-ios Debug 2>&1 | tail -20
```

Expected: build proceeds past the guard (look for `MINOS_BUILD_VIA_JUST=1 → proceeding with build.` in the log) and runs cargokit. The build may still fail later for unrelated reasons (codesigning, simulator vs device, etc.) — for this step, success is "guard didn't fire."

- [ ] **Step 8: Cleanup and commit**

Run: `rm .env.local`

```bash
git add apps/mobile/ios/Runner.xcodeproj/project.pbxproj
git commit -m "feat(ios): add Pre-Build guard requiring MINOS_BUILD_VIA_JUST=1

Direct Xcode IDE Build/Run silently bakes localhost into the Rust FFI
binary because LaunchAction env vars don't reach BuildAction-time cargo
compile. The just recipes set MINOS_BUILD_VIA_JUST=1 and export the
needed vars at the xcodebuild scope; this guard fails fast for any
other invocation path with a clear pointer at the design doc."
```

### Task 5.3: End-to-end smoke — the original bug fix

- [ ] **Step 1: Set up a non-localhost test**

```bash
cat > .env.local <<EOF
MINOS_BACKEND_URL=wss://example.test/devices
MINOS_JWT_SECRET=01234567890123456789012345678901
EOF
```

- [ ] **Step 2: Build a release IPA**

```bash
just build-mobile-ios Release 2>&1 | tail -10
```

Expected: build completes. Locate the output:
```bash
find apps/mobile/build/ios -name 'Runner.app' -type d | head -1
```

- [ ] **Step 3: Verify the URL is baked into the binary**

```bash
APP=$(find apps/mobile/build/ios -name 'Runner.app' -type d | head -1)
strings "$APP/Frameworks/minos_ffi_frb.framework/minos_ffi_frb" 2>/dev/null \
    | grep -E 'wss?://[^"]+/devices'
```

Expected: shows `wss://example.test/devices`. **Not** `ws://127.0.0.1:8787/devices`.

If the framework path differs, run `find "$APP" -name '*minos*'` to locate the Rust staticlib and re-grep.

- [ ] **Step 4: Verify Xcode IDE Build is now blocked**

Open `apps/mobile/ios/Runner.xcworkspace` in Xcode. Choose any simulator destination. Press Cmd+B (Build).

Expected: the build fails almost immediately with a build-log entry visible in Xcode's Issue Navigator showing the three `error:` lines from the guard script.

- [ ] **Step 5: Cleanup**

```bash
rm .env.local
```

- [ ] **Step 6: No commit (this step is verification only).**

### Task 5.4: Phase 5 verification

- [ ] **Step 1: The bug is gone**

Verified by Task 5.3 Step 3. Note this in the worktree's progress log if you keep one.

- [ ] **Step 2: No secrets in the file tree**

Run:
```bash
rg 'CF_ACCESS_CLIENT_SECRET=[a-f0-9]{30,}' apps/
```

Expected: no output (was previously hitting the xcscheme).

- [ ] **Step 3: Pause for review.**

---

## Phase 6: Dead-var cleanup

### Task 6.1: Update error-message fixtures

**Files:**
- Modify: `crates/minos-domain/src/error.rs` (around lines 751, 753)

- [ ] **Step 1: Locate the fixture**

Run:
```bash
grep -n 'MINOS_BACKEND_CF_ACCESS_CLIENT_ID' crates/minos-domain/src/error.rs
```

Expected: lines 751 and 753 (or thereabouts).

- [ ] **Step 2: Update the strings**

Replace:
```rust
reason: "missing MINOS_BACKEND_CF_ACCESS_CLIENT_ID".into(),
```

With:
```rust
reason: "missing CF_ACCESS_CLIENT_ID".into(),
```

And:
```rust
assert!(format!("{e}").contains("missing MINOS_BACKEND_CF_ACCESS_CLIENT_ID"));
```

With:
```rust
assert!(format!("{e}").contains("missing CF_ACCESS_CLIENT_ID"));
```

- [ ] **Step 3: Verify**

Run: `cargo test -p minos-domain --lib error::`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-domain/src/error.rs
git commit -m "refactor(domain): use canonical CF_ACCESS_CLIENT_ID in error fixtures

Drops the stale MINOS_BACKEND_ prefix; that backend-side var was removed
when ADR 0016 moved CF Access to client-side build config."
```

### Task 6.2: Update `cloudflare-tunnel-setup.md`

**Files:**
- Modify: `docs/ops/cloudflare-tunnel-setup.md` (lines 185, 188, 202)

- [ ] **Step 1: Inspect**

```bash
grep -n 'MINOS_BACKEND_PUBLIC_URL\|MINOS_BACKEND_CF_ACCESS' docs/ops/cloudflare-tunnel-setup.md
```

- [ ] **Step 2: Replace lines 185–202 region**

Find the section that currently contains these vars (likely a "Backend env" or similar block) and replace with:

```markdown
The backend itself does not need `MINOS_BACKEND_URL`,
`CF_ACCESS_CLIENT_ID`, or `CF_ACCESS_CLIENT_SECRET`. Mobile and daemon
clients dial the URL baked at build time (set via `.env.local`,
documented in `unified-config-pipeline-design.md`). Cloudflare Access
service tokens are configured on clients only — the backend is unaware
of CF Access (it sees post-edge HTTP loopback).
```

(Adjust the surrounding paragraph to fit; the goal is to remove the dead vars from the runbook so a reader following it doesn't try to set them.)

- [ ] **Step 3: Commit**

```bash
git add docs/ops/cloudflare-tunnel-setup.md
git commit -m "docs(ops): drop dead MINOS_BACKEND_PUBLIC_URL/CF_ACCESS refs

Per ADR 0016 + unified-config-pipeline-design.md §4.4, the backend no
longer reads these vars. The tunnel runbook was the last live mention
outside of historical ADRs."
```

### Task 6.3: Phase 6 verification

- [ ] **Step 1: Grep the corpus**

Run:
```bash
rg 'MINOS_BACKEND_PUBLIC_URL|MINOS_BACKEND_CF_ACCESS' crates/ apps/ .github/ 2>&1
```

Expected: zero matches in `crates/`, `apps/`, `.github/`.

`docs/superpowers/plans/05-mobile-migration-and-ui-protocol.md` and `docs/adr/0014-...md` will still match — that's intentional (historical record). Phase 8 adds banners.

- [ ] **Step 2: Pause for review.**

---

## Phase 7: Fail-fast on missing release config

### Task 7.1: Extend `minos-mobile/build.rs` with PROFILE-aware warnings

**Files:**
- Modify: `crates/minos-mobile/build.rs`

- [ ] **Step 1: Replace the file**

Path: `crates/minos-mobile/build.rs`

```rust
// Cargo's incremental cache pins `option_env!` outputs to the env snapshot at
// the time of the last successful build. Without these declarations, changing
// MINOS_BACKEND_URL or CF_ACCESS_CLIENT_{ID,SECRET} between builds (e.g.
// dev → release) silently reuses the previously baked-in values. Declaring
// `rerun-if-env-changed` forces cargo to mark the crate dirty and recompile
// `build_config.rs` whenever any of the three change.
//
// Additionally surfaces missing-env diagnostics: a debug build with no
// MINOS_BACKEND_URL emits a cargo:warning so the silent localhost fallback
// is visible in the build log; a release build with no MINOS_BACKEND_URL
// panics, preventing the localhost-baking bug from shipping in a release
// artifact.
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
                 .env.local and invoke `just build-mobile-rust ... release` \
                 or `just build-mobile-ios Release`."
            );
        }
        (_, false) => {
            println!(
                "cargo:warning=MINOS_BACKEND_URL unset (debug build) — \
                 minos-mobile is using the dev-fallback DEV_BACKEND_URL."
            );
        }
        _ => {}
    }
}
```

- [ ] **Step 2: Verify debug build still succeeds with a warning**

Run: `cargo build -p minos-ffi-frb --profile dev 2>&1 | grep 'warning:'`
Expected: includes `warning: MINOS_BACKEND_URL unset (debug build)`.

(If the env happens to be set in your shell, unset it: `env -u MINOS_BACKEND_URL cargo build -p minos-ffi-frb --profile dev`.)

- [ ] **Step 3: Verify release build panics without env**

Run: `env -u MINOS_BACKEND_URL cargo build -p minos-ffi-frb --release 2>&1 | tail -5`
Expected: build fails. Last lines include `MINOS_BACKEND_URL is unset for a release build.`

- [ ] **Step 4: Verify release build succeeds with env**

Run: `MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices cargo build -p minos-ffi-frb --release 2>&1 | tail -3`
Expected: build completes successfully (look for `Finished`).

- [ ] **Step 5: Commit**

```bash
git add crates/minos-mobile/build.rs
git commit -m "feat(mobile): fail-fast on missing MINOS_BACKEND_URL in release

Debug builds emit a cargo:warning when the env var is missing so the
silent localhost fallback is visible. Release builds panic at build.rs
time, preventing the localhost-baking bug from shipping. Combined with
Phase 5's Pre-Build hook this is two layers of guard against the
original bug."
```

### Task 7.2: Add `build.rs` to `minos-daemon`

**Files:**
- Create: `crates/minos-daemon/build.rs`
- Modify: `crates/minos-daemon/Cargo.toml`

- [ ] **Step 1: Add `build = "build.rs"` to Cargo.toml**

In `crates/minos-daemon/Cargo.toml`, in the `[package]` section, add:
```toml
build = "build.rs"
```

(Cargo only invokes `build.rs` when this key is set, despite the file being conventionally named.)

- [ ] **Step 2: Write the build script**

Path: `crates/minos-daemon/build.rs`

```rust
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
```

- [ ] **Step 3: Verify**

```bash
env -u MINOS_BACKEND_URL cargo build -p minos-daemon --release 2>&1 | tail -3
```

Expected: build fails with the panic message.

```bash
MINOS_BACKEND_URL=ws://127.0.0.1:8787/devices cargo build -p minos-daemon --release 2>&1 | tail -3
```

Expected: `Finished`.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/build.rs crates/minos-daemon/Cargo.toml
git commit -m "feat(daemon): add build.rs for env tracking + release fail-fast"
```

### Task 7.3: Phase 7 verification

- [ ] **Step 1: workspace check still passes**

Run: `just check`
Expected: PASS. Test runs are debug-mode, so the warning fires but no panic.

- [ ] **Step 2: Pause.**

---

## Phase 8: Documentation

### Task 8.1: Rewrite `README.md` "Local setup" section

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Locate the section**

Run: `grep -n 'cargo run -p minos-backend\|MINOS_JWT_SECRET' README.md`

The current "Local setup" content lives roughly around lines 127–148.

- [ ] **Step 2: Replace with a just-driven flow**

Replace the existing block with:

```markdown
## Local setup

```sh
# 1. Install the task runner (one-time):
brew install just  # or: cargo install just

# 2. Configure environment (one-time):
cp .env.example .env.local
# Edit .env.local: at minimum set MINOS_BACKEND_URL and MINOS_JWT_SECRET.
# Generate a JWT secret: openssl rand -hex 32

# 3. Run the backend:
just backend

# 4. Smoke a fake peer (in another terminal):
just smoke-fake-peer register

# 5. Build the mobile app:
just build-mobile-ios Release
```

All build and run commands go through `just`. Direct `cargo build` / `flutter
build` invocations bypass the env-var injection that bakes
`MINOS_BACKEND_URL` and CF Access credentials into the Rust FFI compile;
the mobile build's Xcode Pre-Build hook will explicitly fail any direct
IDE Build with a pointer to the right recipe.

See `docs/superpowers/specs/unified-config-pipeline-design.md` for the
config pipeline design and `docs/adr/0018-just-config-pipeline.md` for
the policy decision.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): rewrite Local setup around just + .env.local"
```

### Task 8.2: Rewrite `apps/mobile/README.md`

**Files:**
- Modify: `apps/mobile/README.md`

- [ ] **Step 1: Replace the entire file**

Path: `apps/mobile/README.md`

```markdown
# Minos Mobile

Flutter shell for the Minos mobile client.

## Build & run

All commands go through `just` from the workspace root. See the workspace
README for one-time setup (`cp .env.example .env.local`).

```sh
# Production iOS build (Release configuration).
just build-mobile-ios Release

# Hot-reload dev workflow on a simulator or attached device.
just dev-mobile-ios

# Android stub (placeholder; not currently part of the supported surface).
just build-mobile-android
```

Direct invocation of `flutter run` or Xcode IDE Build/Run is **not
supported** — see the Pre-Build error message and
`docs/superpowers/specs/unified-config-pipeline-design.md` §4.6 for why.

## Configuration

`MINOS_BACKEND_URL` and `CF_ACCESS_CLIENT_*` are baked at build time
from `.env.local` (workspace root). The Rust FFI reads them via
`option_env!`; the Dart layer reads CF Access via `String.fromEnvironment`
which `flutter run` populates with `--dart-define` (the just recipe wires
both paths from the same `.env.local`).

iOS Keychain (`flutter_secure_storage`) holds only Minos business state:
`device_id`, `device_secret`, `account_id`, refresh tokens — never the
backend URL or CF Access tokens.
```

- [ ] **Step 2: Commit**

```bash
git add apps/mobile/README.md
git commit -m "docs(mobile): rewrite README around just recipes"
```

### Task 8.3: Add ADR 0018

**Files:**
- Create: `docs/adr/0018-just-config-pipeline.md`

- [ ] **Step 1: Write the ADR**

Path: `docs/adr/0018-just-config-pipeline.md`

```markdown
# 0018 · Just-Driven Config & Build Pipeline

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-29 |
| Deciders | fannnzhang |

## Context

Configuration was scattered across five surfaces (backend clap-env,
daemon `option_env!`, mobile FFI `option_env!`, Flutter `--dart-define`,
per-developer shell + Xcode scheme) with no single entry point. The
mobile build silently baked `localhost` into release artifacts because
`Runner.xcscheme`'s `<EnvironmentVariables>` block applied at LaunchAction
time, after cargokit's BuildAction-time Rust compile had already resolved
`option_env!("MINOS_BACKEND_URL")` to its fallback. Cloudflare Access
service-token secrets ended up in plaintext in the xcscheme.

## Decision

A single workspace-root `.env.local` is the source of truth, loaded by
a `justfile` that is the **only** sanctioned entry point for build and
run commands. Recipes export the loaded vars to subprocess environments
(cargo, flutter, xcodebuild). The Runner Xcode target gets a Pre-Build
Run Script Phase that fails the build if `MINOS_BUILD_VIA_JUST=1` is
absent — guaranteeing IDE Build/Run can't bypass the env-injection.
Mobile FFI and daemon `build.rs` panic on `release` builds with
`MINOS_BACKEND_URL` unset.

Secrets live in `.env.local` (gitignored) for developers and in GitHub
Secrets for CI. The xcscheme's `<EnvironmentVariables>` block is empty;
no plaintext credentials in the source tree.

Default-string consolidation: three duplicate `ws://127.0.0.1:8787/devices`
fallbacks collapse into `minos_domain::defaults::DEV_BACKEND_URL`.

Dead vars (`MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_*`)
removed from active source and ops docs; historical ADRs (0014, 0016)
keep their references with banners pointing here.

## Consequences

Positive:

- One file to edit per environment switch.
- The localhost-baking bug cannot recur: build fails fast in release,
  warns loudly in debug, and the IDE-direct path is blocked entirely.
- CF Access secrets do not enter the source tree.
- A new contributor's onboarding becomes `cp .env.example .env.local
  && just backend` — no shell rcfile editing, no Xcode poking.

Negative:

- Adds `just` as a hard dependency (mitigated by ubiquitous availability:
  `brew install just`, `cargo install just`).
- Disallows the Xcode IDE Build/Run muscle memory; requires either a
  custom Xcode scheme runner or terminal-driven workflow.
- Pbxproj contains a hand-written PBXShellScriptBuildPhase block; if
  Flutter's build tooling regenerates pbxproj it could clobber the
  guard. Flutter currently does not regenerate Runner.pbxproj on
  ordinary builds (only `flutter create` would), so this is low-risk
  but worth flagging.

## Alternatives considered

- **`cargo xtask` extension to wrap flutter and xcodebuild.** Rejected:
  xtask is Rust-only by convention, and pulling in flutter/xcodebuild
  invocation logic muddles its scope. just is polyglot by design.
- **Multi-environment manifests (`envs/{local,staging,prod}.env`).**
  Deferred (design doc §2.2): single `.env.local` is sufficient for
  current workflows.
- **Login-page Server-URL field for runtime override.** Rejected:
  the URL is needed for the very first request (login), so a runtime
  override would have to live before authentication, adding an
  onboarding step.
- **Pre-Build hook as a build-script-only check (no Xcode integration).**
  Rejected: the build script doesn't run for IDE Build until cargokit
  runs, by which time the bake has already happened. The Xcode-level
  hook fires before any Rust source compiles.

Refines (does not replace) ADR 0013, 0014, 0016 — those decisions about
what gets baked into the binary remain in force; this ADR specifies
*how* the baking is invoked and where the values live.
```

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0018-just-config-pipeline.md
git commit -m "docs(adr): 0018 — just-driven config & build pipeline"
```

### Task 8.4: Add `secrets-rotation.md` runbook

**Files:**
- Create: `docs/ops/secrets-rotation.md`

- [ ] **Step 1: Write**

Path: `docs/ops/secrets-rotation.md`

```markdown
# Secrets Rotation Runbook

## Cloudflare Access service token

The mobile app and daemon authenticate to the Cloudflare Access edge
using a service-token pair (`CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`).
Rotate when:
- The secret has been seen in any committed file or shared chat log.
- A developer with access to `.env.local` leaves the project.
- On a quarterly schedule (good hygiene).

### Procedure

1. **Cloudflare Zero Trust → Access → Service Tokens → Minos token → Rotate.**
   Cloudflare displays a NEW client_secret exactly once; copy it to a
   secure scratchpad immediately. The OLD secret continues to work
   until you revoke it.

2. **Update `.env.local`** for each developer:
   ```
   CF_ACCESS_CLIENT_SECRET=<new value>
   ```
   The client_id rarely changes; only the secret rotates.

3. **Update the GitHub Actions secret** at the repository's Settings →
   Secrets and variables → Actions → `CF_ACCESS_CLIENT_SECRET`.

4. **Wait for the next CI build** of the iOS release artifact (~5 min)
   so production has a binary signed with the new value. Tag a release
   if your deploy pipeline requires it.

5. **Revoke the old secret** in Cloudflare Zero Trust. The overlap
   window must be at least one full CI build cycle to avoid breaking
   already-running mobile sessions whose binary still has the old
   secret baked in.

## Backend JWT secret

`MINOS_JWT_SECRET` signs account-auth bearer tokens. Rotation
invalidates all live sessions (users must log in again). Rotate when:
- The secret has been exposed.
- Quarterly hygiene.

Procedure:

1. Generate: `openssl rand -hex 32`
2. Update GitHub Actions secret `MINOS_JWT_SECRET` (production deploy).
3. Update each developer's `.env.local`.
4. Restart the backend (`just backend`).
5. All existing access tokens become invalid; mobile clients receive
   401s on the next request and re-prompt for login.

There is no overlap-window mechanism — JWT rotation is destructive
to live sessions by design. Coordinate with users if the impact matters.

## Backend SQLite dump

Out of scope for this runbook (operations doc, not secrets).
```

- [ ] **Step 2: Commit**

```bash
git add docs/ops/secrets-rotation.md
git commit -m "docs(ops): add CF Access + JWT secret rotation runbook"
```

### Task 8.5: Banner superseded ADRs

**Files:**
- Modify: `docs/adr/0013-macos-relay-client-cutover.md`
- Modify: `docs/adr/0014-backend-assembled-pairing-qr.md`
- Modify: `docs/adr/0016-client-env-cloudflare-access.md`

- [ ] **Step 1: ADR 0013 banner**

In `docs/adr/0013-macos-relay-client-cutover.md`, find the table row:
```markdown
| Status | Accepted |
```

Replace with:
```markdown
| Status | Refined by 0018 (entry-point and storage) |
```

- [ ] **Step 2: ADR 0014 banner**

In `docs/adr/0014-backend-assembled-pairing-qr.md`, replace the Status row:
```markdown
| Status | Partially superseded by 0016 (CF Access) and 0018 (URL distribution) |
```

- [ ] **Step 3: ADR 0016 banner**

In `docs/adr/0016-client-env-cloudflare-access.md`, replace:
```markdown
| Status | Refined by 0018 (entry-point and storage) |
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0013-macos-relay-client-cutover.md \
        docs/adr/0014-backend-assembled-pairing-qr.md \
        docs/adr/0016-client-env-cloudflare-access.md
git commit -m "docs(adr): banner 0013/0014/0016 as refined by 0018"
```

### Task 8.6: Optional — `just rotate-cf-access` recipe

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Append**

```just
# Print the CF Access rotation runbook. Pure documentation; no state mutation.
rotate-cf-access:
    @cat docs/ops/secrets-rotation.md
```

- [ ] **Step 2: Verify**

Run: `just rotate-cf-access | head -10`
Expected: prints the runbook header.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "feat(just): add rotate-cf-access recipe (prints runbook)"
```

### Task 8.7: Phase 8 verification

- [ ] **Step 1: All docs render**

Run:
```bash
ls docs/adr/0018-just-config-pipeline.md docs/ops/secrets-rotation.md
```
Expected: both files exist.

- [ ] **Step 2: Pause for review.**

---

## Phase 9: CI workflow update

### Task 9.1: Replace `cargo build` invocations

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Inspect existing build steps**

Run: `grep -n 'cargo build\|cargo xtask\|flutter build' .github/workflows/ci.yml`

Identify each invocation. The known one is at line 70 (`cargo build -p minos-daemon --bin minos-daemon`).

- [ ] **Step 2: Install just in CI**

Find the existing toolchain-install step (rust-toolchain action or similar). Add a step before any `just` invocation:

```yaml
      - name: Install just
        uses: extractions/setup-just@v2
```

(Or, if you prefer cargo: `cargo install just --locked`. The action is faster.)

- [ ] **Step 3: Rewrite cargo build → just**

For each `cargo build -p ...` step, replace with the equivalent `just <recipe>`. Example transformation:

Before:
```yaml
      - name: cargo build -p minos-daemon --bin minos-daemon
        run: cargo build -p minos-daemon --bin minos-daemon
```

After:
```yaml
      - name: just build-daemon
        run: just build-daemon
```

The job-level `env:` block where `MINOS_BACKEND_URL: ${{ secrets.MINOS_BACKEND_URL }}` is set continues to work — `just` reads from the parent process env.

- [ ] **Step 4: Set MINOS_BUILD_VIA_JUST=1 in the env block**

Add to the workflow's job-level `env:` block:

```yaml
    env:
      MINOS_BACKEND_URL: ${{ secrets.MINOS_BACKEND_URL }}
      CF_ACCESS_CLIENT_ID: ${{ secrets.CF_ACCESS_CLIENT_ID }}
      CF_ACCESS_CLIENT_SECRET: ${{ secrets.CF_ACCESS_CLIENT_SECRET }}
      MINOS_BUILD_VIA_JUST: "1"  # required by Runner.xcodeproj Pre-Build hook
```

(Adjust to the actual existing variable names in your workflow.)

- [ ] **Step 5: Handle PR-from-fork case**

If the workflow runs on PRs from forks where `secrets.MINOS_BACKEND_URL` resolves to empty, the release build will fail at `build.rs` panic time. Two options:

(a) Run `just build-daemon debug` (not release) for those jobs — debug emits warning, doesn't panic.

(b) Skip mobile-FFI builds on fork PRs entirely (gate with `if: github.event.pull_request.head.repo.full_name == github.repository`).

The plan defaults to (a). Choose based on what your existing workflow does.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: route builds through just; install just step

Identical failure modes between local and CI: same env-injection path,
same Pre-Build hook, same release fail-fast. PR-from-fork paths build
debug profile to skip the release-mode env-required panic."
```

### Task 9.2: Phase 9 verification

- [ ] **Step 1: Push the worktree branch and watch CI**

```bash
git push -u origin feature/unified-config-pipeline
```

Watch the CI run on GitHub. All jobs should pass. If a job fails because of a recipe-name mismatch or missing secret, fix and push again.

- [ ] **Step 2: Pause.**

---

## Phase 10: End-to-end verification

### Task 10.1: Final completeness checklist

- [ ] **Verify §7.1 — workspace check**

Run: `just check`
Expected: PASS.

- [ ] **Verify §7.2 — single source of truth**

Run: `rg 'ws://127\.0\.0\.1:8787/devices' crates/`
Expected: only `crates/minos-domain/src/defaults.rs`.

- [ ] **Verify §7.3 — dead vars removed**

Run: `rg 'MINOS_BACKEND_PUBLIC_URL|MINOS_BACKEND_CF_ACCESS' crates/ apps/ .github/`
Expected: zero matches.

- [ ] **Verify §7.4 — no committed plaintext secrets**

Run: `rg 'CF_ACCESS_CLIENT_SECRET=[a-f0-9]{30,}' apps/`
Expected: zero matches.

- [ ] **Verify §7.5 — release build bakes the configured URL**

```bash
cat > .env.local <<EOF
MINOS_BACKEND_URL=wss://minos.fan-nn.top/devices
MINOS_JWT_SECRET=01234567890123456789012345678901
CF_ACCESS_CLIENT_ID=$CF_ACCESS_CLIENT_ID  # if you have it; else leave unset
CF_ACCESS_CLIENT_SECRET=$CF_ACCESS_CLIENT_SECRET
EOF
just clean
just build-mobile-ios Release
APP=$(find apps/mobile/build/ios -name 'Runner.app' -type d | head -1)
strings "$APP/Frameworks/minos_ffi_frb.framework/minos_ffi_frb" | grep '/devices'
```

Expected: shows `wss://minos.fan-nn.top/devices`. **Not** `ws://127.0.0.1:8787/devices`.

- [ ] **Verify §7.6 — IDE Build is blocked**

Open `apps/mobile/ios/Runner.xcworkspace` in Xcode. Cmd+B.
Expected: build fails with the via-just guard error.

- [ ] **Verify §7.7 — `just check` is the gate**

Already covered by §7.1.

- [ ] **Verify §7.8 — README walkthrough**

Have a fresh shell (no env vars set) follow the README's "Local setup" verbatim. Confirm it works without side instructions.

- [ ] **Cleanup**

```bash
rm .env.local
```

### Task 10.2: Open a PR

- [ ] **Step 1: Push and open**

```bash
git push -u origin feature/unified-config-pipeline
gh pr create --title "Unified config pipeline + mobile localhost-baking fix" --body "$(cat <<'EOF'
## Summary
- Single `.env.local` + `justfile` becomes the only sanctioned entry point for build/run commands.
- Fixes mobile silently baking `localhost` into release builds — Pre-Build hook in `Runner.xcodeproj` blocks IDE-direct builds; release `build.rs` panics if `MINOS_BACKEND_URL` is unset.
- Strips plaintext CF Access secrets from `Runner.xcscheme` (path α: was uncommitted, no rotation needed; path β: see runbook).
- Consolidates three duplicate `ws://127.0.0.1:8787/devices` fallbacks into `minos_domain::defaults::DEV_BACKEND_URL`.
- Removes dead vars (`MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_*`) from active source and ops docs.
- New ADR 0018; banners on superseded ADRs 0013/0014/0016.

## Test plan
- [x] `just check` (workspace gate) passes
- [x] `rg 'ws://127\.0\.0\.1:8787/devices' crates/` returns only `defaults.rs`
- [x] `rg 'MINOS_BACKEND_PUBLIC_URL|MINOS_BACKEND_CF_ACCESS' crates/ apps/ .github/` returns zero matches
- [x] `MINOS_BACKEND_URL=wss://minos.fan-nn.top/devices just build-mobile-ios Release` produces a binary with the configured URL baked in (verified via `strings`)
- [x] Xcode IDE Build fails with the via-just guard message
- [x] CI workflow runs `just <recipe>` invocations successfully

EOF
)"
```

### Task 10.3: Memory update (post-merge)

After the PR merges, update memory with the policy:

- [ ] **Add a feedback memory** about not running `cargo build` / `flutter build` directly (always go through `just`). This becomes a standing rule for future work in this repo.

(Use the auto-memory system as documented in the system prompt.)

---

## Self-review notes

- **Spec coverage:** All 9 MVP-scope items from `unified-config-pipeline-design.md §2.1` are addressed. §2.1.9 (CI compatibility) is Phase 9.
- **Placeholder scan:** No "TBD" / "implement later" — every step has exact code or commands. The strict-release-config Cargo feature toggle is mentioned in design doc §4.7 as an open question, but the plan picks the simpler path (always panic in release) and notes the alternative.
- **Type consistency:** `DEV_BACKEND_URL: &'static str`, `DEV_BACKEND_LISTEN: &'static str`. Used as: (a) `&str` in `option_env!` match arm, (b) `.to_string()` for clap `default_value_t`, (c) `.parse::<SocketAddr>()` for backend listen default. All three usages are valid for `&'static str`. ✓
- **Out-of-scope items waived in design:** runtime URL override, multi-env manifests, vault integration, macOS app pipeline. Plan does not regress on these (they remain TODO for future plans).
