//! Durable storage for the host bootstrap Ed25519 signing key.
//!
//! The backend stores the public half using TOFU for each host installation.
//! The private seed must stay stable across restarts so the same installation
//! can keep proving ownership of its `self_device_id`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::SigningKey;
use minos_domain::MinosError;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "host-bootstrap-key.json";
const VERSION: u32 = 1;
const ALGORITHM: &str = "ed25519";
const PRIVATE_KEY_PREFIX: &str = "ed25519-seed:";
const PUBLIC_KEY_PREFIX: &str = "ed25519:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedHostBootstrapKey {
    version: u32,
    algorithm: String,
    private_key_seed: String,
    public_key: String,
}

pub fn default_path() -> Result<PathBuf, MinosError> {
    Ok(crate::paths::secrets_dir()?.join(FILE_NAME))
}

pub fn load_or_generate() -> Result<SigningKey, MinosError> {
    let path = default_path()?;
    load_or_generate_file(&path)
}

fn load_or_generate_file(path: &Path) -> Result<SigningKey, MinosError> {
    if path.exists() {
        let key = read_file(path)?;
        tracing::info!(
            target: "minos_daemon::host_bootstrap",
            path = %path.display(),
            "host bootstrap signing key loaded"
        );
        return Ok(key);
    }
    let key = generate_key()?;
    write_file(path, &key)?;
    tracing::info!(
        target: "minos_daemon::host_bootstrap",
        path = %path.display(),
        "host bootstrap signing key generated"
    );
    Ok(key)
}

fn generate_key() -> Result<SigningKey, MinosError> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|e| MinosError::StoreIo {
        path: FILE_NAME.into(),
        message: format!("generate host bootstrap key: {e}"),
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

fn read_file(path: &Path) -> Result<SigningKey, MinosError> {
    let bytes = fs::read(path).map_err(|e| MinosError::StoreIo {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let persisted: PersistedHostBootstrapKey =
        serde_json::from_slice(&bytes).map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
    if persisted.version != VERSION || persisted.algorithm != ALGORITHM {
        return Err(MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: "unsupported host bootstrap key format".into(),
        });
    }
    let key = signing_key_from_seed(path, &persisted.private_key_seed)?;
    let expected_public_key = public_key_text(&key);
    if persisted.public_key != expected_public_key {
        return Err(MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: "host bootstrap public key does not match private seed".into(),
        });
    }
    Ok(key)
}

fn write_file(path: &Path, key: &SigningKey) -> Result<(), MinosError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| MinosError::StoreIo {
            path: parent.display().to_string(),
            message: e.to_string(),
        })?;
    }
    let persisted = PersistedHostBootstrapKey {
        version: VERSION,
        algorithm: ALGORITHM.into(),
        private_key_seed: format!(
            "{PRIVATE_KEY_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(key.to_bytes())
        ),
        public_key: public_key_text(key),
    };
    let bytes = serde_json::to_vec_pretty(&persisted).map_err(|e| MinosError::StoreCorrupt {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    fs::write(path, bytes).map_err(|e| MinosError::StoreIo {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

fn signing_key_from_seed(path: &Path, text: &str) -> Result<SigningKey, MinosError> {
    let encoded =
        text.strip_prefix(PRIVATE_KEY_PREFIX)
            .ok_or_else(|| MinosError::StoreCorrupt {
                path: path.display().to_string(),
                message: "host bootstrap private seed prefix is invalid".into(),
            })?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|e| MinosError::StoreCorrupt {
            path: path.display().to_string(),
            message: format!("decode host bootstrap private seed: {e}"),
        })?;
    let seed: [u8; 32] = decoded.try_into().map_err(|_| MinosError::StoreCorrupt {
        path: path.display().to_string(),
        message: "host bootstrap private seed length is invalid".into(),
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

fn public_key_text(key: &SigningKey) -> String {
    format!(
        "{PUBLIC_KEY_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_generate_creates_and_reuses_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);

        let first = load_or_generate_file(&path).unwrap();
        let second = load_or_generate_file(&path).unwrap();

        assert_eq!(first.to_bytes(), second.to_bytes());
        assert!(path.exists());
    }

    #[test]
    fn corrupt_json_returns_store_corrupt() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, b"{not-json").unwrap();

        let error = load_or_generate_file(&path).unwrap_err();

        assert!(matches!(error, MinosError::StoreCorrupt { .. }));
    }

    #[test]
    fn mismatched_public_key_returns_store_corrupt() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let key = generate_key().unwrap();
        write_file(&path, &key).unwrap();
        let mut persisted: PersistedHostBootstrapKey =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        persisted.public_key = format!("{PUBLIC_KEY_PREFIX}bad");
        fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();

        let error = load_or_generate_file(&path).unwrap_err();

        assert!(matches!(error, MinosError::StoreCorrupt { .. }));
    }
}
