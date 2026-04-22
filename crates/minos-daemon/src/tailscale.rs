//! Tailscale 100.x IP discovery. MVP shells out to `tailscale ip --4`.
//!
//! Returns `None` if `tailscale` is not installed or returns no IP. Callers
//! should map `None` to `MinosError::BindFailed { addr: "<unknown>", ... }`
//! and surface "please start Tailscale" to the user.
//!
//! This path is exercised during Swift app startup before any Tokio executor is
//! guaranteed to exist on the calling thread. Keep the implementation runtime-
//! agnostic so a missing Tokio reactor degrades into a normal "no tailscale IP"
//! failure instead of a panic crossing the FFI boundary.

use std::io::{self, Read};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const OVERRIDE_ENV: &str = "MINOS_TAILSCALE_BIN";
const COMMON_TAILSCALE_PATHS: &[&str] = &[
    "/opt/homebrew/bin/tailscale",
    "/usr/local/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    "/Applications/Tailscale.app/Contents/Resources/tailscale",
];

pub async fn discover_ip() -> Option<String> {
    discover_ip_with_reason().await.ok()
}

pub async fn discover_ip_with_reason() -> Result<String, String> {
    discover_ip_blocking()
}

fn discover_ip_blocking() -> Result<String, String> {
    let mut tried = Vec::new();
    for candidate in command_candidates() {
        match run_tailscale_ip(&candidate) {
            Ok(stdout) => {
                return parse_tailscale_ipv4(&stdout).ok_or_else(|| {
                    format!(
                        "no Tailscale CGNAT IPv4 found in `{}` output{}",
                        command_label(&candidate),
                        format_output_suffix(&stdout),
                    )
                });
            }
            Err(CommandError::NotFound) => tried.push(candidate),
            Err(CommandError::Spawn(err)) => {
                return Err(format!(
                    "failed to execute `{}`: {err}",
                    command_label(&candidate)
                ));
            }
            Err(CommandError::Timeout) => {
                return Err(format!(
                    "`{}` timed out after {}s",
                    command_label(&candidate),
                    DISCOVERY_TIMEOUT.as_secs()
                ));
            }
            Err(CommandError::Exited {
                status,
                stdout,
                stderr,
            }) => {
                return Err(format!(
                    "`{}` failed with {status}{}",
                    command_label(&candidate),
                    format_stdio_suffix(&stdout, &stderr),
                ));
            }
        }
    }

    Err(format!(
        "tailscale CLI not found on PATH or common install locations ({})",
        tried
            .iter()
            .map(|candidate| format!("`{}`", candidate.display()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn run_tailscale_ip(candidate: &Path) -> Result<String, CommandError> {
    let mut child = Command::new(candidate)
        .args(["ip", "--4"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(map_spawn_error)?;

    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    loop {
        match child.try_wait().map_err(CommandError::Spawn)? {
            Some(status) => {
                let mut stdout = String::new();
                if let Some(mut handle) = child.stdout.take() {
                    handle
                        .read_to_string(&mut stdout)
                        .map_err(CommandError::Spawn)?;
                }
                let mut stderr = String::new();
                if let Some(mut handle) = child.stderr.take() {
                    handle
                        .read_to_string(&mut stderr)
                        .map_err(CommandError::Spawn)?;
                }

                if status.success() {
                    return Ok(stdout);
                }

                return Err(CommandError::Exited {
                    status,
                    stdout,
                    stderr,
                });
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandError::Timeout);
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn command_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(override_path) = std::env::var(OVERRIDE_ENV) {
        let override_path = override_path.trim();
        if !override_path.is_empty() {
            candidates.push(PathBuf::from(override_path));
        }
    }

    candidates.push(PathBuf::from("tailscale"));
    candidates.extend(COMMON_TAILSCALE_PATHS.iter().map(PathBuf::from));

    if let Ok(home) = std::env::var("HOME") {
        candidates
            .push(PathBuf::from(&home).join("Applications/Tailscale.app/Contents/MacOS/Tailscale"));
        candidates.push(
            PathBuf::from(home).join("Applications/Tailscale.app/Contents/Resources/tailscale"),
        );
    }

    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    unique
}

fn parse_tailscale_ipv4(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .filter_map(|token| token.parse::<Ipv4Addr>().ok())
        .find(is_tailscale_cgnat_ipv4)
        .map(|ip| ip.to_string())
}

fn is_tailscale_cgnat_ipv4(ip: &Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && (64..=127).contains(&second)
}

fn command_label(candidate: &Path) -> String {
    format!("{} ip --4", candidate.display())
}

fn format_output_suffix(output: &str) -> String {
    let normalized = normalize_output(output);
    if normalized.is_empty() {
        "; output was empty".into()
    } else {
        format!(": `{normalized}`")
    }
}

fn format_stdio_suffix(stdout: &str, stderr: &str) -> String {
    let mut parts = Vec::new();

    let stderr = normalize_output(stderr);
    if !stderr.is_empty() {
        parts.push(format!("stderr: `{stderr}`"));
    }

    let stdout = normalize_output(stdout);
    if !stdout.is_empty() {
        parts.push(format!("stdout: `{stdout}`"));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join("; "))
    }
}

fn normalize_output(output: &str) -> String {
    const MAX_LEN: usize = 160;

    let mut normalized = output.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() > MAX_LEN {
        normalized.truncate(MAX_LEN - 3);
        normalized.push_str("...");
    }
    normalized
}

fn map_spawn_error(err: io::Error) -> CommandError {
    if err.kind() == io::ErrorKind::NotFound {
        CommandError::NotFound
    } else {
        CommandError::Spawn(err)
    }
}

enum CommandError {
    NotFound,
    Spawn(io::Error),
    Timeout,
    Exited {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_mock_tailscale(body: &str) -> std::path::PathBuf {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tailscale-mock.sh");
        fs::write(&path, body).unwrap();

        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();

        // Keep the tempdir alive for the duration of the test by persisting it
        // in the path's parent via leak. Tests are short-lived and serialized.
        let leaked = Box::leak(Box::new(dir));
        leaked.path().join("tailscale-mock.sh")
    }

    #[test]
    fn parse_returns_first_cgnat_ipv4() {
        let ip = super::parse_tailscale_ipv4("198.51.100.7\n100.72.1.9\nfd7a:115c:a1e0::1");
        assert_eq!(ip.as_deref(), Some("100.72.1.9"));
    }

    #[test]
    fn parse_rejects_non_cgnat_ipv4() {
        let ip = super::parse_tailscale_ipv4("100.12.0.9\n192.168.0.8");
        assert!(ip.is_none());
    }

    #[test]
    fn uses_override_binary_outside_tokio_runtime() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mock = write_mock_tailscale("#!/bin/sh\nprintf '100.72.1.9\\n'");
        let _env = EnvVarGuard::set(super::OVERRIDE_ENV, mock.to_str().unwrap());

        let ip = futures::executor::block_on(super::discover_ip());
        assert_eq!(ip.as_deref(), Some("100.72.1.9"));
    }

    #[tokio::test]
    async fn uses_override_binary_inside_tokio_runtime() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mock = write_mock_tailscale("#!/bin/sh\nprintf '100.88.0.42\\n'");
        let _env = EnvVarGuard::set(super::OVERRIDE_ENV, mock.to_str().unwrap());

        let ip = super::discover_ip().await;
        assert_eq!(ip.as_deref(), Some("100.88.0.42"));
    }

    #[test]
    fn reports_reason_when_output_has_no_cgnat_ipv4() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mock = write_mock_tailscale("#!/bin/sh\nprintf '192.168.0.8\\n'");
        let _env = EnvVarGuard::set(super::OVERRIDE_ENV, mock.to_str().unwrap());

        let err = super::discover_ip_blocking().unwrap_err();
        assert!(err.contains("no Tailscale CGNAT IPv4 found"));
    }
}
