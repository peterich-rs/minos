//! Smoke test for `PtyAgent` — spawns a simple shell script, verifies
//! stdout lines arrive as `RawIngest` events, and that `send_user_message`
//! reaches the child's stdin.
//!
//! Spec R3.4 / Plan P6.3.

use minos_agent_runtime::config::RawIngest;
use minos_agent_runtime::pty_agent::PtyAgent;
use minos_domain::AgentName;
use tokio::sync::broadcast;

#[tokio::test]
async fn pty_agent_reads_stdout_and_accepts_stdin() {
    let (tx, mut rx) = broadcast::channel::<RawIngest>(64);

    // Spawn a shell that prints "hello", reads one line, then prints "got <line>"
    let workspace = std::env::temp_dir();

    // We need to pass args to sh, but PtyAgent::spawn takes a cli_path only.
    // For the test, we'll write a temp script.
    let script_dir = tempfile::tempdir().unwrap();
    let script_path = script_dir.path().join("test_agent.sh");
    std::fs::write(
        &script_path,
        "#!/bin/sh\nprintf 'hello\\n'\nread line\nprintf 'got %s\\n' \"$line\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut agent = PtyAgent::spawn(
        &script_path,
        &workspace,
        AgentName::Claude,
        "test-thread-1".to_string(),
        tx.clone(),
    )
    .expect("spawn should succeed");

    // Wait for the first stdout line ("hello")
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("should receive within 5s")
        .expect("channel should not be closed");

    assert_eq!(first.thread_id, "test-thread-1");
    assert_eq!(first.agent, AgentName::Claude);
    let payload = first.payload;
    assert_eq!(payload["kind"], "raw");
    assert_eq!(payload["raw_kind"], "stdout");
    // payload_json is the JSON-encoded string "hello"
    assert_eq!(payload["payload_json"], "\"hello\"");

    // Send a message to stdin
    agent
        .send_user_message("world")
        .await
        .expect("send should succeed");

    // Wait for the second stdout line ("got world")
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("should receive within 5s")
        .expect("channel should not be closed");

    assert_eq!(second.payload["raw_kind"], "stdout");
    assert_eq!(second.payload["payload_json"], "\"got world\"");

    // Close the agent
    agent.close(&tx).await;

    // Should get a thread_closed event
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("should receive close event")
        .expect("channel should not be closed");

    assert_eq!(closed.payload["kind"], "thread_closed");
    assert_eq!(closed.payload["thread_id"], "test-thread-1");
}
