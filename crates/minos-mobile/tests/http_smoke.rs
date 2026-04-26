//! Round-trip tests for [`minos_mobile::http::MobileHttpClient`] against an
//! in-process `minos-backend` axum router.

use minos_backend::http::{router, test_support::backend_state};
use minos_domain::{DeviceId, DeviceRole};
use minos_mobile::http::MobileHttpClient;
use minos_protocol::PairConsumeRequest;

#[tokio::test(flavor = "multi_thread")]
async fn pair_consume_round_trips_against_real_backend() {
    let state = backend_state().await;
    let mac_id = DeviceId::new();
    minos_backend::store::devices::insert_device(
        &state.store,
        mac_id,
        "Mac",
        DeviceRole::AgentHost,
        0,
    )
    .await
    .unwrap();
    let svc = minos_backend::pairing::PairingService::new(state.store.clone());
    let (token, _) = svc
        .request_token(mac_id, std::time::Duration::from_mins(5))
        .await
        .unwrap();

    // Seed a live Mac session so consume can deliver Event::Paired.
    let (handle, mut mac_outbox) =
        minos_backend::session::SessionHandle::new(mac_id, DeviceRole::AgentHost);
    state.registry.insert(handle);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let consumer_id = DeviceId::new();
    let client = MobileHttpClient::new(&format!("ws://{addr}/devices"), consumer_id, None).unwrap();
    let resp = client
        .pair_consume(PairConsumeRequest {
            token,
            device_name: "iPhone".into(),
        })
        .await
        .unwrap();

    assert_eq!(resp.peer_device_id, mac_id);
    assert_eq!(resp.peer_name, "Mac");
    assert_eq!(resp.your_device_secret.as_str().len(), 43);
    let _ = mac_outbox.recv().await.unwrap(); // Event::Paired delivered
}
