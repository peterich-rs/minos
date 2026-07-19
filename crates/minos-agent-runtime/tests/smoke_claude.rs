use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use minos_agent_runtime::{
    manager_event::ManagerEvent, state_machine::ThreadState, thread_handle::ThreadHandle,
    IngestSink,
};

fn should_run() -> bool {
    std::env::var("MINOS_XTASK_WITH_CLAUDE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[tokio::test]
#[ignore = "requires a local claude CLI and is opt-in via MINOS_XTASK_WITH_CLAUDE"]
async fn claude_real_smoke_start_and_chat() {
    if !should_run() {
        return;
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let events_tx = IngestSink::new(256);
    let mut events_rx = events_tx.subscribe();
    let (manager_tx, _) = tokio::sync::broadcast::channel::<ManagerEvent>(32);
    let threads = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    threads.lock().await.insert(
        "smoke_test_thread".into(),
        ThreadHandle::new(
            "smoke_test_thread".into(),
            workspace.clone(),
            minos_domain::AgentName::Claude,
            ThreadState::Running {
                turn_started_at_ms: 0,
            },
            0,
        ),
    );
    let cli_path = PathBuf::from("claude");

    let session = minos_agent_runtime::ClaudeNdjsonSession::start_turn(
        &cli_path,
        &workspace,
        "smoke_test_thread".into(),
        "Say hello in one word",
        Some("b0c2c7f6-841b-4af6-9dc7-05d860b4a9b1"),
        None,
        threads,
        manager_tx,
        events_tx,
        &Arc::new(std::collections::HashMap::new()),
        None,
        None,
        None,
    )
    .await
    .expect("claude spawn failed");

    let timeout = tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(ingest) = events_rx.recv().await {
            if ingest
                .json_value()
                .expect("raw ingest should contain JSON payload")
                .get("type")
                .and_then(serde_json::Value::as_str)
                == Some("result")
            {
                return true;
            }
        }
        false
    })
    .await;

    assert!(timeout.is_ok(), "claude smoke test timed out");
    drop(session);
}
