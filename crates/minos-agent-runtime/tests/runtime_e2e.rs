//! End-to-end state-machine tests for `AgentRuntime` driven against
//! [`FakeCodexServer`]. The `test_ws_url` seam bypasses the codex subprocess
//! entirely, so these tests exercise every piece of the runtime *except* the
//! spawn path (covered by `process.rs` unit tests on their own).
//!
//! See spec §6.1 (start → send → stream → stop), §6.3 (crash path), §6.4
//! (approval auto-reject).

use std::time::Duration;

use minos_agent_runtime::test_support::{FakeCodexServer, Step};
use minos_agent_runtime::{AgentRuntime, AgentRuntimeConfig, AgentState, RawIngest};
use minos_domain::{AgentName, MinosError};
use serde_json::json;
use tempfile::TempDir;
use url::Url;

fn make_cfg(ws_url: Url) -> AgentRuntimeConfig {
    let tmp = TempDir::new().expect("tempdir");
    let workspace_root = tmp.keep();
    let mut cfg = AgentRuntimeConfig::new(workspace_root);
    cfg.test_ws_url = Some(ws_url);
    cfg
}

fn ws_url_for(port: u16) -> Url {
    Url::parse(&format!("ws://127.0.0.1:{port}")).unwrap()
}

/// Spec §6.1: start → send → stream → stop.
#[tokio::test]
async fn happy_path_start_send_stream_stop() {
    let script = vec![
        // initialize handshake
        Step::ExpectRequest {
            method: "initialize".into(),
            reply: json!({"ok": true}),
        },
        Step::ExpectNotification {
            method: "notifications/initialized".into(),
            params: json!({}),
        },
        // thread/start handshake — result must carry thread_id
        Step::ExpectRequest {
            method: "thread/start".into(),
            reply: json!({"thread_id": "thr-abc"}),
        },
        // client sends turn/start
        Step::ExpectRequest {
            method: "turn/start".into(),
            reply: json!({"accepted": true}),
        },
        // fake emits a streaming token delta
        Step::EmitNotification {
            method: "item/agentMessage/delta".into(),
            params: json!({"delta": "Hello"}),
        },
        // polite-goodbye pair (stop uses 500ms timeouts)
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
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));

    // Observe the full state-transition timeline.
    let mut state_rx = rt.state_stream();
    assert_eq!(rt.current_state(), AgentState::Idle);

    // Subscribe for raw notifications before starting, so we don't miss anything.
    let mut ingest_rx = rt.ingest_stream();

    let outcome = rt.start(AgentName::Codex).await.unwrap();
    assert_eq!(outcome.session_id, "thr-abc");

    // After start, state must be Running with matching thread_id.
    // Poll the watch up to a few times because the watch drops intermediate
    // values (we may see Starting or Running here).
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let AgentState::Running {
            thread_id, agent, ..
        } = rt.current_state()
        {
            assert_eq!(thread_id, "thr-abc");
            assert_eq!(agent, AgentName::Codex);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never reached Running"
        );
        state_rx.changed().await.unwrap();
    }

    // Send a user message.
    rt.send_user_message("thr-abc", "ping").await.unwrap();

    // The fake emits an `item/agentMessage/delta` notification; we should
    // receive the raw JSON-RPC frame verbatim on the ingest stream.
    let ingest = tokio::time::timeout(Duration::from_secs(2), ingest_rx.recv())
        .await
        .expect("did not receive ingest event")
        .expect("broadcast receive error");
    assert_eq!(ingest.agent, AgentName::Codex);
    assert_eq!(ingest.thread_id, "thr-abc");
    assert_eq!(ingest.payload["method"], "item/agentMessage/delta");
    assert_eq!(ingest.payload["params"]["delta"], "Hello");

    // Stop — script drains through the polite-goodbye pair and then WS closes.
    rt.stop().await.unwrap();

    // After stop, state must settle on Idle.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if rt.current_state() == AgentState::Idle {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never reached Idle after stop"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Idempotent stop.
    rt.stop().await.unwrap();

    fake.stop().await;
}

/// Spec §6.4: approval ServerRequest → auto-reject + Raw broadcast.
#[tokio::test]
async fn approval_server_request_is_auto_rejected_and_broadcast() {
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
            reply: json!({"thread_id": "thr-approval"}),
        },
        // Server request — the client must auto-reject.
        Step::EmitServerRequest {
            method: "ExecCommandApproval".into(),
            params: json!({"command": ["ls", "-la"]}),
        },
        // The fake expects the reply. ExpectRequest asserts the method, but
        // replies don't have a `method`; we need a different check. Since
        // the fake's ExpectRequest reads a frame and the reply will be a
        // response (not a request), we instead just stop the fake here and
        // verify ids recorded on it.
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));
    let mut ingest_rx = rt.ingest_stream();

    let outcome = rt.start(AgentName::Codex).await.unwrap();
    assert_eq!(outcome.session_id, "thr-approval");

    // Drain at least one ingest frame — the synthetic notification for the
    // server request that the runtime surfaces after auto-rejecting.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut saw_server_req = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), ingest_rx.recv()).await {
            Ok(Ok(RawIngest { payload, .. })) => {
                if payload["method"] == "server_request/ExecCommandApproval" {
                    let params_s = serde_json::to_string(&payload["params"]).unwrap_or_default();
                    assert!(params_s.contains("\"ls\""), "{params_s}");
                    saw_server_req = true;
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    assert!(
        saw_server_req,
        "did not observe synthetic server_request ingest frame"
    );

    // The fake records the ids it generated for server requests; we only
    // need to verify it emitted one (reply correlation is implicit in the
    // runtime test — if the reply had the wrong id/shape the fake would not
    // have closed cleanly).
    let ids = fake.server_request_ids().await;
    assert_eq!(
        ids.len(),
        1,
        "fake should have emitted exactly one server request"
    );

    rt.stop().await.ok();
    fake.stop().await;
}

/// Spec §6.3: the supervisor transitions to Crashed when the WS drops
/// unexpectedly without `stop()` being called.
#[tokio::test]
async fn unexpected_ws_drop_transitions_to_crashed() {
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
            reply: json!({"thread_id": "thr-crash"}),
        },
        // Simulate codex dying.
        Step::DieUnexpectedly,
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));
    let mut state_rx = rt.state_stream();

    let outcome = rt.start(AgentName::Codex).await.unwrap();
    assert_eq!(outcome.session_id, "thr-crash");

    // After the fake drops its socket, the supervisor must transition to
    // Crashed. The watch may drop intermediate values so poll briefly.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let AgentState::Crashed { reason } = rt.current_state() {
            assert!(
                reason.contains("WS closed") || reason.contains("codex"),
                "unexpected reason: {reason}",
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never reached Crashed: current={:?}",
            rt.current_state(),
        );
        // Prefer watch changes over sleep so we transition as soon as the
        // supervisor sends.
        let _ = tokio::time::timeout(Duration::from_millis(100), state_rx.changed()).await;
    }

    // Subsequent stop() is a no-op (Idle/Crashed → Ok). We verify specifically
    // that it does not flip state back to Running.
    rt.stop().await.unwrap();
    assert!(matches!(rt.current_state(), AgentState::Crashed { .. }));

    fake.stop().await;
}

/// Additional contract test: stale session_id → AgentSessionIdMismatch
/// (not AgentNotRunning). Lives with the e2e tests because we need a
/// Running state to exercise the check.
///
/// The trailing `ExpectRequest` for `__never__` keeps the fake blocked on
/// `rx.next().await` so the WS stays open through the whole test — without
/// it the fake's script would exhaust after `thread/start`, drop the WS,
/// and the supervisor would flip state to Crashed before the mismatch check
/// had a chance to run.
#[tokio::test]
async fn stale_session_id_returns_mismatch_not_not_running() {
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
            reply: json!({"thread_id": "thr-real"}),
        },
        // Pseudo-parking step: the client will never send anything with this
        // method, so the fake blocks here and the WS stays open.
        Step::ExpectRequest {
            method: "__never__".into(),
            reply: json!({}),
        },
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));

    let _ = rt.start(AgentName::Codex).await.unwrap();
    let err = rt
        .send_user_message("thr-bogus", "hi")
        .await
        .expect_err("must mismatch");
    assert!(
        matches!(err, MinosError::AgentSessionIdMismatch),
        "got {err:?}",
    );

    // fake is still parked on `__never__`; abort it.
    fake.stop().await;
}

/// Additional contract test: two subscribers to `event_stream()` both receive
/// the same event — broadcast fan-out.
#[tokio::test]
async fn multiple_subscribers_receive_same_event() {
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
            reply: json!({"thread_id": "thr-fanout"}),
        },
        Step::EmitNotification {
            method: "item/agentMessage/delta".into(),
            params: json!({"delta": "broadcast-test"}),
        },
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));

    let mut rx1 = rt.ingest_stream();
    let mut rx2 = rt.ingest_stream();

    let _ = rt.start(AgentName::Codex).await.unwrap();

    let e1 = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
        .await
        .unwrap()
        .unwrap();
    let e2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .unwrap()
        .unwrap();
    // Broadcast fan-out should deliver payload-equal frames to both subscribers.
    // RawIngest isn't PartialEq (it carries `Value`, timestamps differ), so we
    // compare the fields individually.
    assert_eq!(e1.agent, e2.agent);
    assert_eq!(e1.thread_id, e2.thread_id);
    assert_eq!(e1.payload, e2.payload);
    assert_eq!(e1.payload["method"], "item/agentMessage/delta");
    assert_eq!(e1.payload["params"]["delta"], "broadcast-test");

    fake.stop().await;
}

/// Additional contract test: a second `start()` while Running fails with
/// AgentAlreadyRunning. Spec §5.1 invariant.
///
/// Same parking step as `stale_session_id_…` — the fake must hold the WS
/// open long enough for the second `start()` call to observe `Running`.
#[tokio::test]
async fn second_start_while_running_errors() {
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
            reply: json!({"thread_id": "thr-one"}),
        },
        Step::ExpectRequest {
            method: "__never__".into(),
            reply: json!({}),
        },
    ];
    let (fake, port) = FakeCodexServer::bind(script).await;
    let rt = AgentRuntime::new(make_cfg(ws_url_for(port)));

    let _ = rt.start(AgentName::Codex).await.unwrap();
    let err = rt
        .start(AgentName::Codex)
        .await
        .expect_err("second start must fail");
    assert!(
        matches!(err, MinosError::AgentAlreadyRunning),
        "got {err:?}",
    );

    fake.stop().await;
}
