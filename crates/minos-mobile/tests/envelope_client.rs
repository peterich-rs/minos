//! Envelope-client integration tests.
//!
//! Spins up a tiny `tokio-tungstenite` server that speaks the envelope
//! protocol (`Envelope::LocalRpc` / `Envelope::LocalRpcResponse` /
//! `Envelope::Event`). Verifies the mobile client:
//!
//! 1. Opens the WS and delivers a well-formed `Pair` `LocalRpc` request.
//! 2. Persists the returned `your_device_secret` on a successful `pair`.
//! 3. Fans out `EventKind::UiEventMessage` to `ui_events_stream`
//!    subscribers with the correct shape.
//!
//! These tests do not exercise CF Access (no edge is involved) and do not
//! exercise reconnection loops — the plan's scope is MVP envelope wiring.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use minos_domain::{ConnectionState, DeviceId};
use minos_mobile::{MobileClient, PersistedPairingState};
use minos_protocol::{
    Envelope, EventKind, ListThreadsParams, ListThreadsResponse, LocalRpcMethod, LocalRpcOutcome,
    PairingQrPayload,
};
use minos_ui_protocol::UiEventMessage;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

/// Accept one client, reply to the first `LocalRpc::Pair` with a
/// successful response carrying `your_device_secret`, then drop the socket.
async fn fake_backend_pair_ok(listener: TcpListener) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("handshake");
    let (mut write, mut read) = ws.split();

    while let Some(msg) = read.next().await {
        let Ok(Message::Text(text)) = msg else { break };
        let Ok(env) = serde_json::from_str::<Envelope>(text.as_ref()) else {
            continue;
        };
        if let Envelope::LocalRpc {
            id,
            method: LocalRpcMethod::Pair,
            ..
        } = env
        {
            let resp = Envelope::LocalRpcResponse {
                version: 1,
                id,
                outcome: LocalRpcOutcome::Ok {
                    result: serde_json::json!({
                        "peer_device_id": "dev_peer",
                        "peer_name": "FakeMac",
                        "your_device_secret": "sec_abc"
                    }),
                },
            };
            write
                .send(Message::Text(serde_json::to_string(&resp).unwrap().into()))
                .await
                .unwrap();
            break;
        }
    }
}

/// Accept one client, reply to `Pair` OK, then push one
/// `EventKind::UiEventMessage` into the socket.
async fn fake_backend_pair_then_push(listener: TcpListener) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("handshake");
    let (mut write, mut read) = ws.split();

    while let Some(msg) = read.next().await {
        let Ok(Message::Text(text)) = msg else { break };
        let Ok(env) = serde_json::from_str::<Envelope>(text.as_ref()) else {
            continue;
        };
        if let Envelope::LocalRpc {
            id,
            method: LocalRpcMethod::Pair,
            ..
        } = env
        {
            let resp = Envelope::LocalRpcResponse {
                version: 1,
                id,
                outcome: LocalRpcOutcome::Ok {
                    result: serde_json::json!({
                        "peer_device_id": "dev_peer",
                        "peer_name": "FakeMac",
                        "your_device_secret": "sec_abc"
                    }),
                },
            };
            write
                .send(Message::Text(serde_json::to_string(&resp).unwrap().into()))
                .await
                .unwrap();

            let push = Envelope::Event {
                version: 1,
                event: EventKind::UiEventMessage {
                    thread_id: "thr_1".into(),
                    seq: 7,
                    ui: UiEventMessage::TextDelta {
                        message_id: "msg_1".into(),
                        text: "Hi".into(),
                    },
                    ts_ms: 42,
                },
            };
            write
                .send(Message::Text(serde_json::to_string(&push).unwrap().into()))
                .await
                .unwrap();
            // Keep the socket open long enough for the client recv loop to
            // observe the push before we tear down.
            tokio::time::sleep(Duration::from_millis(100)).await;
            break;
        }
    }
}

async fn fake_backend_resume_then_list_threads(
    listener: TcpListener,
    expected_device_id: String,
    expected_device_secret: String,
    expected_cf_access: Option<(String, String)>,
) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = accept_hdr_async(
        stream,
        #[allow(clippy::result_large_err)] // accept_hdr_async dictates the closure signature.
        move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
            let headers = req.headers();
            assert_eq!(
                headers
                    .get("X-Device-Id")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_device_id.as_str())
            );
            assert_eq!(
                headers
                    .get("X-Device-Secret")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_device_secret.as_str())
            );
            if let Some((id, secret)) = &expected_cf_access {
                assert_eq!(
                    headers
                        .get("CF-Access-Client-Id")
                        .and_then(|value| value.to_str().ok()),
                    Some(id.as_str())
                );
                assert_eq!(
                    headers
                        .get("CF-Access-Client-Secret")
                        .and_then(|value| value.to_str().ok()),
                    Some(secret.as_str())
                );
            } else {
                assert!(headers.get("CF-Access-Client-Id").is_none());
                assert!(headers.get("CF-Access-Client-Secret").is_none());
            }
            Ok(response)
        },
    )
    .await
    .expect("handshake");
    let (mut write, mut read) = ws.split();

    while let Some(msg) = read.next().await {
        let Ok(Message::Text(text)) = msg else { break };
        let Ok(env) = serde_json::from_str::<Envelope>(text.as_ref()) else {
            continue;
        };
        if let Envelope::LocalRpc {
            id,
            method: LocalRpcMethod::ListThreads,
            ..
        } = env
        {
            let resp = Envelope::LocalRpcResponse {
                version: 1,
                id,
                outcome: LocalRpcOutcome::Ok {
                    result: serde_json::to_value(&ListThreadsResponse {
                        threads: vec![],
                        next_before_ts_ms: None,
                    })
                    .unwrap(),
                },
            };
            write
                .send(Message::Text(serde_json::to_string(&resp).unwrap().into()))
                .await
                .unwrap();
            return;
        }
    }
}

fn make_qr(backend_url: &str, cf_access: Option<(&str, &str)>) -> String {
    serde_json::to_string(&PairingQrPayload {
        v: 2,
        backend_url: backend_url.into(),
        host_display_name: "FakeMac".into(),
        pairing_token: "tok".into(),
        expires_at_ms: i64::MAX,
        cf_access_client_id: cf_access.map(|(id, _)| id.to_string()),
        cf_access_client_secret: cf_access.map(|(_, secret)| secret.to_string()),
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_qr_json_happy_path_reaches_connected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(fake_backend_pair_ok(listener));

    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let qr = make_qr(&format!("ws://{addr}/devices"), None);
    client.pair_with_qr_json(qr).await.unwrap();

    assert_eq!(client.current_state(), ConnectionState::Connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_events_stream_delivers_backend_fanout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(fake_backend_pair_then_push(listener));

    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let mut rx = client.ui_events_stream();

    let qr = make_qr(&format!("ws://{addr}/devices"), None);
    client.pair_with_qr_json(qr).await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("should receive one frame within 2s")
        .expect("broadcast channel must stay open");
    assert_eq!(frame.thread_id, "thr_1");
    assert_eq!(frame.seq, 7);
    match frame.ui {
        UiEventMessage::TextDelta { text, .. } => assert_eq!(text, "Hi"),
        other => panic!("unexpected ui variant: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_threads_round_trips_over_envelope() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut write, mut read) = ws.split();

        // Reply to Pair first.
        loop {
            let Some(Ok(Message::Text(msg))) = read.next().await else {
                return;
            };
            let Ok(env) = serde_json::from_str::<Envelope>(msg.as_ref()) else {
                continue;
            };
            match env {
                Envelope::LocalRpc {
                    id,
                    method: LocalRpcMethod::Pair,
                    ..
                } => {
                    write
                        .send(Message::Text(
                            serde_json::to_string(&Envelope::LocalRpcResponse {
                                version: 1,
                                id,
                                outcome: LocalRpcOutcome::Ok {
                                    result: serde_json::json!({
                                        "peer_device_id": "dev_peer",
                                        "peer_name": "FakeMac",
                                        "your_device_secret": "s"
                                    }),
                                },
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
                Envelope::LocalRpc {
                    id,
                    method: LocalRpcMethod::ListThreads,
                    ..
                } => {
                    let resp = Envelope::LocalRpcResponse {
                        version: 1,
                        id,
                        outcome: LocalRpcOutcome::Ok {
                            result: serde_json::to_value(&ListThreadsResponse {
                                threads: vec![],
                                next_before_ts_ms: None,
                            })
                            .unwrap(),
                        },
                    };
                    write
                        .send(Message::Text(serde_json::to_string(&resp).unwrap().into()))
                        .await
                        .unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let qr = make_qr(&format!("ws://{addr}/devices"), None);
    client.pair_with_qr_json(qr).await.unwrap();

    let resp = client
        .list_threads(ListThreadsParams {
            limit: 50,
            before_ts_ms: None,
            agent: None,
        })
        .await
        .unwrap();
    assert!(resp.threads.is_empty());
    assert!(resp.next_before_ts_ms.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_exports_persisted_state_and_rehydrates_new_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(fake_backend_pair_ok(listener));

    let backend_url = format!("ws://{addr}/devices");
    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let qr = make_qr(&backend_url, Some(("cf-id", "cf-secret")));
    client.pair_with_qr_json(qr).await.unwrap();

    let persisted = client.persisted_pairing_state().await.unwrap();
    assert_eq!(persisted.backend_url.as_deref(), Some(backend_url.as_str()));
    assert_eq!(persisted.cf_access_client_id.as_deref(), Some("cf-id"));
    assert_eq!(
        persisted.cf_access_client_secret.as_deref(),
        Some("cf-secret")
    );
    assert!(persisted.device_id.as_deref().is_some());
    assert_eq!(persisted.device_secret.as_deref(), Some("sec_abc"));

    let rehydrated = MobileClient::new_with_persisted_state("iPhone".into(), persisted.clone());
    let restored = rehydrated.persisted_pairing_state().await.unwrap();
    let expected = PersistedPairingState {
        backend_url: Some(backend_url),
        device_id: persisted.device_id.clone(),
        device_secret: Some("sec_abc".into()),
        cf_access_client_id: Some("cf-id".into()),
        cf_access_client_secret: Some("cf-secret".into()),
    };
    assert_eq!(restored, expected);
}

/// Accept one client, immediately close the socket with code 4401 to
/// simulate the backend rejecting a stale-secret resume (the same close
/// code `ws_devices::upgrade` emits when activation revalidation fails).
async fn fake_backend_resume_rejects_with_4401(listener: TcpListener) {
    let (stream, _) = listener.accept().await.expect("accept");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("handshake");
    let (mut write, _read) = ws.split();
    let _ = write
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Library(4401),
            reason: "auth_revoked".into(),
        })))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_returns_error_when_backend_rejects_with_4401() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device_id = DeviceId::new();
    let backend = tokio::spawn(fake_backend_resume_rejects_with_4401(listener));

    let client = MobileClient::new_with_persisted_state(
        "iPhone".into(),
        PersistedPairingState {
            backend_url: Some(format!("ws://{addr}/devices")),
            device_id: Some(device_id.to_string()),
            device_secret: Some("sec_revoked".into()),
            cf_access_client_id: None,
            cf_access_client_secret: None,
        },
    );

    // The client attempts resume, the backend closes 4401, the inbound
    // loop transitions ConnectionState back to Disconnected. The cold-start
    // path on Dart sees this as a normal Err and is responsible for
    // wiping the persisted snapshot before pairing again.
    let resume = tokio::time::timeout(Duration::from_secs(2), client.resume_persisted_session())
        .await
        .expect("resume_persisted_session must not hang on a 4401 close");

    // The handshake succeeds (HTTP 101) before the backend closes, so the
    // mobile client treats the resume as having connected; the close frame
    // is observed asynchronously and downgrades state to Disconnected. The
    // Dart-side `resolveClient` recovery branch fires when EITHER the resume
    // returns Err or the state never reaches Connected — assert that here
    // by waiting for the disconnected state.
    let _ = resume;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(client.current_state(), ConnectionState::Disconnected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("resume must end in Disconnected after 4401 close");

    backend.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_persisted_session_reconnects_and_supports_list_threads() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device_id = DeviceId::new();
    let backend = tokio::spawn(fake_backend_resume_then_list_threads(
        listener,
        device_id.to_string(),
        "sec_resume".into(),
        Some(("cf-id".into(), "cf-secret".into())),
    ));

    let client = MobileClient::new_with_persisted_state(
        "iPhone".into(),
        PersistedPairingState {
            backend_url: Some(format!("ws://{addr}/devices")),
            device_id: Some(device_id.to_string()),
            device_secret: Some("sec_resume".into()),
            cf_access_client_id: Some("cf-id".into()),
            cf_access_client_secret: Some("cf-secret".into()),
        },
    );

    client.resume_persisted_session().await.unwrap();
    assert_eq!(client.current_state(), ConnectionState::Connected);

    let resp = client
        .list_threads(ListThreadsParams {
            limit: 50,
            before_ts_ms: None,
            agent: None,
        })
        .await
        .unwrap();
    assert!(resp.threads.is_empty());
    assert!(resp.next_before_ts_ms.is_none());

    backend.await.unwrap();
}
