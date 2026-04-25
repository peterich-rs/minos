//! Integration tests for `minos_agent_runtime::ingest::Ingestor`.
//!
//! Spins up a fake `tokio-tungstenite` server, connects the Ingestor, and
//! asserts the envelope shape coming across the wire.

use futures_util::StreamExt;
use minos_agent_runtime::ingest::Ingestor;
use minos_domain::AgentName;
use tokio::net::TcpListener;

/// Minimal accept loop: reads one frame, returns it as a `String`.
async fn accept_one_frame(listener: TcpListener) -> String {
    let (stream, _) = listener.accept().await.unwrap();
    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    let (_write, mut read) = ws.split();
    let msg = read.next().await.unwrap().unwrap();
    msg.into_text().unwrap().to_string()
}

#[tokio::test]
async fn ingestor_sends_one_envelope_ingest_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(accept_one_frame(listener));

    let (ingestor, _handle) = Ingestor::connect(&format!("ws://{addr}"), "device-id", None)
        .await
        .unwrap();

    ingestor
        .push(
            AgentName::Codex,
            "thr_1",
            serde_json::json!({"method":"item/started","params":{}}),
        )
        .await
        .unwrap();

    let text = server.await.unwrap();
    let env: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(env["kind"], "ingest");
    assert_eq!(env["v"], 1);
    assert_eq!(env["agent"], "codex");
    assert_eq!(env["thread_id"], "thr_1");
    assert_eq!(env["seq"], 1);
    assert_eq!(env["payload"]["method"], "item/started");
    // ts_ms must be present and non-zero on a non-degenerate clock.
    assert!(env["ts_ms"].is_i64());
}

#[tokio::test]
async fn ingestor_seq_counter_increments_per_thread() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (_write, mut read) = ws.split();
        let mut out = Vec::new();
        for _ in 0..4 {
            let msg = read.next().await.unwrap().unwrap();
            let v: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            out.push(v);
        }
        out
    });

    let (ingestor, _handle) = Ingestor::connect(&format!("ws://{addr}"), "device-id", None)
        .await
        .unwrap();

    for payload in [
        serde_json::json!({"method":"a"}),
        serde_json::json!({"method":"b"}),
        serde_json::json!({"method":"c"}),
    ] {
        ingestor
            .push(AgentName::Codex, "thr_1", payload)
            .await
            .unwrap();
    }
    ingestor
        .push(AgentName::Codex, "thr_2", serde_json::json!({"method":"z"}))
        .await
        .unwrap();

    let frames = server.await.unwrap();
    assert_eq!(frames[0]["seq"], 1);
    assert_eq!(frames[0]["thread_id"], "thr_1");
    assert_eq!(frames[1]["seq"], 2);
    assert_eq!(frames[1]["thread_id"], "thr_1");
    assert_eq!(frames[2]["seq"], 3);
    assert_eq!(frames[2]["thread_id"], "thr_1");
    // thr_2 counter is independent: starts at 1.
    assert_eq!(frames[3]["seq"], 1);
    assert_eq!(frames[3]["thread_id"], "thr_2");
}
