//! Download Hub media blobs into the agent workspace before prompting.

use std::path::{Path, PathBuf};

use minos_protocol::DispatchAttachment;
use tracing::{info, warn};

/// Download attachments under `{workspace}/.minos/attachments/{origin}/` and
/// return absolute paths (for `@path` / localImage style agent prompts).
pub async fn materialize_attachments(
    workspace: &Path,
    origin_message_id: Option<&str>,
    attachments: &[DispatchAttachment],
) -> Vec<PathBuf> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let origin = origin_message_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("anonymous");
    let dir = workspace.join(".minos").join("attachments").join(origin);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        warn!(
            target: "minos_daemon::media",
            error = %e,
            path = %dir.display(),
            "failed to create attachment dir"
        );
        return Vec::new();
    }

    let client = reqwest::Client::new();
    let mut paths = Vec::with_capacity(attachments.len());
    for (idx, att) in attachments.iter().enumerate() {
        let filename = safe_filename(att, idx);
        let dest = dir.join(&filename);
        match download_one(&client, &att.download_url, &dest).await {
            Ok(()) => {
                info!(
                    target: "minos_daemon::media",
                    blob_id = %att.blob_id,
                    path = %dest.display(),
                    "materialized attachment"
                );
                paths.push(dest);
            }
            Err(e) => {
                warn!(
                    target: "minos_daemon::media",
                    blob_id = %att.blob_id,
                    error = %e,
                    "attachment download failed"
                );
            }
        }
    }
    paths
}

/// Append `@/abs/path` lines so Grok / path-aware agents see files; empty text ok.
#[must_use]
pub fn append_attachment_paths(text: &str, paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return text.to_string();
    }
    let refs: Vec<String> = paths.iter().map(|p| format!("@{}", p.display())).collect();
    let body = text.trim();
    if body.is_empty() {
        refs.join("\n")
    } else {
        format!("{body}\n\n{}", refs.join("\n"))
    }
}

fn safe_filename(att: &DispatchAttachment, idx: usize) -> String {
    if let Some(name) = att.original_filename.as_deref() {
        let base = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(name)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if !base.is_empty() && base != "." && base != ".." {
            return format!("{idx}_{base}");
        }
    }
    let ext = match att.content_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    };
    format!("{idx}_{}.{ext}", &att.blob_id[..8.min(att.blob_id.len())])
}

async fn download_one(client: &reqwest::Client, url: &str, dest: &Path) -> Result<(), String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        // Relative paths need an absolute backend origin (host is not the API).
        let base = std::env::var("MINOS_BACKEND_URL")
            .or_else(|_| std::env::var("MINOS_MEDIA_PUBLIC_BASE_URL"))
            .unwrap_or_default()
            .trim()
            .trim_end_matches('/')
            .to_string();
        if base.is_empty() {
            return Err(
                "relative download_url requires MINOS_BACKEND_URL or MINOS_MEDIA_PUBLIC_BASE_URL on host"
                    .into(),
            );
        }
        format!("{base}{url}")
    };
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("http {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("body: {e}"))?;
    tokio::fs::write(dest, &bytes)
        .await
        .map_err(|e| format!("write: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_paths() {
        let text = append_attachment_paths(
            "look",
            &[PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.pdf")],
        );
        assert_eq!(text, "look\n\n@/tmp/a.png\n@/tmp/b.pdf");
    }

    #[test]
    fn attachment_only_message() {
        let text = append_attachment_paths("", &[PathBuf::from("/tmp/a.png")]);
        assert_eq!(text, "@/tmp/a.png");
    }
}
