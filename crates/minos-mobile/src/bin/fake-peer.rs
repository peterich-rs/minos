//! `fake-peer` — dev tool that impersonates an `ios-client` role
//! against the backend broker so the Mac app's pairing + dispatch flow
//! can be smoke-tested without an actual iPhone.
//!
//! Plan 05 Phase L.1 / spec §10.3 / plan 08b Phase 12 Task 12.1.
//!
//! Phase 2 Task 2.3 made `/v1/pairing/consume` bearer-gated for the
//! `ios-client` role, so every subcommand must establish auth before it
//! pairs. Phase 12 Task 12.1 split the binary into three subcommands:
//!
//! - `pair` — register a throwaway account if necessary then redeem a
//!   pairing token. Inbound WS frames are printed to stderr until the
//!   socket closes.
//! - `register` — explicit register-then-pair flow. Same wire shape as
//!   `pair` but with a clear "this is a fresh account" intent in the
//!   subcommand name.
//! - `smoke-session` — full register-or-login → pair → `start_agent` →
//!   tail UiEvents loop. Drives the agent dispatch surface that Wave 2
//!   landed on the mobile Rust side, end-to-end against a real backend.
//!
//! Usage:
//!
//! ```text
//! # 1. Start a local backend
//! cargo run -p minos-backend -- --listen 127.0.0.1:8787 --db /tmp/relay.db
//!
//! # 2. Show the QR in the Mac app, decode it, copy the pairing token
//!
//! # 3a. Pair-only (account created on demand, default credentials)
//! cargo run -p minos-mobile --bin fake-peer --features cli -- pair \
//!     --backend ws://127.0.0.1:8787/devices \
//!     --token <token-from-qr> \
//!     --device-name "Fan's fake iPhone"
//!
//! # 3b. Register a new account first, then pair
//! cargo run -p minos-mobile --bin fake-peer --features cli -- register \
//!     --backend ws://127.0.0.1:8787/devices \
//!     --email fan+smoke@example.com \
//!     --password Sup3rSecret! \
//!     --token <token-from-qr> \
//!     --device-name "Fan's fake iPhone"
//!
//! # 3c. Drive a full smoke session: register-or-login, pair, start_agent
//! cargo run -p minos-mobile --bin fake-peer --features cli -- smoke-session \
//!     --backend ws://127.0.0.1:8787/devices \
//!     --email fan+smoke@example.com \
//!     --password Sup3rSecret! \
//!     --token <token-from-qr> \
//!     --prompt "Hello from fake-peer" \
//!     --device-name "Fan's fake iPhone"
//! ```
//!
//! No retry logic, no Keychain persistence — this is dev-only.

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use minos_domain::defaults::DEV_BACKEND_URL;
use minos_domain::{AgentName, ConnectionState, DeviceId, DeviceRole, MinosError, PairingToken};
use minos_mobile::http::MobileHttpClient;
use minos_mobile::{MobileClient, PersistedPairingState};
use minos_protocol::{Envelope, PairConsumeRequest};
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser, Debug)]
#[command(
    name = "fake-peer",
    about = "Smoke-test Mac pairing + agent dispatch without iOS by impersonating an ios-client.",
    long_about = "Three subcommands cover the dev smoke surfaces: `pair` to \
                  redeem a single pairing token, `register` to create an \
                  account before pairing, and `smoke-session` to drive the \
                  full register-or-login → pair → start_agent loop."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Pair-only flow: register a throwaway account on demand and
    /// redeem the supplied pairing token. Inbound frames stream to
    /// stderr until the socket closes.
    Pair {
        /// Relay backend URL (the `/devices` WebSocket endpoint).
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        /// Pairing token captured from the Mac's QR payload.
        #[arg(long)]
        token: String,
        /// Display name announced to the host during pair.
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
        /// Email used to register the throwaway account.
        #[arg(long, default_value = "fake-peer@example.com")]
        email: String,
        /// Password for the throwaway account.
        #[arg(long, default_value = "fake-peer-pw-12345")]
        password: String,
    },
    /// Register a fresh account then pair. Same wire shape as `pair`,
    /// but exits as soon as the WS handshake completes; use this when
    /// you only need to warm up an account before driving traffic by
    /// other means.
    Register {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        token: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
    },
    /// Full smoke session: try login, fall back to register, pair if
    /// needed, then call `start_agent` and stream `UiEventFrame`s to
    /// stderr until the user interrupts.
    SmokeSession {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        token: String,
        #[arg(long)]
        prompt: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
        /// Agent to start. Mirrors the Mac-side AgentName variants.
        #[arg(long, default_value = "codex")]
        agent: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Pair {
            backend,
            token,
            device_name,
            email,
            password,
        } => {
            // Pair-only: try login first so the same throwaway email can
            // be reused across runs without colliding on EmailTaken.
            let auth = login_or_register(&backend, &email, &password).await?;
            run_pair_then_tail(&backend, &token, &device_name, auth.access_token).await
        }
        Cmd::Register {
            backend,
            email,
            password,
            token,
            device_name,
        } => {
            // Register-only: surface EmailTaken back to the operator so a
            // typo on a reused email doesn't silently fall through to
            // login.
            let auth = register_account(&backend, &email, &password).await?;
            run_pair_then_tail(&backend, &token, &device_name, auth.access_token).await
        }
        Cmd::SmokeSession {
            backend,
            email,
            password,
            token,
            prompt,
            device_name,
            agent,
        } => {
            let agent_name = parse_agent(&agent)?;
            run_smoke_session(
                &backend,
                &email,
                &password,
                &token,
                &prompt,
                &device_name,
                agent_name,
            )
            .await
        }
    }
}

fn parse_agent(s: &str) -> Result<AgentName> {
    match s {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        other => anyhow::bail!("unknown agent {other:?}; want one of codex/claude/gemini"),
    }
}

/// Register an account via HTTP. Returns the freshly issued auth tuple.
struct RegisteredAuth {
    access_token: String,
    refresh_token: String,
    account_id: String,
    account_email: String,
}

async fn register_account(backend: &str, email: &str, password: &str) -> Result<RegisteredAuth> {
    let device_id = DeviceId::new();
    let http = MobileHttpClient::new(backend, device_id, None).context("build MobileHttpClient")?;
    eprintln!("→ POST /v1/auth/register email={email}");
    let resp = http
        .register(email, password, None)
        .await
        .context("POST /v1/auth/register")?;
    eprintln!(
        "← account_id={} email={}",
        resp.account.account_id, resp.account.email
    );
    Ok(RegisteredAuth {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        account_id: resp.account.account_id,
        account_email: resp.account.email,
    })
}

/// Try login first; fall back to register on UnknownAccount /
/// InvalidCredentials so the same command works whether the account
/// already exists or not.
async fn login_or_register(backend: &str, email: &str, password: &str) -> Result<RegisteredAuth> {
    let device_id = DeviceId::new();
    let http = MobileHttpClient::new(backend, device_id, None).context("build MobileHttpClient")?;
    eprintln!("→ POST /v1/auth/login email={email}");
    match http.login(email, password, None).await {
        Ok(resp) => {
            eprintln!(
                "← login OK account_id={} email={}",
                resp.account.account_id, resp.account.email
            );
            Ok(RegisteredAuth {
                access_token: resp.access_token,
                refresh_token: resp.refresh_token,
                account_id: resp.account.account_id,
                account_email: resp.account.email,
            })
        }
        Err(e) => {
            eprintln!("← login failed ({e:?}); falling back to register");
            register_account(backend, email, password).await
        }
    }
}

/// Pair via HTTP, open the authenticated WS, and tail inbound frames to
/// stderr. Used by both the `pair` and `register` subcommands.
async fn run_pair_then_tail(
    backend: &str,
    token: &str,
    device_name: &str,
    access_token: String,
) -> Result<()> {
    let device_id = DeviceId::new();
    let http = MobileHttpClient::new(backend, device_id, None).context("build MobileHttpClient")?;
    let pair_req = PairConsumeRequest {
        token: PairingToken(token.to_string()),
        device_name: device_name.to_string(),
    };
    eprintln!("→ POST /v1/pairing/consume token={token}");
    let pair_resp = http
        .pair_consume(pair_req, &access_token)
        .await
        .context("POST /v1/pairing/consume")?;
    eprintln!(
        "← peer_device_id={} peer_name={}",
        pair_resp.peer_device_id, pair_resp.peer_name
    );

    let mut request = backend
        .to_string()
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
        DeviceRole::MobileClient
            .to_string()
            .parse()
            .context("encode device-role header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-device-name"),
        device_name.parse().context("encode device-name header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("authorization"),
        format!("Bearer {access_token}")
            .parse()
            .context("encode authorization header")?,
    );

    eprintln!("connecting as {device_id} (role=ios-client) to {backend}");
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

/// Drive the SmokeSession flow end-to-end via the real `MobileClient`.
/// This exercises the same code path the iOS client takes, so any
/// regression in the Wave 2 agent-dispatch surface lights up here.
async fn run_smoke_session(
    backend: &str,
    email: &str,
    password: &str,
    token: &str,
    prompt: &str,
    device_name: &str,
    agent: AgentName,
) -> Result<()> {
    // Establish auth via the dedicated HTTP client first so the
    // MobileClient is built with the live tuple already in place.
    // login-then-register lets the same invocation work for both
    // first-launch and subsequent runs.
    let auth = login_or_register(backend, email, password).await?;

    // Seed the MobileClient via the persisted-state ctor so the auth
    // tuple is in place before we call `pair_with_qr_json_at` (the pair
    // path looks the access token up from `auth_session`).
    let now_ms = chrono::Utc::now().timestamp_millis();
    let persisted = PersistedPairingState {
        device_id: None,
        access_token: Some(auth.access_token),
        access_expires_at_ms: Some(now_ms + 15 * 60 * 1000),
        refresh_token: Some(auth.refresh_token),
        account_id: Some(auth.account_id),
        account_email: Some(auth.account_email),
    };
    let client = MobileClient::new_with_persisted_state(device_name.to_string(), persisted);

    let mut ui_events = client.ui_events_stream();

    // Build a fresh QR JSON envelope around the supplied pairing token
    // so we don't have to ask the user to paste full JSON.
    let qr_json = serde_json::json!({
        "v": 2,
        "host_display_name": "FakeMac",
        "pairing_token": token,
        "expires_at_ms": i64::MAX,
    })
    .to_string();
    eprintln!("→ pair_with_qr_json_at backend={backend}");
    client
        .pair_with_qr_json_at(qr_json, backend)
        .await
        .context("pair_with_qr_json_at")?;
    eprintln!("← paired; current_state={:?}", client.current_state());

    // Wait for the WS to land in `Connected` so the outbox is registered
    // before we call `start_agent` (which requires an outbox).
    wait_for_connected(&client).await?;

    eprintln!("→ start_agent agent={agent:?} prompt={prompt:?}");
    let resp = client
        .start_agent(agent, prompt.to_string())
        .await
        .context("start_agent")?;
    eprintln!("← session_id={} cwd={}", resp.session_id, resp.cwd);
    eprintln!(
        "→ send_user_message session_id={} text={prompt:?}",
        resp.session_id
    );
    client
        .send_user_message(resp.session_id.clone(), prompt.to_string())
        .await
        .context("send_user_message")?;
    eprintln!("← send_user_message ok");

    eprintln!("tailing ui_events_stream — Ctrl-C to exit");
    loop {
        match ui_events.recv().await {
            Ok(frame) => eprintln!("← ui_event seq={} ui={:?}", frame.seq, frame.ui),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("← ui_events_stream lagged by {n} frames");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                eprintln!("← ui_events_stream closed");
                break;
            }
        }
    }
    Ok(())
}

async fn wait_for_connected(client: &MobileClient) -> Result<(), MinosError> {
    // Pair with QR transitions to Connected before returning, but the
    // outbox handshake is processed on the recv loop's first frame; a
    // brief retry window protects against the race.
    for _ in 0..50 {
        if matches!(client.current_state(), ConnectionState::Connected) {
            return Ok(());
        }
        sleep(Duration::from_millis(20)).await;
    }
    Err(MinosError::NotConnected)
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
