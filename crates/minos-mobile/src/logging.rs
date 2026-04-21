//! mars-xlog wiring for the iOS-side core process.
//!
//! Sink directory comes from the Dart layer (frb-callback in plan 03) so that
//! `iOS app Documents/Minos/Logs/` is honored even though Rust doesn't know
//! the exact app sandbox path. For unit-test builds, callers may pass a
//! tempdir directly.

use std::path::Path;
use std::sync::OnceLock;

use mars_xlog::{LogLevel, Xlog, XlogConfig, XlogLayer, XlogLayerConfig, XlogLayerHandle};
use minos_domain::MinosError;
use tracing_subscriber::prelude::*;

static HANDLE: OnceLock<XlogLayerHandle> = OnceLock::new();

const NAME_PREFIX: &str = "mobile-rust";

/// Initialize logging for the mobile-side Rust core. `log_dir` is supplied by
/// the host (Dart side via frb in plan 03; tempdir in tests).
#[allow(clippy::missing_errors_doc)]
pub fn init(log_dir: &Path) -> Result<(), MinosError> {
    if HANDLE.get().is_some() {
        return Ok(());
    }
    std::fs::create_dir_all(log_dir).map_err(|e| MinosError::StoreIo {
        path: log_dir.display().to_string(),
        message: e.to_string(),
    })?;
    let cfg = XlogConfig::new(log_dir.to_string_lossy().to_string(), NAME_PREFIX);
    let logger = Xlog::init(cfg, LogLevel::Info).map_err(|e| MinosError::StoreIo {
        path: log_dir.display().to_string(),
        message: e.to_string(),
    })?;

    let (layer, handle) =
        XlogLayer::with_config(logger, XlogLayerConfig::new(LogLevel::Info).enabled(true));

    let _ = HANDLE.set(handle);

    let subscriber = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);

    tracing::info!(name_prefix = NAME_PREFIX, dir = %log_dir.display(), "mobile logging initialized");
    Ok(())
}

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
    fn init_creates_log_dir() {
        let dir = tempdir().unwrap();
        init(dir.path()).unwrap();
        init(dir.path()).unwrap(); // idempotent
        assert!(dir.path().exists());
    }
}
