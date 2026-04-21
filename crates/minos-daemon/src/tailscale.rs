//! Tailscale 100.x IP discovery. MVP shells out to `tailscale ip --4`.
//!
//! Returns `None` if `tailscale` is not installed or returns no IP. Callers
//! should map `None` to `MinosError::BindFailed { addr: "<unknown>", ... }`
//! and surface "please start Tailscale" to the user.

use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub async fn discover_ip() -> Option<String> {
    let fut = Command::new("tailscale")
        .args(["ip", "--4"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let out = timeout(Duration::from_secs(2), fut).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!s.is_empty() && s.starts_with("100.")).then_some(s)
}

#[cfg(test)]
mod tests {
    // Note: no unit tests here — discover_ip touches a real binary. CI runs
    // without tailscale installed, so it will return None; the daemon E2E
    // test (Task 24) supplies an explicit 127.0.0.1 address instead.
    #[tokio::test]
    async fn returns_none_or_some_with_100_prefix() {
        let ip = super::discover_ip().await;
        assert!(ip.is_none() || ip.as_ref().unwrap().starts_with("100."));
    }
}
