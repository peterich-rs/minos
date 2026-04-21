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
    // Tests opt out of HOME-based pathing by setting MINOS_LOG_DIR; keeps
    // each test's log directory isolated without mutating process-global HOME.
    if let Ok(d) = std::env::var("MINOS_LOG_DIR") {
        return PathBuf::from(d);
    }
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

/// Return an absolute path to the current day's xlog file, after flushing
/// pending writes to disk. Swift uses this for "在 Finder 中显示今日日志…"
/// (spec §6.4).
///
/// Errors:
/// - `StoreIo` if the expected file does not exist (no log record written yet).
#[allow(clippy::missing_errors_doc)]
pub fn today() -> Result<PathBuf, MinosError> {
    // mars-xlog 0.1.0-preview.2 exposes flush via `Xlog::flush_all(sync)` (a
    // doc-hidden but public static) and `Xlog::flush(&self, sync)` on an
    // instance. `XlogLayerHandle` itself has no flush method. We don't retain
    // the `Xlog` instance (only `XlogLayerHandle` in HANDLE), so drive the
    // sync flush through `flush_all` which covers every registered instance.
    if HANDLE.get().is_some() {
        Xlog::flush_all(true);
    }

    let dir = log_dir();
    // mars-xlog filename convention (verified in `mars-xlog-core`
    // `file_manager::build_path_for_index`): `{prefix}_{YYYYMMDD}.xlog` using
    // `chrono::Local` for the day key. Using UTC here would produce an
    // incorrect path on days when UTC and local dates straddle midnight.
    let stamp = chrono::Local::now().format("%Y%m%d").to_string();
    let path = dir.join(format!("{NAME_PREFIX}_{stamp}.xlog"));

    if !path.exists() {
        return Err(MinosError::StoreIo {
            path: path.display().to_string(),
            message: "no log file written yet".to_string(),
        });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};
    use tempfile::{tempdir, TempDir};

    // All three logging tests share process-global state (mars-xlog's
    // registry, the tracing subscriber, and `MINOS_LOG_DIR`). The `Mutex`
    // serializes them, and the `LazyLock<TempDir>` pins a single tempdir
    // that every test can reuse so `Xlog::init` is consistent across calls
    // (mars-xlog rejects re-init with a different dir for the same prefix).
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static SHARED_LOG_DIR: LazyLock<TempDir> = LazyLock::new(|| tempdir().expect("shared tempdir"));

    fn use_shared_log_dir() {
        std::env::set_var("MINOS_LOG_DIR", SHARED_LOG_DIR.path());
    }

    #[test]
    fn init_creates_log_dir_and_emits_once() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Use MINOS_LOG_DIR override so test logs go to a tempdir without
        // mutating process-global HOME (HOME mutation triggers warnings under
        // the upcoming Rust unsafe-env story).
        use_shared_log_dir();
        init().unwrap();
        // Idempotent
        init().unwrap();
        let computed = log_dir();
        assert!(computed.exists());
    }

    #[test]
    fn today_returns_existing_path_after_a_log() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        use_shared_log_dir();
        init().unwrap();

        // Emit one log record so mars-xlog opens the day's file.
        tracing::info!("probe");

        // today() flushes and returns the path.
        let p = today().unwrap();
        assert!(p.to_string_lossy().ends_with(".xlog"));
        assert!(p.exists(), "today() must return an existing file");
    }

    #[test]
    fn today_errors_before_any_log_written() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Point at a *fresh* tempdir that has no xlog file. We explicitly do
        // NOT use the shared log dir here — we want the path to not exist.
        let dir = tempdir().unwrap();
        std::env::set_var("MINOS_LOG_DIR", dir.path());
        // Don't call init() — we want the path to not exist.
        // However, init() is idempotent and static across the test binary, so
        // if a sibling test called init() first, subsequent calls are no-ops
        // and log_dir() still returns the tempdir we just set. This test is
        // rigorous only on fresh test runs; we make the assertion conservative.
        let r = today();
        match r {
            Err(MinosError::StoreIo { .. }) => { /* expected */ }
            Ok(p) => {
                // Prior init() opened a file in a DIFFERENT dir — tolerate that.
                assert!(
                    !p.starts_with(dir.path()),
                    "today() returned a file in the fresh dir despite no init"
                );
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
}
