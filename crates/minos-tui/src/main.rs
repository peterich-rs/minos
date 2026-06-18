use std::sync::Arc;

use crate::backend::BackendKind;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
};
use minos_daemon::local_rpc::LocalRpcConfig;
use minos_domain::AgentName;
use ratatui::DefaultTerminal;

mod action;
mod agent_route;
mod app;
mod backend;
mod effect;
mod event;
mod focus;
mod frame;
mod group_chat;
mod input;
mod logging;
mod render;
mod skills;
mod state;
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
    max_instances: Option<usize>,

    #[arg(long)]
    log_file: Option<std::path::PathBuf>,

    #[arg(long, value_enum, default_value = "embedded")]
    backend: BackendKind,

    #[arg(long)]
    daemon_url: Option<String>,

    #[arg(long)]
    mcp_disable_list_room_messages: bool,

    #[arg(long)]
    mcp_disable_delegate_to_agent: bool,

    #[arg(long)]
    mcp_disable_get_delegation_status: bool,

    #[arg(long)]
    mcp_disable_cancel_delegation: bool,

    #[arg(long)]
    mcp_disable_ask_user_question: bool,

    #[arg(long)]
    mcp_disable_check_user_feedback: bool,

    #[arg(long)]
    mcp_disable_post_room_update: bool,

    #[arg(long)]
    mcp_disable_react_to_message: bool,
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
    room_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_list_room_messages: bool,

    #[arg(long)]
    disable_delegate_to_agent: bool,

    #[arg(long)]
    disable_get_delegation_status: bool,

    #[arg(long)]
    disable_cancel_delegation: bool,

    #[arg(long)]
    disable_ask_user_question: bool,

    #[arg(long)]
    disable_check_user_feedback: bool,

    #[arg(long)]
    disable_post_room_update: bool,

    #[arg(long)]
    disable_react_to_message: bool,
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
            room_id: self.room_id,
            source_agent,
            permissions: minos_chat_store::mcp_server::McpToolPermissions {
                list_room_messages: !self.disable_list_room_messages,
                delegate_to_agent: !self.disable_delegate_to_agent,
                get_delegation_status: !self.disable_get_delegation_status,
                cancel_delegation: !self.disable_cancel_delegation,
                ask_user_question: !self.disable_ask_user_question,
                check_user_feedback: !self.disable_check_user_feedback,
                post_room_update: !self.disable_post_room_update,
                react_to_message: !self.disable_react_to_message,
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
        _ => anyhow::bail!("unknown agent: {s} (expected codex|claude|gemini|opencode)"),
    }
}

fn validate_backend_args(cli: &Cli) -> Result<()> {
    if matches!(cli.backend, BackendKind::Daemon) && cli.max_instances.is_some() {
        anyhow::bail!("--max-instances only applies to --backend embedded");
    }
    if matches!(cli.backend, BackendKind::Embedded) && cli.daemon_url.is_some() {
        anyhow::bail!("--daemon-url only applies to --backend daemon");
    }
    if matches!(cli.backend, BackendKind::Daemon) && has_mcp_policy_overrides(cli) {
        anyhow::bail!("--mcp-* policy flags only apply to --backend embedded");
    }
    Ok(())
}

fn has_mcp_policy_overrides(cli: &Cli) -> bool {
    cli.mcp_disable_list_room_messages
        || cli.mcp_disable_delegate_to_agent
        || cli.mcp_disable_get_delegation_status
        || cli.mcp_disable_cancel_delegation
        || cli.mcp_disable_ask_user_question
        || cli.mcp_disable_check_user_feedback
        || cli.mcp_disable_post_room_update
        || cli.mcp_disable_react_to_message
}

fn mcp_permissions_from_cli(cli: &Cli) -> minos_chat_store::mcp_server::McpToolPermissions {
    minos_chat_store::mcp_server::McpToolPermissions {
        list_room_messages: !cli.mcp_disable_list_room_messages,
        delegate_to_agent: !cli.mcp_disable_delegate_to_agent,
        get_delegation_status: !cli.mcp_disable_get_delegation_status,
        cancel_delegation: !cli.mcp_disable_cancel_delegation,
        ask_user_question: !cli.mcp_disable_ask_user_question,
        check_user_feedback: !cli.mcp_disable_check_user_feedback,
        post_room_update: !cli.mcp_disable_post_room_update,
        react_to_message: !cli.mcp_disable_react_to_message,
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
    let group_chat_db_path = minos_home.join("daemon.sqlite");
    let local_rpc_config = LocalRpcConfig {
        addr: "127.0.0.1:0".parse()?,
        discovery_path: discovery_path.clone(),
        group_chat_db_path,
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
    let url = resolve_daemon_url(None)?;
    let backend = crate::backend::DaemonBackend::connect(&url).await?;
    Ok((Arc::new(backend), Some(handle)))
}

fn setup_terminal() -> Result<DefaultTerminal> {
    let mut terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut DefaultTerminal) -> Result<()> {
    execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    ratatui::try_restore()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    if let Some(command) = cli.command.take() {
        match command {
            SidecarCommand::MinosTeamworkMcp(args) => return args.serve().await,
        }
    }
    validate_backend_args(&cli)?;
    let workspace = std::fs::canonicalize(&cli.workspace).unwrap_or_else(|_| cli.workspace.clone());
    let log_path = logging::resolve_log_path(&workspace, cli.log_file.clone());
    logging::init(&log_path)?;
    let mcp_permissions = mcp_permissions_from_cli(&cli);
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

    let max_instances = cli.max_instances.unwrap_or(3);
    let (backend, managed_daemon): (
        Arc<dyn crate::backend::AgentBackend>,
        Option<Arc<minos_daemon::DaemonHandle>>,
    ) = match cli.backend {
        BackendKind::Embedded => (
            Arc::new(
                crate::backend::EmbeddedBackend::new(
                    workspace.clone(),
                    max_instances,
                    std::time::Duration::from_secs(300),
                    mcp_permissions,
                )
                .await?,
            ),
            None,
        ),
        BackendKind::Daemon => connect_or_start_daemon_backend(cli.daemon_url.clone()).await?,
    };

    let mut app = app::App::new(backend.clone(), cli.readonly, workspace.clone());
    app.init().await?;

    let mut terminal = setup_terminal()?;
    let (frame_requester, mut frame_rx) = frame::frame_channel();
    app.set_frame_requester(frame_requester);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx.clone());
    backend.start_mcp_socket_handler(tx.clone())?;

    let ingest_rx = backend.subscribe_ingest().await;
    event::spawn_ingest_pump(ingest_rx, tx.clone());

    let manager_rx = backend.subscribe_manager_events().await;
    event::spawn_manager_event_pump(manager_rx, tx.clone());

    event::spawn_terminal_pump(tx.clone());
    event::spawn_tick_pump(tx, 200);

    if let Some(agent_str) = &cli.agent {
        let agent = parse_agent_name(agent_str)?;
        backend.start_agent(agent, workspace.clone()).await?;
    }

    terminal.draw(|f| {
        ui::render_ui(f, app.ui());
    })?;
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                let _ = app.handle_event(event).await;
            }
            frame = frame_rx.recv() => {
                if frame.is_none() {
                    break;
                }
                terminal.draw(|f| {
                    ui::render_ui(f, app.ui());
                })?;
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

    fn test_cli(backend: BackendKind) -> Cli {
        Cli {
            command: None,
            agent: None,
            workspace: ".".into(),
            readonly: false,
            max_instances: None,
            log_file: None,
            backend,
            daemon_url: None,
            mcp_disable_list_room_messages: false,
            mcp_disable_delegate_to_agent: false,
            mcp_disable_get_delegation_status: false,
            mcp_disable_cancel_delegation: false,
            mcp_disable_ask_user_question: false,
            mcp_disable_check_user_feedback: false,
            mcp_disable_post_room_update: false,
            mcp_disable_react_to_message: false,
        }
    }

    #[test]
    fn validate_backend_args_rejects_incompatible_flags() {
        let mut daemon_cli = test_cli(BackendKind::Daemon);
        daemon_cli.max_instances = Some(3);
        assert!(validate_backend_args(&daemon_cli).is_err());

        let mut embedded_cli = test_cli(BackendKind::Embedded);
        embedded_cli.daemon_url = Some("ws://127.0.0.1:43123".into());
        assert!(validate_backend_args(&embedded_cli).is_err());

        let mut daemon_cli = test_cli(BackendKind::Daemon);
        daemon_cli.mcp_disable_post_room_update = true;
        assert!(validate_backend_args(&daemon_cli).is_err());
    }

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
