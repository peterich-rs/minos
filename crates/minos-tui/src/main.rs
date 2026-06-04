use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use crate::backend::AgentBackend;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use minos_domain::AgentName;
use ratatui::{backend::CrosstermBackend, Terminal};

mod app;
mod backend;
mod event;
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

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let max_instances = cli.max_instances.unwrap_or(3);
    let backend = Arc::new(
        backend::EmbeddedBackend::new(
            cli.workspace.clone(),
            max_instances,
            std::time::Duration::from_secs(300),
        )
        .await?,
    );

    let mut app = app::App::new(backend.clone(), cli.readonly);
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

    loop {
        terminal.draw(|f| {
            ui::render_ui(f, app.ui());
        })?;

        if let Some(event) = rx.recv().await {
            app.handle_event(event).await;
        }

        if app.should_quit() {
            break;
        }
    }

    app.shutdown().await;
    restore_terminal(&mut terminal)?;

    Ok(())
}
