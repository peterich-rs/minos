use std::sync::Arc;

use crate::backend::AgentBackend;
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
    let log_path = logging::resolve_log_path(&cli.workspace, cli.log_file.clone());
    logging::init(&log_path)?;

    let max_instances = cli.max_instances.unwrap_or(3);
    let backend = Arc::new(
        backend::EmbeddedBackend::new(
            cli.workspace.clone(),
            max_instances,
            std::time::Duration::from_secs(300),
        )
        .await?,
    );

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
