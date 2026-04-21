//! mars-xlog wiring for the Mac-side daemon process.
//!
//! Layout: `~/Library/Logs/Minos/<name_prefix>-YYYYMMDD.xlog`. Use prefix
//! `daemon` per spec §9.4. Decoder: `decode_mars_nocrypt_log_file.py` from
//! the upstream Mars repo (Tencent).

use std::path::PathBuf;
use std::sync::OnceLock;

use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig, XlogLayerHandle};
use minos_domain::MinosError;
use tracing_subscriber::prelude::*;

static HANDLE: OnceLock<XlogLayerHandle> = OnceLock::new();

const NAME_PREFIX: &str = "daemon";

#[must_use]
pub fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Logs/Minos")
    } else {
        PathBuf::from(home).join(".minos/logs")
    }
}

/// Idempotent global initialization. Subsequent calls are no-ops.
#[allow(clippy::missing_errors_doc)]
pub fn init() -> Result<(), MinosError> {
    if HANDLE.get().is_some() {
        return Ok(());
    }
    let dir = log_dir();
    std::fs::create_dir_all(&dir).map_err(|e| MinosError::StoreIo {
        path: dir.display().to_string(),
        message: e.to_string(),
    })?;

    let cfg = XlogConfig::new(dir.to_string_lossy().to_string(), NAME_PREFIX);
    let logger = Xlog::init(cfg, LogLevel::Info).map_err(|e| MinosError::StoreIo {
        path: dir.display().to_string(),
        message: e.to_string(),
    })?;

    let (layer, handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(LogLevel::Info).enabled(true));

    let _ = HANDLE.set(handle);

    let subscriber = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);

    tracing::info!(name_prefix = NAME_PREFIX, dir = %dir.display(), "daemon logging initialized");
    Ok(())
}

/// Toggle level at runtime (for the menubar "diagnostics" switch in plan 02).
pub fn set_debug(enabled: bool) {
    if let Some(h) = HANDLE.get() {
        h.set_level(if enabled {
            LogLevel::Debug
        } else {
            LogLevel::Info
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_creates_log_dir_and_emits_once() {
        // Override HOME so test logs go to a tempdir, not the real ~/Library/Logs.
        let dir = tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        init().unwrap();
        // Idempotent
        init().unwrap();
        let computed = log_dir();
        assert!(computed.exists());
    }
}
