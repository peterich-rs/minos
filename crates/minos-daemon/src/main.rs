use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use minos_daemon::{paths, DaemonConfig, DaemonHandle};

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
    /// Print resolved runtime paths and Tailscale discovery status.
    Doctor,
    /// Start the daemon and keep it running until Ctrl-C.
    Start(StartArgs),
}

#[derive(Args, Debug)]
struct StartArgs {
    /// Human-readable Mac name shown to the peer during pairing.
    #[arg(long)]
    mac_name: Option<String>,
    /// Bind to a specific socket instead of auto-discovering a Tailscale IP.
    #[arg(long)]
    bind: Option<SocketAddr>,
    /// Print a fresh pairing QR payload as JSON after startup.
    #[arg(long)]
    print_qr: bool,
}

#[derive(Args, Debug)]
struct CliPaths {
    /// Root directory used by the CLI for daemon state and logs.
    #[arg(long)]
    minos_home: Option<PathBuf>,
    /// Override the pairing-store directory. Writes `devices.json` here.
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
    apply_paths(&resolved_paths);

    match cli.command {
        Command::Doctor => doctor(&resolved_paths).await,
        Command::Start(args) => {
            minos_daemon::logging::init()?;
            start(args, &resolved_paths).await
        }
    }
}

async fn doctor(paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "minos home: {}",
        display_optional(paths.minos_home.as_deref())
    );
    println!("data dir:   {}", display_path(&paths.data_dir));
    println!(
        "devices:    {}",
        display_path(&paths.data_dir.join("devices.json"))
    );
    println!("log dir:    {}", display_path(&paths.log_dir));

    match minos_daemon::discover_tailscale_ip_with_reason().await {
        Ok(ip) => println!("tailscale:  ok ({ip})"),
        Err(message) => println!("tailscale:  error ({message})"),
    }

    Ok(())
}

async fn start(args: StartArgs, paths: &ResolvedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let mac_name = args.mac_name.unwrap_or_else(default_mac_name);
    println!(
        "minos home: {}",
        display_optional(paths.minos_home.as_deref())
    );
    println!("data dir:   {}", display_path(&paths.data_dir));
    println!("log dir:    {}", display_path(&paths.log_dir));

    let handle = if let Some(bind_addr) = args.bind {
        println!("binding:    explicit {bind_addr}");
        DaemonHandle::start(DaemonConfig {
            mac_name,
            bind_addr,
        })
        .await?
    } else {
        println!("binding:    autobind via Tailscale");
        DaemonHandle::start_autobind(mac_name).await?
    };

    println!("endpoint:   {}:{}", handle.host(), handle.port());

    if args.print_qr {
        let qr = handle.pairing_qr()?;
        println!("pairing_qr:");
        println!("{}", serde_json::to_string_pretty(&qr)?);
    }

    println!("status:     running (Ctrl-C to stop)");
    tokio::signal::ctrl_c().await?;
    println!("status:     stopping");
    handle.stop().await?;
    Ok(())
}

fn resolve_paths(args: &CliPaths) -> Result<ResolvedPaths, Box<dyn std::error::Error>> {
    if args.platform_paths {
        let data_dir = args.data_dir.clone().unwrap_or_else(platform_data_dir);
        let log_dir = args.log_dir.clone().unwrap_or_else(platform_log_dir);
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

fn apply_paths(paths: &ResolvedPaths) {
    env::set_var("MINOS_DATA_DIR", &paths.data_dir);
    env::set_var("MINOS_LOG_DIR", &paths.log_dir);
}

fn platform_data_dir() -> PathBuf {
    if let Ok(dir) = env::var("MINOS_DATA_DIR") {
        return PathBuf::from(dir);
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/minos")
    } else {
        PathBuf::from(home).join(".minos")
    }
}

fn platform_log_dir() -> PathBuf {
    minos_daemon::logging::log_dir()
}

fn expand_tilde(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let text = path.to_string_lossy();
    if text == "~" {
        return Ok(PathBuf::from(env::var("HOME")?));
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return Ok(PathBuf::from(env::var("HOME")?).join(rest));
    }
    Ok(path.to_path_buf())
}

fn default_mac_name() -> String {
    env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Minos CLI".into())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn display_optional(path: Option<&Path>) -> String {
    path.map_or_else(|| "<platform-defaults>".into(), display_path)
}

struct ResolvedPaths {
    minos_home: Option<PathBuf>,
    data_dir: PathBuf,
    log_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let home = env::var("HOME").unwrap();
        let expanded = expand_tilde(Path::new("~/minos")).unwrap();
        assert_eq!(expanded, PathBuf::from(home).join("minos"));
    }
}
