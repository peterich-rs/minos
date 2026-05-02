//! Regression test for the FK-constraint bug surfaced post-Phase-D: codex
//! events were getting dropped with `FOREIGN KEY constraint failed (787)`
//! the moment the user sent the first message, because `AgentGlue::start_agent`
//! wasn't persisting the parent `threads` / `workspaces` rows the
//! `events.thread_id` FK depends on.
//!
//! The test wires a `FakeCodexBackend` so codex doesn't have to be installed
//! on the host, drives `AgentGlue::start_agent`, and then asserts (a) both
//! parent rows landed and (b) `EventWriter::write_live` for the new
//! `thread_id` succeeds. Without the fix the second assertion fails with
//! the SQLite 787 error.

#![cfg(feature = "test-support")]

use minos_agent_runtime::config::AgentRuntimeConfig;
use minos_agent_runtime::test_support::FakeCodexBackend;
use minos_agent_runtime::{AgentKind, AgentManager, InstanceCaps, RawIngest};
use minos_daemon::agent::AgentGlue;
use minos_daemon::store::event_writer::EventWriter;
use minos_daemon::store::LocalStore;
use minos_domain::AgentName;
use minos_protocol::{AgentLaunchMode, StartAgentRequest};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread")]
async fn start_agent_persists_thread_so_event_writer_does_not_fk_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();

    let store = Arc::new(
        LocalStore::open(&tmp.path().join("daemon.sqlite"))
            .await
            .unwrap(),
    );

    // FakeCodexBackend stands in for `codex app-server`. Its auto-responder
    // accepts `initialize` / `thread/start` / `turn/start` so AgentManager
    // can complete its handshake without a real codex binary on the host.
    let (_fake, url) = FakeCodexBackend::install().await;
    let mut cfg = AgentRuntimeConfig::new(workspace.clone());
    cfg.test_ws_url = Some(url);
    let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

    let (relay_tx, _relay_rx) = mpsc::channel(64);
    let writer = Arc::new(EventWriter::spawn(store.clone(), relay_tx));
    let glue = AgentGlue::wire_with(manager.clone(), writer.clone(), store.clone(), workspace);

    let resp = glue
        .start_agent(StartAgentRequest {
            agent: AgentName::Codex,
            workspace: String::new(),
            mode: Some(AgentLaunchMode::Server),
        })
        .await
        .expect("start_agent should succeed against the fake codex");

    // (a) parent rows now exist.
    let threads = store.list_threads(None, None).await.unwrap();
    assert_eq!(
        threads.len(),
        1,
        "exactly one thread row should land after start_agent",
    );
    assert_eq!(threads[0].thread_id, resp.session_id);
    assert_eq!(threads[0].agent, "codex");
    assert_eq!(threads[0].status, "idle");
    assert_eq!(
        threads[0].codex_session_id.as_deref(),
        Some(resp.session_id.as_str()),
        "codex_session_id must be populated for §9.3 jsonl recovery",
    );

    let workspace_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(workspace_count, 1, "workspace row must be upserted");

    // (b) writing a synthetic ingest event for the new thread_id no longer
    //     trips the events.thread_id FK. This is the actual user-visible
    //     symptom: pre-fix, AgentGlue's bridge spammed
    //     "EventWriter.write_live failed; event dropped {error=... 787}"
    //     the moment codex sent its first frame.
    let seq = writer
        .write_live(RawIngest {
            agent: AgentKind::Codex,
            thread_id: resp.session_id.clone(),
            payload: serde_json::json!({"kind": "smoke"}),
            ts_ms: 1,
        })
        .await
        .expect("write_live should not fail with FK 787 once the thread row exists");
    assert_eq!(seq, 1);

    // The new event row reflects the writer's commit.
    let rows = store.read_events(&resp.session_id, 1, 1).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, 1);
    assert_eq!(rows[0].source, "live");
}
