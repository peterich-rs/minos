use std::env;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use minos_cli_detect::{capture_user_shell_env, detect_all, RealCommandRunner};
use minos_daemon::local_rpc::LocalRpcConfig;
use minos_daemon::{paths, AgentGlue, DaemonHandle, LocalState, RelayConfig};
use minos_domain::{AgentDescriptor, AgentName, AgentStatus, MinosError};
use serde::Serialize;
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
    #[command(name = "__minos-teamwork-mcp", hide = true)]
    MinosTeamworkMcp(McpSidecarArgs),
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
    /// Connect once and list paired mobile/account rows for this host.
    Peers(OutputArgs),
    /// Forget one paired mobile/account row, or the first row when omitted.
    ForgetPeer(ForgetPeerArgs),
    /// Read persisted session summaries from the local daemon store.
    Threads(ThreadsArgs),
    /// Read one persisted session summary + live state from the local store.
    Thread(ThreadArgs),
    /// Start the daemon (dials the relay) and keep it running until Ctrl-C.
    Start(StartArgs),
}

#[derive(Args, Debug)]
struct StartArgs {
    /// Human-readable Mac name shown to the peer during pairing.
    #[arg(long)]
    mac_name: Option<String>,
    /// Print a fresh pairing QR payload as JSON after startup.
    /// Enable local JSON-RPC server for TUI communication.
    #[arg(long)]
    local_rpc: bool,
    /// Override bind address for the local RPC server (default: 127.0.0.1:0).
    #[arg(long)]
    local_rpc_addr: Option<String>,
}

#[derive(Args, Debug)]
struct McpSidecarArgs {
    #[arg(long)]
    socket_path: PathBuf,

    #[arg(long)]
    conversation_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    source_session_id: Option<String>,

    #[arg(long)]
    disable_list_conversation_messages: bool,

    #[arg(long)]
    disable_list_conversation_roster: bool,

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

    #[arg(long)]
    disable_post_git_update: bool,
}

impl McpSidecarArgs {
    async fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
        let source_agent = self
            .source_agent
            .as_deref()
            .map(parse_agent_name)
            .transpose()?;
        minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
            socket_path: self.socket_path,
            conversation_id: self.conversation_id,
            source_agent,
            source_session_id: self.source_session_id,
            permissions: minos_chat_store::mcp_server::McpToolPermissions {
                list_conversation_messages: !self.disable_list_conversation_messages,
                list_conversation_roster: !self.disable_list_conversation_roster,
                delegate_to_agent: !self.disable_delegate_to_agent,
                get_delegation_status: !self.disable_get_delegation_status,
                wait_delegation: !self.disable_wait_delegation,
                cancel_delegation: !self.disable_cancel_delegation,
                post_conversation_update: !self.disable_post_conversation_update,
                post_git_update: !self.disable_post_git_update,
            },
        })
        .await?;
        Ok(())
    }
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
    /// Session id to inspect.
    session_id: String,
    /// Print JSON instead of human-readable text.
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
    let command = cli.command;
    let command = match command {
        Command::MinosTeamworkMcp(args) => return args.serve().await,
        command => command,
    };
    let resolved_paths = resolve_paths(&cli.paths)?;

    match command {
        Command::MinosTeamworkMcp(_) => unreachable!("handled before path resolution"),
        Command::Doctor => doctor(&resolved_paths).await,
        Command::ListClis(args) => list_clis(args).await,
        Command::HostSkills(args) => host_skills(args, &resolved_paths).await,
        Command::SetHostSkill(args) => set_host_skill(args, &resolved_paths).await,
        Command::Status(args) => status(args, &resolved_paths).await,
        Command::Peers(args) => peers(args, &resolved_paths).await,
        Command::ForgetPeer(args) => forget_peer(args, &resolved_paths).await,
        Command::Threads(args) => sessions(args, &resolved_paths).await,
        Command::Thread(args) => thread(args, &resolved_paths).await,
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
        (Err(err), _) | (Ok(()), Err(err)) => Err(err),
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
        (Err(err), _) | (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
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

    let local_rpc_config = if args.local_rpc {
        let addr: SocketAddr = args
            .local_rpc_addr
            .as_deref()
            .unwrap_or("127.0.0.1:0")
            .parse()
            .map_err(|e| std::io::Error::other(format!("invalid --local-rpc-addr: {e}")))?;
        let run_dir = paths::run_dir()?;
        let discovery_path = run_dir.join("tui-daemon-rpc.json");
        Some(LocalRpcConfig {
            addr,
            discovery_path: discovery_path.clone(),
        })
    } else {
        None
    };

    let handle = DaemonHandle::start_with_local_rpc(
        config,
        local_state.self_device_id,
        None,
        None,
        mac_name,
        local_rpc_config,
    )
    .await?;

    if args.local_rpc {
        let discovery = paths::run_dir()?.join("tui-daemon-rpc.json");
        println!("local rpc:  {}", discovery.display());
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

async fn sessions(
    args: ThreadsArgs,
    paths: &ResolvedPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = start_ephemeral(paths, None).await?;
    let action = async {
        let response = started
            .handle
            .list_sessions(minos_protocol::ListSessionsParams {
                limit: args.limit,
                before_ts_ms: args.before_ts_ms,
                agent: args.agent.as_deref().map(parse_agent_name).transpose()?,
            })
            .await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            print_threads(&response.sessions);
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
            .get_session(minos_protocol::GetSessionParams {
                session_id: args.session_id.clone(),
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
    agent_state: minos_daemon::SessionState,
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

struct LocalAgentContext {
    agent: Arc<AgentGlue>,
    _home_guard: MinosHomeEnvGuard,
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
    let agent = Arc::new(AgentGlue::new(
        home.join("workspaces"),
        subprocess_env,
        store,
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
        format_session_state(&snapshot.agent_state)
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

fn format_session_state(state: &minos_daemon::SessionState) -> &'static str {
    match state {
        minos_daemon::SessionState::Starting => "starting",
        minos_daemon::SessionState::Idle => "idle",
        minos_daemon::SessionState::Running { .. } => "running",
        minos_daemon::SessionState::Suspended { .. } => "suspended",
        minos_daemon::SessionState::Resuming => "resuming",
        minos_daemon::SessionState::Closed { .. } => "closed",
    }
}

fn format_protocol_session_state(state: &minos_protocol::SessionState) -> &'static str {
    match state {
        minos_protocol::SessionState::Starting => "starting",
        minos_protocol::SessionState::Idle => "idle",
        minos_protocol::SessionState::Running { .. } => "running",
        minos_protocol::SessionState::Suspended { .. } => "suspended",
        minos_protocol::SessionState::Resuming => "resuming",
        minos_protocol::SessionState::Closed { .. } => "closed",
    }
}

fn parse_agent_name(value: &str) -> Result<AgentName, Box<dyn std::error::Error>> {
    match value {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        "opencode" => Ok(AgentName::Opencode),
        "grok" => Ok(AgentName::Grok),
        other => Err(format!(
            "unknown agent {other:?}; want one of codex/claude/gemini/opencode/grok"
        )
        .into()),
    }
}

fn format_agent_name(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
        AgentName::Gemini => "gemini",
        AgentName::Opencode => "opencode",
        AgentName::Grok => "grok",
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

fn print_threads(sessions: &[minos_protocol::SessionSummary]) {
    if sessions.is_empty() {
        println!("no persisted sessions");
        return;
    }

    for thread in sessions {
        println!(
            "{}  agent={}  title={}  messages={}  last_ts_ms={}",
            thread.session_id,
            format_agent_name(thread.agent),
            thread.title.as_deref().unwrap_or("<untitled>"),
            thread.message_count,
            thread.last_ts_ms
        );
    }
}

fn print_thread_snapshot(thread: &minos_protocol::GetSessionResponse) {
    println!("session id:     {}", thread.thread.session_id);
    println!("agent:         {}", format_agent_name(thread.thread.agent));
    println!(
        "state:         {}",
        format_protocol_session_state(&thread.state)
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

#[allow(clippy::unnecessary_wraps)]
fn relay_config_from_env() -> Result<RelayConfig, MinosError> {
    Ok(relay_config_from_values(env::var("MINOS_BACKEND_URL").ok()))
}

fn relay_config_from_values(backend_url: Option<String>) -> RelayConfig {
    let backend_url = blank_to_none(backend_url).unwrap_or_default();
    RelayConfig::new(backend_url)
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
    use tempfile::TempDir;

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
        let config = relay_config_from_values(None);
        assert!(config.backend_url.is_empty());
    }

    #[test]
    fn relay_config_from_values_trims_backend_url() {
        let config = relay_config_from_values(Some(" wss://relay.example/devices ".into()));
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
}
