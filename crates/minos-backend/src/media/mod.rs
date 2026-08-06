//! Media blob service: metadata in SQL, object bytes in R2 or local dir.
//!
//! ## Configuration
//!
//! Prefer Cloudflare R2 (S3-compatible). When R2 env is incomplete, fall back
//! to a local directory for dev/tests. When neither is configured, the service
//! is present but all mutating APIs return `MediaError::NotConfigured`.
//!
//! | Env | Purpose |
//! |-----|---------|
//! | `MINOS_R2_ACCOUNT_ID` | Cloudflare account id (endpoint host) |
//! | `MINOS_R2_ACCESS_KEY_ID` | R2 API token access key |
//! | `MINOS_R2_SECRET_ACCESS_KEY` | R2 API token secret |
//! | `MINOS_R2_BUCKET` | Bucket name |
//! | `MINOS_R2_ENDPOINT` | Optional override (default `https://{account}.r2.cloudflarestorage.com`) |
//! | `MINOS_MEDIA_LOCAL_DIR` | Local filesystem root when R2 is unset |
//! | `MINOS_MEDIA_MAX_BYTES` | Max object size (default 10 MiB) |
//! | `MINOS_MEDIA_DOWNLOAD_TTL_SECS` | Signed download TTL (default 900) |

mod object_store;
mod validation;

pub use object_store::{BlobObjectStore, LocalDirObjectStore, ObjectStoreKind, R2ObjectStore};
pub use validation::{infer_kind, validate_content_type, validate_upload_declaration, MediaKind};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::BackendError;
use crate::store::{self, media_blobs::MediaBlobRow, StoreHandle};

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_DOWNLOAD_TTL_SECS: u64 = 900;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media storage is not configured (set MINOS_R2_* or MINOS_MEDIA_LOCAL_DIR)")]
    NotConfigured,
    #[error("invalid media request: {0}")]
    Invalid(String),
    #[error("media blob not found")]
    NotFound,
    #[error("media blob not ready")]
    NotReady,
    #[error("forbidden")]
    Forbidden,
    #[error("payload too large")]
    TooLarge,
    #[error("payload size mismatch")]
    SizeMismatch,
    #[error("object store error: {0}")]
    ObjectStore(String),
    #[error(transparent)]
    Backend(#[from] BackendError),
}

impl MediaError {
    pub fn status_code(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            Self::NotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            Self::Invalid(_) | Self::SizeMismatch => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NotReady => StatusCode::CONFLICT,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ObjectStore(_) | Self::Backend(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "media_not_configured",
            Self::Invalid(_) => "invalid_request",
            Self::NotFound => "not_found",
            Self::NotReady => "not_ready",
            Self::Forbidden => "forbidden",
            Self::TooLarge => "payload_too_large",
            Self::SizeMismatch => "size_mismatch",
            Self::ObjectStore(_) => "object_store_error",
            Self::Backend(_) => "internal_error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub max_bytes: u64,
    pub download_ttl: Duration,
    pub backend: Option<ObjectStoreKind>,
    /// Public base URL for building download links (optional; clients may use relative paths).
    pub public_base_url: Option<String>,
}

impl MediaConfig {
    /// Resolve from process environment. R2 wins when fully specified; else local dir.
    #[must_use]
    pub fn from_env() -> Self {
        let max_bytes = std::env::var("MINOS_MEDIA_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_BYTES);
        let download_ttl = Duration::from_secs(
            std::env::var("MINOS_MEDIA_DOWNLOAD_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_DOWNLOAD_TTL_SECS),
        );
        let public_base_url = std::env::var("MINOS_MEDIA_PUBLIC_BASE_URL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());

        let backend = match R2ObjectStore::from_env() {
            Ok(Some(r2)) => Some(ObjectStoreKind::R2(Arc::new(r2))),
            Ok(None) => {
                LocalDirObjectStore::from_env().map(|local| ObjectStoreKind::Local(Arc::new(local)))
            }
            Err(e) => {
                tracing::error!(
                    target: "minos_backend::media",
                    error = %e,
                    "R2 media config invalid; media disabled until fixed"
                );
                None
            }
        };

        if let Some(kind) = &backend {
            tracing::info!(
                target: "minos_backend::media",
                backend = kind.name(),
                max_bytes,
                "media object store ready"
            );
        } else {
            tracing::warn!(
                target: "minos_backend::media",
                "media object store not configured (set MINOS_R2_* or MINOS_MEDIA_LOCAL_DIR)"
            );
        }

        Self {
            max_bytes,
            download_ttl,
            backend,
            public_base_url,
        }
    }

    /// Test / explicit construction helper.
    #[must_use]
    pub fn local_for_tests(dir: PathBuf, max_bytes: u64) -> Self {
        Self {
            max_bytes,
            download_ttl: Duration::from_secs(DEFAULT_DOWNLOAD_TTL_SECS),
            backend: Some(ObjectStoreKind::Local(Arc::new(LocalDirObjectStore::new(
                dir,
            )))),
            public_base_url: None,
        }
    }
}

#[derive(Clone)]
pub struct MediaService {
    store: StoreHandle,
    config: Arc<MediaConfig>,
    /// HMAC secret for short-lived download tokens (reuse JWT secret).
    token_secret: Arc<str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaBlobDto {
    pub blob_id: String,
    pub content_type: String,
    pub byte_size: i64,
    pub kind: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256_hex: Option<String>,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_at_ms: Option<i64>,
}

impl From<&MediaBlobRow> for MediaBlobDto {
    fn from(row: &MediaBlobRow) -> Self {
        Self {
            blob_id: row.blob_id.clone(),
            content_type: row.content_type.clone(),
            byte_size: row.byte_size,
            kind: row.kind.clone(),
            status: row.status.clone(),
            original_filename: row.original_filename.clone(),
            sha256_hex: row.sha256_hex.clone(),
            created_at_ms: row.created_at_ms,
            ready_at_ms: row.ready_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateUploadResult {
    pub blob: MediaBlobDto,
    /// Relative path clients PUT raw bytes to (with account bearer).
    pub upload_path: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub blob: MediaBlobDto,
    /// Relative path or absolute URL to stream content.
    pub download_url: String,
    pub expires_at_ms: i64,
}

impl MediaService {
    #[must_use]
    pub fn new(store: StoreHandle, config: MediaConfig, token_secret: impl Into<String>) -> Self {
        Self {
            store,
            config: Arc::new(config),
            token_secret: Arc::from(token_secret.into()),
        }
    }

    #[must_use]
    pub fn from_env(store: StoreHandle, token_secret: impl Into<String>) -> Self {
        Self::new(store, MediaConfig::from_env(), token_secret)
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.config.backend.is_some()
    }

    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.config
            .backend
            .as_ref()
            .map(ObjectStoreKind::name)
            .unwrap_or("disabled")
    }

    #[must_use]
    pub fn config_max_bytes(&self) -> u64 {
        self.config.max_bytes
    }

    fn require_backend(&self) -> Result<&ObjectStoreKind, MediaError> {
        self.config
            .backend
            .as_ref()
            .ok_or(MediaError::NotConfigured)
    }

    pub async fn create_upload(
        &self,
        account_id: &str,
        content_type: &str,
        byte_size: u64,
        original_filename: Option<&str>,
    ) -> Result<CreateUploadResult, MediaError> {
        self.require_backend()?;
        let content_type = validate_content_type(content_type).map_err(MediaError::Invalid)?;
        validate_upload_declaration(byte_size, self.config.max_bytes)
            .map_err(MediaError::Invalid)?;
        if byte_size > self.config.max_bytes {
            return Err(MediaError::TooLarge);
        }
        let kind = infer_kind(&content_type, original_filename);
        let blob_id = Uuid::new_v4().to_string();
        let object_key = format!(
            "accounts/{account_id}/{}/{blob_id}{}",
            kind.as_str(),
            extension_for(&content_type, original_filename)
        );
        let at_ms = now_ms();
        let row = store::media_blobs::insert_pending(
            &self.store,
            &blob_id,
            account_id,
            &object_key,
            &content_type,
            i64::try_from(byte_size).unwrap_or(i64::MAX),
            original_filename,
            kind.as_str(),
            at_ms,
        )
        .await?;
        Ok(CreateUploadResult {
            blob: MediaBlobDto::from(&row),
            upload_path: format!("/v1/media/blobs/{blob_id}/content"),
            max_bytes: self.config.max_bytes,
        })
    }

    pub async fn put_content(
        &self,
        account_id: &str,
        blob_id: &str,
        body: bytes::Bytes,
        content_type_header: Option<&str>,
    ) -> Result<MediaBlobDto, MediaError> {
        let backend = self.require_backend()?;
        let row = store::media_blobs::get_by_id(&self.store, blob_id)
            .await?
            .ok_or(MediaError::NotFound)?;
        if row.account_id != account_id {
            return Err(MediaError::Forbidden);
        }
        if row.status != "pending" {
            return Err(MediaError::Invalid(format!(
                "blob status is '{}', expected pending",
                row.status
            )));
        }
        let expected = u64::try_from(row.byte_size).unwrap_or(0);
        if body.len() as u64 != expected {
            return Err(MediaError::SizeMismatch);
        }
        if body.len() as u64 > self.config.max_bytes {
            return Err(MediaError::TooLarge);
        }
        if let Some(ct) = content_type_header {
            let ct = ct
                .split(';')
                .next()
                .unwrap_or(ct)
                .trim()
                .to_ascii_lowercase();
            if !ct.is_empty() && ct != row.content_type {
                return Err(MediaError::Invalid(
                    "Content-Type does not match upload declaration".into(),
                ));
            }
        }

        let mut hasher = Sha256::new();
        hasher.update(&body);
        let sha256_hex = format!("{:x}", hasher.finalize());

        if let Err(e) = backend
            .put(&row.object_key, body.clone(), &row.content_type)
            .await
        {
            let _ = store::media_blobs::mark_failed(&self.store, blob_id, now_ms()).await;
            return Err(MediaError::ObjectStore(e));
        }

        let ready = store::media_blobs::mark_ready(
            &self.store,
            blob_id,
            row.byte_size,
            Some(&sha256_hex),
            now_ms(),
        )
        .await?
        .ok_or(MediaError::NotFound)?;
        Ok(MediaBlobDto::from(&ready))
    }

    pub async fn get_download(
        &self,
        account_id: &str,
        blob_id: &str,
    ) -> Result<DownloadResult, MediaError> {
        self.require_backend()?;
        let row = store::media_blobs::get_by_id(&self.store, blob_id)
            .await?
            .ok_or(MediaError::NotFound)?;
        if row.account_id != account_id {
            return Err(MediaError::Forbidden);
        }
        if row.status != "ready" {
            return Err(MediaError::NotReady);
        }
        let expires_at_ms =
            now_ms() + i64::try_from(self.config.download_ttl.as_millis()).unwrap_or(900_000);
        let token = self.sign_download_token(blob_id, account_id, expires_at_ms);
        let path = format!("/v1/media/blobs/{blob_id}/content?token={token}");
        let download_url = match &self.config.public_base_url {
            Some(base) => format!("{base}{path}"),
            None => path,
        };
        Ok(DownloadResult {
            blob: MediaBlobDto::from(&row),
            download_url,
            expires_at_ms,
        })
    }

    /// Authorize content read via bearer account **or** signed download token.
    pub async fn authorize_read(
        &self,
        blob_id: &str,
        account_id: Option<&str>,
        token: Option<&str>,
    ) -> Result<MediaBlobRow, MediaError> {
        let row = store::media_blobs::get_by_id(&self.store, blob_id)
            .await?
            .ok_or(MediaError::NotFound)?;
        if row.status == "deleted" {
            return Err(MediaError::NotFound);
        }
        if let Some(aid) = account_id {
            if row.account_id == aid {
                return Ok(row);
            }
        }
        if let Some(tok) = token {
            if self.verify_download_token(tok, blob_id, &row.account_id)? {
                return Ok(row);
            }
        }
        Err(MediaError::Forbidden)
    }

    pub async fn read_content(&self, row: &MediaBlobRow) -> Result<bytes::Bytes, MediaError> {
        let backend = self.require_backend()?;
        if row.status != "ready" {
            return Err(MediaError::NotReady);
        }
        backend
            .get(&row.object_key)
            .await
            .map_err(MediaError::ObjectStore)
    }

    pub async fn delete(
        &self,
        account_id: &str,
        blob_id: &str,
    ) -> Result<MediaBlobDto, MediaError> {
        let backend = self.require_backend()?;
        let row = store::media_blobs::get_by_id(&self.store, blob_id)
            .await?
            .ok_or(MediaError::NotFound)?;
        if row.account_id != account_id {
            return Err(MediaError::Forbidden);
        }
        if let Err(e) = backend.delete(&row.object_key).await {
            tracing::warn!(
                target: "minos_backend::media",
                blob_id,
                error = %e,
                "object delete failed; still soft-deleting metadata"
            );
        }
        let deleted = store::media_blobs::soft_delete(&self.store, blob_id, now_ms())
            .await?
            .ok_or(MediaError::NotFound)?;
        Ok(MediaBlobDto::from(&deleted))
    }

    fn sign_download_token(&self, blob_id: &str, account_id: &str, expires_at_ms: i64) -> String {
        let payload = format!("{blob_id}|{account_id}|{expires_at_ms}");
        let mut mac =
            HmacSha256::new_from_slice(self.token_secret.as_bytes()).expect("HMAC key length");
        mac.update(payload.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
        format!("{expires_at_ms}.{sig_b64}")
    }

    fn verify_download_token(
        &self,
        token: &str,
        blob_id: &str,
        account_id: &str,
    ) -> Result<bool, MediaError> {
        let (exp_s, _sig_b64) = token
            .split_once('.')
            .ok_or_else(|| MediaError::Invalid("malformed download token".into()))?;
        let expires_at_ms: i64 = exp_s
            .parse()
            .map_err(|_| MediaError::Invalid("malformed download token expiry".into()))?;
        if expires_at_ms < now_ms() {
            return Err(MediaError::Invalid("download token expired".into()));
        }
        let expected = self.sign_download_token(blob_id, account_id, expires_at_ms);
        use subtle::ConstantTimeEq;
        Ok(expected.as_bytes().ct_eq(token.as_bytes()).into())
    }
}

fn extension_for(content_type: &str, filename: Option<&str>) -> String {
    if let Some(name) = filename {
        if let Some((_, ext)) = name.rsplit_once('.') {
            let ext = ext.trim().to_ascii_lowercase();
            if !ext.is_empty() && ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return format!(".{ext}");
            }
        }
    }
    match content_type {
        "image/png" => ".png".into(),
        "image/jpeg" => ".jpg".into(),
        "image/webp" => ".webp".into(),
        "image/gif" => ".gif".into(),
        "image/heic" => ".heic".into(),
        "audio/mpeg" => ".mp3".into(),
        "audio/wav" | "audio/x-wav" => ".wav".into(),
        "audio/webm" => ".webm".into(),
        "video/mp4" => ".mp4".into(),
        "video/webm" => ".webm".into(),
        "application/pdf" => ".pdf".into(),
        "text/plain" => ".txt".into(),
        _ => String::new(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use tempfile::tempdir;

    async fn setup_service() -> (MediaService, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let pool = store::connect("sqlite::memory:").await.unwrap();
        // Need an account row for FK.
        sqlx::query(
            "INSERT INTO accounts (account_id, email, minos_id, display_name, supabase_sub, created_at_ms)
             VALUES ('acc_1', 'a@example.com', 'm1', 'A', NULL, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let svc = MediaService::new(
            StoreHandle::Sqlite(pool),
            MediaConfig::local_for_tests(dir.path().to_path_buf(), 1_000_000),
            "test-secret-at-least-32-bytes-long!!",
        );
        (svc, dir)
    }

    #[tokio::test]
    async fn upload_round_trip() {
        let (svc, _dir) = setup_service().await;
        let created = svc
            .create_upload("acc_1", "image/png", 4, Some("shot.png"))
            .await
            .unwrap();
        assert_eq!(created.blob.status, "pending");
        assert!(created.upload_path.contains(&created.blob.blob_id));

        let body = bytes::Bytes::from_static(b"\x89PNG");
        let ready = svc
            .put_content(
                "acc_1",
                &created.blob.blob_id,
                body.clone(),
                Some("image/png"),
            )
            .await
            .unwrap();
        assert_eq!(ready.status, "ready");
        assert!(ready.sha256_hex.is_some());

        let dl = svc
            .get_download("acc_1", &created.blob.blob_id)
            .await
            .unwrap();
        assert!(dl.download_url.contains("token="));

        let row = svc
            .authorize_read(&created.blob.blob_id, None, {
                let q = dl.download_url.split("token=").nth(1).unwrap();
                Some(q)
            })
            .await
            .unwrap();
        let bytes = svc.read_content(&row).await.unwrap();
        assert_eq!(bytes.as_ref(), b"\x89PNG");
    }

    #[tokio::test]
    async fn rejects_size_mismatch() {
        let (svc, _dir) = setup_service().await;
        let created = svc
            .create_upload("acc_1", "text/plain", 10, None)
            .await
            .unwrap();
        let err = svc
            .put_content(
                "acc_1",
                &created.blob.blob_id,
                bytes::Bytes::from_static(b"short"),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MediaError::SizeMismatch));
    }

    #[tokio::test]
    async fn forbids_other_account() {
        let (svc, _dir) = setup_service().await;
        let created = svc
            .create_upload("acc_1", "text/plain", 3, None)
            .await
            .unwrap();
        let err = svc
            .put_content(
                "acc_other",
                &created.blob.blob_id,
                bytes::Bytes::from_static(b"abc"),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MediaError::Forbidden));
    }
}
