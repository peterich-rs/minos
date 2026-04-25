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
    assert!(
        runner.which("ls").await.is_none(),
        "no PATH means no resolution"
    );
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
