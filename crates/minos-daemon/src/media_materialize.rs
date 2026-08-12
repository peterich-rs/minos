//! Download Hub media blobs into the agent workspace before prompting.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use minos_protocol::DispatchAttachment;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

/// Match Hub default `MINOS_MEDIA_MAX_BYTES` (10 MiB).
const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// Hex chars from sha256(origin); enough entropy, filesystem-safe.
const ORIGIN_DIR_HEX_LEN: usize = 32;

/// Download attachments under `{workspace}/.minos/attachments/{hash}/` and
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
    let attachments_root = workspace.join(".minos").join("attachments");
    if let Err(e) = tokio::fs::create_dir_all(&attachments_root).await {
        warn!(
            target: "minos_daemon::media",
            error = %e,
            path = %attachments_root.display(),
            "failed to create attachments root"
        );
        return Vec::new();
    }
    let Ok(root_canon) = tokio::fs::canonicalize(&attachments_root).await else {
        warn!(
            target: "minos_daemon::media",
            path = %attachments_root.display(),
            "failed to canonicalize attachments root"
        );
        return Vec::new();
    };

    let dir = attachments_root.join(origin_dir_component(origin));
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        warn!(
            target: "minos_daemon::media",
            error = %e,
            path = %dir.display(),
            "failed to create attachment dir"
        );
        return Vec::new();
    }
    let Ok(dir_canon) = tokio::fs::canonicalize(&dir).await else {
        warn!(
            target: "minos_daemon::media",
            path = %dir.display(),
            "failed to canonicalize attachment dir"
        );
        return Vec::new();
    };
    if !dir_canon.starts_with(&root_canon) {
        warn!(
            target: "minos_daemon::media",
            dir = %dir_canon.display(),
            root = %root_canon.display(),
            "attachment dir escaped attachments root"
        );
        return Vec::new();
    }

    let client = match reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(
                target: "minos_daemon::media",
                error = %e,
                "failed to build download client"
            );
            return Vec::new();
        }
    };

    let mut paths = Vec::with_capacity(attachments.len());
    for (idx, att) in attachments.iter().enumerate() {
        let filename = safe_filename(att, idx);
        let dest = dir_canon.join(&filename);
        // Filename is sanitized to a single path component; still assert containment.
        if !dest.starts_with(&dir_canon) || dest == dir_canon {
            warn!(
                target: "minos_daemon::media",
                dest = %dest.display(),
                "refusing attachment dest outside origin dir"
            );
            continue;
        }
        match download_one(&client, &att.download_url, &dest).await {
            Ok(()) => {
                // After write, re-check resolved path stays under attachments root.
                match tokio::fs::canonicalize(&dest).await {
                    Ok(canon) if canon.starts_with(&root_canon) => {
                        info!(
                            target: "minos_daemon::media",
                            blob_id = %att.blob_id,
                            path = %canon.display(),
                            "materialized attachment"
                        );
                        paths.push(canon);
                    }
                    Ok(canon) => {
                        warn!(
                            target: "minos_daemon::media",
                            path = %canon.display(),
                            "materialized path escaped attachments root; removing"
                        );
                        let _ = tokio::fs::remove_file(&canon).await;
                    }
                    Err(e) => {
                        warn!(
                            target: "minos_daemon::media",
                            error = %e,
                            path = %dest.display(),
                            "failed to canonicalize materialized path"
                        );
                    }
                }
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

/// Hex prefix of sha256(origin). Never use the remote string as a path component.
#[must_use]
pub(crate) fn origin_dir_component(origin: &str) -> String {
    let digest = Sha256::digest(origin.as_bytes());
    let mut out = String::with_capacity(ORIGIN_DIR_HEX_LEN);
    for b in digest.iter().take(ORIGIN_DIR_HEX_LEN / 2) {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
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
        let base = base.trim_matches('.').to_string();
        if !base.is_empty()
            && base != "."
            && base != ".."
            && !base.contains("..")
            && base.len() <= 180
        {
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
    let blob = att.blob_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>();
    let stem = if blob.is_empty() {
        "blob".to_string()
    } else {
        blob.chars().take(8).collect::<String>()
    };
    format!("{idx}_{stem}.{ext}")
}

async fn download_one(client: &reqwest::Client, url: &str, dest: &Path) -> Result<(), String> {
    let url = resolve_download_url(url)?;
    ensure_download_url_allowed(&url)?;

    let resp = client
        .get(url.as_str())
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("http {}", resp.status()));
    }
    if let Some(len) = resp.content_length() {
        if len > MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "content-length {len} exceeds cap {MAX_ATTACHMENT_BYTES}"
            ));
        }
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("create: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut written: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("body: {e}"))?;
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_ATTACHMENT_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(format!(
                "body exceeded cap {MAX_ATTACHMENT_BYTES} bytes"
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write: {e}"))?;
    }
    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

fn resolve_download_url(url: &str) -> Result<url::Url, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url::Url::parse(url).map_err(|e| format!("parse url: {e}"));
    }
    // Relative paths need an absolute backend origin (host is not the API).
    let base = configured_http_bases()
        .into_iter()
        .next()
        .ok_or_else(|| {
            "relative download_url requires MINOS_BACKEND_URL or MINOS_MEDIA_PUBLIC_BASE_URL on host"
                .to_string()
        })?;
    base.join(url).map_err(|e| format!("join url: {e}"))
}

/// Allowlist host == configured backend / media public origin.
pub(crate) fn ensure_download_url_allowed(url: &url::Url) -> Result<(), String> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!("unsupported download scheme {}", url.scheme()));
    }
    let Some(host) = url.host_str() else {
        return Err("download url missing host".into());
    };
    let allowed = configured_http_bases();
    if allowed.is_empty() {
        return Err("no allowed download origins configured".into());
    }
    let ok = allowed.iter().any(|base| {
        base.scheme() == url.scheme()
            && base.host_str() == Some(host)
            && base.port_or_known_default() == url.port_or_known_default()
    });
    if ok {
        Ok(())
    } else {
        Err(format!("download host not in allowlist: {host}"))
    }
}

fn configured_http_bases() -> Vec<url::Url> {
    let mut out = Vec::new();
    let mut push_raw = |raw: &str| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(base) = crate::relay_http::http_base(trimmed) {
            if let Ok(u) = url::Url::parse(&base) {
                if !out.iter().any(|e| e == &u) {
                    out.push(u);
                }
                return;
            }
        }
        if let Ok(u) = url::Url::parse(trimmed) {
            if (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some() {
                // Prefer origin form (scheme/host/port only).
                if let Some(base) = crate::relay_http::http_base(trimmed) {
                    if let Ok(b) = url::Url::parse(&base) {
                        if !out.iter().any(|e| e == &b) {
                            out.push(b);
                        }
                        return;
                    }
                }
                if !out.iter().any(|e| e == &u) {
                    out.push(u);
                }
            }
        }
    };

    if let Ok(v) = std::env::var("MINOS_MEDIA_PUBLIC_BASE_URL") {
        push_raw(&v);
    }
    if let Ok(v) = std::env::var("MINOS_BACKEND_URL") {
        push_raw(&v);
    }
    // Bake-time fallback so relative URLs work with the compiled-in hub origin.
    push_raw(crate::config::BACKEND_URL);
    out
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

    #[test]
    fn origin_dir_is_hex_and_ignores_path_escape() {
        let a = origin_dir_component("../etc/passwd");
        let b = origin_dir_component("/tmp/x");
        let c = origin_dir_component("msg_550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(a.len(), ORIGIN_DIR_HEX_LEN);
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(!a.contains('.') && !a.contains('/') && !a.contains('\\'));
    }

    #[test]
    fn safe_filename_strips_path_parts_and_dotdot() {
        let att = DispatchAttachment {
            blob_id: "blobdeadbeef".into(),
            content_type: "image/png".into(),
            byte_size: 12,
            original_filename: Some("../evil/../../x.png".into()),
            download_url: "https://example.com/x".into(),
        };
        let name = safe_filename(&att, 0);
        assert_eq!(name, "0_x.png");
        assert!(!name.contains(".."));
        assert!(!name.contains('/'));
    }

    #[test]
    fn safe_filename_falls_back_for_dotdot_only() {
        let att = DispatchAttachment {
            blob_id: "blobdeadbeef".into(),
            content_type: "image/png".into(),
            byte_size: 1,
            original_filename: Some("..".into()),
            download_url: "https://example.com/x".into(),
        };
        let name = safe_filename(&att, 3);
        assert_eq!(name, "3_blobdead.png");
    }

    #[test]
    fn rejects_non_allowlisted_download_host() {
        // BACKEND_URL bake + env may vary; force isolation via a clearly foreign host.
        let url = url::Url::parse("https://evil.example/steal").unwrap();
        // If somehow evil.example were allowlisted (it isn't), this would fail the test intent.
        let err = ensure_download_url_allowed(&url).unwrap_err();
        assert!(
            err.contains("not in allowlist") || err.contains("no allowed"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn allowlists_configured_backend_origin() {
        let bases = configured_http_bases();
        assert!(
            !bases.is_empty(),
            "expected at least bake-time BACKEND_URL origin"
        );
        let base = &bases[0];
        let mut allowed = base.clone();
        allowed.set_path("/v1/media/blobs/x/content");
        assert!(ensure_download_url_allowed(&allowed).is_ok());
    }

    #[tokio::test]
    async fn path_escape_origin_stays_under_attachments_root() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();
        // No network: empty attachments list short-circuits, so create dir via
        // a failed download that still creates the hashed origin folder.
        let evil = "../../../../etc/passwd";
        let paths = materialize_attachments(
            workspace,
            Some(evil),
            &[DispatchAttachment {
                blob_id: "blobdeadbeef".into(),
                content_type: "text/plain".into(),
                byte_size: 1,
                original_filename: Some("a.txt".into()),
                download_url: "https://evil.example/x".into(),
            }],
        )
        .await;
        assert!(paths.is_empty());
        let attachments = workspace.join(".minos").join("attachments");
        assert!(attachments.is_dir());
        let hashed = attachments.join(origin_dir_component(evil));
        assert!(
            hashed.is_dir(),
            "expected hashed origin dir under attachments, got {:?}",
            std::fs::read_dir(&attachments)
                .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
        // No sibling escape dirs created next to .minos
        assert!(!workspace.join("etc").exists());
        assert!(!workspace.join("passwd").exists());
    }
}
