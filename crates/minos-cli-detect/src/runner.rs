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
    async fn run(&self, bin: &str, args: &[&str], timeout: Duration) -> Result<CommandOutcome, MinosError>;
}
