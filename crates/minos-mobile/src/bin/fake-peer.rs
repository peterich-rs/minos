//! `fake-peer` — dev tool that impersonates a mobile client against the
//! backend so Host Link + session dispatch can be smoke-tested without a phone.
//!
//! Host binding is **Desktop "Link this Mac"** (account↔host). This binary only
//! authenticates as a mobile client and drives sessions against already-linked
//! hosts.
//!
//! Subcommands:
//!
//! - `register` — create an account (no host bind)
//! - `login` — login and print account id
//! - `list-hosts` — login + `GET /v1/hosts`
//! - `smoke-session` — login → pick linked host → resume WS → send message
//!
//! ```text
//! cargo run -p minos-mobile --bin fake-peer --features cli -- list-hosts \
//!     --backend http://127.0.0.1:8787 \
//!     --email you@example.com --password secret
//!
//! cargo run -p minos-mobile --bin fake-peer --features cli -- smoke-session \
//!     --backend http://127.0.0.1:8787 \
//!     --email you@example.com --password secret \
//!     --prompt "Hello from fake-peer"
//! ```

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use minos_domain::defaults::DEV_BACKEND_URL;
use minos_domain::{AgentName, DeviceId, MinosError};
use minos_mobile::http::MobileHttpClient;
use minos_mobile::{MobileClient, PersistedPairingState};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Parser, Debug)]
#[command(
    name = "fake-peer",
    about = "Smoke-test mobile auth + linked hosts without QR pairing."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a fresh account (Host Link still happens on Desktop).
    Register {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
    },
    /// Login and print account metadata.
    Login {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
    },
    /// Login and list linked hosts (`GET /v1/hosts`).
    ListHosts {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
    },
    /// Login, select a linked host, open WS, send a prompt, tail UI events.
    SmokeSession {
        #[arg(long, default_value_t = DEV_BACKEND_URL.to_string())]
        backend: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        prompt: String,
        #[arg(long, default_value = "fake-peer")]
        device_name: String,
        #[arg(long, default_value = "codex")]
        agent: String,
        /// Optional host installation id; default = first online, else first linked.
        #[arg(long)]
        host_installation_id: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Register {
            backend,
            email,
            password,
            device_name,
        } => {
            let auth = register_account(&backend, &email, &password, &device_name).await?;
            eprintln!(
                "registered account_id={} email={}",
                auth.account_id, auth.account_email
            );
            Ok(())
        }
        Cmd::Login {
            backend,
            email,
            password,
            device_name,
        } => {
            let auth = login_or_register(&backend, &email, &password, &device_name).await?;
            eprintln!(
                "login ok account_id={} email={}",
                auth.account_id, auth.account_email
            );
            Ok(())
        }
        Cmd::ListHosts {
            backend,
            email,
            password,
            device_name,
        } => {
            let auth = login_or_register(&backend, &email, &password, &device_name).await?;
            let http = MobileHttpClient::new(&backend, auth.device_id, device_name)
                .context("build MobileHttpClient")?;
            let hosts = http
                .list_hosts(&auth.access_token)
                .await
                .context("GET /v1/hosts")?;
            if hosts.hosts.is_empty() {
                eprintln!("no linked hosts — use Desktop “Link this Mac” first");
            }
            for h in hosts.hosts {
                eprintln!(
                    "host={} name={} online={} linked_at_ms={}",
                    h.host_device_id, h.host_display_name, h.online, h.paired_at_ms
                );
            }
            Ok(())
        }
        Cmd::SmokeSession {
            backend,
            email,
            password,
            prompt,
            device_name,
            agent,
            host_installation_id,
        } => {
            let _agent = parse_agent(&agent)?;
            run_smoke_session(
                &backend,
                &email,
                &password,
                &prompt,
                &device_name,
                host_installation_id.as_deref(),
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
        "opencode" => Ok(AgentName::Opencode),
        "grok" => Ok(AgentName::Grok),
        other => {
            anyhow::bail!("unknown agent {other:?}; want one of codex/claude/gemini/opencode/grok")
        }
    }
}

struct RegisteredAuth {
    device_id: DeviceId,
    access_token: String,
    refresh_token: String,
    account_id: String,
    account_email: String,
}

async fn register_account(
    backend: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> Result<RegisteredAuth> {
    let device_id = DeviceId::new();
    let http =
        MobileHttpClient::new(backend, device_id, device_name).context("build MobileHttpClient")?;
    eprintln!("→ POST /v1/auth/register email={email}");
    let resp = http
        .register(email, password)
        .await
        .context("POST /v1/auth/register")?;
    Ok(RegisteredAuth {
        device_id,
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        account_id: resp.account.account_id,
        account_email: resp.account.email,
    })
}

async fn login_or_register(
    backend: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> Result<RegisteredAuth> {
    let device_id = DeviceId::new();
    let http =
        MobileHttpClient::new(backend, device_id, device_name).context("build MobileHttpClient")?;
    eprintln!("→ POST /v1/auth/login email={email}");
    match http.login(email, password).await {
        Ok(resp) => Ok(RegisteredAuth {
            device_id,
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            account_id: resp.account.account_id,
            account_email: resp.account.email,
        }),
        Err(e) => {
            eprintln!("← login failed ({e:?}); falling back to register");
            register_account(backend, email, password, device_name).await
        }
    }
}

async fn run_smoke_session(
    backend: &str,
    email: &str,
    password: &str,
    prompt: &str,
    device_name: &str,
    host_installation_id: Option<&str>,
) -> Result<()> {
    let auth = login_or_register(backend, email, password, device_name).await?;
    let http = MobileHttpClient::new(backend, auth.device_id, device_name)
        .context("build MobileHttpClient")?;
    let hosts = http
        .list_hosts(&auth.access_token)
        .await
        .context("GET /v1/hosts")?;
    let host = pick_host(&hosts.hosts, host_installation_id)?;
    eprintln!(
        "using host={} online={} name={}",
        host.host_device_id, host.online, host.host_display_name
    );

    let now_ms = chrono::Utc::now().timestamp_millis();
    let persisted = PersistedPairingState {
        device_id: Some(auth.device_id.to_string()),
        access_token: Some(auth.access_token),
        access_expires_at_ms: Some(now_ms + 15 * 60 * 1000),
        refresh_token: Some(auth.refresh_token),
        account_id: Some(auth.account_id),
        account_email: Some(auth.account_email),
    };
    let client = MobileClient::new_with_persisted_state(device_name.to_string(), persisted);
    client
        .set_active_host(host.host_device_id.clone())
        .await
        .context("set_active_host")?;
    client
        .resume_persisted_session()
        .await
        .context("resume_persisted_session")?;
    wait_for_connected(&client).await?;

    let mut ui_events = client.ui_events_stream();
    eprintln!("→ send_user_message prompt={prompt:?}");
    client
        .send_user_message(String::new(), prompt.to_string())
        .await
        .context("send_user_message")?;
    eprintln!("← send ok; tailing ui_events — Ctrl-C to exit");

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

fn pick_host(
    hosts: &[minos_protocol::HostSummary],
    preferred: Option<&str>,
) -> Result<minos_protocol::HostSummary> {
    if hosts.is_empty() {
        anyhow::bail!("no linked hosts — Link this Mac on Desktop first");
    }
    if let Some(id) = preferred {
        return hosts
            .iter()
            .find(|h| h.host_device_id.to_string() == id)
            .cloned()
            .with_context(|| format!("host_installation_id {id} not in GET /v1/hosts"));
    }
    Ok(hosts
        .iter()
        .find(|h| h.online)
        .or_else(|| hosts.first())
        .cloned()
        .expect("hosts non-empty"))
}

async fn wait_for_connected(client: &MobileClient) -> Result<(), MinosError> {
    for _ in 0..50 {
        if matches!(
            client.current_state(),
            minos_domain::ConnectionState::Connected
        ) {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }
    Err(MinosError::ConnectFailed {
        url: "ws".into(),
        message: format!(
            "timed out waiting for Connected; state={:?}",
            client.current_state()
        ),
    })
}
