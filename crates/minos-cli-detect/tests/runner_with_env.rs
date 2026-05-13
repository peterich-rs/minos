//! Integration test: verify RealCommandRunner respects the injected env.
//! Spawns real subprocesses (no mocks) — the only way to catch env_clear
//! regressions and which-crate breakage is to actually exec something.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use minos_cli_detect::{CommandRunner, RealCommandRunner};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn path_sep() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

#[cfg(unix)]
fn script_file_name(stem: &str) -> String {
    stem.to_owned()
}

#[cfg(windows)]
fn script_file_name(stem: &str) -> String {
    format!("{stem}.cmd")
}

#[cfg(unix)]
fn script_contents(mode: ScriptMode) -> String {
    match mode {
        ScriptMode::EchoOk => "#!/bin/sh\nprintf 'ok\\n'\n".to_owned(),
        ScriptMode::DumpEnv => "#!/bin/sh\nprintf 'MINOS_TEST_SENTINEL=%s\\n' \"$MINOS_TEST_SENTINEL\"\nprintf 'MINOS_PARENT_ONLY=%s\\n' \"${MINOS_PARENT_ONLY-}\"\n".to_owned(),
    }
}

#[cfg(windows)]
fn script_contents(mode: ScriptMode) -> String {
    match mode {
        ScriptMode::EchoOk => "@echo off\r\necho ok\r\n".to_owned(),
        ScriptMode::DumpEnv => "@echo off\r\necho MINOS_TEST_SENTINEL=%MINOS_TEST_SENTINEL%\r\nif defined MINOS_PARENT_ONLY (echo MINOS_PARENT_ONLY=%MINOS_PARENT_ONLY%) else echo MINOS_PARENT_ONLY=\r\n".to_owned(),
    }
}

enum ScriptMode {
    EchoOk,
    DumpEnv,
}

fn make_test_script(dir: &Path, stem: &str, mode: ScriptMode) -> PathBuf {
    let path = dir.join(script_file_name(stem));
    fs::write(&path, script_contents(mode)).expect("write test script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&path).expect("stat test script").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod test script");
    }
    path
}

#[tokio::test]
async fn which_walks_injected_path_only() {
    let dir = TempDir::new().expect("temp dir");
    let script = make_test_script(dir.path(), "minos-test-which", ScriptMode::EchoOk);
    let mut env = HashMap::new();
    env.insert("PATH".to_owned(), dir.path().display().to_string());
    let runner = RealCommandRunner::new(Arc::new(env));

    let query = script
        .file_name()
        .and_then(|name| name.to_str())
        .expect("script file name should be utf-8");
    let resolved = runner
        .which(query)
        .await
        .expect("script must resolve from injected path");
    assert_eq!(resolved, script.display().to_string());
}

#[tokio::test]
async fn which_accepts_windows_style_path_key() {
    let dir = TempDir::new().expect("temp dir");
    let script = make_test_script(dir.path(), "minos-test-path-key", ScriptMode::EchoOk);
    let mut env = HashMap::new();
    env.insert("Path".to_owned(), dir.path().display().to_string());
    let runner = RealCommandRunner::new(Arc::new(env));

    let query = script
        .file_name()
        .and_then(|name| name.to_str())
        .expect("script file name should be utf-8");
    let resolved = runner
        .which(query)
        .await
        .expect("script must resolve from injected Path key");
    assert_eq!(resolved, script.display().to_string());
}

#[tokio::test]
async fn which_returns_none_when_path_missing() {
    let runner = RealCommandRunner::new(Arc::new(HashMap::new()));
    assert!(
        runner.which("definitely-missing-bin").await.is_none(),
        "no PATH means no resolution"
    );
}

#[tokio::test]
async fn run_subprocess_sees_only_injected_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let dir = TempDir::new().expect("temp dir");
    let script = make_test_script(dir.path(), "minos-test-env", ScriptMode::DumpEnv);
    std::env::set_var("MINOS_PARENT_ONLY", "from-parent");

    let mut env = HashMap::new();
    env.insert(
        "PATH".to_owned(),
        format!(
            "{}{}{}",
            dir.path().display(),
            path_sep(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    env.insert("MINOS_TEST_SENTINEL".to_owned(), "snowflake".to_owned());
    let runner = RealCommandRunner::new(Arc::new(env));

    let outcome = runner
        .run(&script.display().to_string(), &[], Duration::from_secs(5))
        .await
        .expect("script must succeed");
    std::env::remove_var("MINOS_PARENT_ONLY");
    assert_eq!(outcome.exit_code, 0);
    assert!(
        outcome.stdout.contains("MINOS_TEST_SENTINEL=snowflake"),
        "missing injected sentinel in script output:\n{}",
        outcome.stdout,
    );
    assert!(
        outcome.stdout.contains("MINOS_PARENT_ONLY=")
            && !outcome.stdout.contains("MINOS_PARENT_ONLY=from-parent"),
        "child saw parent-only env despite env_clear regression:\n{}",
        outcome.stdout,
    );
}
