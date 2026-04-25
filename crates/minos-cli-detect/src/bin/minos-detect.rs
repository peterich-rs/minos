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
