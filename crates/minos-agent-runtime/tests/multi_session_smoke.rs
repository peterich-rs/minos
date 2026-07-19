//! C22 multi-session smoke test against the FakeCodexBackend.
//!
//! Spec §12.2 lays out the scenario; the goal is to exercise spawn, send,
//! interrupt, implicit resume, and idle-reaper paths against a stand-in
//! codex server so the test stays hermetic on hosts without a real codex
//! binary.

use minos_agent_runtime::config::AgentRuntimeConfig;
use minos_agent_runtime::state_machine::{CloseReason, PauseReason};
use minos_agent_runtime::test_support::FakeCodexBackend;
use minos_agent_runtime::{AgentKind, AgentManager, InstanceCaps, ThreadState};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn multi_session_smoke() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, url) = FakeCodexBackend::install().await;
    let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
    cfg.test_ws_url = Some(url);
    let caps = InstanceCaps {
        max_instances: 8,
        idle_timeout: Duration::from_millis(150),
    };
    let mgr = Arc::new(AgentManager::new(cfg, caps));

    // 1. Two workspaces, two threads each.
    let a1 = mgr
        .start_agent(AgentKind::Codex, "/w-A".into())
        .await
        .unwrap();
    let a2 = mgr
        .start_agent(AgentKind::Codex, "/w-A".into())
        .await
        .unwrap();
    let b1 = mgr
        .start_agent(AgentKind::Codex, "/w-B".into())
        .await
        .unwrap();
    let b2 = mgr
        .start_agent(AgentKind::Codex, "/w-B".into())
        .await
        .unwrap();
    assert_eq!(mgr.open_workspaces().await.len(), 2);
    assert_eq!(mgr.thread_count().await, 4);

    // 2. send_user_message on a1 — the test relies on the auto-responder
    //    accepting `turn/start`. The Idle -> Running transition is observable
    //    via the per-thread state stream.
    mgr.send_user_message(&a1.thread_id, "hello".into())
        .await
        .unwrap();
    assert!(matches!(
        mgr.thread_state(&a1.thread_id).await.unwrap(),
        ThreadState::Running { .. }
    ));

    // 3. interrupt a2; verify Suspended.
    // Move a2 to Running first so interrupt is legal (Idle threads also
    // accept interrupt per the validator, but Running is the production
    // case).
    mgr.send_user_message(&a2.thread_id, "ping".into())
        .await
        .unwrap();
    mgr.interrupt_thread(&a2.thread_id).await.unwrap();
    assert!(matches!(
        mgr.thread_state(&a2.thread_id).await.unwrap(),
        ThreadState::Suspended {
            reason: PauseReason::UserInterrupt
        }
    ));

    // 4. send_user_message on a2 (suspended) -> Resuming -> Idle -> Running.
    mgr.send_user_message(&a2.thread_id, "resume me".into())
        .await
        .unwrap();
    assert!(matches!(
        mgr.thread_state(&a2.thread_id).await.unwrap(),
        ThreadState::Running { .. }
    ));

    // 5. close_thread on b2 (Idle); verify Closed.
    mgr.close_thread(&b2.thread_id).await.unwrap();
    assert!(matches!(
        mgr.thread_state(&b2.thread_id).await.unwrap(),
        ThreadState::Closed {
            reason: CloseReason::UserClose
        }
    ));

    // 6. Wait past the idle timeout, then drive the reaper. Both threads on
    //    /w-B are non-Running (b1 Idle, b2 Closed), so /w-B should reap.
    //    /w-A still has Running threads (a1 / a2) so it stays.
    tokio::time::sleep(Duration::from_millis(300)).await;
    mgr.tick_reaper_once().await;
    let open = mgr.open_workspaces().await;
    assert!(
        !open.contains(&std::path::PathBuf::from("/w-B")),
        "/w-B should be reaped"
    );
    assert!(open.contains(&std::path::PathBuf::from("/w-A")));
    assert!(matches!(
        mgr.thread_state(&b1.thread_id).await.unwrap(),
        ThreadState::Suspended {
            reason: PauseReason::InstanceReaped
        }
    ));

    fake.stop().await;
}
