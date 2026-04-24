#![cfg(feature = "test-support")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use minos_agent_runtime::test_support::{FakeCodexServer, Step};
use minos_agent_runtime::{AgentRuntime, AgentRuntimeConfig};
use minos_daemon::{AgentGlue, AgentState, DaemonConfig, DaemonHandle};
use minos_domain::AgentName;
use minos_protocol::{MinosRpcClient, SendUserMessageRequest, StartAgentRequest};
use serde_json::json;

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn start_send_stream_stop_against_fake_codex_server() {
    let script = vec![
        Step::ExpectRequest {
            method: "initialize".into(),
            reply: json!({"ok": true}),
        },
        Step::ExpectNotification {
            method: "notifications/initialized".into(),
            params: json!({}),
        },
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: json!({"thread_id": "thr-daemon"}),
        },
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({"accepted": true}),
        },
        Step::EmitNotification {
            method: "item/agentMessage/delta".into(),
            params: json!({"delta": "Hello from fake codex"}),
        },
        Step::ExpectRequest {
            method: "turn/interrupt".into(),
            reply: json!({}),
        },
        Step::ExpectRequest {
            method: "thread/archive".into(),
            reply: json!({}),
        },
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;

    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("MINOS_DATA_DIR", temp.path());
    std::env::set_var("MINOS_LOG_DIR", temp.path().join("logs"));

    let mut agent_cfg = AgentRuntimeConfig::new(temp.path().join("workspaces"));
    agent_cfg.test_ws_url = Some(format!("ws://127.0.0.1:{port}").parse().unwrap());
    let agent = Arc::new(AgentGlue::new_with_runtime(AgentRuntime::new(agent_cfg)));

    // Observe the agent's raw ingest stream directly. The subscribe_events
    // JSON-RPC surface was removed in plan §B3 — events now flow to the
    // backend via the `Ingestor` (plan §B4), not through jsonrpsee.
    let mut ingest_rx = agent.ingest_stream();

    let handle = DaemonHandle::start_with_agent_glue(
        DaemonConfig {
            mac_name: "agent-e2e".into(),
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        },
        agent,
    )
    .await
    .unwrap();

    let client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(&format!("ws://{}", handle.addr()))
        .await
        .unwrap();

    let start = MinosRpcClient::start_agent(
        &client,
        StartAgentRequest {
            agent: AgentName::Codex,
        },
    )
    .await
    .unwrap();
    assert_eq!(start.session_id, "thr-daemon");

    let running_deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let AgentState::Running {
            thread_id, agent, ..
        } = handle.current_agent_state()
        {
            assert_eq!(thread_id, start.session_id);
            assert_eq!(agent, AgentName::Codex);
            break;
        }
        assert!(
            std::time::Instant::now() < running_deadline,
            "agent never reached Running"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    MinosRpcClient::send_user_message(
        &client,
        SendUserMessageRequest {
            session_id: start.session_id.clone(),
            text: "ping".into(),
        },
    )
    .await
    .unwrap();

    let ingest = tokio::time::timeout(Duration::from_secs(2), ingest_rx.recv())
        .await
        .expect("expected an ingest frame within 2s")
        .expect("ingest broadcast receive error");
    assert_eq!(ingest.agent, AgentName::Codex);
    assert_eq!(ingest.thread_id, "thr-daemon");
    assert_eq!(ingest.payload["method"], "item/agentMessage/delta");
    assert_eq!(ingest.payload["params"]["delta"], "Hello from fake codex");

    MinosRpcClient::stop_agent(&client).await.unwrap();

    let idle_deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if handle.current_agent_state() == AgentState::Idle {
            break;
        }
        assert!(
            std::time::Instant::now() < idle_deadline,
            "agent never returned to Idle"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    drop(ingest_rx);
    drop(client);
    handle.stop().await.unwrap();
    fake.stop().await;
}
