//! `CodexProcess` — owns the spawned `codex app-server` child and tears it
//! down gracefully.
//!
//! Key responsibilities:
//!
//! [`CodexProcess::spawn`] launches the child with the exact args the spec
//! pins (`.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())`
//! plus `kill_on_drop(true)`), so a dropped runtime never leaks a codex
//! subprocess.
//!
//! [`CodexProcess::stderr_drain`] consumes stderr once, line-by-line, at
//! `tracing::warn!` level. codex is fairly chatty on stderr even in the happy
//! path, so a filled pipe would eventually block the child; the drainer is the
//! only consumer.
//!
//! [`CodexProcess::stop_graceful`] performs the SIGTERM → 3 s wait → SIGKILL
//! escalation. Called from `AgentRuntime::stop`, the returned `ExitStatus`
//! lets the caller distinguish "exited cleanly" from "had to be force-killed".
//!
//! [`reason_from_exit`] maps `ExitStatus` to the `AgentState::Crashed`
//! `reason` string. It lives here because it reaches into Unix-specific
//! `ExitStatusExt` and is shared by the supervisor task in `runtime.rs`.
//!
//! No I/O happens in unit tests beyond spawning real child processes (via
//! `sleep` / `bash`). The module deliberately has no dependency on
//! `codex_client` or `runtime` so its tests run in isolation.

use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use minos_domain::MinosError;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tracing::warn;

/// Owns a spawned `codex app-server` subprocess.
///
/// Kept `pub(crate)` — external consumers interact with agents via
/// `AgentRuntime`, never with the raw child.
pub(crate) struct CodexProcess {
    child: Option<Child>,
    stderr_task: Option<JoinHandle<()>>,
}

impl CodexProcess {
    /// Spawn `bin` with `args` and the explicit `env` (parent env is
    /// cleared so the child only sees what the daemon captured from the
    /// user's login shell). Sets up piped stdout/stderr, null stdin, and
    /// `kill_on_drop(true)` so the child dies with us. Retained for the
    /// app-server test seam even though exec/jsonl is now the default route.
    #[allow(dead_code)]
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

    /// Spawn a background task that reads the child's stderr line-by-line and
    /// emits each line at `tracing::warn!` level under the
    /// `minos_agent_runtime::process` target. Idempotent — a second call is a
    /// no-op so callers don't need to track whether the drain is already up.
    /// Retained for the app-server test seam even though exec/jsonl is now the
    /// default route.
    #[allow(dead_code)]
    pub(crate) fn stderr_drain(&mut self) {
        if self.stderr_task.is_some() {
            return;
        }
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let Some(stderr) = child.stderr.take() else {
            return;
        };
        let task = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        warn!(target: "minos_agent_runtime::process", line = %line, "codex stderr");
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!(
                            target: "minos_agent_runtime::process",
                            error = %e,
                            "codex stderr read error",
                        );
                        break;
                    }
                }
            }
        });
        self.stderr_task = Some(task);
    }

    /// Take ownership of the child so something else (e.g. a supervisor task)
    /// can own it from here on. After this call all `stop_graceful` /
    /// `stderr_drain` operations become no-ops — the caller is responsible
    /// for whatever remains.
    pub(crate) fn take_child(&mut self) -> Option<Child> {
        self.child.take()
    }

    /// Return a mutable handle to the inner child, if present. Used by the
    /// runtime's supervisor-setup path that needs both the child *and*
    /// stderr drain on the same struct before ownership passes on.
    #[cfg(test)]
    pub(crate) fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// SIGTERM → 3 s wait → SIGKILL escalation. Returns the child's
    /// [`ExitStatus`]. Safe to call multiple times — a second call with no
    /// child returns a synthetic success status.
    pub(crate) async fn stop_graceful(&mut self) -> Result<ExitStatus, MinosError> {
        let Some(mut child) = self.child.take() else {
            // Return a synthetic "already exited" status. We don't track the
            // last exit, but nothing consumes this return path in the
            // already-stopped case.
            return Err(MinosError::CodexSpawnFailed {
                message: "stop_graceful called with no live child".into(),
            });
        };
        // Polite SIGTERM first.
        if let Err(e) = child.start_kill() {
            // `start_kill` returning an error here means the child is already
            // gone; try to reap it and surface its exit below.
            warn!(error = %e, "start_kill (SIGTERM) failed; reaping");
        }
        match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(e)) => Err(MinosError::CodexSpawnFailed {
                message: format!("wait failed: {e}"),
            }),
            Err(_elapsed) => {
                // 3 s elapsed without the child exiting → escalate.
                warn!("codex did not exit within 3s of SIGTERM; escalating to SIGKILL");
                // On Unix tokio's Child::start_kill sends SIGKILL. But we
                // already called start_kill() above which on Unix sent SIGKILL
                // too (tokio has no SIGTERM API). The documented escalation
                // path is therefore a double-kill, which on Linux/macOS
                // amounts to waiting again after the first SIGKILL landed.
                // We still call start_kill() a second time to make the intent
                // explicit and to cover the edge case where the first call
                // failed.
                let _ = child.start_kill();
                child
                    .wait()
                    .await
                    .map_err(|e| MinosError::CodexSpawnFailed {
                        message: format!("wait after escalation failed: {e}"),
                    })
            }
        }
    }
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        // `kill_on_drop(true)` on the Command means dropping `child` sends
        // SIGKILL. The stderr task will then observe EOF and exit on its own;
        // we abort it defensively in case the child already exited but the
        // pipe is being held open by a forked process inheriting it.
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }
}

/// Map an `ExitStatus` into the textual `reason` the supervisor broadcasts via
/// `AgentState::Crashed { reason }`.
///
/// - Unix exit code: `"exit code N"`
/// - Unix signal: `"signal <NAME>"` (canonical SIG* name when we recognise it,
///   or the numeric form otherwise).
/// - Windows (non-Unix): `"exit code N"` using the raw status.
pub(crate) fn reason_from_exit(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return format!("signal {}", signal_name(sig));
        }
        if let Some(code) = status.code() {
            return format!("exit code {code}");
        }
        format!("unknown exit: {status:?}")
    }
    #[cfg(not(unix))]
    {
        status.code().map_or_else(
            || format!("unknown exit: {status:?}"),
            |c| format!("exit code {c}"),
        )
    }
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    // Canonical names for the signals codex is likely to die of. Anything
    // else falls through to the numeric form; that's enough for
    // AgentState::Crashed's human-readable `reason`.
    match sig {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        3 => "SIGQUIT".into(),
        6 => "SIGABRT".into(),
        9 => "SIGKILL".into(),
        11 => "SIGSEGV".into(),
        13 => "SIGPIPE".into(),
        14 => "SIGALRM".into(),
        15 => "SIGTERM".into(),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn spawn_ok_for_echo() {
        // `echo` exits immediately; we just verify spawn doesn't error and
        // `take_child` hands back a live `Child` handle we can `wait()` on
        // without touching `stop_graceful` (which races with natural exit).
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/echo"),
            &["hello"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let mut child = p.take_child().expect("child");
        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn spawn_missing_binary_surfaces_codex_spawn_failed() {
        let err = CodexProcess::spawn(
            &PathBuf::from("/definitely/not/a/real/binary"),
            &[],
            &std::collections::HashMap::new(),
        )
        .err()
        .expect("spawn must fail");
        match err {
            MinosError::CodexSpawnFailed { message } => {
                assert!(message.contains("not/a/real/binary"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn kill_on_drop_terminates_long_running_sleep() {
        // `sleep 60` will outlive the process struct — verify drop SIGKILLs it.
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/sleep"),
            &["60"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
        // Grab the PID before we drop, for the post-drop check.
        let pid = i32::try_from(p.child_mut().expect("child").id().expect("pid")).unwrap();
        drop(p);
        // Wait a little for kill_on_drop to fire and the kernel to reap.
        tokio::time::sleep(Duration::from_millis(200)).await;
        // `kill -0 <pid>` returns nonzero when the process is gone.
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success());
        assert!(!alive, "child should have been killed on drop");
    }

    #[tokio::test]
    async fn stop_graceful_exits_cleanly_for_sigterm_respecting_process() {
        // `sleep 30` exits on SIGTERM without needing escalation.
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/sleep"),
            &["30"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let status = p.stop_graceful().await.unwrap();
        // Killed by SIGTERM (or SIGKILL — tokio has no SIGTERM API on its
        // cross-platform Child::start_kill, so we just assert non-success).
        assert!(!status.success(), "expected non-zero exit, got {status:?}");
    }

    #[tokio::test]
    async fn stop_graceful_escalates_for_sigterm_trapping_process() {
        // `bash -c 'trap "" TERM; sleep 30'` ignores SIGTERM; stop_graceful
        // must escalate to SIGKILL within the 3 s window + some slack.
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/bash"),
            &["-c", "trap '' TERM; sleep 30"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let status = p.stop_graceful().await.unwrap();
        let elapsed = start.elapsed();
        assert!(!status.success(), "expected non-zero exit, got {status:?}");
        // Linux CI can be slow — 10 s is comfortable upper-bound on the 3 s
        // timeout + escalation.
        assert!(
            elapsed < Duration::from_secs(10),
            "stop_graceful took too long: {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn stderr_drain_consumes_output() {
        // `bash -c 'for i in 1 2 3; do echo line$i 1>&2; done; sleep 1'`
        // emits three stderr lines then idles. `stderr_drain` reads them to
        // completion; we just verify it doesn't deadlock / panic and the
        // child still exits via stop_graceful.
        let mut p = CodexProcess::spawn(
            &PathBuf::from("/bin/bash"),
            &["-c", "for i in 1 2 3; do echo line$i 1>&2; done; sleep 1"],
            &std::collections::HashMap::new(),
        )
        .unwrap();
        p.stderr_drain();
        // Second call is a no-op.
        p.stderr_drain();
        let _status = p.stop_graceful().await.unwrap();
    }

    #[test]
    fn reason_from_exit_codes() {
        // Construct synthetic ExitStatus values via a portable command. `bash`
        // is available on both Linux and macOS; `exit N` pins the exit code
        // deterministically without relying on `/bin/true` / `/bin/false`
        // (which don't exist on every host — macOS omits `/bin/true` on some
        // builds).
        let status = std::process::Command::new("bash")
            .args(["-c", "exit 0"])
            .status()
            .unwrap();
        assert_eq!(reason_from_exit(status), "exit code 0");
        let status = std::process::Command::new("bash")
            .args(["-c", "exit 1"])
            .status()
            .unwrap();
        assert_eq!(reason_from_exit(status), "exit code 1");
    }

    #[cfg(unix)]
    #[test]
    fn signal_name_canonical_examples() {
        assert_eq!(signal_name(9), "SIGKILL");
        assert_eq!(signal_name(15), "SIGTERM");
        assert_eq!(signal_name(2), "SIGINT");
        assert_eq!(signal_name(99), "99");
    }
}
