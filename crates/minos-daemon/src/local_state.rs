//! Plain-JSON persistence for the Mac-side non-secret state:
//! `self_device_id` (UUIDv4) + `peer` (nullable `PeerRecord`).
//! Secrets (CF tokens, device_secret) are NOT stored here — they go to
//! the Keychain via `keychain_store.rs`.

use crate::relay_pairing::PeerRecord;
use minos_domain::{DeviceId, MinosError};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalState {
    pub self_device_id: DeviceId,
    pub peer: Option<PeerRecord>,
}

impl LocalState {
    pub fn default_path() -> Result<PathBuf, MinosError> {
        Ok(crate::paths::state_dir()?.join("local-state.json"))
    }

    /// Load or initialize. If missing, create fresh with a new DeviceId.
    /// If present but unparseable, return `StoreCorrupt` — caller surfaces
    /// as a bootError; user deletes the file manually.
    pub fn load_or_init(path: &Path) -> Result<Self, MinosError> {
        if !path.exists() {
            let state = Self {
                self_device_id: DeviceId::new(),
                peer: None,
            };
            state.save(path)?;
            return Ok(state);
        }
        let bytes = fs::read(path).map_err(|e| MinosError::StoreIo {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        serde_json::from_slice(&bytes).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), MinosError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
                path: parent.display().to_string(),
                message: e.to_string(),
            })?;
        }
        let buf = serde_json::to_vec_pretty(self).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        fs::write(path, buf).map_err(|e| MinosError::StoreIo {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_or_init_creates_fresh_state_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        let s = LocalState::load_or_init(&p).unwrap();
        assert!(s.peer.is_none());
        assert!(p.exists());
    }

    #[test]
    fn load_round_trips_peer() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        let original = LocalState {
            self_device_id: DeviceId::new(),
            peer: Some(PeerRecord {
                device_id: DeviceId::new(),
                name: "iPhone".into(),
                paired_at: chrono::Utc::now(),
            }),
        };
        original.save(&p).unwrap();
        let back = LocalState::load_or_init(&p).unwrap();
        assert_eq!(original.self_device_id, back.self_device_id);
        assert_eq!(original.peer.as_ref().unwrap().name, "iPhone");
    }

    #[test]
    fn load_on_corrupt_file_returns_store_corrupt() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("local-state.json");
        fs::write(&p, b"{this is not json").unwrap();
        let err = LocalState::load_or_init(&p).unwrap_err();
        assert!(matches!(err, MinosError::StoreCorrupt { .. }));
    }
}
