use std::sync::Arc;
use std::time::Duration;

use minos_agent_runtime::config::AgentRuntimeConfig;
use minos_agent_runtime::test_support::{FakeCodexServer, Step};
use minos_agent_runtime::{AgentKind, AgentManager, InstanceCaps, ThreadState};
use serde_json::json;

fn fake_thread_response(thread_id: &str) -> serde_json::Value {
    json!({
        "approvalPolicy": "never",
        "approvalsReviewer": "user",
        "cwd": "/tmp",
        "instructionSources": [],
        "model": "fake",
        "modelProvider": "fake",
        "sandbox": { "type": "dangerFullAccess" },
        "thread": {
            "id": thread_id,
            "cliVersion": "0.0.0-fake",
            "createdAt": 0,
            "cwd": "/tmp",
            "ephemeral": true,
            "modelProvider": "fake",
            "preview": "",
            "source": "appServer",
            "status": { "type": "idle" },
            "turns": [],
            "updatedAt": 0
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn turn_completed_notification_returns_thread_to_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let thread_id = "thr-turn-complete";
    let script = vec![
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: fake_thread_response(thread_id),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({
                "turn": {
                    "id": "turn-1",
                    "items": [],
                    "status": "inProgress"
                }
            }),
        },
        Step::EmitNotification {
            method: "turn/completed".into(),
            params: json!({
                "threadId": thread_id,
                "finishedAtMs": 123
            }),
        },
        Step::Sleep { ms: 250 },
    ];
    let (server, port) = FakeCodexServer::bind(script).await;
    let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
    cfg.test_ws_url = Some(
        url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
    );
    let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

    let session = mgr
        .start_agent(AgentKind::Codex, "/w-turn-complete".into())
        .await
        .unwrap();
    assert_eq!(session.thread_id, thread_id);

    mgr.send_user_message(thread_id, "hello".into())
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(mgr.thread_state(thread_id).await, Some(ThreadState::Idle)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("turn/completed should transition thread back to idle");

    server.stop().await;
}
