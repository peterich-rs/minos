use minos_agent_runtime::{AgentKind, IngestSink, RawBody, RawIngest};

fn ingest_with_i(i: u64) -> RawIngest {
    RawIngest::from_json(
        AgentKind::Codex,
        "thr-order".into(),
        serde_json::json!({ "i": i }),
        i64::try_from(i).unwrap(),
    )
}

fn ingest_i(ingest: &RawIngest) -> u64 {
    let RawBody::InlineBytes { bytes, .. } = &ingest.body else {
        panic!("expected inline bytes body");
    };
    serde_json::from_slice::<serde_json::Value>(bytes)
        .unwrap()
        .get("i")
        .and_then(serde_json::Value::as_u64)
        .unwrap()
}

#[tokio::test]
async fn emit_preserves_order_under_backpressure() {
    let sink = IngestSink::new(64);
    let mut rx = sink.install_durable_stream();
    let producer_sink = sink.clone();

    let producer = tokio::spawn(async move {
        for i in 0..2_000u64 {
            producer_sink.emit(ingest_with_i(i)).await.unwrap();
        }
    });

    let mut seen = Vec::with_capacity(2_000);
    while seen.len() < 2_000 {
        let ingest = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for durable ingest")
            .expect("durable channel closed");
        seen.push(ingest_i(&ingest));
    }

    producer.await.unwrap();
    assert_eq!(seen, (0..2_000u64).collect::<Vec<_>>());
}

#[tokio::test]
async fn emit_returns_error_when_durable_sink_is_closed() {
    let sink = IngestSink::new(64);
    let rx = sink.install_durable_stream();
    drop(rx);

    let error = sink.emit(ingest_with_i(1)).await.unwrap_err();
    assert_eq!(error.to_string(), "durable ingest sink is closed");
}
