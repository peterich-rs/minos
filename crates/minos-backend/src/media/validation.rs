//! Upload declaration validation (MIME allowlist + size).

use std::collections::HashSet;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    File,
    Audio,
    Video,
}

impl MediaKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::File => "file",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

static ALLOWED_TYPES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "image/heic",
        "image/heif",
        "image/avif",
        "audio/mpeg",
        "audio/wav",
        "audio/x-wav",
        "audio/webm",
        "audio/ogg",
        "audio/mp4",
        "video/mp4",
        "video/webm",
        "video/quicktime",
        "application/pdf",
        "text/plain",
        "text/markdown",
        "application/json",
        "application/zip",
        "application/octet-stream",
    ])
});

/// Normalize and validate Content-Type. Returns lowercase type without params.
pub fn validate_content_type(raw: &str) -> Result<String, String> {
    let base = raw
        .split(';')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    let canonical = match base.as_str() {
        "image/jpg" => "image/jpeg".to_string(),
        "audio/mp3" => "audio/mpeg".to_string(),
        other if ALLOWED_TYPES.contains(other) => other.to_string(),
        other => return Err(format!("unsupported content_type: {other}")),
    };
    Ok(canonical)
}

pub fn validate_upload_declaration(byte_size: u64, max_bytes: u64) -> Result<(), String> {
    if byte_size == 0 {
        return Err("byte_size must be > 0".into());
    }
    if byte_size > max_bytes {
        return Err(format!("byte_size {byte_size} exceeds max {max_bytes}"));
    }
    Ok(())
}

#[must_use]
pub fn infer_kind(content_type: &str, filename: Option<&str>) -> MediaKind {
    if content_type.starts_with("image/") {
        return MediaKind::Image;
    }
    if content_type.starts_with("audio/") {
        return MediaKind::Audio;
    }
    if content_type.starts_with("video/") {
        return MediaKind::Video;
    }
    if let Some(name) = filename {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "avif" => return MediaKind::Image,
            "mp3" | "wav" | "ogg" | "m4a" => return MediaKind::Audio,
            "mp4" | "webm" | "mov" | "mkv" => return MediaKind::Video,
            _ => {}
        }
    }
    MediaKind::File
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_png_and_normalizes_jpg() {
        assert_eq!(validate_content_type("image/png").unwrap(), "image/png");
        assert_eq!(
            validate_content_type("image/jpg; charset=binary").unwrap(),
            "image/jpeg"
        );
    }

    #[test]
    fn rejects_unknown() {
        assert!(validate_content_type("application/x-msdownload").is_err());
    }

    #[test]
    fn infers_image_kind() {
        assert_eq!(infer_kind("image/png", Some("a.png")), MediaKind::Image);
        assert_eq!(
            infer_kind("application/pdf", Some("a.pdf")),
            MediaKind::File
        );
    }
}
