//! Envelope-client integration tests.
//!
//! Spins up a tiny `tokio-tungstenite` server that speaks the envelope
//! protocol (`Envelope::LocalRpc` / `Envelope::LocalRpcResponse` /
//! `Envelope::Event`). Verifies the mobile client:
//!
//! 1. Opens the WS and delivers a well-formed `Pair` `LocalRpc` request.
//! 2. Persists the returned `device_secret` on a successful `pair`.
//! 3. Fans out `EventKind::UiEventMessage` to `ui_events_stream`
//!    subscribers with the correct shape.
//!
//! These tests do not exercise CF Access (no edge is involved) and do not
//! exercise reconnection loops — the plan's scope is MVP envelope wiring.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use minos_domain::ConnectionState;
use minos_mobile::MobileClient;
use minos_protocol::{
    Envelope, EventKind, ListThreadsParams, ListThreadsResponse, LocalRpcMethod, LocalRpcOutcome,
    PairingQrPayload,
};
use minos_ui_protocol::UiEventMessage;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Accept one client, reply to the first `LocalRpc::Pair` with a
/// successful response carrying `device_secret`, then drop the socket.
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
                    result: serde_json::json!({"device_secret": "sec_abc"}),
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
                    result: serde_json::json!({"device_secret": "sec_abc"}),
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

fn make_qr(backend_url: &str) -> String {
    serde_json::to_string(&PairingQrPayload {
        v: 2,
        backend_url: backend_url.into(),
        host_display_name: "FakeMac".into(),
        pairing_token: "tok".into(),
        expires_at_ms: i64::MAX,
        cf_access_client_id: None,
        cf_access_client_secret: None,
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_qr_json_happy_path_reaches_connected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(fake_backend_pair_ok(listener));

    let client = MobileClient::new_with_in_memory_store("iPhone".into());
    let qr = make_qr(&format!("ws://{addr}/devices"));
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

    let qr = make_qr(&format!("ws://{addr}/devices"));
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
                                    result: serde_json::json!({"device_secret": "s"}),
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
    let qr = make_qr(&format!("ws://{addr}/devices"));
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
