//! `PairingStore` impl backed by a JSON file under
//! `~/Library/Application Support/minos/devices.json` (Mac convention).
//!
//! On parse failure, the existing file is renamed to `.bak` and `load()`
//! returns `MinosError::StoreCorrupt` so the daemon can surface it to UI.

use std::fs;
use std::path::PathBuf;

use minos_domain::MinosError;
use minos_pairing::{PairingStore, TrustedDevice};

pub struct FilePairingStore {
    path: PathBuf,
}

impl FilePairingStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Default Mac path: `~/Library/Application Support/minos/devices.json`.
    /// On non-Mac targets (e.g. CI Linux), falls back to `$HOME/.minos/devices.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        if cfg!(target_os = "macos") {
            PathBuf::from(home).join("Library/Application Support/minos/devices.json")
        } else {
            PathBuf::from(home).join(".minos/devices.json")
        }
    }
}

impl PairingStore for FilePairingStore {
    fn load(&self) -> Result<Vec<TrustedDevice>, MinosError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let bytes = fs::read(&self.path).map_err(|e| MinosError::StoreIo {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })?;
        match serde_json::from_slice::<Vec<TrustedDevice>>(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                let bak = self.path.with_extension("json.bak");
                let _ = fs::rename(&self.path, &bak);
                Err(MinosError::StoreCorrupt {
                    path: self.path.display().to_string(),
                    message: e.to_string(),
                })
            }
        }
    }

    fn save(&self, devices: &[TrustedDevice]) -> Result<(), MinosError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
                path: parent.display().to_string(),
                message: e.to_string(),
            })?;
        }
        let json = serde_json::to_vec_pretty(devices).map_err(|e| MinosError::StoreCorrupt {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })?;
        fs::write(&self.path, json).map_err(|e| MinosError::StoreIo {
            path: self.path.display().to_string(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use minos_domain::DeviceId;

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePairingStore::new(dir.path().join("d.json"));
        let dev = TrustedDevice {
            device_id: DeviceId::new(),
            name: "iPhone".into(),
            host: "100.64.0.42".into(),
            port: 7878,
            paired_at: Utc::now(),
        };
        store.save(&[dev.clone()]).unwrap();
        let back = store.load().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].device_id, dev.device_id);
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePairingStore::new(dir.path().join("never.json"));
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn corrupt_file_renamed_to_bak_and_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.json");
        fs::write(&path, b"not json").unwrap();
        let store = FilePairingStore::new(path.clone());
        let r = store.load();
        assert!(matches!(r, Err(MinosError::StoreCorrupt { .. })));
        assert!(!path.exists());
        assert!(path.with_extension("json.bak").exists());
    }
}
