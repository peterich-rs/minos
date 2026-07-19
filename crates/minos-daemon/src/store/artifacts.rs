use anyhow::{Context, Result};
use minos_ui_protocol::ArtifactRef;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ArtifactRange {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub total_size: u64,
    pub eof: bool,
}

impl ArtifactStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn write_bytes(
        &self,
        thread_id: &str,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<ArtifactRef> {
        let sha256 = hex_sha256(bytes);
        let artifact_id = format!("art_{sha256}");
        let dir = self.thread_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create artifact dir {}", dir.display()))?;
        let path = dir.join(&artifact_id);
        fs::write(&path, bytes)
            .await
            .with_context(|| format!("write artifact {}", path.display()))?;
        Ok(ArtifactRef {
            thread_id: thread_id.to_string(),
            artifact_id,
            size_bytes: bytes.len() as u64,
            sha256,
            media_type: media_type.to_string(),
        })
    }

    pub async fn read_range(
        &self,
        thread_id: &str,
        artifact_id: &str,
        offset: u64,
        limit: u32,
    ) -> Result<ArtifactRange> {
        let path = self.thread_dir(thread_id).join(artifact_id);
        let bytes = fs::read(&path)
            .await
            .with_context(|| format!("read artifact {}", path.display()))?;
        let total_size = bytes.len() as u64;
        let start = usize::try_from(offset.min(total_size)).unwrap_or(bytes.len());
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let end = start.saturating_add(limit).min(bytes.len());
        Ok(ArtifactRange {
            bytes: bytes[start..end].to_vec(),
            offset,
            total_size,
            eof: end >= bytes.len(),
        })
    }

    pub async fn delete_thread_artifacts(&self, thread_id: &str) -> Result<()> {
        let dir = self.thread_dir(thread_id);
        if fs::try_exists(&dir).await.unwrap_or(false) {
            fs::remove_dir_all(&dir)
                .await
                .with_context(|| format!("remove artifact dir {}", dir.display()))?;
        }
        Ok(())
    }

    fn thread_dir(&self, thread_id: &str) -> PathBuf {
        self.root.join("threads").join(thread_id)
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
