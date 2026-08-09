use sha2::{Digest, Sha256};

/// Hex-encoded SHA-256 of `bytes`. Stable across platforms for the same input.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Normalize fragment text for hashing and injection: trim trailing whitespace
/// on each line, trim leading/trailing blank lines, use `\n` line endings, no
/// trailing newline on the final body (join sites add separators).
pub(crate) fn normalize_fragment(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = normalized.lines().map(str::trim_end).collect();
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_is_stable_across_newlines() {
        let a = normalize_fragment("hello\r\nworld  \n\n");
        let b = normalize_fragment("hello\nworld");
        assert_eq!(a, b);
        assert_eq!(a, "hello\nworld");
    }

    #[test]
    fn digest_is_deterministic() {
        let d1 = sha256_hex(b"minos");
        let d2 = sha256_hex(b"minos");
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64);
    }
}
