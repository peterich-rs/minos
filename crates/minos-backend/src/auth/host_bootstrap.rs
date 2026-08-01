use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dashmap::DashMap;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use minos_domain::{DeviceId, DeviceRole};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{device_installations, AsStorePool};

pub const BOOTSTRAP_NONCE_TTL: Duration = Duration::from_secs(60);
const PUBLIC_KEY_PREFIX: &str = "ed25519:";
const SIGNATURE_PREFIX: &str = "ed25519-sig:";

#[derive(Debug, Clone)]
pub struct BootstrapNonce {
    pub nonce: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct NonceEntry {
    installation_id: String,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
enum BootstrapNonceBackend {
    InMemory(Arc<DashMap<String, NonceEntry>>),
    Redis { client: redis::Client },
}

/// Single-use bootstrap nonces for host Ed25519 proofs.
///
/// Production multi-instance deployments back this with Redis (`GETDEL` on
/// consume). Dev/tests use an in-memory map. Both share the same issue/consume
/// contract so callers do not branch on backend kind.
#[derive(Debug, Clone)]
pub struct BootstrapNonceStore {
    backend: BootstrapNonceBackend,
}

impl Default for BootstrapNonceStore {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl BootstrapNonceStore {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            backend: BootstrapNonceBackend::InMemory(Arc::new(DashMap::new())),
        }
    }

    pub fn redis(redis_url: &str) -> Result<Self, BackendError> {
        let client = redis::Client::open(redis_url).map_err(|error| BackendError::Cache {
            operation: "bootstrap_nonce.redis_client".into(),
            message: error.to_string(),
        })?;
        Ok(Self {
            backend: BootstrapNonceBackend::Redis { client },
        })
    }

    pub async fn issue(
        &self,
        installation_id: &str,
        now_ms: i64,
    ) -> Result<BootstrapNonce, BackendError> {
        let nonce = generate_nonce();
        let expires_at_ms = now_ms + BOOTSTRAP_NONCE_TTL.as_millis() as i64;
        match &self.backend {
            BootstrapNonceBackend::InMemory(entries) => {
                entries.insert(
                    nonce.clone(),
                    NonceEntry {
                        installation_id: installation_id.to_string(),
                        expires_at_ms,
                    },
                );
            }
            BootstrapNonceBackend::Redis { client } => {
                let mut conn =
                    client
                        .get_multiplexed_async_connection()
                        .await
                        .map_err(|error| BackendError::Cache {
                            operation: "bootstrap_nonce.redis_connect".into(),
                            message: error.to_string(),
                        })?;
                let ttl_secs = BOOTSTRAP_NONCE_TTL.as_secs().max(1);
                let _: () = redis::cmd("SET")
                    .arg(nonce_key(&nonce))
                    .arg(installation_id)
                    .arg("EX")
                    .arg(ttl_secs)
                    .query_async(&mut conn)
                    .await
                    .map_err(|error| BackendError::Cache {
                        operation: "bootstrap_nonce.redis_set".into(),
                        message: error.to_string(),
                    })?;
            }
        }
        Ok(BootstrapNonce {
            nonce,
            expires_at_ms,
        })
    }

    pub async fn consume(
        &self,
        installation_id: &str,
        nonce: &str,
        now_ms: i64,
    ) -> Result<(), HostBootstrapError> {
        let entry_installation_id = match &self.backend {
            BootstrapNonceBackend::InMemory(entries) => {
                let Some((_, entry)) = entries.remove(nonce) else {
                    return Err(HostBootstrapError::NonceInvalid);
                };
                if entry.expires_at_ms <= now_ms {
                    return Err(HostBootstrapError::NonceInvalid);
                }
                entry.installation_id
            }
            BootstrapNonceBackend::Redis { client } => {
                let mut conn = client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|error| {
                        HostBootstrapError::Store(BackendError::Cache {
                            operation: "bootstrap_nonce.redis_connect".into(),
                            message: error.to_string(),
                        })
                    })?;
                let payload: Option<String> = redis::cmd("GETDEL")
                    .arg(nonce_key(nonce))
                    .query_async(&mut conn)
                    .await
                    .map_err(|error| {
                        HostBootstrapError::Store(BackendError::Cache {
                            operation: "bootstrap_nonce.redis_getdel".into(),
                            message: error.to_string(),
                        })
                    })?;
                payload.ok_or(HostBootstrapError::NonceInvalid)?
            }
        };

        if entry_installation_id != installation_id {
            return Err(HostBootstrapError::NonceInvalid);
        }
        Ok(())
    }
}

fn nonce_key(nonce: &str) -> String {
    format!("minos:bootstrap_nonce:{nonce}")
}

pub struct HostBootstrapProof<'a> {
    pub installation_id: &'a str,
    pub nonce: &'a str,
    pub public_key: Option<&'a str>,
    pub signature: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum HostBootstrapError {
    #[error("host bootstrap nonce invalid")]
    NonceInvalid,
    #[error("host bootstrap proof invalid")]
    ProofInvalid,
    #[error("host bootstrap public key mismatch")]
    PublicKeyMismatch,
    #[error("store error: {0}")]
    Store(#[from] BackendError),
}

pub async fn verify_and_register<S>(
    store: &S,
    nonce_store: &BootstrapNonceStore,
    proof: HostBootstrapProof<'_>,
    path: &str,
    display_name: &str,
    now_ms: i64,
) -> Result<DeviceId, HostBootstrapError>
where
    S: AsStorePool,
{
    let installation_id = Uuid::parse_str(proof.installation_id)
        .map(DeviceId)
        .map_err(|_| HostBootstrapError::ProofInvalid)?;
    nonce_store
        .consume(proof.installation_id, proof.nonce, now_ms)
        .await?;

    let existing = device_installations::get_device(store, installation_id).await?;
    if let Some(row) = existing.as_ref() {
        if row.role != DeviceRole::AgentHost {
            return Err(HostBootstrapError::ProofInvalid);
        }
    }

    let stored_public_key = existing.as_ref().and_then(|row| row.public_key.as_deref());
    let public_key_text = match (stored_public_key, proof.public_key) {
        (Some(stored), Some(provided)) if stored != provided => {
            return Err(HostBootstrapError::PublicKeyMismatch);
        }
        (Some(stored), _) => stored,
        (None, Some(provided)) => provided,
        (None, None) => return Err(HostBootstrapError::ProofInvalid),
    };

    verify_signature(
        public_key_text,
        proof.signature,
        proof.installation_id,
        proof.nonce,
        path,
    )?;

    if existing.is_none() {
        // Postgres CHECK requires host rows to have public_key NOT NULL.
        // Insert key atomically at TOFU register rather than insert-then-patch.
        device_installations::insert_host_with_public_key(
            store,
            installation_id,
            display_name,
            public_key_text,
            now_ms,
        )
        .await?;
    } else if stored_public_key.is_none() {
        device_installations::set_public_key_if_absent(store, &installation_id, public_key_text)
            .await?;
    }

    Ok(installation_id)
}

fn verify_signature(
    public_key_text: &str,
    signature_text: &str,
    installation_id: &str,
    nonce: &str,
    path: &str,
) -> Result<(), HostBootstrapError> {
    let public_key = decode_prefixed(public_key_text, PUBLIC_KEY_PREFIX, 32)?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| HostBootstrapError::ProofInvalid)?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| HostBootstrapError::ProofInvalid)?;

    let signature = decode_prefixed(signature_text, SIGNATURE_PREFIX, 64)?;
    let signature =
        Signature::from_slice(&signature).map_err(|_| HostBootstrapError::ProofInvalid)?;
    let payload = format!("{installation_id}:{nonce}:{path}");
    verifying_key
        .verify(payload.as_bytes(), &signature)
        .map_err(|_| HostBootstrapError::ProofInvalid)
}

fn decode_prefixed(
    text: &str,
    prefix: &str,
    expected_len: usize,
) -> Result<Vec<u8>, HostBootstrapError> {
    let encoded = text
        .strip_prefix(prefix)
        .ok_or(HostBootstrapError::ProofInvalid)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| HostBootstrapError::ProofInvalid)?;
    if decoded.len() != expected_len {
        return Err(HostBootstrapError::ProofInvalid);
    }
    Ok(decoded)
}

fn generate_nonce() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    format!("nonce_{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const REQUEST_CODE_PATH: &str = "/v1/host/pairing/request-code";

    fn keypair() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn public_key(signing_key: &SigningKey) -> String {
        format!(
            "{PUBLIC_KEY_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes())
        )
    }

    fn signature(signing_key: &SigningKey, installation_id: &str, nonce: &str) -> String {
        let payload = format!("{installation_id}:{nonce}:{REQUEST_CODE_PATH}");
        format!(
            "{SIGNATURE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(signing_key.sign(payload.as_bytes()).to_bytes())
        )
    }

    #[tokio::test]
    async fn verify_and_register_tofu_stores_public_key() {
        let pool = crate::store::test_support::memory_pool().await;
        let store = BootstrapNonceStore::default();
        let installation_id = DeviceId::new().to_string();
        let nonce = store.issue(&installation_id, 100).await.unwrap().nonce;
        let host_signing_key = keypair();
        let host_public_key = public_key(&host_signing_key);
        let host_signature = signature(&host_signing_key, &installation_id, &nonce);

        let id = verify_and_register(
            &pool,
            &store,
            HostBootstrapProof {
                installation_id: &installation_id,
                nonce: &nonce,
                public_key: Some(&host_public_key),
                signature: &host_signature,
            },
            REQUEST_CODE_PATH,
            "host",
            100,
        )
        .await
        .unwrap();

        let row = device_installations::get_device(&pool, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.public_key.as_deref(), Some(host_public_key.as_str()));
        assert_eq!(row.role, DeviceRole::AgentHost);
    }

    #[tokio::test]
    async fn nonce_is_single_use() {
        let pool = crate::store::test_support::memory_pool().await;
        let store = BootstrapNonceStore::default();
        let installation_id = DeviceId::new().to_string();
        let nonce = store.issue(&installation_id, 100).await.unwrap().nonce;
        let host_signing_key = keypair();
        let host_public_key = public_key(&host_signing_key);
        let host_signature = signature(&host_signing_key, &installation_id, &nonce);

        verify_and_register(
            &pool,
            &store,
            HostBootstrapProof {
                installation_id: &installation_id,
                nonce: &nonce,
                public_key: Some(&host_public_key),
                signature: &host_signature,
            },
            REQUEST_CODE_PATH,
            "host",
            100,
        )
        .await
        .unwrap();

        let err = verify_and_register(
            &pool,
            &store,
            HostBootstrapProof {
                installation_id: &installation_id,
                nonce: &nonce,
                public_key: Some(&host_public_key),
                signature: &host_signature,
            },
            REQUEST_CODE_PATH,
            "host",
            100,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, HostBootstrapError::NonceInvalid));
    }

    #[tokio::test]
    async fn public_key_mismatch_is_rejected() {
        let pool = crate::store::test_support::memory_pool().await;
        let store = BootstrapNonceStore::default();
        let installation_id = DeviceId::new().to_string();
        let host_signing_key = keypair();
        let host_public_key = public_key(&host_signing_key);
        let nonce = store.issue(&installation_id, 100).await.unwrap().nonce;
        let host_signature = signature(&host_signing_key, &installation_id, &nonce);

        verify_and_register(
            &pool,
            &store,
            HostBootstrapProof {
                installation_id: &installation_id,
                nonce: &nonce,
                public_key: Some(&host_public_key),
                signature: &host_signature,
            },
            REQUEST_CODE_PATH,
            "host",
            100,
        )
        .await
        .unwrap();

        let different_key = SigningKey::from_bytes(&[9_u8; 32]);
        let different_public_key = public_key(&different_key);
        let nonce = store.issue(&installation_id, 200).await.unwrap().nonce;
        let signature = signature(&different_key, &installation_id, &nonce);

        let err = verify_and_register(
            &pool,
            &store,
            HostBootstrapProof {
                installation_id: &installation_id,
                nonce: &nonce,
                public_key: Some(&different_public_key),
                signature: &signature,
            },
            REQUEST_CODE_PATH,
            "host",
            200,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, HostBootstrapError::PublicKeyMismatch));
    }
}
