//! End-to-end integration test for the Minos backend.
//!
//! Spawns a real axum server on an ephemeral port with a `tempfile`-backed
//! SQLite DB, drives it with two raw `tokio-tungstenite` clients, and walks
//! the full spec §7.1 pairing + forward + forget_peer loop. The test is the
//! capstone correctness proof for the backend crate — if this passes, the
//! envelope protocol, session registry, local-RPC dispatcher, pairing
//! service, and WS handshake all round-trip together over real sockets.
//!
//! # Test layout
//!
//! 1. `e2e_pair_forward_forget` — happy path:
//!    - Two fresh clients connect (mac-host A, ios-client B) and each
//!      observes `Event::Unpaired` as their first server frame.
//!    - A calls `request_pairing_token`; receives `{ token, expires_at }`.
//!    - B calls `pair(token, device_name)`; receives
//!      `{ peer_device_id, peer_name, your_device_secret }` and A observes
//!      `Event::Paired` with B's info plus its own fresh device secret.
//!      **Note**: the implementation only pushes `Event::Paired` to the
//!      issuer (A); B learns about the pairing via its own RPC response.
//!      This matches `handle_pair` in `envelope::local_rpc` — the plan's
//!      "both sides observe Paired" bullet over-states what step 8 ships.
//!    - A `Forward`s a JSON-RPC-shaped payload; B observes `Forwarded`.
//!    - B `Forward`s a response; A observes `Forwarded`.
//!    - A calls `forget_peer`; both sides observe `Event::Unpaired`.
//!    - Assertion: `devices` has 2 rows, `pairings` has 0, `pairing_tokens`
//!      has one row with `consumed_at` non-null.
//!
//! 2. `e2e_pair_rejects_invalid_token` — B connects and sends `pair` with a
//!    bogus token. Asserts `LocalRpcOutcome::Err` with code
//!    `"pairing_token_invalid"`.
//!
//! 3. `e2e_reconnect_with_wrong_secret_returns_401` — a device paired once,
//!    then reconnects with a bogus secret. Step 9 rejects this
//!    **pre-upgrade** with HTTP 401 (not WS close 4401); the plan's §12
//!    bullet says "close 4401" but defers to step 9's actual design. See
//!    `src/http/ws_devices.rs` module header for why (saves a round trip).
//! 4. `e2e_reconnect_supersedes_old_socket` — a second authenticated socket
//!    for the same device actively revokes the first one, and the new socket
//!    still answers LocalRpc traffic after the old cleanup runs.
//! 5. `e2e_presence_tracks_live_peer_membership` — paired reconnects surface
//!    `PeerOffline` / `PeerOnline` from actual live sessions, notify the
//!    opposite live peer on connect, and emit `PeerOffline` on disconnect.
//!
//! # Timing
//!
//! Each test uses short `tokio::time::timeout` wrappers around `recv` so a
//! hang surfaces as a clear failure instead of stalling the CI run. Total
//! wall-clock for all three tests is well under 2s.

#![allow(clippy::too_many_lines)] // happy-path test narrates a full sequence; splitting it hides the flow.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use minos_backend::{
    http::{router, RelayState},
    pairing::{secret::hash_secret, PairingService},
    session::SessionRegistry,
    store,
};
use minos_domain::{DeviceId, DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind, LocalRpcMethod, LocalRpcOutcome};
use sqlx::SqlitePool;
use tempfile::NamedTempFile;
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    tungstenite::{client::ClientRequestBuilder, http::Uri, protocol::Message, Error as WsError},
    MaybeTlsStream, WebSocketStream,
};

/// Short timeout for individual `recv` calls. Keep this comfortably above
/// the whole-file parallel test jitter while still failing fast on hangs.
/// Sized for slow shared CI runners (GHA Linux occasionally takes >1.5s
/// per round-trip under load); local runs complete well under the bound.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TOKEN_TTL: Duration = Duration::from_mins(5);

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ── relay harness ────────────────────────────────────────────────────────

/// A live in-process relay bound to 127.0.0.1 on an ephemeral port with a
/// per-test `tempfile` SQLite database.
struct Relay {
    addr: SocketAddr,
    pool: SqlitePool,
    /// Kept alive (not dropped) for the duration of the test so the file
    /// isn't removed out from under the running pool. `NamedTempFile` auto-
    /// cleans on drop.
    _db_file: NamedTempFile,
    _db_path: PathBuf,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        // Abort the serve task so parallel tests don't leak tokio resources.
        // The server is local-only and short-lived; a hard abort here is
        // safe because each test owns its own relay instance.
        self.task.abort();
    }
}

/// Boot a fresh relay on an ephemeral port backed by a tempfile DB.
async fn spawn_relay() -> anyhow::Result<Relay> {
    spawn_relay_with_token_ttl(DEFAULT_TOKEN_TTL).await
}

/// Boot a fresh relay using an explicit pairing-token TTL.
async fn spawn_relay_with_token_ttl(token_ttl: Duration) -> anyhow::Result<Relay> {
    // Create + immediately close the tempfile so SQLite can reopen it.
    // `NamedTempFile` is kept alive in the returned `Relay` so Drop will
    // unlink the file when the test ends.
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let db_url = format!("sqlite://{}?mode=rwc", tmp_path.display());
    let pool = store::connect(&db_url).await?;

    let state = RelayState {
        registry: Arc::new(SessionRegistry::new()),
        pairing: Arc::new(PairingService::new(pool.clone())),
        store: pool.clone(),
        token_ttl,
        version: "e2e-test",
    };
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(Relay {
        addr,
        pool,
        _db_file: tmp,
        _db_path: tmp_path,
        task,
    })
}

// ── client helpers ───────────────────────────────────────────────────────

/// Open a WS client to the given relay `/devices` endpoint with the
/// supplied auth headers. Returns the upgraded stream; errors propagate
/// the `tungstenite::Error` so handshake-level failures (401) are testable.
async fn connect_client(
    relay: &Relay,
    device_id: DeviceId,
    role: DeviceRole,
    secret: Option<&str>,
    name: Option<&str>,
) -> Result<WsClient, WsError> {
    let url: Uri = format!("ws://{}/devices", relay.addr).parse().unwrap();
    let mut builder = ClientRequestBuilder::new(url)
        .with_header("X-Device-Id", device_id.to_string())
        .with_header("X-Device-Role", role.to_string());
    if let Some(s) = secret {
        builder = builder.with_header("X-Device-Secret", s.to_string());
    }
    if let Some(n) = name {
        builder = builder.with_header("X-Device-Name", n.to_string());
    }
    let (ws, _resp) = tokio_tungstenite::connect_async(builder).await?;
    Ok(ws)
}

/// Receive the next text frame as an `Envelope`, transparently ignoring
/// any server-initiated Ping/Pong so tests see only application frames.
async fn recv_envelope(ws: &mut WsClient) -> anyhow::Result<Envelope> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for envelope"))?;
        match next {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(&t)?),
            Some(Ok(Message::Ping(p))) => {
                ws.send(Message::Pong(p)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(f))) => {
                return Err(anyhow::anyhow!("unexpected close frame: {f:?}"));
            }
            Some(Ok(other)) => return Err(anyhow::anyhow!("unexpected frame: {other:?}")),
            Some(Err(e)) => return Err(anyhow::anyhow!("ws error: {e}")),
            None => return Err(anyhow::anyhow!("stream ended unexpectedly")),
        }
    }
}

/// Send an already-constructed `Envelope` over the client socket.
async fn send_envelope(ws: &mut WsClient, env: &Envelope) -> anyhow::Result<()> {
    let text = serde_json::to_string(env)?;
    ws.send(Message::Text(text.into())).await?;
    Ok(())
}

/// Wait for the relay to actively close a superseded socket.
async fn expect_close_frame(ws: &mut WsClient) -> anyhow::Result<()> {
    loop {
        let next = timeout(RECV_TIMEOUT, ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for close frame"))?;
        match next {
            Some(Ok(Message::Close(_))) | None => return Ok(()),
            Some(Ok(Message::Ping(p))) => {
                ws.send(Message::Pong(p)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(other)) => {
                return Err(anyhow::anyhow!(
                    "expected relay to close the socket, got {other:?}"
                ));
            }
            Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => {
                return Ok(());
            }
            Some(Err(e)) => return Err(anyhow::anyhow!("ws error while waiting for close: {e}")),
        }
    }
}

/// Await the first frame and assert it's `Event::Unpaired`. Used right
/// after a first-time connect.
async fn expect_unpaired_event(ws: &mut WsClient) -> anyhow::Result<()> {
    match recv_envelope(ws).await? {
        Envelope::Event {
            event: EventKind::Unpaired,
            version: 1,
        } => Ok(()),
        other => Err(anyhow::anyhow!(
            "expected Event::Unpaired as first frame, got {other:?}"
        )),
    }
}

/// Receive until we get a `LocalRpcResponse` matching `expected_id`. Any
/// intervening `Event` frames are returned in the second tuple field so
/// callers can cross-check ordering without prescribing it. A bounded
/// buffer (at most 4 events) is enforced to catch runaway chatter.
async fn recv_response_matching(
    ws: &mut WsClient,
    expected_id: u64,
) -> anyhow::Result<(LocalRpcOutcome, Vec<EventKind>)> {
    let mut events = Vec::new();
    loop {
        match recv_envelope(ws).await? {
            Envelope::LocalRpcResponse { id, outcome, .. } if id == expected_id => {
                return Ok((outcome, events));
            }
            Envelope::Event { event, .. } => {
                events.push(event);
                if events.len() > 4 {
                    return Err(anyhow::anyhow!(
                        "too many events before response id={expected_id}: {events:?}"
                    ));
                }
            }
            other => return Err(anyhow::anyhow!("unexpected envelope: {other:?}")),
        }
    }
}

// ── happy path ───────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_pair_forward_forget() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    // --- step 2: connect both clients ------------------------------------
    let a_id = DeviceId::new();
    let b_id = DeviceId::new();

    let mut a = connect_client(&relay, a_id, DeviceRole::MacHost, None, Some("Fan's Mac")).await?;
    let mut b = connect_client(
        &relay,
        b_id,
        DeviceRole::IosClient,
        None,
        Some("Fan's iPhone"),
    )
    .await?;

    expect_unpaired_event(&mut a).await?;
    expect_unpaired_event(&mut b).await?;

    // --- step 3: A requests a pairing token ------------------------------
    send_envelope(
        &mut a,
        &Envelope::LocalRpc {
            version: 1,
            id: 1,
            method: LocalRpcMethod::RequestPairingToken,
            params: serde_json::json!({}),
        },
    )
    .await?;

    let (outcome, stray_events) = recv_response_matching(&mut a, 1).await?;
    assert!(
        stray_events.is_empty(),
        "unexpected events before token response: {stray_events:?}"
    );
    let token = match outcome {
        LocalRpcOutcome::Ok { result } => {
            assert!(
                result["expires_at"].is_string(),
                "missing expires_at: {result:?}"
            );
            result["token"]
                .as_str()
                .expect("token is a string")
                .to_owned()
        }
        other => panic!("expected Ok for request_pairing_token, got {other:?}"),
    };

    // --- step 4: B pairs with the token ----------------------------------
    send_envelope(
        &mut b,
        &Envelope::LocalRpc {
            version: 1,
            id: 1,
            method: LocalRpcMethod::Pair,
            params: serde_json::json!({
                "token": token,
                "device_name": "Fan's iPhone",
            }),
        },
    )
    .await?;

    // B's RPC response carries the Mac's info + B's own fresh secret.
    let (b_pair_outcome, b_stray) = recv_response_matching(&mut b, 1).await?;
    assert!(
        b_stray.is_empty(),
        "B received stray events before pair response: {b_stray:?}"
    );
    let b_secret = match b_pair_outcome {
        LocalRpcOutcome::Ok { result } => {
            assert_eq!(
                result["peer_device_id"].as_str().unwrap(),
                a_id.to_string(),
                "peer_device_id should be A's id"
            );
            assert_eq!(result["peer_name"], "Fan's Mac");
            result["your_device_secret"]
                .as_str()
                .expect("your_device_secret is a string")
                .to_owned()
        }
        other => panic!("expected Ok for pair, got {other:?}"),
    };

    // --- step 5: A observes Event::Paired with B's info ------------------
    // The implementation only pushes Event::Paired to the issuer (A).
    let a_paired = recv_envelope(&mut a).await?;
    let a_secret = match a_paired {
        Envelope::Event {
            event:
                EventKind::Paired {
                    peer_device_id,
                    peer_name,
                    your_device_secret,
                },
            ..
        } => {
            assert_eq!(peer_device_id, b_id, "A's Paired event should name B");
            assert_eq!(peer_name, "Fan's iPhone");
            your_device_secret.as_str().to_owned()
        }
        other => panic!("expected Event::Paired on A, got {other:?}"),
    };
    assert_ne!(a_secret, b_secret, "each device gets its own fresh secret");

    // --- step 6: A Forwards a JSON-RPC call ------------------------------
    let payload_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "list_clis",
        "id": 1,
    });
    send_envelope(
        &mut a,
        &Envelope::Forward {
            version: 1,
            payload: payload_req.clone(),
        },
    )
    .await?;

    // --- step 7: B receives Forwarded and sends a response back ----------
    let forwarded_to_b = recv_envelope(&mut b).await?;
    match forwarded_to_b {
        Envelope::Forwarded { from, payload, .. } => {
            assert_eq!(from, a_id, "Forwarded should name A as the sender");
            assert_eq!(payload, payload_req, "payload must round-trip verbatim");
        }
        other => panic!("expected Forwarded on B, got {other:?}"),
    }

    let payload_resp = serde_json::json!({
        "jsonrpc": "2.0",
        "result": ["claude-code", "codex"],
        "id": 1,
    });
    send_envelope(
        &mut b,
        &Envelope::Forward {
            version: 1,
            payload: payload_resp.clone(),
        },
    )
    .await?;

    // --- step 8: A receives the Forwarded response -----------------------
    let forwarded_to_a = recv_envelope(&mut a).await?;
    match forwarded_to_a {
        Envelope::Forwarded { from, payload, .. } => {
            assert_eq!(from, b_id, "Forwarded should name B as the sender");
            assert_eq!(payload, payload_resp, "response payload must round-trip");
        }
        other => panic!("expected Forwarded on A, got {other:?}"),
    }

    // --- step 9: A calls forget_peer; both observe Event::Unpaired -------
    send_envelope(
        &mut a,
        &Envelope::LocalRpc {
            version: 1,
            id: 2,
            method: LocalRpcMethod::ForgetPeer,
            params: serde_json::json!({}),
        },
    )
    .await?;

    let (forget_outcome, a_forget_events) = recv_response_matching(&mut a, 2).await?;
    match forget_outcome {
        LocalRpcOutcome::Ok { result } => {
            assert_eq!(result, serde_json::json!({"ok": true}));
        }
        other => panic!("expected Ok for forget_peer, got {other:?}"),
    }

    // A should have received Event::Unpaired either before or after the
    // response. Accept either order.
    let a_saw_unpaired = a_forget_events
        .iter()
        .any(|e| matches!(e, EventKind::Unpaired));
    let a_saw_unpaired = if a_saw_unpaired {
        true
    } else {
        match recv_envelope(&mut a).await? {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => true,
            other => panic!("A expected Event::Unpaired after forget_peer, got {other:?}"),
        }
    };
    assert!(a_saw_unpaired, "A must observe Event::Unpaired");

    // B also observes Event::Unpaired (peer-side push).
    match recv_envelope(&mut b).await? {
        Envelope::Event {
            event: EventKind::Unpaired,
            ..
        } => {}
        other => panic!("B expected Event::Unpaired after peer forget, got {other:?}"),
    }

    // --- step 10: tear down the sockets ---------------------------------
    a.send(Message::Close(None)).await.ok();
    b.send(Message::Close(None)).await.ok();
    drop(a);
    drop(b);

    // --- step 11: DB assertions -----------------------------------------
    let devices_count: i64 = sqlx::query_scalar("SELECT count(*) FROM devices")
        .fetch_one(&relay.pool)
        .await?;
    assert_eq!(devices_count, 2, "two devices must remain");

    let pairings_count: i64 = sqlx::query_scalar("SELECT count(*) FROM pairings")
        .fetch_one(&relay.pool)
        .await?;
    assert_eq!(pairings_count, 0, "pairings row gone after forget_peer");

    let consumed_tokens: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pairing_tokens WHERE consumed_at IS NOT NULL")
            .fetch_one(&relay.pool)
            .await?;
    assert_eq!(
        consumed_tokens, 1,
        "the pair token is consumed exactly once"
    );

    Ok(())
}

#[tokio::test]
async fn e2e_request_pairing_token_respects_configured_ttl() -> anyhow::Result<()> {
    let token_ttl = Duration::from_secs(42);
    let relay = spawn_relay_with_token_ttl(token_ttl).await?;

    let mac_id = DeviceId::new();
    let mut mac =
        connect_client(&relay, mac_id, DeviceRole::MacHost, None, Some("Fan's Mac")).await?;
    expect_unpaired_event(&mut mac).await?;

    let before = chrono::Utc::now();
    send_envelope(
        &mut mac,
        &Envelope::LocalRpc {
            version: 1,
            id: 1,
            method: LocalRpcMethod::RequestPairingToken,
            params: serde_json::json!({}),
        },
    )
    .await?;

    let (outcome, stray_events) = recv_response_matching(&mut mac, 1).await?;
    assert!(
        stray_events.is_empty(),
        "unexpected events before token response: {stray_events:?}"
    );

    let after = chrono::Utc::now();
    let expires_at = match outcome {
        LocalRpcOutcome::Ok { result } => chrono::DateTime::parse_from_rfc3339(
            result["expires_at"]
                .as_str()
                .expect("expires_at is a string"),
        )?
        .with_timezone(&chrono::Utc),
        other => panic!("expected Ok for request_pairing_token, got {other:?}"),
    };

    let ttl = chrono::Duration::from_std(token_ttl).unwrap();
    let lower_bound = before + ttl;
    let upper_bound = after + ttl;
    assert!(
        expires_at >= lower_bound && expires_at <= upper_bound,
        "configured TTL not applied: expires_at={expires_at:?}, expected between {lower_bound:?} and {upper_bound:?}"
    );

    Ok(())
}

// ── negative: invalid pairing token ──────────────────────────────────────

#[tokio::test]
async fn e2e_pair_rejects_invalid_token() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let b_id = DeviceId::new();
    let mut b = connect_client(&relay, b_id, DeviceRole::IosClient, None, Some("ios")).await?;
    expect_unpaired_event(&mut b).await?;

    send_envelope(
        &mut b,
        &Envelope::LocalRpc {
            version: 1,
            id: 1,
            method: LocalRpcMethod::Pair,
            params: serde_json::json!({
                "token": "bogus_token_no_match",
                "device_name": "ios",
            }),
        },
    )
    .await?;

    let (outcome, stray) = recv_response_matching(&mut b, 1).await?;
    assert!(stray.is_empty(), "unexpected events: {stray:?}");
    match outcome {
        LocalRpcOutcome::Err { error } => {
            assert_eq!(
                error.code, "pairing_token_invalid",
                "wrong error code: {error:?}"
            );
        }
        other => panic!("expected Err for bogus token, got {other:?}"),
    }

    Ok(())
}

// ── negative: wrong X-Device-Secret ──────────────────────────────────────

#[tokio::test]
async fn e2e_reconnect_with_wrong_secret_returns_401() -> anyhow::Result<()> {
    // Spec §10.3 reserves WS close 4401 for auth failure, but step 9
    // rejects bad creds PRE-UPGRADE with HTTP 401 to avoid a wasted WS
    // round trip (see `src/http/ws_devices.rs` module header). That's the
    // semantically-equivalent contract this test asserts.
    let relay = spawn_relay().await?;

    // Seed a device row with a known secret hash (bypass the first-connect
    // flow — we want the reconnect path where a hash is already on file).
    let id = DeviceId::new();
    let good = DeviceSecret::generate();
    let good_hash = hash_secret(&good)?;
    store::devices::insert_device(&relay.pool, id, "seeded", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, id, &good_hash).await?;

    // Now reconnect with a wrong secret.
    let err = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some("definitely-not-the-right-secret"),
        None,
    )
    .await
    .expect_err("wrong secret must be rejected at handshake");

    match err {
        WsError::Http(resp) => assert_eq!(resp.status().as_u16(), 401, "expected HTTP 401"),
        other => panic!("expected WsError::Http(401), got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn e2e_reconnect_supersedes_old_socket() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let id = DeviceId::new();
    let secret = DeviceSecret::generate();
    let secret_hash = hash_secret(&secret)?;
    store::devices::insert_device(&relay.pool, id, "ios", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, id, &secret_hash).await?;

    let mut first = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some(secret.as_str()),
        Some("ios"),
    )
    .await?;
    expect_unpaired_event(&mut first).await?;

    let mut second = connect_client(
        &relay,
        id,
        DeviceRole::IosClient,
        Some(secret.as_str()),
        Some("ios"),
    )
    .await?;
    expect_unpaired_event(&mut second).await?;

    expect_close_frame(&mut first).await?;

    send_envelope(
        &mut second,
        &Envelope::LocalRpc {
            version: 1,
            id: 7,
            method: LocalRpcMethod::Ping,
            params: serde_json::json!({}),
        },
    )
    .await?;
    let (outcome, stray) = recv_response_matching(&mut second, 7).await?;
    assert!(
        stray.is_empty(),
        "unexpected events before ping response: {stray:?}"
    );
    match outcome {
        LocalRpcOutcome::Ok { result } => assert_eq!(result, serde_json::json!({"ok": true})),
        other => panic!("expected Ok ping response on replacement socket, got {other:?}"),
    }

    second.send(Message::Close(None)).await.ok();
    drop(second);

    Ok(())
}

#[tokio::test]
async fn e2e_presence_tracks_live_peer_membership() -> anyhow::Result<()> {
    let relay = spawn_relay().await?;

    let mac_id = DeviceId::new();
    let ios_id = DeviceId::new();
    let mac_secret = DeviceSecret::generate();
    let ios_secret = DeviceSecret::generate();
    let mac_hash = hash_secret(&mac_secret)?;
    let ios_hash = hash_secret(&ios_secret)?;

    store::devices::insert_device(&relay.pool, mac_id, "mac", DeviceRole::MacHost, 0).await?;
    store::devices::insert_device(&relay.pool, ios_id, "ios", DeviceRole::IosClient, 0).await?;
    store::devices::upsert_secret_hash(&relay.pool, mac_id, &mac_hash).await?;
    store::devices::upsert_secret_hash(&relay.pool, ios_id, &ios_hash).await?;
    store::pairings::insert_pairing(&relay.pool, mac_id, ios_id, 0).await?;

    let mut mac = connect_client(
        &relay,
        mac_id,
        DeviceRole::MacHost,
        Some(mac_secret.as_str()),
        Some("mac"),
    )
    .await?;
    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected initial PeerOffline on mac, got {other:?}"),
    }

    let mut ios = connect_client(
        &relay,
        ios_id,
        DeviceRole::IosClient,
        Some(ios_secret.as_str()),
        Some("ios"),
    )
    .await?;
    match recv_envelope(&mut ios).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, mac_id),
        other => panic!("expected initial PeerOnline on ios, got {other:?}"),
    }

    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOnline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOnline on mac after ios connect, got {other:?}"),
    }

    ios.send(Message::Close(None)).await.ok();
    drop(ios);

    match recv_envelope(&mut mac).await? {
        Envelope::Event {
            event: EventKind::PeerOffline { peer_device_id },
            ..
        } => assert_eq!(peer_device_id, ios_id),
        other => panic!("expected PeerOffline on mac after ios disconnect, got {other:?}"),
    }

    mac.send(Message::Close(None)).await.ok();
    drop(mac);

    Ok(())
}
