use std::sync::Arc;

use crate::backend::BackendKind;
use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use minos_domain::AgentName;
use ratatui::DefaultTerminal;

mod app;
mod backend;
mod event;
mod group_chat;
mod logging;
mod skills;
mod translation;
mod ui;

#[derive(Parser, Debug)]
#[command(name = "minos-tui", about = "Minos Agent TUI - local debug console")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

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
    mcp_disable_read_chat: bool,

    #[arg(long)]
    mcp_disable_mention_agent: bool,

    #[arg(long)]
    mcp_disable_mention_user: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(name = "minos-mcp", hide = true)]
    MinosMcp(MinosMcpArgs),
}

#[derive(Parser, Debug)]
struct MinosMcpArgs {
    #[arg(long)]
    db_path: Option<std::path::PathBuf>,

    #[arg(long)]
    socket_path: Option<std::path::PathBuf>,

    #[arg(long)]
    room_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_read_chat: bool,

    #[arg(long)]
    disable_mention_agent: bool,

    #[arg(long)]
    disable_mention_user: bool,
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
    cli.mcp_disable_read_chat
        || cli.mcp_disable_mention_agent
        || cli.mcp_disable_mention_user
}

fn mcp_permissions_from_cli(cli: &Cli) -> minos_chat_store::mcp_server::McpToolPermissions {
    minos_chat_store::mcp_server::McpToolPermissions {
        read_chat: !cli.mcp_disable_read_chat,
        mention_agent: !cli.mcp_disable_mention_agent,
        mention_user: !cli.mcp_disable_mention_user,
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

fn setup_terminal() -> Result<DefaultTerminal> {
    let mut terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableMouseCapture)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut DefaultTerminal) -> Result<()> {
    execute!(std::io::stdout(), DisableMouseCapture)?;
    terminal.show_cursor()?;
    ratatui::try_restore()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Command::MinosMcp(args)) = cli.command {
        let source_agent = args
            .source_agent
            .as_deref()
            .map(parse_agent_name)
            .transpose()?;
        let db_path = args
            .db_path
            .unwrap_or_else(|| minos_chat_store::default_db_path().expect("default db path"));
        let minos_home = db_path.parent().expect("db_path parent").to_path_buf();
        let socket_path = args
            .socket_path
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| {
                let id = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                minos_home.join("run").join(format!("mcp-{id}.sock"))
            });
        return minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
            socket_path,
            db_path,
            room_id: args.room_id,
            source_agent,
            permissions: minos_chat_store::mcp_server::McpToolPermissions {
                read_chat: !args.disable_read_chat,
                mention_agent: !args.disable_mention_agent,
                mention_user: !args.disable_mention_user,
            },
        })
        .await;
    }

    validate_backend_args(&cli)?;
    let workspace = std::fs::canonicalize(&cli.workspace).unwrap_or_else(|_| cli.workspace.clone());
    let log_path = logging::resolve_log_path(&workspace, cli.log_file.clone());
    logging::init(&log_path)?;
    match skills::install_global_agent_skills() {
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
    let mcp_permissions = mcp_permissions_from_cli(&cli);
    let backend: Arc<dyn crate::backend::AgentBackend> = match cli.backend {
        BackendKind::Embedded => Arc::new(
            crate::backend::EmbeddedBackend::new(
                workspace.clone(),
                max_instances,
                std::time::Duration::from_secs(300),
                mcp_permissions,
            )
            .await?,
        ),
        BackendKind::Daemon => Arc::new(
            crate::backend::DaemonBackend::connect(&resolve_daemon_url(cli.daemon_url)?).await?,
        ),
    };

    let mut app = app::App::new(backend.clone(), cli.readonly, workspace.clone());
    app.init().await?;

    let mut terminal = setup_terminal()?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx.clone());

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
        if let Some(event) = rx.recv().await {
            if app.handle_event(event).await {
                terminal.draw(|f| {
                    ui::render_ui(f, app.ui());
                })?;
            }
        } else {
            break;
        }

        if app.should_quit() {
            break;
        }
    }

    app.shutdown().await;
    restore_terminal(&mut terminal)?;

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
            mcp_disable_read_chat: false,
            mcp_disable_mention_agent: false,
            mcp_disable_mention_user: false,
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
        daemon_cli.mcp_disable_mention_user = true;
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
