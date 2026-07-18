use std::sync::Arc;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute, SynchronizedUpdate,
};
use minos_daemon::local_rpc::LocalRpcConfig;
use minos_domain::AgentName;
use ratatui::DefaultTerminal;
use std::io::stdout;

mod action;
mod agent_route;
mod app;
mod backend;
mod effect;
mod event;
mod focus;
mod frame;
mod input;
mod logging;
mod nav;
mod path_complete;
mod render;
mod skills;
mod state;
mod teamwork;
mod translation;
mod ui;
mod update;

#[derive(Parser, Debug)]
#[command(name = "minos-tui", about = "Minos Agent TUI - local debug console")]
struct Cli {
    #[command(subcommand)]
    command: Option<SidecarCommand>,

    #[arg(short, long)]
    agent: Option<String>,

    #[arg(short, long, default_value = ".")]
    workspace: std::path::PathBuf,

    #[arg(long)]
    readonly: bool,

    #[arg(long)]
    log_file: Option<std::path::PathBuf>,

    /// Explicit daemon local RPC URL. When omitted, discovery is used and a
    /// managed daemon is started if needed.
    #[arg(long)]
    daemon_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum SidecarCommand {
    #[command(name = "__minos-teamwork-mcp", hide = true)]
    MinosTeamworkMcp(McpSidecarArgs),
}

#[derive(Args, Debug)]
struct McpSidecarArgs {
    #[arg(long)]
    socket_path: std::path::PathBuf,

    #[arg(long)]
    conversation_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    source_thread_id: Option<String>,

    #[arg(long)]
    disable_list_conversation_messages: bool,

    #[arg(long)]
    disable_delegate_to_agent: bool,

    #[arg(long)]
    disable_get_delegation_status: bool,

    #[arg(long)]
    disable_wait_delegation: bool,

    #[arg(long)]
    disable_cancel_delegation: bool,

    #[arg(long)]
    disable_post_conversation_update: bool,
}

impl McpSidecarArgs {
    async fn serve(self) -> Result<()> {
        let source_agent = self
            .source_agent
            .as_deref()
            .map(parse_agent_name)
            .transpose()?;
        minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
            socket_path: self.socket_path,
            conversation_id: self.conversation_id,
            source_agent,
            source_thread_id: self.source_thread_id,
            permissions: minos_chat_store::mcp_server::McpToolPermissions {
                list_conversation_messages: !self.disable_list_conversation_messages,
                delegate_to_agent: !self.disable_delegate_to_agent,
                get_delegation_status: !self.disable_get_delegation_status,
                wait_delegation: !self.disable_wait_delegation,
                cancel_delegation: !self.disable_cancel_delegation,
                post_conversation_update: !self.disable_post_conversation_update,
            },
        })
        .await
    }
}

fn parse_agent_name(s: &str) -> Result<AgentName> {
    match s.to_lowercase().as_str() {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        "opencode" => Ok(AgentName::Opencode),
        "grok" => Ok(AgentName::Grok),
        _ => anyhow::bail!("unknown agent: {s} (expected codex|claude|gemini|opencode|grok)"),
    }
}

fn resolve_minos_home() -> Result<std::path::PathBuf> {
    if let Ok(path) = std::env::var("MINOS_HOME") {
        return Ok(path.into());
    }
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(std::path::PathBuf::from(home).join(".minos"));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(std::path::PathBuf::from(user_profile).join(".minos"));
    }
    let home_drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty());
    let home_path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty());
    if let (Some(drive), Some(path)) = (home_drive, home_path) {
        return Ok(std::path::PathBuf::from(drive).join(path).join(".minos"));
    }
    anyhow::bail!("unable to resolve MINOS_HOME from environment")
}

fn resolve_daemon_discovery_path() -> Result<std::path::PathBuf> {
    Ok(resolve_minos_home()?
        .join("run")
        .join("tui-daemon-rpc.json"))
}

fn resolve_daemon_url(override_url: Option<String>) -> Result<String> {
    if let Some(url) = override_url {
        return Ok(url);
    }
    let discovery_path = resolve_daemon_discovery_path()?;
    let content = std::fs::read_to_string(&discovery_path).map_err(|error| {
        anyhow::anyhow!(
            "failed to read daemon discovery file at {}: {error}. start `minos-daemon start --local-rpc` or pass --daemon-url",
            discovery_path.display()
        )
    })?;
    let payload: serde_json::Value = serde_json::from_str(&content).map_err(|error| {
        anyhow::anyhow!(
            "failed to parse daemon discovery file at {}: {error}",
            discovery_path.display()
        )
    })?;
    payload["url"].as_str().map(str::to_owned).ok_or_else(|| {
        anyhow::anyhow!(
            "daemon discovery file at {} does not contain a `url` field",
            discovery_path.display()
        )
    })
}

fn relay_config_from_env() -> minos_daemon::RelayConfig {
    let backend_url = std::env::var("MINOS_BACKEND_URL")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_default();
    minos_daemon::RelayConfig::new(backend_url)
}

fn default_mac_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Minos Host".into())
}

async fn start_managed_daemon_for_tui() -> Result<Arc<minos_daemon::DaemonHandle>> {
    let minos_home = minos_daemon::paths::minos_home()?;
    let local_state_path = minos_home.join("local-state.json");
    let local_state = minos_daemon::LocalState::load_or_init(&local_state_path)?;
    let discovery_path = minos_daemon::paths::run_dir()?.join("tui-daemon-rpc.json");
    let local_rpc_config = LocalRpcConfig {
        addr: "127.0.0.1:0".parse()?,
        discovery_path: discovery_path.clone(),
    };
    let handle = minos_daemon::DaemonHandle::start_with_local_rpc(
        relay_config_from_env(),
        local_state.self_device_id,
        None,
        None,
        default_mac_name(),
        Some(local_rpc_config),
    )
    .await?;
    tracing::info!(
        target: "minos_tui",
        discovery_path = %discovery_path.display(),
        "started managed daemon for TUI"
    );
    Ok(handle)
}

async fn connect_or_start_daemon_backend(
    override_url: Option<String>,
) -> Result<(
    Arc<dyn crate::backend::AgentBackend>,
    Option<Arc<minos_daemon::DaemonHandle>>,
)> {
    let explicit_url = override_url.is_some();
    match resolve_daemon_url(override_url.clone()) {
        Ok(url) => match crate::backend::DaemonBackend::connect(&url).await {
            Ok(backend) => return Ok((Arc::new(backend), None)),
            Err(error) if explicit_url => return Err(error),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui",
                    error = %error,
                    "failed to connect to discovered daemon; starting managed daemon"
                );
            }
        },
        Err(error) if explicit_url => return Err(error),
        Err(error) => {
            tracing::warn!(
                target: "minos_tui",
                error = %error,
                "daemon discovery unavailable; starting managed daemon"
            );
        }
    }

    let handle = start_managed_daemon_for_tui().await?;
    let url = handle
        .local_rpc_url()
        .or_else(|| resolve_daemon_url(None).ok())
        .ok_or_else(|| anyhow::anyhow!("managed daemon has no local RPC URL"))?;
    let backend = crate::backend::DaemonBackend::connect(&url).await?;
    Ok((Arc::new(backend), Some(handle)))
}

fn setup_terminal() -> Result<DefaultTerminal> {
    let mut terminal = ratatui::try_init()?;
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut DefaultTerminal) -> Result<()> {
    execute!(stdout(), DisableBracketedPaste, DisableMouseCapture)?;
    terminal.show_cursor()?;
    ratatui::try_restore()?;
    Ok(())
}

/// Draw the full UI inside a terminal synchronized-update block to reduce flicker.
fn draw_ui(terminal: &mut DefaultTerminal, app: &mut app::App) -> Result<()> {
    stdout().sync_update(|_stdout| {
        terminal.draw(|f| {
            ui::render_ui(f, app.ui());
        })
    })??;
    Ok(())
}

/// Merge consecutive wheel events of the same direction/position so a trackpad
/// burst becomes one larger scroll step. Tick count is stored in
/// `MouseEvent.modifiers.bits()` for `wheel_lines` in the event loop.
fn coalesce_scroll_batch(events: Vec<event::AppEvent>) -> Vec<event::AppEvent> {
    use crossterm::event::MouseEventKind;
    use event::AppEvent;

    let mut out: Vec<AppEvent> = Vec::with_capacity(events.len());
    for event in events {
        let AppEvent::Mouse(mouse) = &event else {
            out.push(event);
            continue;
        };
        let is_wheel = matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        );
        if !is_wheel {
            out.push(event);
            continue;
        }

        if let Some(AppEvent::Mouse(prev)) = out.last_mut() {
            if prev.kind == mouse.kind && prev.column == mouse.column && prev.row == mouse.row {
                let count = prev.modifiers.bits().saturating_add(1).min(40);
                prev.modifiers = crossterm::event::KeyModifiers::from_bits_truncate(count);
                continue;
            }
        }
        let mut first = *mouse;
        first.modifiers = crossterm::event::KeyModifiers::from_bits_truncate(1);
        out.push(AppEvent::Mouse(first));
    }
    out
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    if let Some(command) = cli.command.take() {
        match command {
            SidecarCommand::MinosTeamworkMcp(args) => return args.serve().await,
        }
    }
    let workspace = std::fs::canonicalize(&cli.workspace).unwrap_or_else(|_| cli.workspace.clone());
    let log_path = logging::resolve_log_path(&workspace, cli.log_file.clone());
    logging::init(&log_path)?;
    let mcp_permissions = minos_chat_store::mcp_server::McpToolPermissions::default();
    let mcp_skill_refs = minos_chat_store::teamwork_mcp::TeamworkMcpToolCatalog::default_catalog()
        .skill_refs(mcp_permissions);
    match skills::install_global_agent_skills(&mcp_skill_refs) {
        Ok(report) => {
            tracing::info!(
                count = report.installed_paths.len(),
                "installed Minos teamwork skills"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to install Minos teamwork skills; agents may need manual skill setup"
            );
        }
    }

    let (backend, managed_daemon) = connect_or_start_daemon_backend(cli.daemon_url.clone()).await?;

    let mut app = app::App::new(backend.clone(), cli.readonly, workspace.clone());
    app.init().await?;

    let mut terminal = setup_terminal()?;
    let (frame_requester, mut frame_rx) = frame::frame_channel();
    app.set_frame_requester(frame_requester);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx.clone());

    let ingest_rx = backend.subscribe_ingest().await;
    event::spawn_ingest_pump(ingest_rx, tx.clone());

    let manager_rx = backend.subscribe_manager_events().await;
    event::spawn_manager_event_pump(manager_rx, tx.clone());

    let conversation_message_rx = backend.subscribe_conversation_message_events().await;
    event::spawn_conversation_message_event_pump(conversation_message_rx, tx.clone());

    event::spawn_terminal_pump(tx.clone());
    event::spawn_tick_pump(tx, 200);

    if let Some(agent_str) = &cli.agent {
        let agent = parse_agent_name(agent_str)?;
        backend.start_agent(agent, workspace.clone()).await?;
    }

    draw_ui(&mut terminal, &mut app)?;
    loop {
        // Prefer draws over event floods so wheel/key scroll stays smooth when
        // the terminal pump dumps many events between frames.
        tokio::select! {
            biased;
            frame = frame_rx.recv() => {
                if frame.is_none() {
                    break;
                }
                // Coalesce: one paint for any queued frame tokens.
                while frame_rx.try_recv().is_some() {}
                draw_ui(&mut terminal, &mut app)?;
                // Viewport overscan materialization may need another pass.
                if app.ui().take_render_followup() {
                    app.request_frame_public();
                }
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                // Drain a burst of ready events so a trackpad wheel dump is
                // applied as one scroll jump before the next paint.
                let mut batch = vec![event];
                while batch.len() < 64 {
                    match rx.try_recv() {
                        Ok(next) => batch.push(next),
                        Err(_) => break,
                    }
                }
                let batch = coalesce_scroll_batch(batch);
                for event in batch {
                    let _ = app.handle_event(event).await;
                }
            }
        }

        if app.should_quit() {
            break;
        }
    }

    app.shutdown().await;
    restore_terminal(&mut terminal)?;
    if let Some(handle) = managed_daemon {
        handle.stop().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_daemon_url_reads_discovery_file_from_minos_home() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!("minos-tui-main-{unique}"));
        let discovery_dir = temp_root.join("run");
        std::fs::create_dir_all(&discovery_dir).expect("create discovery dir");
        let discovery_path = discovery_dir.join("tui-daemon-rpc.json");
        std::fs::write(&discovery_path, r#"{"url":"ws://127.0.0.1:43123"}"#)
            .expect("write discovery file");
        std::env::set_var("MINOS_HOME", &temp_root);

        let url = resolve_daemon_url(None).expect("resolve daemon url");

        assert_eq!(url, "ws://127.0.0.1:43123");

        std::env::remove_var("MINOS_HOME");
        std::fs::remove_dir_all(&temp_root).expect("remove temp root");
    }
}
