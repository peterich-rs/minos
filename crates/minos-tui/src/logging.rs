use anyhow::Result;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

pub fn resolve_log_path(workspace: &Path, explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| workspace.join(".minos/logs/minos-tui.log"))
}

/// Shared log file writer that never panics on I/O failure.
///
/// Uses a single opened `File` behind a mutex instead of `try_clone()` on every
/// log line, so transient clone/fd failures cannot crash the TUI.
#[derive(Clone)]
struct SharedLogWriter(Arc<Mutex<File>>);

impl<'a> MakeWriter<'a> for SharedLogWriter {
    type Writer = SharedLogWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedLogWriterGuard(Arc::clone(&self.0))
    }
}

struct SharedLogWriterGuard(Arc<Mutex<File>>);

impl Write for SharedLogWriterGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut file) => match file.write(buf) {
                Ok(n) => Ok(n),
                // Swallow write errors so logging never takes down the TUI.
                Err(_) => Ok(buf.len()),
            },
            Err(_) => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.lock() {
            Ok(mut file) => {
                let _ = file.flush();
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }
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

    let writer = SharedLogWriter(Arc::new(Mutex::new(file)));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
    tracing::info!(path = %log_path.display(), "minos-tui logging initialized");
    Ok(())
}
