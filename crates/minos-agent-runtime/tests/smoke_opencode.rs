use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn should_run() -> bool {
    std::env::var("MINOS_XTASK_WITH_OPENCODE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[tokio::test]
#[ignore]
async fn opencode_real_smoke_create_session_and_prompt() {
    if !should_run() {
        return;
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let password = uuid::Uuid::new_v4().to_string();

    let config = minos_agent_runtime::opencode_driver::OpencodeServerConfig {
        opencode_bin: PathBuf::from("opencode"),
        port: 4199,
        password,
        subprocess_env: Arc::new(std::collections::HashMap::new()),
        opencode_config_content: None,
    };

    let mut instance = minos_agent_runtime::OpencodeServerInstance::spawn(&workspace, config)
        .await
        .expect("opencode spawn failed");

    let session_id = instance
        .create_session()
        .await
        .expect("create session failed");

    instance
        .send_prompt(&session_id, "Say hello in one word")
        .await
        .expect("send prompt failed");

    tokio::time::sleep(Duration::from_secs(10)).await;
    instance.close().await;
}
