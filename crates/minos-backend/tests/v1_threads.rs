use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use minos_backend::auth::jwt;
use minos_backend::http::test_support::TEST_JWT_SECRET;
use minos_backend::http::{router, test_support::backend_state};
use minos_backend::session::SessionHandle;
use minos_backend::store::{account_host_pairings, devices::insert_device, pending_approvals};
use minos_domain::{AgentName, DeviceId, DeviceRole};
use minos_protocol::{ApprovalDecisionRequest, Envelope, EventKind, ListThreadsResponse};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;

mod common;

/// Seed an account and a paired (Mac, iOS) where both device rows are
/// linked to the new account_id. Returns
/// `(mac_id, ios_id, ios_secret, account_id)`.
async fn paired_pair_with_account(
    state: &minos_backend::http::BackendState,
    email: &str,
) -> (DeviceId, DeviceId, minos_domain::DeviceSecret, String) {
    let host = DeviceId::new();
    let ios = DeviceId::new();
    insert_device(&state.store, host, "Mac", DeviceRole::AgentHost, 0)
        .await
        .unwrap();
    insert_device(&state.store, ios, "iPhone", DeviceRole::MobileClient, 0)
        .await
        .unwrap();

    // After ADR-0020 the iOS rail is bearer-only and `secret_hash` stays
    // NULL; we no longer mint an iOS device secret. The Mac side is still
    // secret-bound, so we generate a Mac secret to keep the legacy
    // assertions and signature-compat callers happy.
    let secret = minos_domain::DeviceSecret::generate();
    let hash = minos_backend::pairing::secret::hash_secret(&secret).unwrap();
    minos_backend::store::devices::upsert_secret_hash(&state.store, host, &hash)
        .await
        .unwrap();

    // Phase 2 Task 2.6 / ADR-0020: link both device rows to a real
    // account_id, then record the pair via the account_host_pairings table
    // (the legacy device-keyed `pairings` module has been retired).
    let account = minos_backend::store::accounts::create(&state.store, email, "phc")
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &host, &account.account_id)
        .await
        .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &ios, &account.account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, host, &account.account_id, ios, 0)
        .await
        .unwrap();

    (host, ios, secret, account.account_id)
}

/// Convenience: signed bearer JWT bound to the given (account_id, device_id).
fn bearer_for(account_id: &str, device_id: DeviceId) -> String {
    jwt::sign(
        TEST_JWT_SECRET.as_bytes(),
        account_id,
        &device_id.to_string(),
    )
    .expect("test bearer signs cleanly")
}

/// Backwards-compat shim used by the tests that don't care about
/// account scoping; they still need a paired pair + bearer to satisfy
/// the new threads-route requirements.
async fn paired_pair(
    state: &minos_backend::http::BackendState,
) -> (DeviceId, DeviceId, minos_domain::DeviceSecret, String) {
    paired_pair_with_account(state, "threads-test@example.com").await
}

fn authed_post(
    uri: &str,
    _ios_id: DeviceId,
    _secret: &minos_domain::DeviceSecret,
    auth_hdr: &str,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", auth_hdr)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn seed_live_bound_session(
    state: &minos_backend::http::BackendState,
    device_id: DeviceId,
    role: DeviceRole,
    account_id: &str,
) -> tokio::sync::mpsc::Receiver<Envelope> {
    let (handle, outbox_rx) = SessionHandle::new(device_id, role);
    handle.set_account_id(account_id.to_string());
    state.registry.insert(handle);
    outbox_rx
}

fn spawn_approval_decision_responder(
    registry: Arc<minos_backend::session::SessionRegistry>,
    store: sqlx::SqlitePool,
    host_device_id: DeviceId,
    mut host_rx: tokio::sync::mpsc::Receiver<Envelope>,
) -> tokio::task::JoinHandle<ApprovalDecisionRequest> {
    tokio::spawn(async move {
        let frame = tokio::time::timeout(Duration::from_secs(2), host_rx.recv())
            .await
            .expect("approval decision should reach the host before timeout")
            .expect("host should receive approval decision rpc");
        let Envelope::Forwarded { from, payload, .. } = frame else {
            panic!("expected forwarded rpc envelope");
        };
        assert_eq!(payload["method"], "minos_approval_decision");
        let request: ApprovalDecisionRequest = serde_json::from_value(payload["params"].clone())
            .expect("approval decision params decode");

        let (host_session, _unused_rx) = SessionHandle::new(host_device_id, DeviceRole::AgentHost);
        let handled = minos_backend::envelope::handle_forward(
            &host_session,
            registry.as_ref(),
            &store,
            from,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": payload["id"].clone(),
                "result": null,
            }),
        )
        .await;
        assert!(
            handled.is_none(),
            "approval decision response should be consumed server-side"
        );
        request
    })
}

async fn assert_approval_host_command(
    pool: &sqlx::SqlitePool,
    host_device_id: DeviceId,
    requested_by_account_id: Option<&str>,
    request_id: &str,
    thread_id: &str,
    decision: serde_json::Value,
) {
    let command_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM host_commands")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(command_count, 1);

    let row = sqlx::query(
        "SELECT host_installation_id, method, params_json, requested_by_account_id, status, finished_at_ms
           FROM host_commands",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>("host_installation_id"),
        host_device_id.to_string()
    );
    assert_eq!(row.get::<String, _>("method"), "minos_approval_decision");
    assert_eq!(
        row.get::<Option<String>, _>("requested_by_account_id")
            .as_deref(),
        requested_by_account_id
    );
    assert_eq!(row.get::<String, _>("status"), "succeeded");
    assert!(row.get::<Option<i64>, _>("finished_at_ms").is_some());

    let params_json = row.get::<String, _>("params_json");
    let params: serde_json::Value = serde_json::from_str(&params_json).unwrap();
    assert_eq!(
        params,
        serde_json::json!({
            "request_id": request_id,
            "thread_id": thread_id,
            "decision": decision,
        })
    );
}

#[tokio::test]
async fn get_threads_returns_owner_scoped_list() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    // Seed two threads owned by the Mac.
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_b",
        AgentName::Claude,
        &mac_id.to_string(),
        300,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = authed_post(
        "/v1/threads/query",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({ "limit": 50 }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: ListThreadsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.threads.len(), 2);
}

#[tokio::test]
async fn get_thread_events_paginates() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    // Seed a `thread/started` event — the codex translator yields a
    // `ThreadOpened` UI event for this without prerequisite state, so the
    // assertion below can confirm the helper actually translates.
    minos_backend::store::raw_events::insert_if_absent(
        &state.store,
        "thr_a",
        1,
        AgentName::Codex,
        &serde_json::json!({
            "method":"thread/started",
            "params":{"threadId":"thr_a","createdAtMs":100}
        }),
        100,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = authed_post(
        "/v1/threads/read",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "thread_id": "thr_a",
            "limit": 10
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::ReadThreadResponse = serde_json::from_value(body).unwrap();
    assert!(!resp.ui_events.is_empty());
}

#[tokio::test]
async fn get_thread_last_seq_returns_max() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::raw_events::insert_if_absent(
        &state.store,
        "thr_a",
        7,
        AgentName::Codex,
        &serde_json::json!({"method":"x"}),
        100,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = authed_post(
        "/v1/threads/last-seq",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({ "thread_id": "thr_a" }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.last_seq, 7);
}

#[tokio::test]
async fn post_thread_last_seq_path_uses_path_thread_id() {
    let state = backend_state().await;
    let (mac_id, ios_id, _secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_path",
        AgentName::Codex,
        &mac_id.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::raw_events::insert_if_absent(
        &state.store,
        "thr_path",
        9,
        AgentName::Codex,
        &serde_json::json!({"method":"x"}),
        100,
    )
    .await
    .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/threads/thr_path/last_seq")
        .header("content-type", "application/json")
        .header("authorization", &auth_hdr)
        .body(Body::empty())
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: minos_protocol::GetThreadLastSeqResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.last_seq, 9);
}

#[tokio::test]
async fn routing_threads_filtered_by_account() {
    // Phase 2 Task 2.6: list_threads must scope by the bearer's
    // account_id. With two paired pairs on two distinct accounts, an iOS
    // bearer for account A only sees threads owned by A's Mac.
    let state = backend_state().await;
    let (mac_a, ios_a, secret_a, account_a) =
        paired_pair_with_account(&state, "alice@example.com").await;
    let (mac_b, _ios_b, _secret_b, _account_b) =
        paired_pair_with_account(&state, "bob@example.com").await;
    // Seed threads for both Macs.
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a1",
        AgentName::Codex,
        &mac_a.to_string(),
        100,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_a2",
        AgentName::Claude,
        &mac_a.to_string(),
        300,
    )
    .await
    .unwrap();
    minos_backend::store::threads::upsert(
        &state.store,
        "thr_b1",
        AgentName::Codex,
        &mac_b.to_string(),
        500,
    )
    .await
    .unwrap();

    let bearer_a = bearer_for(&account_a, ios_a);
    let auth_hdr = format!("Bearer {bearer_a}");
    let mut app = router(state);
    let req = authed_post(
        "/v1/threads/query",
        ios_a,
        &secret_a,
        &auth_hdr,
        serde_json::json!({ "limit": 50 }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    let resp: ListThreadsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(resp.threads.len(), 2, "iOS A must see only A's threads");
    let ids: Vec<&str> = resp.threads.iter().map(|t| t.thread_id.as_str()).collect();
    assert!(ids.contains(&"thr_a1"));
    assert!(ids.contains(&"thr_a2"));
    assert!(
        !ids.contains(&"thr_b1"),
        "B's thread must not leak across accounts"
    );
}

#[tokio::test]
async fn legacy_get_threads_route_is_rejected() {
    let state = backend_state().await;
    let (_mac_id, ios_id, _secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    let mut app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/threads?limit=50")
        .header("authorization", &auth_hdr)
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn get_threads_without_bearer_returns_401() {
    // After ADR-0020 the iOS rail is bearer-only. A request without an
    // Authorization header is rejected with 401 regardless of any
    // x-device-secret presented (which is no longer consulted on the iOS
    // path).
    let state = backend_state().await;
    let id = DeviceId::new();
    insert_device(&state.store, id, "iPhone", DeviceRole::MobileClient, 0)
        .await
        .unwrap();

    let mut app = router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/threads/query")
        .header("content-type", "application/json")
        .header("x-device-id", id.to_string())
        .header("x-device-role", "mobile-client")
        .body(Body::from(r#"{"limit":10}"#))
        .unwrap();
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn approval_request_timeout_broadcasts_timeout_and_auto_rejects() {
    let state = backend_state().await;
    let (mac_id, ios_id, _secret, account_id) = paired_pair(&state).await;
    let mut mobile_rx =
        seed_live_bound_session(&state, ios_id, DeviceRole::MobileClient, &account_id);
    let host_rx = seed_live_bound_session(&state, mac_id, DeviceRole::AgentHost, &account_id);
    let responder = spawn_approval_decision_responder(
        Arc::clone(&state.registry),
        state
            .store
            .sqlite_pool_cloned()
            .expect("thread approval tests run only against sqlite test state"),
        mac_id,
        host_rx,
    );

    let ts_ms = chrono::Utc::now().timestamp_millis();
    state
        .ingest
        .execute(minos_backend::ingest::use_case::IngestCommand {
            agent: AgentName::Codex,
            thread_id: "thr-approval-timeout".to_string(),
            seq: 1,
            payload: serde_json::json!({
                "method": "approval/request",
                "params": {
                    "request_id": "req-timeout",
                    "thread_id": "thr-approval-timeout",
                    "turn_id": "turn-1",
                    "method": "item/commandExecution/requestApproval",
                    "params": { "command": "rm -rf /tmp/demo" },
                    "timeout_ms": 1
                }
            }),
            ts_ms,
            owner_device_id: mac_id,
        })
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(1), mobile_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match first {
        Envelope::Event {
            event:
                EventKind::ApprovalRequest {
                    thread_id,
                    request_id,
                    method,
                    timeout_ms,
                    ..
                },
            ..
        } => {
            assert_eq!(thread_id, "thr-approval-timeout");
            assert_eq!(request_id, "req-timeout");
            assert_eq!(method, "item/commandExecution/requestApproval");
            assert_eq!(timeout_ms, 1);
        }
        other => panic!("expected approval request event, got {other:?}"),
    }

    let second = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match mobile_rx.recv().await {
                Some(
                    frame @ Envelope::Event {
                        event: EventKind::ApprovalTimeout { .. },
                        ..
                    },
                ) => return frame,
                Some(_) => {}
                None => panic!("mobile session closed before timeout event"),
            }
        }
    })
    .await
    .expect("approval timeout should be broadcast");

    match second {
        Envelope::Event {
            event:
                EventKind::ApprovalTimeout {
                    thread_id,
                    request_id,
                    reason,
                },
            ..
        } => {
            assert_eq!(thread_id, "thr-approval-timeout");
            assert_eq!(request_id, "req-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected approval timeout event, got {other:?}"),
    }

    let decision = responder.await.unwrap();
    assert_eq!(decision.request_id, "req-timeout");
    assert_eq!(decision.thread_id, "thr-approval-timeout");
    assert_eq!(
        decision.decision,
        serde_json::json!({ "decision": "decline" })
    );

    let row = pending_approvals::get(&state.store, "req-timeout")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.resolution.as_deref(), Some("timeout"));
    assert!(row.resolved_at_ms.is_some());

    assert_approval_host_command(
        &state.store,
        mac_id,
        None,
        "req-timeout",
        "thr-approval-timeout",
        serde_json::json!({ "decision": "decline" }),
    )
    .await;
}

#[tokio::test]
async fn approvals_respond_endpoint_accepts_formal_shape_and_persists_command() {
    let state = backend_state().await;
    let (mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let host_rx = seed_live_bound_session(&state, mac_id, DeviceRole::AgentHost, &account_id);
    let responder = spawn_approval_decision_responder(
        Arc::clone(&state.registry),
        state
            .store
            .sqlite_pool_cloned()
            .expect("thread approval tests run only against sqlite test state"),
        mac_id,
        host_rx,
    );
    let now_ms = chrono::Utc::now().timestamp_millis();

    pending_approvals::insert(
        &state.store,
        "req-approvals-respond",
        "thr-approvals-respond",
        "turn-1",
        mac_id,
        "item/commandExecution/requestApproval",
        &serde_json::json!({ "command": "echo hi" }),
        now_ms,
        now_ms + 10_000,
    )
    .await
    .unwrap();

    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    let mut app = router(state.clone());
    let req = authed_post(
        "/v1/approvals/respond",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "request_id": "req-approvals-respond",
            "decision": { "decision": "approve" },
            "client_request_id": "client-approval-respond-1"
        }),
    );
    let (status, body) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, serde_json::Value::Null);

    let forwarded = responder.await.unwrap();
    assert_eq!(forwarded.request_id, "req-approvals-respond");
    assert_eq!(forwarded.thread_id, "thr-approvals-respond");
    assert_eq!(
        forwarded.decision,
        serde_json::json!({ "decision": "approve" })
    );

    let row = pending_approvals::get(&state.store, "req-approvals-respond")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.resolution.as_deref(), Some("user_decision"));
    assert!(row.resolved_at_ms.is_some());

    assert_approval_host_command(
        &state.store,
        mac_id,
        Some(&account_id),
        "req-approvals-respond",
        "thr-approvals-respond",
        serde_json::json!({ "decision": "approve" }),
    )
    .await;
}

#[tokio::test]
async fn legacy_approval_decision_endpoint_is_removed() {
    let state = backend_state().await;
    let (_mac_id, ios_id, secret, account_id) = paired_pair(&state).await;
    let bearer = bearer_for(&account_id, ios_id);
    let auth_hdr = format!("Bearer {bearer}");
    let mut app = router(state);
    let req = authed_post(
        "/v1/threads/approval-decision",
        ios_id,
        &secret,
        &auth_hdr,
        serde_json::json!({
            "request_id": "req-user-decision",
            "thread_id": "thr-user-decision",
            "decision": { "decision": "approve" }
        }),
    );
    let (status, _) = common::send(&mut app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_resolution_auto_rejects_pending_approval() {
    let state = backend_state().await;
    let (mac_id, _ios_id, _secret, account_id) = paired_pair(&state).await;
    let host_rx = seed_live_bound_session(&state, mac_id, DeviceRole::AgentHost, &account_id);
    let responder = spawn_approval_decision_responder(
        Arc::clone(&state.registry),
        state
            .store
            .sqlite_pool_cloned()
            .expect("thread approval tests run only against sqlite test state"),
        mac_id,
        host_rx,
    );
    let now_ms = chrono::Utc::now().timestamp_millis();

    pending_approvals::insert(
        &state.store,
        "req-disconnect",
        "thr-disconnect",
        "turn-1",
        mac_id,
        "item/commandExecution/requestApproval",
        &serde_json::json!({ "command": "echo bye" }),
        now_ms,
        now_ms + 10_000,
    )
    .await
    .unwrap();

    state
        .approval_relay
        .resolve_disconnected_for_account(&account_id)
        .await
        .unwrap();

    let forwarded = responder.await.unwrap();
    assert_eq!(forwarded.request_id, "req-disconnect");
    assert_eq!(forwarded.thread_id, "thr-disconnect");
    assert_eq!(
        forwarded.decision,
        serde_json::json!({ "decision": "decline" })
    );

    let row = pending_approvals::get(&state.store, "req-disconnect")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.resolution.as_deref(), Some("disconnected"));
    assert!(row.resolved_at_ms.is_some());

    assert_approval_host_command(
        &state.store,
        mac_id,
        None,
        "req-disconnect",
        "thr-disconnect",
        serde_json::json!({ "decision": "decline" }),
    )
    .await;
}

#[tokio::test]
async fn disconnect_resolution_skips_hosts_with_another_online_paired_account() {
    let state = backend_state().await;
    let (mac_id, _ios_a, _secret, account_a) =
        paired_pair_with_account(&state, "threads-a@example.com").await;

    let ios_b = DeviceId::new();
    insert_device(&state.store, ios_b, "iPhone B", DeviceRole::MobileClient, 0)
        .await
        .unwrap();
    let account_b =
        minos_backend::store::accounts::create(&state.store, "threads-b@example.com", "phc")
            .await
            .unwrap();
    minos_backend::store::devices::set_account_id(&state.store, &ios_b, &account_b.account_id)
        .await
        .unwrap();
    account_host_pairings::insert_pair(&state.store, mac_id, &account_b.account_id, ios_b, 1)
        .await
        .unwrap();

    let _online_mobile_b = seed_live_bound_session(
        &state,
        ios_b,
        DeviceRole::MobileClient,
        &account_b.account_id,
    );

    let now_ms = chrono::Utc::now().timestamp_millis();
    pending_approvals::insert(
        &state.store,
        "req-disconnect-shared-host",
        "thr-disconnect-shared-host",
        "turn-1",
        mac_id,
        "item/commandExecution/requestApproval",
        &serde_json::json!({ "command": "echo still-online" }),
        now_ms,
        now_ms + 10_000,
    )
    .await
    .unwrap();

    state
        .approval_relay
        .resolve_disconnected_for_account(&account_a)
        .await
        .unwrap();

    let row = pending_approvals::get(&state.store, "req-disconnect-shared-host")
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.resolution.is_none(),
        "pending approval must remain unresolved while another paired account stays online"
    );
    assert!(row.resolved_at_ms.is_none());
}
