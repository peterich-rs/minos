//! `send_user_message` must broadcast a synthetic `item/started{userMessage}`
//! ingest event so the user input lands in the persistence pipeline (local
//! daemon SQLite + relay-forwarded backend store) regardless of whether
//! codex itself ever notifies on the user input.
//!
//! Codex 2026-04 carries the user content inside the synchronous
//! `turn/start` request body and does NOT echo it as a streaming
//! notification. Without this synthesis a process kill would leave the
//! user message persisted nowhere.

use minos_agent_runtime::config::AgentRuntimeConfig;
use minos_agent_runtime::test_support::FakeCodexBackend;
use minos_agent_runtime::{AgentKind, AgentManager, InstanceCaps};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn send_user_message_emits_synth_user_item_started() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, url) = FakeCodexBackend::install().await;
    let mut cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
    cfg.test_ws_url = Some(url);
    let mgr = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));

    let session = mgr
        .start_agent(AgentKind::Codex, "/w-synth".into())
        .await
        .unwrap();

    // Subscribe BEFORE sending so we see the synth event. The fake
    // auto-responder doesn't push notifications, so any ingest event here
    // can only come from manager-side synthesis.
    let mut rx = mgr.ingest_stream();

    mgr.send_user_message(&session.thread_id, "hello world".into())
        .await
        .unwrap();

    // Pull broadcast frames with a small timeout. The synth happens on the
    // same task that handles the RPC, so it lands before send_user_message
    // returns; an immediate recv with a generous timeout keeps the test
    // stable on busy CI runners.
    let ingest = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("synth ingest must arrive within 2s")
        .expect("ingest broadcast must yield a frame");

    assert_eq!(ingest.thread_id, session.thread_id);

    let payload = &ingest.payload;
    assert_eq!(
        payload.get("method").and_then(Value::as_str),
        Some("item/started"),
        "wrong method: {payload}"
    );
    let params = payload.get("params").expect("params");
    let item = params.get("item").expect("item");
    assert_eq!(
        item.get("type").and_then(Value::as_str),
        Some("userMessage")
    );
    assert!(
        item.get("id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "item.id must be non-empty UUID: {item}"
    );

    let content = item
        .get("content")
        .and_then(Value::as_array)
        .expect("content must be an array");
    assert_eq!(content.len(), 1);
    let first = &content[0];
    assert_eq!(first.get("type").and_then(Value::as_str), Some("text"));
    assert_eq!(
        first.get("text").and_then(Value::as_str),
        Some("hello world")
    );

    assert_eq!(
        params.get("threadId").and_then(Value::as_str),
        Some(session.thread_id.as_str())
    );

    fake.stop().await;
}
