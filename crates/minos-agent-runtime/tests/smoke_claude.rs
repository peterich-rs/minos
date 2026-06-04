use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn should_run() -> bool {
    std::env::var("MINOS_XTASK_WITH_CLAUDE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[tokio::test]
#[ignore]
async fn claude_real_smoke_start_and_chat() {
    if !should_run() {
        return;
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel(256);
    let cli_path = PathBuf::from("claude");

    let session = minos_agent_runtime::ClaudeNdjsonSession::start_turn(
        &cli_path,
        &workspace,
        "smoke_test_thread".into(),
        "Say hello in one word",
        None,
        events_tx,
        &Arc::new(std::collections::HashMap::new()),
    )
    .await
    .expect("claude spawn failed");

    let timeout = tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(ingest) = events_rx.recv().await {
            if ingest
                .payload
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
