use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use minos_agent_runtime::{AgentManager, AgentRuntimeConfig, InstanceCaps};
use minos_domain::AgentName;

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
    let mut cfg = AgentRuntimeConfig::new(workspace.clone());
    cfg.subprocess_env = Arc::new(std::collections::HashMap::new());
    let mgr = AgentManager::new(cfg, InstanceCaps::default());
    let started = mgr
        .start_agent(AgentName::Claude, workspace)
        .await
        .expect("start claude agent");
    let mut rx = mgr.ingest_stream();
    mgr.send_user_message(&started.session_id, "Say hello in one word".into())
        .await
        .expect("send prompt");

    let timeout = tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(ingest) = rx.recv().await {
            if ingest.session_id == started.session_id
                && ingest
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

    assert!(
        timeout.is_ok() && timeout.unwrap(),
        "claude smoke test timed out"
    );
}
