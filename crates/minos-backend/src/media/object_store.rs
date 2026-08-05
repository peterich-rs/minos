//! Object storage backends: Cloudflare R2 (S3 API) and local directory.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

/// Trait for blob put/get/delete. Keys are relative object keys (no leading slash).
#[async_trait]
pub trait BlobObjectStore: Send + Sync {
    async fn put(&self, key: &str, body: Bytes, content_type: &str) -> Result<(), String>;
    async fn get(&self, key: &str) -> Result<Bytes, String>;
    async fn delete(&self, key: &str) -> Result<(), String>;
}

#[derive(Clone)]
pub enum ObjectStoreKind {
    R2(Arc<R2ObjectStore>),
    Local(Arc<LocalDirObjectStore>),
}

impl std::fmt::Debug for ObjectStoreKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl ObjectStoreKind {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::R2(_) => "r2",
            Self::Local(_) => "local",
        }
    }

    pub async fn put(&self, key: &str, body: Bytes, content_type: &str) -> Result<(), String> {
        match self {
            Self::R2(s) => s.put(key, body, content_type).await,
            Self::Local(s) => s.put(key, body, content_type).await,
        }
    }

    pub async fn get(&self, key: &str) -> Result<Bytes, String> {
        match self {
            Self::R2(s) => s.get(key).await,
            Self::Local(s) => s.get(key).await,
        }
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        match self {
            Self::R2(s) => s.delete(key).await,
            Self::Local(s) => s.delete(key).await,
        }
    }
}

// ── Local directory (dev / tests) ──────────────────────────────────────

pub struct LocalDirObjectStore {
    root: PathBuf,
}

impl LocalDirObjectStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `MINOS_MEDIA_LOCAL_DIR` → Some(store).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var("MINOS_MEDIA_LOCAL_DIR").ok()?;
        let dir = dir.trim();
        if dir.is_empty() {
            return None;
        }
        Some(Self::new(PathBuf::from(dir)))
    }

    fn resolve(&self, key: &str) -> Result<PathBuf, String> {
        if key.is_empty() || key.starts_with('/') || key.contains("..") || key.contains('\\') {
            return Err("invalid object key".into());
        }
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl BlobObjectStore for LocalDirObjectStore {
    async fn put(&self, key: &str, body: Bytes, _content_type: &str) -> Result<(), String> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create_dir_all: {e}"))?;
        }
        tokio::fs::write(&path, &body)
            .await
            .map_err(|e| format!("write: {e}"))
    }

    async fn get(&self, key: &str) -> Result<Bytes, String> {
        let path = self.resolve(key)?;
        let data = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("read: {e}"))?;
        Ok(Bytes::from(data))
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.resolve(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("delete: {e}")),
        }
    }
}

// ── Cloudflare R2 via S3-compatible API (aws-sdk-s3) ───────────────────

pub struct R2ObjectStore {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl R2ObjectStore {
    /// Build from env. Returns `Ok(None)` when R2 vars are absent; `Err` when partial/invalid.
    pub fn from_env() -> Result<Option<Self>, String> {
        let account_id = env_opt("MINOS_R2_ACCOUNT_ID");
        let access_key = env_opt("MINOS_R2_ACCESS_KEY_ID");
        let secret_key = env_opt("MINOS_R2_SECRET_ACCESS_KEY");
        let bucket = env_opt("MINOS_R2_BUCKET");

        let present = [
            account_id.is_some(),
            access_key.is_some(),
            secret_key.is_some(),
            bucket.is_some(),
        ];
        if present.iter().all(|p| !p) {
            return Ok(None);
        }
        if !present.iter().all(|p| *p) {
            return Err(
                "incomplete R2 config: require MINOS_R2_ACCOUNT_ID, MINOS_R2_ACCESS_KEY_ID, MINOS_R2_SECRET_ACCESS_KEY, MINOS_R2_BUCKET"
                    .into(),
            );
        }

        let account_id = account_id.unwrap();
        let access_key = access_key.unwrap();
        let secret_key = secret_key.unwrap();
        let bucket = bucket.unwrap();

        let endpoint = env_opt("MINOS_R2_ENDPOINT")
            .unwrap_or_else(|| format!("https://{account_id}.r2.cloudflarestorage.com"));

        Self::new(&endpoint, &access_key, &secret_key, &bucket).map(Some)
    }

    pub fn new(
        endpoint: &str,
        access_key_id: &str,
        secret_access_key: &str,
        bucket: &str,
    ) -> Result<Self, String> {
        use aws_credential_types::Credentials;
        use aws_sdk_s3::config::{BehaviorVersion, Region};

        let creds = Credentials::new(access_key_id, secret_access_key, None, None, "minos-r2-env");
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(endpoint)
            .credentials_provider(creds)
            .force_path_style(true)
            .build();
        let client = aws_sdk_s3::Client::from_conf(conf);
        Ok(Self {
            client,
            bucket: bucket.to_string(),
        })
    }
}

#[async_trait]
impl BlobObjectStore for R2ObjectStore {
    async fn put(&self, key: &str, body: Bytes, content_type: &str) -> Result<(), String> {
        use aws_sdk_s3::primitives::ByteStream;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(body.to_vec()))
            .send()
            .await
            .map_err(|e| format!("r2 put_object: {e}"))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Bytes, String> {
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("r2 get_object: {e}"))?;
        let data = out
            .body
            .collect()
            .await
            .map_err(|e| format!("r2 body collect: {e}"))?
            .into_bytes();
        Ok(data)
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("r2 delete_object: {e}"))?;
        Ok(())
    }
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn local_put_get_delete() {
        let dir = tempdir().unwrap();
        let store = LocalDirObjectStore::new(dir.path().to_path_buf());
        store
            .put("a/b.txt", Bytes::from_static(b"hello"), "text/plain")
            .await
            .unwrap();
        let got = store.get("a/b.txt").await.unwrap();
        assert_eq!(got.as_ref(), b"hello");
        store.delete("a/b.txt").await.unwrap();
        assert!(store.get("a/b.txt").await.is_err());
    }

    #[test]
    fn rejects_path_traversal() {
        let store = LocalDirObjectStore::new(PathBuf::from("/tmp"));
        assert!(store.resolve("../etc/passwd").is_err());
    }
}
