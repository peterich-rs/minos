//! `fake-peer` — dev tool that impersonates an `ios-client` role
//! against the backend broker so the Mac app's pairing + dispatch flow
//! can be smoke-tested without an actual iPhone.
//!
//! Plan 05 Phase L.1 / spec §10.3.
//!
//! Usage:
//!
//! ```text
//! # 1. Start a local backend
//! cargo run -p minos-backend -- --listen 127.0.0.1:8787 --db /tmp/relay.db
//!
//! # 2. Show the QR in the Mac app, decode it, copy the token field
//!
//! # 3. Run fake-peer with the captured token
//! cargo run -p minos-mobile --bin fake-peer --features cli -- \
//!     --backend ws://127.0.0.1:8787/devices \
//!     --token <token-from-qr> \
//!     --device-name "Fan's fake iPhone"
//! ```
//!
//! After Pair the backend sends a Paired event back to the Mac and a
//! LocalRpcResponse to the fake-peer. Subsequent inbound frames are
//! printed verbatim to stderr so operators can eyeball Forwarded /
//! Event traffic.
//!
//! No retry logic, no Keychain persistence — this is dev-only.

use anyhow::Context as _;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use minos_domain::{DeviceId, DeviceRole};
use minos_protocol::envelope::{Envelope, LocalRpcMethod};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser, Debug)]
#[command(
    name = "fake-peer",
    about = "Smoke-test Mac pairing without iOS by impersonating an ios-client.",
    long_about = "Connects to a relay broker, sends a single Pair LocalRpc, then \
                  prints inbound frames until interrupted. Intended for local dev only."
)]
struct Args {
    /// Relay backend URL.
    #[arg(long, default_value = "ws://127.0.0.1:8787/devices")]
    backend: String,
    /// Pairing token captured from the Mac's QR payload.
    #[arg(long)]
    token: String,
    /// Display name announced to the host during Pair.
    #[arg(long, default_value = "fake-peer")]
    device_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let device_id = DeviceId::new();

    let mut request = args
        .backend
        .clone()
        .into_client_request()
        .context("parse backend URL")?;
    request.headers_mut().insert(
        HeaderName::from_static("x-device-id"),
        device_id
            .to_string()
            .parse()
            .context("encode device-id header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-device-role"),
        DeviceRole::IosClient
            .to_string()
            .parse()
            .context("encode device-role header")?,
    );

    eprintln!(
        "connecting as {device_id} (role=ios-client) to {}",
        args.backend
    );
    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .context("ws handshake")?;
    let (mut sink, mut stream) = ws.split();

    let pair_request = Envelope::LocalRpc {
        version: 1,
        id: 1,
        method: LocalRpcMethod::Pair,
        params: json!({
            "token": args.token,
            "device_name": args.device_name,
        }),
    };
    let text = serde_json::to_string(&pair_request).context("serialize Pair envelope")?;
    eprintln!("→ {text}");
    sink.send(Message::Text(text.into()))
        .await
        .context("send Pair envelope")?;

    while let Some(msg) = stream.next().await {
        match msg.context("ws read")? {
            Message::Text(text) => match serde_json::from_str::<Envelope>(&text) {
                Ok(envelope) => print_envelope(&envelope, &text),
                Err(e) => eprintln!("← (unparsed) {text} | parse err: {e}"),
            },
            Message::Close(frame) => {
                eprintln!("← close: {frame:?}");
                break;
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    Ok(())
}

fn print_envelope(envelope: &Envelope, raw: &str) {
    match envelope {
        Envelope::LocalRpcResponse { id, outcome, .. } => {
            eprintln!("← local_rpc_response id={id} outcome={outcome:?}");
        }
        Envelope::Event { event, .. } => {
            eprintln!("← event: {event:?}");
        }
        Envelope::Forwarded { from, payload, .. } => {
            eprintln!("← forwarded from={from} payload={payload}");
        }
        Envelope::Forward { .. } | Envelope::LocalRpc { .. } => {
            eprintln!("← unexpected client→relay envelope mirrored back: {raw}");
        }
    }
}
