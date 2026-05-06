//! Durable storage for the host's long-lived `device-secret`.
//!
//! The relay depends on this value to reconnect after pairing. Keep the
//! primary copy in the daemon's own secrets directory so every platform has
//! the same persistence story; on macOS we still mirror to Keychain best-
//! effort for migration and operator inspection.

use std::{
    fs,
    path::{Path, PathBuf},
};

use minos_domain::{DeviceSecret, MinosError};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "device-secret.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedDeviceSecret {
    device_secret: DeviceSecret,
}

pub fn default_path() -> Result<PathBuf, MinosError> {
    Ok(crate::paths::secrets_dir()?.join(FILE_NAME))
}

pub fn read() -> Result<Option<DeviceSecret>, MinosError> {
    let path = default_path()?;
    let from_file = read_file(&path)?;
    if from_file.is_some() {
        return Ok(from_file);
    }

    #[cfg(target_os = "macos")]
    {
        let store = crate::keychain_store::KeychainTrustedDeviceStore;
        match store.read() {
            Ok(Some(secret)) => {
                if let Err(error) = write_file(&path, &secret) {
                    tracing::warn!(
                        target: "minos_daemon::device_secret_store",
                        error = %error,
                        path = %path.display(),
                        "loaded device secret from Keychain but failed to backfill file store"
                    );
                }
                Ok(Some(secret))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                tracing::warn!(
                    target: "minos_daemon::device_secret_store",
                    error = %error,
                    "failed to read device secret from Keychain fallback"
                );
                Ok(None)
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

pub fn write(secret: &DeviceSecret) -> Result<(), MinosError> {
    let path = default_path()?;
    write_file(&path, secret)?;

    #[cfg(target_os = "macos")]
    {
        let store = crate::keychain_store::KeychainTrustedDeviceStore;
        if let Err(error) = store.write(secret) {
            tracing::warn!(
                target: "minos_daemon::device_secret_store",
                error = %error,
                "failed to mirror device secret into Keychain"
            );
        }
    }

    Ok(())
}

pub fn delete() -> Result<(), MinosError> {
    let path = default_path()?;
    delete_file(&path)?;

    #[cfg(target_os = "macos")]
    {
        let store = crate::keychain_store::KeychainTrustedDeviceStore;
        if let Err(error) = store.delete() {
            tracing::warn!(
                target: "minos_daemon::device_secret_store",
                error = %error,
                "failed to delete mirrored device secret from Keychain"
            );
        }
    }

    Ok(())
}

fn read_file(path: &Path) -> Result<Option<DeviceSecret>, MinosError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| MinosError::StoreIo {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let persisted: PersistedDeviceSecret =
        serde_json::from_slice(&bytes).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
    Ok(Some(persisted.device_secret))
}

fn write_file(path: &Path, secret: &DeviceSecret) -> Result<(), MinosError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
            path: parent.display().to_string(),
            message: e.to_string(),
        })?;
    }
    let buf = serde_json::to_vec_pretty(&PersistedDeviceSecret {
        device_secret: secret.clone(),
    })
    .map_err(|e| MinosError::StoreCorrupt {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    fs::write(path, buf).map_err(|e| MinosError::StoreIo {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

fn delete_file(path: &Path) -> Result<(), MinosError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(MinosError::StoreIo {
            path: path.display().to_string(),
            message: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let secret = DeviceSecret("sentinel-secret".into());

        assert!(read_file(&path).unwrap().is_none());
        write_file(&path, &secret).unwrap();
        assert_eq!(read_file(&path).unwrap(), Some(secret.clone()));

        delete_file(&path).unwrap();
        assert!(read_file(&path).unwrap().is_none());
    }

    #[test]
    fn corrupt_file_returns_store_corrupt() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, b"{not-json").unwrap();

        let error = read_file(&path).unwrap_err();
        assert!(matches!(error, MinosError::StoreCorrupt { .. }));
    }
}
