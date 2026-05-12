use std::env;
use std::ffi::OsString;
use std::io::BufRead as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_daemon::{paths, AgentGlue, DaemonHandle, LocalState, RelayConfig};
use minos_domain::{AgentDescriptor, AgentName, AgentStatus, MinosError};
use minos_ui_protocol::{translate_codex, CodexTranslatorState, MessageRole, UiEventMessage};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "minos-daemon",
    about = "CLI entrypoint for the Minos Rust daemon"
)]
struct Cli {
    #[command(flatten)]
    paths: CliPaths,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print resolved runtime paths and the compile-time relay backend URL.
    Doctor,
    /// Detect locally installed CLI agents on this host.
    ListClis(OutputModeArgs),
    /// Inspect host skills for one workspace.
    HostSkills(HostSkillsArgs),
    /// Write one host skill enable/disable override.
    SetHostSkill(SetHostSkillArgs),
    /// Connect to the relay once and print the current daemon status.
    Status(ConnectArgs),
    /// Connect once and print a fresh pairing QR payload as JSON.
    PairingQr(ConnectArgs),
    /// Connect once and list paired mobile/account rows for this host.
    Peers(OutputArgs),
    /// Forget one paired mobile/account row, or the first row when omitted.
    ForgetPeer(ForgetPeerArgs),
    /// Read persisted thread summaries from the local daemon store.
    Threads(ThreadsArgs),
    /// Read one persisted thread summary + live state from the local store.
    Thread(ThreadArgs),
    /// Read translated transcript history for one local thread.
    History(HistoryArgs),
    /// Run one local prompt against the host agent runtime and stream output.
    Run(RunArgs),
    /// Start a persistent local chat session against the host agent runtime.
    Chat(ChatArgs),
    /// Start the daemon (dials the relay) and keep it running until Ctrl-C.
    Start(StartArgs),
}

#[derive(Args, Debug)]
struct StartArgs {
    /// Human-readable Mac name shown to the peer during pairing.
    #[arg(long)]
    mac_name: Option<String>,
    /// Print a fresh pairing QR payload as JSON after startup.
    #[arg(long)]
    print_qr: bool,
}

#[derive(Args, Debug, Clone)]
struct OutputModeArgs {
    /// Print JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct ConnectArgs {
    /// Human-readable Mac name shown to the peer during pairing.
    #[arg(long)]
    mac_name: Option<String>,
    /// Maximum seconds to wait for the relay link to become connected.
    #[arg(long, default_value_t = 15)]
    timeout_s: u64,
    /// Print JSON instead of human-readable text where applicable.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct OutputArgs {
    #[command(flatten)]
    connect: ConnectArgs,
}

#[derive(Args, Debug, Clone)]
struct ForgetPeerArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    /// Forget one specific paired mobile device id. When omitted, forget the
    /// first currently paired row.
    #[arg(long)]
    device_id: Option<String>,
}

#[derive(Args, Debug, Clone)]
struct ThreadsArgs {
    /// Maximum number of thread summaries to return.
    #[arg(long, default_value_t = 50)]
    limit: u32,
    /// Pagination cursor for older entries.
    #[arg(long)]
    before_ts_ms: Option<i64>,
    /// Optional agent filter: codex, claude, or gemini.
    #[arg(long)]
    agent: Option<String>,
    /// Print JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct ThreadArgs {
    /// Thread id to inspect.
    thread_id: String,
    /// Print JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct HistoryArgs {
    /// Thread id to inspect.
    thread_id: String,
    /// Maximum number of recent messages to print in text mode.
    #[arg(long, default_value_t = 20)]
    messages: usize,
    /// Print JSON instead of formatted transcript text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct HostSkillsArgs {
    /// Workspace directory to inspect. Defaults to the daemon workspace root.
    #[arg(long)]
    workspace: Option<String>,
    /// Force a reload instead of using any cached skill inventory.
    #[arg(long)]
    force_reload: bool,
    /// Print JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum SkillToggleArg {
    Enable,
    Disable,
}

impl SkillToggleArg {
    const fn enabled(self) -> bool {
        matches!(self, Self::Enable)
    }
}

#[derive(Args, Debug, Clone)]
struct SetHostSkillArgs {
    /// Workspace directory that owns the skill path.
    #[arg(long)]
    workspace: Option<String>,
    /// Path from a prior `host-skills` listing.
    path: String,
    /// Whether to enable or disable the skill.
    state: SkillToggleArg,
    /// Print JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct RunArgs {
    /// Workspace directory to run against. Defaults to the current directory.
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// Resume an existing persisted thread instead of creating a new one.
    #[arg(long)]
    thread: Option<String>,
    /// Agent runtime to use. Local run currently supports codex only.
    #[arg(long, default_value = "codex")]
    agent: String,
    /// Print a JSON summary instead of streaming plain text.
    #[arg(long)]
    json: bool,
    /// Maximum seconds to wait for the turn to complete.
    #[arg(long, default_value_t = 60)]
    timeout_s: u64,
    /// User prompt to send.
    prompt: String,
}

#[derive(Args, Debug, Clone)]
struct ChatArgs {
    /// Workspace directory to run against. Defaults to the current directory.
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// Resume an existing persisted thread instead of creating a new one.
    #[arg(long)]
    thread: Option<String>,
    /// Agent runtime to use. Local chat currently supports codex only.
    #[arg(long, default_value = "codex")]
    agent: String,
    /// Maximum seconds to wait for each turn to complete.
    #[arg(long, default_value_t = 60)]
    timeout_s: u64,
    /// How many recent messages to replay when attaching to an existing thread.
    #[arg(long, default_value_t = 8)]
    history_messages: usize,
    /// Optional first prompt to send before entering the REPL.
    #[arg(long)]
    prompt: Option<String>,
}

#[derive(Args, Debug)]
struct CliPaths {
    /// Root directory used by the CLI for daemon state and logs.
    #[arg(long)]
    minos_home: Option<PathBuf>,
    /// Override the daemon state directory. Writes `local-state.json` here.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Override the daemon log directory.
    #[arg(long)]
    log_dir: Option<PathBuf>,
    /// Keep the library's platform-native defaults instead of forcing `~/.minos`.
    #[arg(long)]
    platform_paths: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let resolved_paths = resolve_paths(&cli.paths)?;

    match cli.command {
        Command::Doctor => doctor(&resolved_paths).await,
        Command::ListClis(args) => list_clis(args).await,
        Command::HostSkills(args) => host_skills(args, &resolved_paths).await,
        Command::SetHostSkill(args) => set_host_skill(args, &resolved_paths).await,
        Command::Status(args) => status(args, &resolved_paths).await,
        Command::PairingQr(args) => pairing_qr(args, &resolved_paths).await,
        Command::Peers(args) => peers(args, &resolved_paths).await,
        Command::ForgetPeer(args) => forget_peer(args, &resolved_paths).await,
        Command::Threads(args) => threads(args, &resolved_paths).await,
        Command::Thread(args) => thread(args, &resolved_paths).await,
        Command::History(args) => history(args, &resolved_paths).await,
        Command::Run(args) => run_prompt(args, &resolved_paths).await,
        Command::Chat(args) => chat_session(args, &resolved_paths).await,
        Command::Start(args) => {
            let home_guard = maybe_apply_minos_home_override(&resolved_paths);
            minos_daemon::logging::init()?;
            let home = paths::minos_home()?;
            tracing::info!(minos_home = %home.display(), "daemon starting");
            let result = start(args, &resolved_paths).await;
            drop(home_guard);
            result
        }
    }
}

#[allow(clippy::unused_async)]
async fn doctor(paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let relay_config = relay_config_from_env()?;
    let local_state_path = local_state_path(paths);
    let local_state = LocalState::load_or_init(&local_state_path)?;
    println!(
        "minos home: {}",
        display_optional(paths.minos_home.as_deref())
    );
    println!("data dir:   {}", display_path(&paths.data_dir));
    println!("log dir:    {}", display_path(&paths.log_dir));
    println!("state file: {}", display_path(&local_state_path));
    println!("device id:  {}", local_state.self_device_id);
    println!("relay:      {}", relay_config.resolved_backend_url());

    Ok(())
}

async fn list_clis(args: OutputModeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let subprocess_env = Arc::new(capture_user_shell_env().await);
    let runner = Arc::new(RealCommandRunner::new(subprocess_env));
    let descriptors = detect_all(runner).await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&descriptors)?);
    } else {
        print_agent_descriptors(&descriptors);
    }
    Ok(())
}

async fn host_skills(
    args: HostSkillsArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let local = start_local_agent(paths).await?;
    let result = async {
        let response = local
            .agent
            .list_host_skills(minos_protocol::ListHostSkillsRequest {
                workspace: args.workspace.unwrap_or_default(),
                force_reload: args.force_reload,
            })
            .await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            print_host_skills(&response);
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let shutdown = shutdown_local_agent(local).await;
    match (result, shutdown) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn set_host_skill(
    args: SetHostSkillArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let local = start_local_agent(paths).await?;
    let result = async {
        let response = local
            .agent
            .write_host_skill_config(minos_protocol::WriteHostSkillConfigRequest {
                workspace: args.workspace.unwrap_or_default(),
                path: args.path.clone(),
                enabled: args.state.enabled(),
            })
            .await?;
        let result = SetHostSkillResult {
            path: args.path,
            effective_enabled: response.effective_enabled,
        };
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "skill {}: {}",
                result.path,
                if result.effective_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let shutdown = shutdown_local_agent(local).await;
    match (result, shutdown) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn run_prompt(
    args: RunArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let local = start_local_agent(paths).await?;
    let result = run_prompt_inner(&args, &local).await;
    let shutdown = shutdown_local_agent(local).await;
    match (result, shutdown) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn run_prompt_inner(
    args: &RunArgs,
    local: &LocalAgentContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent = parse_agent_name(&args.agent)?;
    if agent != AgentName::Codex {
        return Err(Box::new(std::io::Error::other(format!(
            "local run currently supports codex only; got {:?}",
            agent
        ))));
    }
    let workspace = resolve_workspace_arg(args.workspace.as_ref())?;
    let session = match args.thread.as_deref() {
        Some(thread_id) => resume_local_cli_session(local, thread_id).await?,
        None => start_local_cli_session(local, agent, &workspace).await?,
    };

    if !args.json {
        eprintln!("thread: {}", session.session_id);
        eprintln!("workspace: {}", session.cwd);
    }

    let mut translator = if args.thread.is_some() {
        local
            .agent
            .hydrate_codex_translator(&session.session_id)
            .await?
    } else {
        CodexTranslatorState::new(session.session_id.clone())
    };
    let mut renderer = execute_local_turn(
        local,
        &session.session_id,
        &args.prompt,
        args.timeout_s,
        args.json,
        &mut translator,
    )
    .await?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&RunSummary {
                session_id: session.session_id,
                cwd: session.cwd,
                assistant_text: renderer.assistant_text,
                errors: renderer.errors,
            })?
        );
    } else {
        renderer.finish_stdout_line()?;
    }

    Ok(())
}

async fn chat_session(
    args: ChatArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let local = start_local_agent(paths).await?;
    let result = chat_session_inner(&args, &local).await;
    let shutdown = shutdown_local_agent(local).await;
    match (result, shutdown) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn chat_session_inner(
    args: &ChatArgs,
    local: &LocalAgentContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent = parse_agent_name(&args.agent)?;
    if agent != AgentName::Codex {
        return Err(Box::new(std::io::Error::other(format!(
            "local chat currently supports codex only; got {:?}",
            agent
        ))));
    }
    let workspace = resolve_workspace_arg(args.workspace.as_ref())?;
    let mut session = match args.thread.as_deref() {
        Some(thread_id) => resume_local_cli_session(local, thread_id).await?,
        None => start_local_cli_session(local, agent, &workspace).await?,
    };
    let mut translator = if args.thread.is_some() {
        local
            .agent
            .hydrate_codex_translator(&session.session_id)
            .await?
    } else {
        CodexTranslatorState::new(session.session_id.clone())
    };

    eprintln!("thread: {}", session.session_id);
    eprintln!("workspace: {}", session.cwd);
    eprintln!("commands: /threads  /history  /resume <id>  /status  /interrupt  /exit");

    if args.thread.is_some() {
        show_thread_history(local, &session.session_id, args.history_messages).await?;
    }

    if let Some(prompt) = args.prompt.as_deref() {
        execute_local_turn(
            local,
            &session.session_id,
            prompt,
            args.timeout_s,
            false,
            &mut translator,
        )
        .await?;
    }

    loop {
        let Some(line) = read_repl_line("minos> ")? else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            "/exit" | "/quit" => {
                let _ = local
                    .agent
                    .close_thread(minos_protocol::CloseThreadRequest {
                        thread_id: session.session_id.clone(),
                    })
                    .await;
                break;
            }
            "/status" => {
                let thread = local
                    .agent
                    .get_thread(minos_protocol::GetThreadParams {
                        thread_id: session.session_id.clone(),
                    })
                    .await?;
                print_thread_snapshot(&thread);
            }
            "/threads" => {
                let threads = local
                    .agent
                    .list_threads(minos_protocol::ListThreadsParams {
                        limit: 20,
                        before_ts_ms: None,
                        agent: Some(AgentName::Codex),
                    })
                    .await?;
                print_threads(&threads.threads);
            }
            "/history" => {
                show_thread_history(local, &session.session_id, args.history_messages).await?;
            }
            "/interrupt" => {
                local
                    .agent
                    .interrupt_thread(minos_protocol::InterruptThreadRequest {
                        thread_id: session.session_id.clone(),
                    })
                    .await?;
                eprintln!("interrupted");
            }
            "/help" => {
                eprintln!("commands: /threads  /history  /resume <id>  /status  /interrupt  /exit");
            }
            _ if trimmed.starts_with("/resume ") => {
                let target = trimmed.trim_start_matches("/resume").trim();
                if target.is_empty() {
                    eprintln!("usage: /resume <thread-id>");
                    continue;
                }
                session = resume_local_cli_session(local, target).await?;
                translator = local
                    .agent
                    .hydrate_codex_translator(&session.session_id)
                    .await?;
                eprintln!("resumed thread: {}", session.session_id);
                show_thread_history(local, &session.session_id, args.history_messages).await?;
            }
            other if other.starts_with('/') => {
                eprintln!("unknown command: {other}");
            }
            _ => {
                execute_local_turn(
                    local,
                    &session.session_id,
                    trimmed,
                    args.timeout_s,
                    false,
                    &mut translator,
                )
                .await?;
            }
        }
    }

    Ok(())
}

fn resolve_workspace_arg(
    workspace: Option<&PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match workspace {
        Some(path) => expand_tilde(path),
        None => Ok(env::current_dir()?),
    }
}

async fn start_local_cli_session(
    local: &LocalAgentContext,
    agent: AgentName,
    workspace: &Path,
) -> Result<minos_protocol::StartAgentResponse, Box<dyn std::error::Error>> {
    Ok(local
        .agent
        .start_agent(minos_protocol::StartAgentRequest {
            agent,
            workspace: workspace.display().to_string(),
            mode: Some(minos_protocol::AgentLaunchMode::Server),
        })
        .await?)
}

async fn resume_local_cli_session(
    local: &LocalAgentContext,
    thread_id: &str,
) -> Result<minos_protocol::StartAgentResponse, Box<dyn std::error::Error>> {
    Ok(local.agent.resume_thread(thread_id).await?)
}

async fn execute_local_turn(
    local: &LocalAgentContext,
    session_id: &str,
    prompt: &str,
    timeout_s: u64,
    json_mode: bool,
    translator: &mut CodexTranslatorState,
) -> Result<RunRenderer, Box<dyn std::error::Error>> {
    let mut ingest_rx = local.agent.ingest_stream();
    let mut state_rx = local
        .agent
        .manager
        .thread_state_stream(session_id)
        .await
        .ok_or_else(|| MinosError::CodexProtocolError {
            method: "thread_state_stream".into(),
            message: format!("missing state stream for {session_id}"),
        })?;

    local
        .agent
        .send_user_message(minos_protocol::SendUserMessageRequest {
            session_id: session_id.to_string(),
            text: prompt.to_string(),
        })
        .await?;

    let mut renderer = RunRenderer::default();
    let deadline = Instant::now() + Duration::from_secs(timeout_s);

    loop {
        if Instant::now() >= deadline {
            return Err(Box::new(MinosError::Timeout));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut reached_terminal_state = false;

        tokio::select! {
            recv = ingest_rx.recv() => {
                match recv {
                    Ok(raw) if raw.thread_id == session_id => {
                        renderer.handle_raw_ingest(&raw, translator, json_mode)?;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        reached_terminal_state = true;
                    }
                }
            }
            changed = state_rx.changed() => {
                if changed.is_err() {
                    reached_terminal_state = true;
                } else {
                    let state = state_rx.borrow().clone();
                    if matches!(
                        state,
                        minos_daemon::ThreadState::Idle
                            | minos_daemon::ThreadState::Suspended { .. }
                            | minos_daemon::ThreadState::Closed { .. }
                    ) {
                        reached_terminal_state = true;
                    }
                }
            }
            _ = tokio::time::sleep(remaining) => {
                return Err(Box::new(MinosError::Timeout));
            }
        }

        if reached_terminal_state {
            break;
        }
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(raw) = ingest_rx.try_recv() {
        if raw.thread_id == session_id {
            renderer.handle_raw_ingest(&raw, translator, json_mode)?;
        }
    }
    Ok(renderer)
}

async fn show_thread_history(
    local: &LocalAgentContext,
    thread_id: &str,
    history_messages: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let history = local.agent.read_thread_history(thread_id).await?;
    print_transcript_history(&history.ui_events, history_messages);
    Ok(())
}

fn read_repl_line(prompt: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

#[derive(Default)]
struct TranscriptBuilder {
    order: Vec<String>,
    messages: std::collections::HashMap<String, TranscriptMessage>,
}

#[derive(Clone)]
struct TranscriptMessage {
    role: MessageRole,
    text: String,
    reasoning: String,
}

fn print_transcript_history(events: &[UiEventMessage], max_messages: usize) {
    let messages = transcript_messages(events);
    if messages.is_empty() {
        eprintln!("no local transcript yet");
        return;
    }
    let start = messages.len().saturating_sub(max_messages);
    eprintln!("history:");
    for message in &messages[start..] {
        let label = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let text = if message.text.trim().is_empty() {
            "<empty>"
        } else {
            message.text.trim()
        };
        eprintln!("{label}> {text}");
        if !message.reasoning.trim().is_empty() {
            eprintln!("  reasoning> {}", message.reasoning.trim());
        }
    }
}

fn transcript_messages(events: &[UiEventMessage]) -> Vec<TranscriptMessage> {
    let mut builder = TranscriptBuilder::default();
    for event in events {
        match event {
            UiEventMessage::MessageStarted {
                message_id, role, ..
            } => {
                builder.order.push(message_id.clone());
                builder.messages.insert(
                    message_id.clone(),
                    TranscriptMessage {
                        role: *role,
                        text: String::new(),
                        reasoning: String::new(),
                    },
                );
            }
            UiEventMessage::TextDelta { message_id, text } => {
                if let Some(message) = builder.messages.get_mut(message_id) {
                    message.text.push_str(text);
                }
            }
            UiEventMessage::ReasoningDelta { message_id, text } => {
                if let Some(message) = builder.messages.get_mut(message_id) {
                    message.reasoning.push_str(text);
                }
            }
            UiEventMessage::MessageCompleted { .. }
            | UiEventMessage::ToolCallPlaced { .. }
            | UiEventMessage::ToolCallCompleted { .. }
            | UiEventMessage::Error { .. }
            | UiEventMessage::ThreadOpened { .. }
            | UiEventMessage::ThreadTitleUpdated { .. }
            | UiEventMessage::ThreadClosed { .. }
            | UiEventMessage::Raw { .. } => {}
        }
    }
    builder
        .order
        .into_iter()
        .filter_map(|message_id| builder.messages.remove(&message_id))
        .collect()
}

async fn start(args: StartArgs, paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let config = relay_config_from_env()?;
    let mac_name = args.mac_name.unwrap_or_else(default_mac_name);
    let local_state_path = local_state_path(paths);
    let local_state = LocalState::load_or_init(&local_state_path)?;
    println!(
        "minos home: {}",
        display_optional(paths.minos_home.as_deref())
    );
    println!("data dir:   {}", display_path(&paths.data_dir));
    println!("log dir:    {}", display_path(&paths.log_dir));
    println!("state file: {}", display_path(&local_state_path));
    println!("device id:  {}", local_state.self_device_id);
    println!("relay:      {}", config.resolved_backend_url());

    let handle =
        DaemonHandle::start(config, local_state.self_device_id, None, None, mac_name).await?;

    if args.print_qr {
        let qr = handle.pairing_qr().await?;
        println!("pairing_qr:");
        println!("{}", serde_json::to_string_pretty(&qr)?);
    }

    println!("status:     running (Ctrl-C or SIGTERM to stop)");
    wait_for_termination().await?;
    println!("status:     stopping");
    handle.stop().await?;
    Ok(())
}

async fn status(
    args: ConnectArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, args.mac_name.clone()).await?;
    let action = async {
        wait_for_connected(&started.handle, args.timeout_s).await?;
        let snapshot = StatusSnapshot {
            device_id: started.local_state.self_device_id,
            relay_link: started.handle.current_relay_link(),
            peer: started.handle.current_peer(),
            peers: started.handle.current_peers().await?,
            agent_state: started.handle.current_agent_state(),
            last_error: started.handle.last_error().map(|error| error.to_string()),
        };
        if args.json {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        } else {
            print_status_snapshot(&snapshot);
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn pairing_qr(
    args: ConnectArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, args.mac_name.clone()).await?;
    let action = async {
        wait_for_connected(&started.handle, args.timeout_s).await?;
        let qr = started.handle.pairing_qr().await?;
        println!("{}", serde_json::to_string_pretty(&qr)?);
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn peers(args: OutputArgs, paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, args.connect.mac_name.clone()).await?;
    let action = async {
        wait_for_connected(&started.handle, args.connect.timeout_s).await?;
        let peers = started.handle.current_peers().await?;
        if args.connect.json {
            println!("{}", serde_json::to_string_pretty(&peers)?);
        } else {
            print_peers(&peers);
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn forget_peer(
    args: ForgetPeerArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, args.connect.mac_name.clone()).await?;
    let action = async {
        wait_for_connected(&started.handle, args.connect.timeout_s).await?;
        let peers = started.handle.current_peers().await?;
        let target = if let Some(device_id) = args.device_id.as_deref() {
            Some(minos_domain::DeviceId(Uuid::parse_str(device_id)?))
        } else {
            peers.first().map(|peer| peer.mobile_device_id)
        };

        let result = match target {
            Some(device_id) => {
                started.handle.forget_peer_device(device_id).await?;
                ForgetPeerResult {
                    forgotten: Some(device_id.to_string()),
                    remaining_peers: started.handle.current_peers().await?,
                }
            }
            None => ForgetPeerResult {
                forgotten: None,
                remaining_peers: Vec::new(),
            },
        };

        if args.connect.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else if let Some(device_id) = result.forgotten.as_deref() {
            println!("forgot peer: {device_id}");
            print_peers(&result.remaining_peers);
        } else {
            println!("no paired peers to forget");
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn threads(
    args: ThreadsArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, None).await?;
    let action = async {
        let response = started
            .handle
            .list_threads(minos_protocol::ListThreadsParams {
                limit: args.limit,
                before_ts_ms: args.before_ts_ms,
                agent: args.agent.as_deref().map(parse_agent_name).transpose()?,
            })
            .await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            print_threads(&response.threads);
            if let Some(next) = response.next_before_ts_ms {
                println!("next_before_ts_ms: {next}");
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn thread(args: ThreadArgs, paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, None).await?;
    let action = async {
        let response = started
            .handle
            .get_thread(minos_protocol::GetThreadParams {
                thread_id: args.thread_id.clone(),
            })
            .await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            print_thread_snapshot(&response);
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    stop_ephemeral(started.handle).await?;
    action
}

async fn history(
    args: HistoryArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let local = start_local_agent(paths).await?;
    let result = async {
        let history = local.agent.read_thread_history(&args.thread_id).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&history)?);
        } else {
            print_transcript_history(&history.ui_events, args.messages);
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let shutdown = shutdown_local_agent(local).await;
    match (result, shutdown) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Block until the process receives SIGINT (Ctrl-C) or SIGTERM. C20 shutdown
/// sequence is driven by `DaemonHandle::stop`; this helper just unblocks the
/// `start` future so the runtime can drive the shutdown.
async fn wait_for_termination() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        tokio::select! {
            _ = term.recv() => {},
            _ = int.recv() => {},
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

fn resolve_paths(args: &CliPaths) -> Result<ResolvedPaths, Box<dyn std::error::Error>> {
    if args.platform_paths {
        let data_dir = match args.data_dir.clone() {
            Some(p) => p,
            None => paths::state_dir()?,
        };
        let log_dir = match args.log_dir.clone() {
            Some(p) => p,
            None => platform_log_dir()?,
        };
        return Ok(ResolvedPaths {
            minos_home: None,
            data_dir,
            log_dir,
        });
    }

    let minos_home = match &args.minos_home {
        Some(path) => expand_tilde(path)?,
        None => paths::minos_home()?,
    };

    let data_dir = match &args.data_dir {
        Some(path) => expand_tilde(path)?,
        None => minos_home.clone(),
    };
    let log_dir = match &args.log_dir {
        Some(path) => expand_tilde(path)?,
        None => minos_home.join("logs"),
    };

    Ok(ResolvedPaths {
        minos_home: Some(minos_home),
        data_dir,
        log_dir,
    })
}

fn platform_log_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(minos_daemon::logging::log_dir()?)
}

fn expand_tilde(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let text = path.to_string_lossy();
    if text == "~" {
        return Ok(paths::user_home_dir()?);
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return Ok(paths::user_home_dir()?.join(rest));
    }
    Ok(path.to_path_buf())
}

fn default_mac_name() -> String {
    env::var("HOSTNAME")
        .ok()
        .or_else(|| env::var("COMPUTERNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Minos Host".into())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn display_optional(path: Option<&Path>) -> String {
    path.map_or_else(|| "<platform-defaults>".into(), display_path)
}

fn local_state_path(paths: &ResolvedPaths) -> PathBuf {
    paths.data_dir.join("local-state.json")
}

struct StartedDaemon {
    handle: Arc<DaemonHandle>,
    local_state: LocalState,
    _home_guard: MinosHomeEnvGuard,
}

#[derive(Debug, Serialize)]
struct StatusSnapshot {
    device_id: minos_domain::DeviceId,
    relay_link: minos_domain::RelayLinkState,
    peer: minos_domain::PeerState,
    peers: Vec<minos_protocol::HostPeerSummary>,
    agent_state: minos_daemon::ThreadState,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ForgetPeerResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    forgotten: Option<String>,
    remaining_peers: Vec<minos_protocol::HostPeerSummary>,
}

#[derive(Debug, Serialize)]
struct SetHostSkillResult {
    path: String,
    effective_enabled: bool,
}

#[derive(Debug, Serialize)]
struct RunSummary {
    session_id: String,
    cwd: String,
    assistant_text: String,
    errors: Vec<String>,
}

struct LocalAgentContext {
    agent: Arc<AgentGlue>,
    _home_guard: MinosHomeEnvGuard,
}

#[derive(Default)]
struct RunRenderer {
    assistant_text: String,
    errors: Vec<String>,
    stdout_open: bool,
    message_roles: std::collections::HashMap<String, minos_ui_protocol::MessageRole>,
}

impl RunRenderer {
    fn handle_raw_ingest(
        &mut self,
        raw: &minos_agent_runtime::RawIngest,
        translator: &mut CodexTranslatorState,
        json_mode: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let events = translate_codex(translator, &raw.payload).map_err(|error| {
            std::io::Error::other(format!("translate ingest {}: {error}", raw.thread_id))
        })?;
        for event in events {
            self.handle_event(&event, json_mode)?;
        }
        Ok(())
    }

    fn handle_event(
        &mut self,
        event: &UiEventMessage,
        json_mode: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match event {
            UiEventMessage::MessageStarted {
                message_id, role, ..
            } => {
                self.message_roles.insert(message_id.clone(), *role);
            }
            UiEventMessage::TextDelta { text, .. } => {
                if self.is_assistant_message(event) {
                    self.assistant_text.push_str(text);
                    if !json_mode {
                        print!("{text}");
                        std::io::stdout().flush()?;
                        self.stdout_open = true;
                    }
                }
            }
            UiEventMessage::MessageCompleted { message_id, .. } => {
                self.message_roles.remove(message_id);
                self.finish_stdout_line()?;
            }
            UiEventMessage::ToolCallPlaced { name, .. } if !json_mode => {
                self.finish_stdout_line()?;
                eprintln!("[tool] started {name}");
            }
            UiEventMessage::ToolCallCompleted {
                tool_call_id,
                is_error,
                ..
            } if !json_mode => {
                self.finish_stdout_line()?;
                eprintln!(
                    "[tool] completed {} ({})",
                    tool_call_id,
                    if *is_error { "error" } else { "ok" }
                );
            }
            UiEventMessage::Error { code, message, .. } => {
                self.errors.push(format!("{code}: {message}"));
                if !json_mode {
                    self.finish_stdout_line()?;
                    eprintln!("[error] {code}: {message}");
                }
            }
            UiEventMessage::ThreadClosed { reason, .. } if !json_mode => {
                self.finish_stdout_line()?;
                eprintln!("[thread] closed: {}", serde_json::to_string(reason)?);
            }
            UiEventMessage::ReasoningDelta { .. }
            | UiEventMessage::ToolCallPlaced { .. }
            | UiEventMessage::ToolCallCompleted { .. }
            | UiEventMessage::ThreadClosed { .. }
            | UiEventMessage::ThreadOpened { .. }
            | UiEventMessage::ThreadTitleUpdated { .. }
            | UiEventMessage::Raw { .. } => {}
        }
        Ok(())
    }

    fn finish_stdout_line(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.stdout_open {
            println!();
            std::io::stdout().flush()?;
            self.stdout_open = false;
        }
        Ok(())
    }

    fn is_assistant_message(&self, event: &UiEventMessage) -> bool {
        let message_id = match event {
            UiEventMessage::TextDelta { message_id, .. } => Some(message_id),
            UiEventMessage::ReasoningDelta { message_id, .. } => Some(message_id),
            UiEventMessage::ToolCallPlaced { message_id, .. } => Some(message_id),
            UiEventMessage::Error { message_id, .. } => message_id.as_ref(),
            _ => None,
        };
        message_id
            .and_then(|id| self.message_roles.get(id))
            .is_some_and(|role| *role == minos_ui_protocol::MessageRole::Assistant)
    }
}

async fn start_ephemeral(
    paths: &ResolvedPaths,
    mac_name: Option<String>,
) -> Result<StartedDaemon, Box<dyn std::error::Error>> {
    let home_guard = maybe_apply_minos_home_override(paths);
    let config = relay_config_from_env()?;
    let local_state = LocalState::load_or_init(&local_state_path(paths))?;
    let handle = DaemonHandle::start(
        config,
        local_state.self_device_id,
        None,
        None,
        mac_name.unwrap_or_else(default_mac_name),
    )
    .await?;
    Ok(StartedDaemon {
        handle,
        local_state,
        _home_guard: home_guard,
    })
}

async fn stop_ephemeral(handle: Arc<DaemonHandle>) -> Result<(), Box<dyn std::error::Error>> {
    handle.stop().await?;
    Ok(())
}

async fn start_local_agent(
    paths: &ResolvedPaths,
) -> Result<LocalAgentContext, Box<dyn std::error::Error>> {
    let home_guard = maybe_apply_minos_home_override(paths);
    let home = agent_home_for_runtime(paths)?;
    let subprocess_env = Arc::new(capture_user_shell_env().await);
    let db_path = home.join("daemon.sqlite");
    let store = Arc::new(minos_daemon::store::LocalStore::open(&db_path).await?);
    let (out_tx, _out_rx) = mpsc::channel(8);
    let agent = Arc::new(AgentGlue::new(
        home.join("workspaces"),
        subprocess_env,
        store,
        out_tx,
    ));
    Ok(LocalAgentContext {
        agent,
        _home_guard: home_guard,
    })
}

async fn shutdown_local_agent(local: LocalAgentContext) -> Result<(), Box<dyn std::error::Error>> {
    match local.agent.shutdown().await {
        Ok(()) | Err(MinosError::AgentNotRunning) => {}
        Err(err) => return Err(Box::new(err)),
    }
    local
        .agent
        .manager
        .shutdown_instances(std::time::Duration::from_secs(5))
        .await;
    Ok(())
}

async fn wait_for_connected(
    handle: &DaemonHandle,
    timeout_s: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    loop {
        if matches!(
            handle.current_relay_link(),
            minos_domain::RelayLinkState::Connected
        ) {
            return Ok(());
        }
        if let Some(error) = handle.last_error() {
            return Err(Box::new(error));
        }
        if Instant::now() >= deadline {
            return Err(Box::new(MinosError::Timeout));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn print_status_snapshot(snapshot: &StatusSnapshot) {
    println!("device id:   {}", snapshot.device_id);
    println!("relay link:  {}", format_relay_link(snapshot.relay_link));
    println!("peer state:  {}", format_peer_state(&snapshot.peer));
    println!(
        "agent state: {}",
        format_thread_state(&snapshot.agent_state)
    );
    println!("peers:       {}", snapshot.peers.len());
    if let Some(error) = snapshot.last_error.as_deref() {
        println!("last error:  {error}");
    }
}

fn print_peers(peers: &[minos_protocol::HostPeerSummary]) {
    if peers.is_empty() {
        println!("no paired peers");
        return;
    }

    for peer in peers {
        println!(
            "{}  {}  email={}  online={}  paired_at_ms={}  last_active_at_ms={}",
            peer.mobile_device_id,
            peer.mobile_device_name,
            peer.account_email,
            peer.online,
            peer.paired_at_ms,
            peer.last_active_at_ms
        );
    }
}

fn format_relay_link(state: minos_domain::RelayLinkState) -> String {
    match state {
        minos_domain::RelayLinkState::Disconnected => "disconnected".into(),
        minos_domain::RelayLinkState::Connected => "connected".into(),
        minos_domain::RelayLinkState::Connecting { attempt } => {
            format!("connecting (attempt {attempt})")
        }
    }
}

fn format_peer_state(state: &minos_domain::PeerState) -> String {
    match state {
        minos_domain::PeerState::Unpaired => "unpaired".into(),
        minos_domain::PeerState::Pairing => "pairing".into(),
        minos_domain::PeerState::Paired {
            peer_id,
            peer_name,
            online,
        } => format!("{peer_name} ({peer_id}) online={online}"),
    }
}

fn format_thread_state(state: &minos_daemon::ThreadState) -> &'static str {
    match state {
        minos_daemon::ThreadState::Starting => "starting",
        minos_daemon::ThreadState::Idle => "idle",
        minos_daemon::ThreadState::Running { .. } => "running",
        minos_daemon::ThreadState::Suspended { .. } => "suspended",
        minos_daemon::ThreadState::Resuming => "resuming",
        minos_daemon::ThreadState::Closed { .. } => "closed",
    }
}

fn format_protocol_thread_state(state: &minos_protocol::ThreadState) -> &'static str {
    match state {
        minos_protocol::ThreadState::Starting => "starting",
        minos_protocol::ThreadState::Idle => "idle",
        minos_protocol::ThreadState::Running { .. } => "running",
        minos_protocol::ThreadState::Suspended { .. } => "suspended",
        minos_protocol::ThreadState::Resuming => "resuming",
        minos_protocol::ThreadState::Closed { .. } => "closed",
    }
}

fn parse_agent_name(value: &str) -> Result<AgentName, Box<dyn std::error::Error>> {
    match value {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        other => Err(format!("unknown agent {other:?}; want one of codex/claude/gemini").into()),
    }
}

fn format_agent_name(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
    }
}

fn describe_agent_status(status: &AgentStatus) -> String {
    match status {
        AgentStatus::Ok => "ok".into(),
        AgentStatus::Missing => "missing".into(),
        AgentStatus::Error { reason } => format!("error ({reason})"),
    }
}

fn print_agent_descriptors(descriptors: &[AgentDescriptor]) {
    if descriptors.is_empty() {
        println!("no CLI agents detected");
        return;
    }

    for descriptor in descriptors {
        println!(
            "{}  status={}  version={}  path={}",
            format_agent_name(descriptor.name),
            describe_agent_status(&descriptor.status),
            descriptor.version.as_deref().unwrap_or("<unknown>"),
            descriptor.path.as_deref().unwrap_or("<missing>")
        );
    }
}

fn print_host_skills(response: &minos_protocol::ListHostSkillsResponse) {
    if response.data.is_empty() {
        println!("no host skill entries");
        return;
    }

    for entry in &response.data {
        println!("workspace: {}", entry.cwd);
        if entry.skills.is_empty() {
            println!("  no skills found");
        } else {
            for skill in &entry.skills {
                println!(
                    "  {}  enabled={}  scope={}  path={}",
                    skill.name, skill.enabled, skill.scope, skill.path
                );
                println!("    {}", skill.description);
            }
        }
        if entry.errors.is_empty() {
            continue;
        }
        for error in &entry.errors {
            println!("  error: {}  {}", error.path, error.message);
        }
    }
}

fn print_threads(threads: &[minos_protocol::ThreadSummary]) {
    if threads.is_empty() {
        println!("no persisted threads");
        return;
    }

    for thread in threads {
        println!(
            "{}  agent={}  title={}  messages={}  last_ts_ms={}",
            thread.thread_id,
            format_agent_name(thread.agent),
            thread.title.as_deref().unwrap_or("<untitled>"),
            thread.message_count,
            thread.last_ts_ms
        );
    }
}

fn print_thread_snapshot(thread: &minos_protocol::GetThreadResponse) {
    println!("thread id:     {}", thread.thread.thread_id);
    println!("agent:         {}", format_agent_name(thread.thread.agent));
    println!(
        "state:         {}",
        format_protocol_thread_state(&thread.state)
    );
    println!(
        "title:         {}",
        thread.thread.title.as_deref().unwrap_or("<untitled>")
    );
    println!("message count: {}", thread.thread.message_count);
    println!("first_ts_ms:   {}", thread.thread.first_ts_ms);
    println!("last_ts_ms:    {}", thread.thread.last_ts_ms);
    if let Some(ended_at_ms) = thread.thread.ended_at_ms {
        println!("ended_at_ms:   {ended_at_ms}");
    }
    if let Some(end_reason) = &thread.thread.end_reason {
        println!(
            "end_reason:    {}",
            serde_json::to_string(end_reason).unwrap_or_else(|_| "<invalid>".into())
        );
    }
}

fn relay_config_from_env() -> Result<RelayConfig, MinosError> {
    relay_config_from_values(env::var("MINOS_BACKEND_URL").ok())
}

fn relay_config_from_values(backend_url: Option<String>) -> Result<RelayConfig, MinosError> {
    let backend_url = blank_to_none(backend_url).unwrap_or_default();
    Ok(RelayConfig::new(backend_url))
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn agent_home_for_runtime(paths: &ResolvedPaths) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(home) = &paths.minos_home {
        return Ok(home.clone());
    }
    Ok(paths::minos_home()?)
}

struct MinosHomeEnvGuard {
    previous: Option<OsString>,
    active: bool,
}

impl Drop for MinosHomeEnvGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(previous) = self.previous.take() {
            env::set_var("MINOS_HOME", previous);
        } else {
            env::remove_var("MINOS_HOME");
        }
    }
}

fn maybe_apply_minos_home_override(paths: &ResolvedPaths) -> MinosHomeEnvGuard {
    if let Some(home) = &paths.minos_home {
        let previous = env::var_os("MINOS_HOME");
        env::set_var("MINOS_HOME", home);
        MinosHomeEnvGuard {
            previous,
            active: true,
        }
    } else {
        MinosHomeEnvGuard {
            previous: None,
            active: false,
        }
    }
}

struct ResolvedPaths {
    minos_home: Option<PathBuf>,
    data_dir: PathBuf,
    log_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_agent_runtime::config::AgentRuntimeConfig;
    use minos_agent_runtime::test_support::{FakeCodexServer, Step};
    use minos_agent_runtime::{AgentManager, InstanceCaps};
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolves_cli_defaults_under_dot_minos() {
        let args = CliPaths {
            minos_home: Some(PathBuf::from("/tmp/minos-home")),
            data_dir: None,
            log_dir: None,
            platform_paths: false,
        };

        let resolved = resolve_paths(&args).unwrap();
        assert_eq!(resolved.minos_home, Some(PathBuf::from("/tmp/minos-home")));
        assert_eq!(resolved.data_dir, PathBuf::from("/tmp/minos-home"));
        assert_eq!(resolved.log_dir, PathBuf::from("/tmp/minos-home/logs"));
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = paths::user_home_dir().unwrap();
        let expanded = expand_tilde(Path::new("~/minos")).unwrap();
        assert_eq!(expanded, home.join("minos"));
    }

    #[test]
    fn relay_config_from_values_defaults_to_blank_backend() {
        let config = relay_config_from_values(None).expect("blank config");
        assert!(config.backend_url.is_empty());
    }

    #[test]
    fn relay_config_from_values_trims_backend_url() {
        let config =
            relay_config_from_values(Some(" wss://relay.example/devices ".into())).expect("cfg");
        assert_eq!(config.backend_url, "wss://relay.example/devices");
    }

    #[test]
    fn local_state_path_lives_under_data_dir() {
        let paths = ResolvedPaths {
            minos_home: Some(PathBuf::from("/tmp/minos-home")),
            data_dir: PathBuf::from("/tmp/minos-home/state"),
            log_dir: PathBuf::from("/tmp/minos-home/logs"),
        };
        assert_eq!(
            local_state_path(&paths),
            PathBuf::from("/tmp/minos-home/state/local-state.json")
        );
    }

    #[test]
    fn local_state_load_or_init_persists_self_device_id_across_reloads() {
        let temp = TempDir::new().unwrap();
        let paths = ResolvedPaths {
            minos_home: Some(temp.path().join("home")),
            data_dir: temp.path().join("state"),
            log_dir: temp.path().join("logs"),
        };
        let path = local_state_path(&paths);
        let first = LocalState::load_or_init(&path).unwrap();
        let second = LocalState::load_or_init(&path).unwrap();
        assert_eq!(first.self_device_id, second.self_device_id);
    }

    #[test]
    fn parses_list_clis_command() {
        let cli = Cli::parse_from(["minos-daemon", "list-clis", "--json"]);
        assert!(matches!(
            cli.command,
            Command::ListClis(OutputModeArgs { json: true })
        ));
    }

    #[test]
    fn parses_host_skills_command() {
        let cli = Cli::parse_from([
            "minos-daemon",
            "host-skills",
            "--workspace",
            "/tmp/ws",
            "--force-reload",
        ]);
        match cli.command {
            Command::HostSkills(args) => {
                assert_eq!(args.workspace.as_deref(), Some("/tmp/ws"));
                assert!(args.force_reload);
                assert!(!args.json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_run_command() {
        let cli = Cli::parse_from([
            "minos-daemon",
            "run",
            "--workspace",
            "/tmp/ws",
            "--timeout-s",
            "90",
            "hello from cli",
        ]);
        match cli.command {
            Command::Run(args) => {
                assert_eq!(args.workspace, Some(PathBuf::from("/tmp/ws")));
                assert_eq!(args.timeout_s, 90);
                assert_eq!(args.prompt, "hello from cli");
                assert_eq!(args.agent, "codex");
                assert!(args.thread.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_chat_command() {
        let cli = Cli::parse_from([
            "minos-daemon",
            "chat",
            "--workspace",
            "/tmp/ws",
            "--prompt",
            "boot",
        ]);
        match cli.command {
            Command::Chat(args) => {
                assert_eq!(args.workspace, Some(PathBuf::from("/tmp/ws")));
                assert_eq!(args.prompt.as_deref(), Some("boot"));
                assert_eq!(args.agent, "codex");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_history_command() {
        let cli = Cli::parse_from(["minos-daemon", "history", "--messages", "5", "thr-123"]);
        match cli.command {
            Command::History(args) => {
                assert_eq!(args.thread_id, "thr-123");
                assert_eq!(args.messages, 5);
                assert!(!args.json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_run_thread_command() {
        let cli = Cli::parse_from(["minos-daemon", "run", "--thread", "thr-123", "continue"]);
        match cli.command {
            Command::Run(args) => {
                assert_eq!(args.thread.as_deref(), Some("thr-123"));
                assert_eq!(args.prompt, "continue");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn minos_home_override_sets_and_restores_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("MINOS_HOME");
        let paths = ResolvedPaths {
            minos_home: Some(PathBuf::from("/tmp/cli-home")),
            data_dir: PathBuf::from("/tmp/cli-home"),
            log_dir: PathBuf::from("/tmp/cli-home/logs"),
        };

        {
            let _override = maybe_apply_minos_home_override(&paths);
            assert_eq!(env::var("MINOS_HOME").unwrap(), "/tmp/cli-home");
        }

        assert!(env::var_os("MINOS_HOME").is_none());
    }

    fn fake_thread_response(thread_id: &str) -> serde_json::Value {
        serde_json::json!({
            "approvalPolicy": "never",
            "approvalsReviewer": "user",
            "cwd": "/tmp",
            "instructionSources": [],
            "model": "fake",
            "modelProvider": "fake",
            "sandbox": { "type": "dangerFullAccess" },
            "thread": {
                "id": thread_id,
                "cliVersion": "0.0.0-fake",
                "createdAt": 0,
                "cwd": "/tmp",
                "ephemeral": true,
                "modelProvider": "fake",
                "preview": "",
                "source": "appServer",
                "status": { "type": "idle" },
                "turns": [],
                "updatedAt": 0
            }
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_local_turn_streams_assistant_text() {
        let tmp = tempfile::tempdir().unwrap();
        let thread_id = "thr-main-run";
        let script = vec![
            Step::ExpectRequest {
                method: "thread/start".into(),
                reply: fake_thread_response(thread_id),
            },
            Step::ExpectRequest {
                method: "turn/start".into(),
                reply: serde_json::json!({
                    "turn": {
                        "id": "turn-1",
                        "items": [],
                        "status": "inProgress"
                    }
                }),
            },
            Step::EmitNotification {
                method: "item/started".into(),
                params: serde_json::json!({
                    "threadId": thread_id,
                    "item": { "type": "agentMessage", "id": "a1", "text": "" }
                }),
            },
            Step::EmitNotification {
                method: "item/agentMessage/delta".into(),
                params: serde_json::json!({
                    "threadId": thread_id,
                    "itemId": "a1",
                    "delta": "hello from codex"
                }),
            },
            Step::EmitNotification {
                method: "turn/completed".into(),
                params: serde_json::json!({
                    "threadId": thread_id,
                    "finishedAtMs": 1
                }),
            },
            Step::Sleep { ms: 50 },
        ];
        let (server, port) = FakeCodexServer::bind(script).await;

        let mut cfg = AgentRuntimeConfig::new(tmp.path().join("workspaces"));
        cfg.test_ws_url = Some(
            url::Url::parse(&format!("ws://127.0.0.1:{port}")).expect("loopback URL should parse"),
        );
        let manager = Arc::new(AgentManager::new(cfg, InstanceCaps::default()));
        let store = Arc::new(
            minos_daemon::store::LocalStore::open(&tmp.path().join("daemon.sqlite"))
                .await
                .unwrap(),
        );
        let (out_tx, _out_rx) = mpsc::channel(8);
        let writer = Arc::new(minos_daemon::store::event_writer::EventWriter::spawn(
            store.clone(),
            out_tx,
        ));
        let agent = Arc::new(AgentGlue::wire_with(
            manager,
            writer,
            store,
            tmp.path().join("workspaces"),
        ));
        let local = LocalAgentContext {
            agent,
            _home_guard: MinosHomeEnvGuard {
                previous: None,
                active: false,
            },
        };

        let session = start_local_cli_session(&local, AgentName::Codex, Path::new("/tmp"))
            .await
            .unwrap();
        let mut translator = CodexTranslatorState::new(session.session_id.clone());
        let renderer = execute_local_turn(
            &local,
            &session.session_id,
            "hello",
            2,
            true,
            &mut translator,
        )
        .await
        .unwrap();

        assert_eq!(renderer.assistant_text, "hello from codex");
        assert!(renderer.errors.is_empty());

        server.stop().await;
    }
}
