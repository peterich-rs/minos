//! `fake-peer` — dev tool that impersonates an `ios-client` role
//! against the backend broker so the Mac app's pairing + dispatch flow
//! can be smoke-tested without an actual iPhone.
//!
//! Plan 05 Phase L.1 / spec §10.3.
//!
//! Phase 2 Task 2.3 made `/v1/pairing/consume` bearer-gated for the
//! `ios-client` role, so this binary now registers a throwaway account
//! first and stamps the resulting bearer onto both the HTTP pair call
//! and the subsequent WebSocket handshake. Phase 12 Task 12.1 will
//! restructure this into clap subcommands; for now the single mode is
//! "register-then-pair".
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
//!     --device-name "Fan's fake iPhone" \
//!     --email fake@example.com \
//!     --password Sup3rSecret!
//! ```
//!
//! The pairing handshake uses HTTP `POST /v1/auth/register` followed by
//! `POST /v1/pairing/consume` (the WS `LocalRpc` dispatcher was retired
//! with plan 07 Phase D); the returned `device_secret` and bearer are
//! both stamped onto an authenticated WebSocket connection and inbound
//! frames are printed to stderr until the socket closes.
//!
//! No retry logic, no Keychain persistence — this is dev-only.

use anyhow::Context as _;
use clap::Parser;
use futures_util::StreamExt;
use minos_domain::{DeviceId, DeviceRole, PairingToken};
use minos_mobile::http::MobileHttpClient;
use minos_protocol::{Envelope, PairConsumeRequest};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser, Debug)]
#[command(
    name = "fake-peer",
    about = "Smoke-test Mac pairing without iOS by impersonating an ios-client.",
    long_about = "Registers a throwaway account on the backend, pairs against \
                  the HTTP /v1/pairing/consume route with the resulting bearer, \
                  opens an authenticated WebSocket with the device-secret + \
                  bearer, and prints inbound frames until interrupted."
)]
struct Args {
    /// Relay backend URL (the `/devices` WebSocket endpoint; the HTTP
    /// origin is derived from the same host).
    #[arg(long, default_value = "ws://127.0.0.1:8787/devices")]
    backend: String,
    /// Pairing token captured from the Mac's QR payload.
    #[arg(long)]
    token: String,
    /// Display name announced to the host during pair.
    #[arg(long, default_value = "fake-peer")]
    device_name: String,
    /// Email used to register a throwaway account on the backend. Phase
    /// 2 made the `ios-client` rail bearer-gated, so the binary needs an
    /// account before it can pair.
    #[arg(long, default_value = "fake-peer@example.com")]
    email: String,
    /// Password for the throwaway account. Must satisfy the backend's
    /// password rules (see spec §5.4).
    #[arg(long, default_value = "fake-peer-pw-12345")]
    password: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let device_id = DeviceId::new();

    // 1. Register a throwaway account so we have a bearer for the
    //    bearer-gated pair endpoint and the WS upgrade.
    let http =
        MobileHttpClient::new(&args.backend, device_id, None).context("build MobileHttpClient")?;
    eprintln!("→ POST /v1/auth/register email={}", args.email);
    let auth_resp = http
        .register(&args.email, &args.password)
        .await
        .context("POST /v1/auth/register")?;
    eprintln!(
        "← account_id={} email={}",
        auth_resp.account.account_id, auth_resp.account.email
    );
    let access_token = auth_resp.access_token;

    // 2. Pair via HTTP, stamping the bearer.
    let pair_req = PairConsumeRequest {
        token: PairingToken(args.token.clone()),
        device_name: args.device_name.clone(),
    };
    eprintln!("→ POST /v1/pairing/consume token={}", args.token);
    let pair_resp = http
        .pair_consume(pair_req, &access_token)
        .await
        .context("POST /v1/pairing/consume")?;
    eprintln!(
        "← peer_device_id={} peer_name={}",
        pair_resp.peer_device_id, pair_resp.peer_name
    );
    let secret = pair_resp.your_device_secret;

    // 3. Open the authenticated WebSocket. The bearer is also stamped
    //    on the WS upgrade so the backend's bearer-gated `IosClient`
    //    rail accepts the connection (spec §6.1).
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
    request.headers_mut().insert(
        HeaderName::from_static("x-device-secret"),
        secret
            .as_str()
            .parse()
            .context("encode device-secret header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-device-name"),
        args.device_name
            .parse()
            .context("encode device-name header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("authorization"),
        format!("Bearer {access_token}")
            .parse()
            .context("encode authorization header")?,
    );

    eprintln!(
        "connecting as {device_id} (role=ios-client) to {}",
        args.backend
    );
    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .context("ws handshake")?;
    let (_sink, mut stream) = ws.split();

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
        Envelope::Event { event, .. } => {
            eprintln!("← event: {event:?}");
        }
        Envelope::Forwarded { from, payload, .. } => {
            eprintln!("← forwarded from={from} payload={payload}");
        }
        Envelope::Forward { .. } | Envelope::Ingest { .. } => {
            eprintln!("← unexpected client→relay envelope mirrored back: {raw}");
        }
    }
}
