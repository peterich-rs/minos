# `minos-detect` Binary + Shell Env Import — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the macOS-app daemon detect *and* spawn user-installed CLI agents (codex/claude/gemini) the same way the user's terminal would, by capturing the user's login-shell env once at bootstrap and injecting it into both subprocess sites. Add a no-flag `minos-detect` binary so the same detection can be exercised from a terminal.

**Architecture:** A free function `capture_user_shell_env()` lives in `crates/minos-cli-detect/src/env.rs`; the daemon calls it once during `DaemonHandle::start` and threads the resulting `Arc<HashMap<String, String>>` into both `RealCommandRunner::new(env)` and `AgentGlue::new(workspace, env)`. The agent path forwards env into `AgentRuntimeConfig.subprocess_env` → `CodexProcess::spawn(bin, args, env)`. Both spawn sites apply `cmd.env_clear().envs(env.iter())`. `RealCommandRunner::which` switches from spawning the `which` binary to using the `which` crate against the snapshot's `PATH`. The new `minos-detect` bin is ~25 lines of glue over the same primitives. Failures in shell capture log a warning and fall back to `std::env::vars().collect()`, so the daemon never fails to start.

**Tech Stack:** Rust 2021, `tokio` async runtime, `which = "6"` crate, `tracing` for warnings. No new wire/FFI/UI surface.

**Spec:** `docs/superpowers/specs/cli-detect-bin-and-shell-env-design.md`

---

## File Structure

**Create:**
- `crates/minos-cli-detect/src/env.rs` — `capture_user_shell_env()` + `parse_env_dump()` + unit tests for the parser
- `crates/minos-cli-detect/src/bin/minos-detect.rs` — terminal binary, no flags
- `crates/minos-cli-detect/tests/runner_with_env.rs` — integration test verifying `env_clear` + `which`-via-snapshot-PATH

**Modify:**
- `Cargo.toml` (workspace root) — add `which = "6"` to `[workspace.dependencies]`
- `crates/minos-cli-detect/Cargo.toml` — add `which` dep, add `[[bin]]` section
- `crates/minos-cli-detect/src/lib.rs` — add `pub mod env;` and re-export
- `crates/minos-cli-detect/src/runner.rs` — `RealCommandRunner` becomes `{ env: Arc<HashMap> }`, `which` uses `which` crate, `run` uses `env_clear().envs()`
- `crates/minos-agent-runtime/src/process.rs` — `CodexProcess::spawn` gains `env: &HashMap<String, String>` parameter; six existing inline tests updated
- `crates/minos-agent-runtime/src/runtime.rs` — `AgentRuntimeConfig` gains `subprocess_env: Arc<HashMap<String, String>>`; `start_inner` reads it and forwards to `spawn`
- `crates/minos-daemon/src/agent.rs` — `AgentGlue::new(workspace_root, env)` populates `AgentRuntimeConfig.subprocess_env`
- `crates/minos-daemon/src/handle.rs` — `DaemonHandle::start` calls `capture_user_shell_env().await` once and threads the Arc into both `RealCommandRunner::new` and `AgentGlue::new`
- `crates/minos-daemon/src/rpc_server.rs` — `fake_server()` test fixture passes `Arc::new(HashMap::new())` to `AgentGlue::new`

Build invariant after each task: `cargo build --workspace` succeeds. Commit gate after each task: `cargo xtask check-all` (per project memory).

---

## Task 1: Add `which` workspace dependency

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/minos-cli-detect/Cargo.toml`

- [ ] **Step 1: Add `which` to workspace dependencies**

In `Cargo.toml` (root), in the `[workspace.dependencies]` block, add this line near the other utility crates (e.g., after `dashmap = "6"` alphabetically):

```toml
which = "6"
```

- [ ] **Step 2: Add `which` dep to cli-detect crate**

Replace the entire contents of `crates/minos-cli-detect/Cargo.toml` with:

```toml
[package]
name = "minos-cli-detect"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Local CLI agent detection (which + --version)."

[dependencies]
minos-domain = { path = "../minos-domain", version = "0.1.0" }
tokio = { workspace = true }
tracing = { workspace = true }
async-trait = { workspace = true }
which = { workspace = true }

[dev-dependencies]
mockall = { workspace = true }
tokio-test = { workspace = true }

[lints]
workspace = true
```

The `[[bin]]` section is added in Task 8 alongside the binary's source file — keeping the manifest and the file in lock-step avoids any cargo manifest-vs-disk inconsistency window.

- [ ] **Step 3: Verify the workspace still builds**

Run: `cargo build --workspace`

Expected: build succeeds. The `which` crate is downloaded into `Cargo.lock` but not yet referenced from any Rust source.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/minos-cli-detect/Cargo.toml
git commit -m "build(cli-detect): add which dep for upcoming PATH-walk refactor

Threads which = \"6\" through workspace.dependencies into minos-cli-detect.
Used in the next task to replace the spawned which binary with a
pure-Rust PATH walk. The [[bin]] section for minos-detect lands with
its source file in a later task."
```

---

## Task 2: Create `env.rs` parser with unit tests (TDD)

**Files:**
- Create: `crates/minos-cli-detect/src/env.rs`
- Modify: `crates/minos-cli-detect/src/lib.rs`

- [ ] **Step 1: Wire the new module into lib.rs**

Replace the entire contents of `crates/minos-cli-detect/src/lib.rs` with:

```rust
#![forbid(unsafe_code)]

pub mod detect;
pub mod env;
pub mod runner;

pub use detect::*;
pub use env::*;
pub use runner::*;
```

- [ ] **Step 2: Write the failing parser tests**

Create `crates/minos-cli-detect/src/env.rs` with **only** the tests + module skeleton (no `parse_env_dump` body yet so tests fail on undefined-fn first, then on logic):

```rust
//! User-shell env capture. The macOS app process inherits launchd's minimal
//! `PATH`, so subprocesses spawned by the daemon (detection probes, codex
//! itself) need an env that mirrors what the user sees in their terminal.
//!
//! `capture_user_shell_env()` runs `$SHELL -lic '<dump>'` once at daemon
//! bootstrap. The output is bracketed by control-char sentinels so we can
//! discard rc-script noise; values are NUL-separated via `env -0` so values
//! containing newlines parse correctly.

use std::collections::HashMap;

/// Volatile shell-session keys we strip — they describe the temporary login
/// shell, not anything the user would expect their CLI tools to inherit.
const FILTER: &[&str] = &["_", "SHLVL", "PWD", "OLDPWD"];

const BEGIN: &str = "\x01MINOS_ENV_BEGIN\x01";
const END: &str = "\x01MINOS_ENV_END\x01";

/// Pure parser: given the raw stdout of the dump script, slice between
/// sentinels and split into `(key, value)` pairs. Returns an empty map if
/// the sentinels are missing.
pub(crate) fn parse_env_dump(stdout: &str) -> HashMap<String, String> {
    todo!("implement in next step")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse_env_dump("").is_empty());
    }

    #[test]
    fn missing_sentinels_yields_empty_map() {
        assert!(parse_env_dump("PATH=/usr/bin\0HOME=/home/u\0").is_empty());
    }

    #[test]
    fn parses_minimal_dump() {
        let s = "\x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0HOME=/home/u\0\x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.get("HOME").map(String::as_str), Some("/home/u"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn rc_script_noise_is_discarded() {
        let s = "Welcome to zsh\nLast login: ...\n\
                 \x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0\x01MINOS_ENV_END\x01\n\
                 trailing garbage";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn volatile_keys_are_filtered() {
        let s = "\x01MINOS_ENV_BEGIN\x01\
                 PATH=/usr/bin\0_=/usr/bin/zsh\0SHLVL=2\0PWD=/tmp\0OLDPWD=/home/u\0HOME=/home/u\0\
                 \x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.get("HOME").map(String::as_str), Some("/home/u"));
        assert!(!map.contains_key("_"));
        assert!(!map.contains_key("SHLVL"));
        assert!(!map.contains_key("PWD"));
        assert!(!map.contains_key("OLDPWD"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn value_with_equals_is_preserved() {
        let s = "\x01MINOS_ENV_BEGIN\x01\
                 RUSTFLAGS=-C link-arg=-Wl,-rpath,/opt/lib\0PATH=/usr/bin\0\
                 \x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(
            map.get("RUSTFLAGS").map(String::as_str),
            Some("-C link-arg=-Wl,-rpath,/opt/lib"),
        );
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
    }

    #[test]
    fn empty_key_entries_are_dropped() {
        // Trailing NUL → empty entry between END marker and last NUL.
        let s = "\x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0\0\x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.len(), 1);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail with the `todo!`**

Run: `cargo test -p minos-cli-detect env::tests --lib`

Expected: compilation succeeds; every test fails with a panic from `todo!("implement in next step")`.

- [ ] **Step 4: Replace the `todo!` with the real implementation**

In `crates/minos-cli-detect/src/env.rs`, replace the `parse_env_dump` body:

```rust
pub(crate) fn parse_env_dump(stdout: &str) -> HashMap<String, String> {
    let Some(begin_at) = stdout.find(BEGIN) else {
        return HashMap::new();
    };
    let body_start = begin_at + BEGIN.len();
    let Some(end_offset) = stdout[body_start..].find(END) else {
        return HashMap::new();
    };
    let body = &stdout[body_start..body_start + end_offset];

    body.split('\0')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            if k.is_empty() || FILTER.contains(&k) {
                return None;
            }
            Some((k.to_owned(), v.to_owned()))
        })
        .collect()
}
```

- [ ] **Step 5: Run the tests, expect green**

Run: `cargo test -p minos-cli-detect env::tests --lib`

Expected: 6 passed; 0 failed.

- [ ] **Step 6: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-cli-detect/src/env.rs crates/minos-cli-detect/src/lib.rs
git commit -m "feat(cli-detect): parser for sentinel-bracketed env dumps

Pure parse_env_dump() that slices stdout between MINOS_ENV_BEGIN /
MINOS_ENV_END control-char sentinels and splits NUL-separated KEY=VALUE
entries. Filters volatile shell-session keys (_, SHLVL, PWD, OLDPWD).
Tolerates rc-script noise before/after the sentinel block. Tested in
isolation; the async shell wrapper lands next."
```

---

## Task 3: Add `capture_user_shell_env()` async wrapper

**Files:**
- Modify: `crates/minos-cli-detect/src/env.rs`

- [ ] **Step 1: Append the async function and its constants**

Append to `crates/minos-cli-detect/src/env.rs`:

```rust
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;

const SHELL_TIMEOUT: Duration = Duration::from_secs(3);
const FALLBACK_SHELL: &str = "/bin/zsh";

/// Shell-side dump script. Brackets `env -0` output with control-char
/// sentinels so the parser can discard rc-script chatter on stdout.
/// `\1` is octal-escape for `\x01`, supported by every printf we care about.
const DUMP_SCRIPT: &str =
    "printf '\\1MINOS_ENV_BEGIN\\1'; env -0; printf '\\1MINOS_ENV_END\\1'";

/// Run `$SHELL -lic '<dump>'` once and return the parsed env map. Any
/// failure (timeout, spawn error, non-zero exit, missing sentinels) logs
/// at `warn` level and returns the current process's env via
/// `std::env::vars()`. This function never panics and never returns Err —
/// daemon bootstrap must not be blocked by a broken user shell rc.
pub async fn capture_user_shell_env() -> HashMap<String, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|p| Path::new(p).is_absolute())
        .unwrap_or_else(|| FALLBACK_SHELL.to_owned());

    let fut = Command::new(&shell)
        .args(["-l", "-i", "-c", DUMP_SCRIPT])
        .output();

    match timeout(SHELL_TIMEOUT, fut).await {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let map = parse_env_dump(&stdout);
            if map.is_empty() {
                warn!(
                    shell = %shell,
                    "shell env dump produced no parseable entries; falling back to process env"
                );
                std::env::vars().collect()
            } else {
                map
            }
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                shell = %shell,
                exit = ?out.status.code(),
                stderr_first_line = %stderr.lines().next().unwrap_or(""),
                "shell env dump exited non-zero; falling back to process env",
            );
            std::env::vars().collect()
        }
        Ok(Err(e)) => {
            warn!(
                shell = %shell,
                error = %e,
                "shell env dump spawn failed; falling back to process env",
            );
            std::env::vars().collect()
        }
        Err(_) => {
            warn!(
                shell = %shell,
                timeout_secs = SHELL_TIMEOUT.as_secs(),
                "shell env dump timed out; falling back to process env",
            );
            std::env::vars().collect()
        }
    }
}
```

- [ ] **Step 2: Verify compile + parser tests still green**

Run: `cargo test -p minos-cli-detect --lib`

Expected: all parser tests still pass. No new tests for `capture_user_shell_env` — flaky against real shells in CI; manual verification in Task 9.

- [ ] **Step 3: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-cli-detect/src/env.rs
git commit -m "feat(cli-detect): capture_user_shell_env via login shell

Spawns \$SHELL -lic with a sentinel-bracketed env -0 dump and a 3s
timeout. Any failure (timeout, spawn error, non-zero exit, parse miss)
logs at warn and falls back to std::env::vars(). Never returns Err so
daemon bootstrap is not blocked by a broken rc."
```

---

## Task 4: Refactor `RealCommandRunner` to take env, add integration test

**Files:**
- Modify: `crates/minos-cli-detect/src/runner.rs`
- Modify: `crates/minos-daemon/src/handle.rs`
- Create: `crates/minos-cli-detect/tests/runner_with_env.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/minos-cli-detect/tests/runner_with_env.rs`:

```rust
//! Integration test: verify RealCommandRunner respects the injected env.
//! Spawns real subprocesses (no mocks) — the only way to catch env_clear
//! regressions and which-crate breakage is to actually exec something.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use minos_cli_detect::{CommandRunner, RealCommandRunner};

#[tokio::test]
async fn which_walks_injected_path_only() {
    // Inject a PATH that only points at /bin and /usr/bin — well-known
    // locations across macOS and Linux containing `ls`.
    let mut env = HashMap::new();
    env.insert("PATH".to_owned(), "/bin:/usr/bin".to_owned());
    let runner = RealCommandRunner::new(Arc::new(env));

    let resolved = runner
        .which("ls")
        .await
        .expect("ls must exist on /bin or /usr/bin");
    assert!(
        resolved == "/bin/ls" || resolved == "/usr/bin/ls",
        "unexpected ls path: {resolved}",
    );
}

#[tokio::test]
async fn which_returns_none_when_path_missing() {
    let runner = RealCommandRunner::new(Arc::new(HashMap::new()));
    assert!(runner.which("ls").await.is_none(), "no PATH means no resolution");
}

#[tokio::test]
async fn run_subprocess_sees_only_injected_env() {
    // Set a single sentinel var; verify the child sees it AND verify the
    // child does NOT see TERM (which is almost always set in the parent).
    let mut env = HashMap::new();
    env.insert("PATH".to_owned(), "/bin:/usr/bin".to_owned());
    env.insert("MINOS_TEST_SENTINEL".to_owned(), "snowflake".to_owned());
    let runner = RealCommandRunner::new(Arc::new(env));

    let outcome = runner
        .run("/usr/bin/env", &[], Duration::from_secs(5))
        .await
        .expect("env must succeed");
    assert_eq!(outcome.exit_code, 0);
    assert!(
        outcome.stdout.contains("MINOS_TEST_SENTINEL=snowflake"),
        "missing sentinel in env output:\n{}",
        outcome.stdout,
    );
    assert!(
        !outcome.stdout.contains("TERM="),
        "child saw TERM despite env_clear; env injection regression:\n{}",
        outcome.stdout,
    );
}
```

- [ ] **Step 2: Run the test — expect compile failure**

Run: `cargo test -p minos-cli-detect --test runner_with_env`

Expected: compile error — `RealCommandRunner::new` doesn't exist yet, `RealCommandRunner` is a unit struct.

- [ ] **Step 3: Refactor `RealCommandRunner`**

Replace the entire contents of `crates/minos-cli-detect/src/runner.rs` with:

```rust
//! Subprocess port. The trait exists so unit tests can inject deterministic
//! responses without forking real binaries.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use minos_domain::MinosError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync + 'static {
    async fn which(&self, bin: &str) -> Option<String>;
    async fn run(
        &self,
        bin: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutcome, MinosError>;
}

// ──────────────────────────────────────────────────────────────────────────
// Real implementation (used by the daemon at runtime).
// ──────────────────────────────────────────────────────────────────────────

use std::process::Stdio;
use tokio::process::Command;
use tokio::time::timeout;

pub struct RealCommandRunner {
    env: Arc<HashMap<String, String>>,
}

impl RealCommandRunner {
    /// Wrap a shared env snapshot. Construct once at daemon bootstrap and
    /// share via `Arc` between the runner and any other subprocess site.
    #[must_use]
    pub fn new(env: Arc<HashMap<String, String>>) -> Self {
        Self { env }
    }
}

#[async_trait::async_trait]
impl CommandRunner for RealCommandRunner {
    async fn which(&self, bin: &str) -> Option<String> {
        let path = self.env.get("PATH")?;
        let mut iter = which::which_in_global(bin, Some(path)).ok()?;
        iter.next().map(|p| p.to_string_lossy().into_owned())
    }

    async fn run(
        &self,
        bin: &str,
        args: &[&str],
        timeout_dur: Duration,
    ) -> Result<CommandOutcome, MinosError> {
        let fut = Command::new(bin)
            .args(args)
            .env_clear()
            .envs(self.env.iter())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let out = timeout(timeout_dur, fut)
            .await
            .map_err(|_| MinosError::CliProbeTimeout {
                bin: bin.to_owned(),
                timeout_ms: u64::try_from(timeout_dur.as_millis()).unwrap_or(u64::MAX),
            })?;

        let out = out.map_err(|e| MinosError::CliProbeFailed {
            bin: bin.to_owned(),
            message: e.to_string(),
        })?;

        Ok(CommandOutcome {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
```

- [ ] **Step 4: Update the only production caller in `handle.rs`**

In `crates/minos-daemon/src/handle.rs`, line 87 currently reads:

```rust
            runner: Arc::new(minos_cli_detect::RealCommandRunner),
```

Change to:

```rust
            runner: Arc::new(minos_cli_detect::RealCommandRunner::new(Arc::new(
                std::collections::HashMap::new(),
            ))),
```

This passes an *empty* env map as a placeholder. Task 7 swaps in the real captured env. The daemon will continue to behave exactly as today (empty env → same broken PATH) until Task 7 lands.

If `Arc` isn't already imported at the top of `handle.rs`, it is — line 12 reads `use std::sync::{Arc, Mutex as StdMutex};`. No import change needed.

- [ ] **Step 5: Run the integration test, expect green**

Run: `cargo test -p minos-cli-detect --test runner_with_env`

Expected: 3 passed. The test does not depend on the daemon caller; it exercises `RealCommandRunner::new` directly.

- [ ] **Step 6: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass. The pre-existing `detect.rs` `ScriptRunner` mock-based tests are unaffected because they implement `CommandRunner` themselves.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-cli-detect/src/runner.rs \
        crates/minos-cli-detect/tests/runner_with_env.rs \
        crates/minos-daemon/src/handle.rs
git commit -m "refactor(cli-detect): RealCommandRunner takes Arc<HashMap> env

which() now uses the which crate against the snapshot's PATH instead of
spawning the which binary; run() applies env_clear().envs() so the child
sees only the injected env. Daemon caller passes an empty HashMap as a
placeholder — real shell-env capture wires in two tasks later. Adds an
integration test exercising both new behaviours against real subprocesses."
```

---

## Task 5: Refactor `CodexProcess::spawn` and thread env through `AgentRuntime`

**Files:**
- Modify: `crates/minos-agent-runtime/src/process.rs`
- Modify: `crates/minos-agent-runtime/src/runtime.rs`

- [ ] **Step 1: Update `CodexProcess::spawn` signature**

In `crates/minos-agent-runtime/src/process.rs`, locate `impl CodexProcess { pub(crate) fn spawn(bin: &Path, args: &[&str]) -> ...` (around line 47–64). Replace just that function:

```rust
    /// Spawn `bin` with `args` and the explicit `env` (parent env is
    /// cleared so the child only sees what the daemon captured from the
    /// user's login shell). Sets up piped stdout/stderr, null stdin, and
    /// `kill_on_drop(true)` so the child dies with us.
    pub(crate) fn spawn(
        bin: &Path,
        args: &[&str],
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self, MinosError> {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .env_clear()
            .envs(env.iter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn().map_err(|e| MinosError::CodexSpawnFailed {
            message: format!("spawn {}: {e}", bin.display()),
        })?;
        Ok(Self {
            child: Some(child),
            stderr_task: None,
        })
    }
```

- [ ] **Step 2: Update the six existing `process.rs` unit tests**

The tests at lines 230–315 each call `CodexProcess::spawn(&PathBuf::from("..."), &["..."])`. Each call needs an empty `HashMap` for env. Replace each call as follows.

Test `spawn_ok_for_echo`:

```rust
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/echo"),
            &["hello"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
```

Test `spawn_missing_binary_surfaces_codex_spawn_failed`:

```rust
        let err = CodexProcess::spawn(
            &PathBuf::from("/definitely/not/a/real/binary"),
            &[],
            &std::collections::HashMap::new(),
        )
        .err()
        .expect("spawn must fail");
```

Test `kill_on_drop_terminates_long_running_sleep`:

```rust
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/sleep"),
            &["60"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
```

Test `stop_graceful_exits_cleanly_for_sigterm_respecting_process`:

```rust
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/sleep"),
            &["30"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
```

Test `stop_graceful_escalates_for_sigterm_trapping_process`:

```rust
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/bash"),
            &["-c", "trap '' TERM; sleep 30"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
```

Test `stderr_drain_consumes_output`:

```rust
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/bash"),
            &["-c", "for i in 1 2 3; do echo line$i 1>&2; done; sleep 1"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
```

(`reason_from_exit_codes` and `signal_name_canonical_examples` do not call `spawn` — leave them alone.)

- [ ] **Step 3: Add `subprocess_env` to `AgentRuntimeConfig`**

In `crates/minos-agent-runtime/src/runtime.rs`, the struct `AgentRuntimeConfig` at line 107–119. Replace the struct + its `impl` (line 107–137) with:

```rust
/// Runtime configuration — carries the workspace root, optional explicit
/// binary path, port range, event buffer size, and the snapshot of the
/// user's login-shell env that the daemon captured at bootstrap. The
/// `test_ws_url` seam is gated behind `test-support` and skips subprocess
/// spawn entirely.
#[derive(Debug, Clone)]
pub struct AgentRuntimeConfig {
    pub workspace_root: PathBuf,
    pub codex_bin: Option<PathBuf>,
    pub ws_port_range: std::ops::RangeInclusive<u16>,
    pub event_buffer: usize,
    pub handshake_call_timeout: Duration,
    /// Env snapshot applied with `env_clear` to every spawned codex
    /// subprocess. Defaults to an empty map (caller wiring tested with
    /// the test_ws_url seam, which never spawns).
    pub subprocess_env: Arc<std::collections::HashMap<String, String>>,
    /// Test-only seam: when `Some`, `start()` skips port-probing + codex
    /// spawn + workspace creation and connects directly to this URL.
    /// Production code must leave this as `None`.
    #[cfg(feature = "test-support")]
    pub test_ws_url: Option<Url>,
}

impl AgentRuntimeConfig {
    /// Minimal constructor that fills in sensible defaults for `ws_port_range`
    /// and `event_buffer`. Callers who need custom values set the fields
    /// afterwards.
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            codex_bin: None,
            ws_port_range: 7879..=7883,
            event_buffer: DEFAULT_EVENT_BUFFER,
            handshake_call_timeout: DEFAULT_HANDSHAKE_CALL_TIMEOUT,
            subprocess_env: Arc::new(std::collections::HashMap::new()),
            #[cfg(feature = "test-support")]
            test_ws_url: None,
        }
    }
}
```

- [ ] **Step 4: Pass env into `CodexProcess::spawn` in `start_inner`**

In `crates/minos-agent-runtime/src/runtime.rs`, locate the `start_inner` function. The line that currently reads:

```rust
        let mut process = CodexProcess::spawn(&bin, &args)?;
```

Replace with:

```rust
        let mut process = CodexProcess::spawn(&bin, &args, &self.inner.cfg.subprocess_env)?;
```

- [ ] **Step 5: Verify the agent-runtime tests still pass**

Run: `cargo test -p minos-agent-runtime`

Expected: all process.rs unit tests pass with empty env (since `/bin/echo`, `/bin/sleep`, `/bin/bash` don't depend on env to run). All runtime.rs tests use the `test_ws_url` seam under `#[cfg(feature = "test-support")]`, so they bypass spawn — also unaffected.

- [ ] **Step 6: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-agent-runtime/src/process.rs \
        crates/minos-agent-runtime/src/runtime.rs
git commit -m "refactor(agent-runtime): CodexProcess::spawn takes env arg

Spawn now applies env_clear().envs(env) so codex inherits only the env
the daemon captured. Threaded via AgentRuntimeConfig.subprocess_env
(default empty Arc<HashMap>) → start_inner forwards into spawn(). Six
existing process.rs unit tests updated to pass an empty HashMap; their
binaries (echo/sleep/bash) don't depend on env."
```

---

## Task 6: Refactor `AgentGlue::new` to accept env

**Files:**
- Modify: `crates/minos-daemon/src/agent.rs`
- Modify: `crates/minos-daemon/src/handle.rs`
- Modify: `crates/minos-daemon/src/rpc_server.rs`

- [ ] **Step 1: Update `AgentGlue::new` signature**

In `crates/minos-daemon/src/agent.rs`, replace the `new` method (line 17–20):

```rust
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        subprocess_env: Arc<std::collections::HashMap<String, String>>,
    ) -> Self {
        let mut cfg = AgentRuntimeConfig::new(workspace_root);
        cfg.subprocess_env = subprocess_env;
        Self::new_with_runtime(AgentRuntime::new(cfg))
    }
```

- [ ] **Step 2: Fix the production caller in `handle.rs`**

In `crates/minos-daemon/src/handle.rs`, line 80 currently reads:

```rust
        let agent = Arc::new(AgentGlue::new(paths::minos_home()?.join("workspaces")));
```

Change to:

```rust
        let agent = Arc::new(AgentGlue::new(
            paths::minos_home()?.join("workspaces"),
            Arc::new(std::collections::HashMap::new()),
        ));
```

(Empty placeholder; Task 7 replaces with the captured env.)

- [ ] **Step 3: Fix the test caller in `rpc_server.rs`**

In `crates/minos-daemon/src/rpc_server.rs`, line 238 currently reads:

```rust
            agent: Arc::new(AgentGlue::new(std::env::temp_dir().join("minos-rpc-test"))),
```

Change to:

```rust
            agent: Arc::new(AgentGlue::new(
                std::env::temp_dir().join("minos-rpc-test"),
                Arc::new(std::collections::HashMap::new()),
            )),
```

- [ ] **Step 4: Verify the workspace builds and tests pass**

Run: `cargo test -p minos-daemon`

Expected: all daemon tests pass. The rpc_server fixture compiles with the new signature; runtime config has empty subprocess_env which never gets exercised by any daemon test (production codex spawn doesn't run in the test suite).

- [ ] **Step 5: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-daemon/src/agent.rs \
        crates/minos-daemon/src/handle.rs \
        crates/minos-daemon/src/rpc_server.rs
git commit -m "refactor(daemon): AgentGlue::new accepts subprocess env

Threads Arc<HashMap> through to AgentRuntimeConfig.subprocess_env so
the codex spawn site sees the user's shell env. Production caller in
handle.rs and the rpc_server test fixture updated; both pass empty
HashMaps as placeholders pending the bootstrap wiring task."
```

---

## Task 7: Wire `capture_user_shell_env()` into daemon bootstrap

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs`

- [ ] **Step 1: Capture env once and replace both placeholders**

In `crates/minos-daemon/src/handle.rs`, the `DaemonHandle::start` function. The existing block (lines 78–89) reads:

```rust
        let local_state_path = LocalState::default_path();

        let agent = Arc::new(AgentGlue::new(
            paths::minos_home()?.join("workspaces"),
            Arc::new(std::collections::HashMap::new()),
        ));

        // The relay-client dispatches forwarded peer JSON-RPC into this
        // server impl. Pre-relay it lived behind a jsonrpsee WS server;
        // now there is exactly one shared instance threaded through.
        let rpc_server = Arc::new(crate::rpc_server::RpcServerImpl {
            started_at: std::time::Instant::now(),
            runner: Arc::new(minos_cli_detect::RealCommandRunner::new(Arc::new(
                std::collections::HashMap::new(),
            ))),
            agent: agent.clone(),
        });
```

Replace with:

```rust
        let local_state_path = LocalState::default_path();

        // Capture the user's login-shell env once. Failures fall back to
        // process env internally, so this never blocks bootstrap.
        let subprocess_env = Arc::new(minos_cli_detect::capture_user_shell_env().await);

        let agent = Arc::new(AgentGlue::new(
            paths::minos_home()?.join("workspaces"),
            subprocess_env.clone(),
        ));

        // The relay-client dispatches forwarded peer JSON-RPC into this
        // server impl. Pre-relay it lived behind a jsonrpsee WS server;
        // now there is exactly one shared instance threaded through.
        let rpc_server = Arc::new(crate::rpc_server::RpcServerImpl {
            started_at: std::time::Instant::now(),
            runner: Arc::new(minos_cli_detect::RealCommandRunner::new(
                subprocess_env.clone(),
            )),
            agent: agent.clone(),
        });
```

- [ ] **Step 2: Run daemon tests**

Run: `cargo test -p minos-daemon`

Expected: all pass. Daemon tests don't go through `DaemonHandle::start` end-to-end; they construct `RpcServerImpl` directly via `fake_server()` (which still passes empty HashMap — that's fine, the test fixture isn't supposed to depend on the user's shell).

- [ ] **Step 3: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-daemon/src/handle.rs
git commit -m "feat(daemon): capture user shell env at bootstrap

DaemonHandle::start now calls capture_user_shell_env() once and threads
the resulting Arc<HashMap> into both RealCommandRunner::new (for cli
detection) and AgentGlue::new (for codex spawn). macOS GUI app daemon
now sees the same agents and can launch them with the same env the
user has in their terminal. Test fixtures keep empty placeholders."
```

---

## Task 8: Create the `minos-detect` binary

**Files:**
- Create: `crates/minos-cli-detect/src/bin/minos-detect.rs`
- Modify: `crates/minos-cli-detect/Cargo.toml` (add `[[bin]]` section)

- [ ] **Step 1: Create the binary source**

Create `crates/minos-cli-detect/src/bin/minos-detect.rs`:

```rust
//! Terminal entry point for local CLI agent detection. Mirrors what the
//! daemon does at bootstrap (capture user shell env + run detect_all)
//! so devs can verify detection from a real terminal session, side by
//! side with what the macOS app sees.

use std::sync::Arc;

use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_domain::AgentStatus;

#[tokio::main]
async fn main() {
    let env = Arc::new(capture_user_shell_env().await);
    let runner = Arc::new(RealCommandRunner::new(env));
    let agents = detect_all(runner).await;

    let mut any_ok = false;
    for d in &agents {
        let status = match &d.status {
            AgentStatus::Ok => "OK".to_owned(),
            AgentStatus::Missing => "MISSING".to_owned(),
            AgentStatus::Error { reason } => format!("ERROR ({reason})"),
        };
        println!(
            "{:<8} {:<20} {:<10} {}",
            d.name.bin_name(),
            status,
            d.version.as_deref().unwrap_or("-"),
            d.path.as_deref().unwrap_or("-"),
        );
        if matches!(d.status, AgentStatus::Ok) {
            any_ok = true;
        }
    }
    std::process::exit(i32::from(!any_ok));
}
```

- [ ] **Step 2: Register the bin in Cargo.toml**

Append to `crates/minos-cli-detect/Cargo.toml`, just before the `[lints]` section:

```toml
[[bin]]
name = "minos-detect"
path = "src/bin/minos-detect.rs"
```

- [ ] **Step 3: Build the binary explicitly to confirm the `[[bin]]` config**

Run: `cargo build -p minos-cli-detect --bin minos-detect`

Expected: build succeeds. The binary lands at `target/debug/minos-detect`.

- [ ] **Step 4: Run the binary against your local environment**

Run: `cargo run -p minos-cli-detect --bin minos-detect`

Expected: one line per agent (codex, claude, gemini), with status/version/path for any installed. Exit code 0 if at least one is Ok, else 1.

If you have any of these installed, this should match what `which codex && codex --version` etc. would report from your terminal.

- [ ] **Step 5: Run the workspace gate**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-cli-detect/Cargo.toml \
        crates/minos-cli-detect/src/bin/minos-detect.rs
git commit -m "feat(cli-detect): minos-detect binary for terminal verification

Thin glue over capture_user_shell_env + RealCommandRunner::new +
detect_all. No flags, no clap dep — just text output and exit 0/1.
Lets devs verify what the daemon should see from a real terminal."
```

---

## Task 9: Final acceptance — workspace gate + macOS app smoke

**Files:** None modified. Verification only.

- [ ] **Step 1: Run the workspace gate one more time**

Run: `cargo xtask check-all`

Expected: fmt/clippy/test all pass; `cargo deny check` passes if installed.

- [ ] **Step 2: Run `minos-detect` from a clean terminal**

Run: `cargo run -p minos-cli-detect --bin minos-detect`

Cross-check against ground truth — for each agent (codex/claude/gemini), confirm the path and version match `which <agent>` + `<agent> --version` run by hand.

- [ ] **Step 3: Build and launch the macOS app**

Run: `cargo xtask build-macos` (then open the produced `.app` from Finder, or use the existing dev workflow).

Open the app's agent list. Confirm: agents that previously showed as `Missing` because of the GUI-process PATH issue now show with the correct path + version. This is the primary acceptance criterion for the env-import fix.

- [ ] **Step 4: Trigger a `start_agent` for codex from the app**

Use whatever the existing flow is (mobile pairing → start, or a debug button if one exists in the menubar). Confirm:

- The spawn succeeds (no `MinosError::CodexSpawnFailed` in the daemon logs).
- The spawned codex subprocess sees the user's API keys / shell-env vars (e.g., the codex session can actually authenticate).

If either fails, the env-injection in `CodexProcess::spawn` is broken — investigate by enabling tracing for `minos_cli_detect::env` and `minos_agent_runtime::process` and checking what's actually in `subprocess_env`.

- [ ] **Step 5: No commit needed**

This task is verification only. If everything passes, the implementation is complete. If anything fails, the task that introduced the regression is the place to debug — fix and amend the commit on that task's branch (or add a follow-up commit).

---

## Notes on commit cadence

Per project memory (`feedback_check_all_before_commit.md`), every commit on this plan must be preceded by `cargo xtask check-all`. Each task above includes that step explicitly. Do not skip it — the workspace-level gate has caught frb mirror drift in prior plans that crate-scoped `cargo test -p` missed.

## Notes on execution context

Per project memory (`feedback_direct_plan_execution.md`), execute the tasks directly in the main conversation rather than dispatching subagents per task — even though the plan header recommends `superpowers:subagent-driven-development`. The plan is self-contained; subagent ceremony adds cost without value here.
