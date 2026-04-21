//! Subprocess port. The trait exists so unit tests can inject deterministic
//! responses without forking real binaries.

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

pub struct RealCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for RealCommandRunner {
    async fn which(&self, bin: &str) -> Option<String> {
        let out = Command::new("which")
            .arg(bin)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (!s.is_empty()).then_some(s)
    }

    async fn run(
        &self,
        bin: &str,
        args: &[&str],
        timeout_dur: Duration,
    ) -> Result<CommandOutcome, MinosError> {
        let fut = Command::new(bin)
            .args(args)
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
