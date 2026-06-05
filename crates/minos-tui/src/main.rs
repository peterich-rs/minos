use std::sync::Arc;

use crate::backend::BackendKind;
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use minos_domain::AgentName;
use ratatui::DefaultTerminal;

mod app;
mod backend;
mod event;
mod logging;
mod translation;
mod ui;

#[derive(Parser, Debug)]
#[command(name = "minos-tui", about = "Minos Agent TUI - local debug console")]
struct Cli {
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
    Ok(())
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
    validate_backend_args(&cli)?;
    let log_path = logging::resolve_log_path(&cli.workspace, cli.log_file.clone());
    logging::init(&log_path)?;

    let max_instances = cli.max_instances.unwrap_or(3);
    let backend: Arc<dyn crate::backend::AgentBackend> = match cli.backend {
        BackendKind::Embedded => Arc::new(
            crate::backend::EmbeddedBackend::new(
                cli.workspace.clone(),
                max_instances,
                std::time::Duration::from_secs(300),
            )
            .await?,
        ),
        BackendKind::Daemon => Arc::new(
            crate::backend::DaemonBackend::connect(&resolve_daemon_url(cli.daemon_url)?).await?,
        ),
    };

    let mut app = app::App::new(backend.clone(), cli.readonly, cli.workspace.clone());
    app.init().await?;

    let mut terminal = setup_terminal()?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let ingest_rx = backend.subscribe_ingest().await;
    event::spawn_ingest_pump(ingest_rx, tx.clone());

    let manager_rx = backend.subscribe_manager_events().await;
    event::spawn_manager_event_pump(manager_rx, tx.clone());

    event::spawn_terminal_pump(tx.clone());
    event::spawn_tick_pump(tx, 200);

    if let Some(agent_str) = &cli.agent {
        let agent = parse_agent_name(agent_str)?;
        let workspace = cli.workspace.clone();
        backend.start_agent(agent, workspace).await?;
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
            agent: None,
            workspace: ".".into(),
            readonly: false,
            max_instances: None,
            log_file: None,
            backend,
            daemon_url: None,
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
