use anyhow::Result;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

pub fn resolve_log_path(workspace: &Path, explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| workspace.join(".minos/logs/minos-tui.log"))
}

pub fn init(log_path: &Path) -> Result<()> {
    let parent = log_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("log path has no parent: {}", log_path.display()))?;
    fs::create_dir_all(parent)?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("minos_tui=info,minos_agent_runtime=info"));

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(move || file.try_clone().expect("log file clone should succeed"))
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
    tracing::info!(path = %log_path.display(), "minos-tui logging initialized");
    Ok(())
}
