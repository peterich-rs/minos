//! Identifier newtypes.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable unique identifier for a paired device.
///
/// Newtype over `uuid::Uuid` (v4) so it cannot be confused with other UUIDs
/// in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub Uuid);

impl DeviceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// UniFFI bridge. `DeviceId` is a local type, so this is the *home*
// registration (no `remote` keyword). Downstream crates that want the same
// type across their own tag should pull these impls in with
// `uniffi::use_remote_type!(DeviceId from minos_domain)` rather than
// re-registering (which collides with the home-crate blanket impls).
// Bridging via `String` keeps the registration self-contained so this crate
// doesn't also have to own a `Uuid` bridge.
#[cfg(feature = "uniffi")]
mod uniffi_bridges {
    use super::{DeviceId, DeviceSecret};
    use uuid::Uuid;

    uniffi::custom_type!(DeviceId, String, {
        lower: |id| id.0.to_string(),
        try_lift: |text| Uuid::parse_str(&text).map(DeviceId).map_err(Into::into),
    });

    // `DeviceSecret` is a newtype over `String`; UniFFI marshals it as a
    // transparent string. The base64url contents cross the FFI untouched —
    // Swift never needs to know this is "base64url-no-pad, 43 chars".
    uniffi::custom_type!(DeviceSecret, String, {
        lower: |s| s.0,
        try_lift: |s| Ok(DeviceSecret(s)),
    });
}

/// One-shot pairing token: 32 random bytes, presented as base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PairingToken(pub String);

impl PairingToken {
    /// Generate a fresh token from the OS CSPRNG.
    ///
    /// # Panics
    /// Panics only if `getrandom` cannot supply entropy from the OS, which
    /// indicates an unrecoverable platform fault.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Long-lived per-device bearer secret minted by the relay at pair time.
///
/// 32 random bytes, presented as base64url-no-pad (43 chars). `Debug` and
/// `Display` are **redacted** so accidental log/trace formatting never leaks
/// the secret. Use [`DeviceSecret::as_str`] or the transparent `serde`
/// representation when the plain value is genuinely needed (e.g. sending the
/// WebSocket `Authorization` header or writing to the keychain).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceSecret(pub String);

impl DeviceSecret {
    /// Generate a fresh secret from the OS CSPRNG (32 bytes → base64url-no-pad).
    ///
    /// # Panics
    /// Panics only if `getrandom` cannot supply entropy from the OS, which
    /// indicates an unrecoverable platform fault.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Borrow the underlying base64url string. Prefer this over `Display`
    /// (which is redacted) whenever the plain value is genuinely needed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for DeviceSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted DeviceSecret>")
    }
}

impl std::fmt::Display for DeviceSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted DeviceSecret>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_ne;

    #[test]
    fn device_id_round_trips_through_json() {
        let id = DeviceId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: DeviceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn device_id_default_is_unique() {
        let a = DeviceId::default();
        let b = DeviceId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn pairing_token_is_43_chars_base64url() {
        // 32 bytes base64-encoded with no padding = 43 chars
        let t = PairingToken::generate();
        assert_eq!(t.as_str().len(), 43);
        assert!(t
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    proptest::proptest! {
        #[test]
        fn pairing_token_uniqueness(_iter in 0u32..1000) {
            // 1000 tokens, no collisions (entropy sanity)
            let mut seen = std::collections::HashSet::new();
            for _ in 0..1000 {
                let t = PairingToken::generate();
                assert!(seen.insert(t.0), "collision");
            }
        }
    }

    #[test]
    fn device_secret_debug_and_display_are_redacted() {
        // Use a fixed sentinel so we can assert the plaintext NEVER appears.
        let sentinel = "super-secret-123";
        let s = DeviceSecret(sentinel.to_owned());

        let dbg = format!("{s:?}");
        let disp = format!("{s}");

        assert_eq!(dbg, "<redacted DeviceSecret>");
        assert_eq!(disp, "<redacted DeviceSecret>");
        assert!(
            !dbg.contains(sentinel),
            "Debug must not leak plaintext secret: {dbg}"
        );
        assert!(
            !disp.contains(sentinel),
            "Display must not leak plaintext secret: {disp}"
        );
    }

    #[test]
    fn device_secret_as_str_returns_plaintext() {
        let s = DeviceSecret("abc".to_owned());
        assert_eq!(s.as_str(), "abc");
    }

    #[test]
    fn device_secret_generate_is_43_chars_base64url() {
        // 32 bytes base64-encoded with no padding = 43 chars.
        let s = DeviceSecret::generate();
        assert_eq!(s.as_str().len(), 43);
        assert!(s
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn device_secret_generate_is_unique() {
        let a = DeviceSecret::generate();
        let b = DeviceSecret::generate();
        assert_ne!(a.as_str(), b.as_str());
    }

    #[test]
    fn device_secret_serde_is_transparent_string() {
        let s = DeviceSecret("token-xyz".to_owned());
        let json = serde_json::to_string(&s).unwrap();
        // Transparent → bare JSON string, no struct wrapping.
        assert_eq!(json, "\"token-xyz\"");
        let back: DeviceSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "token-xyz");
        assert_eq!(s, back);
    }

    // Plan 05 Task A.3 contract tests: DeviceSecret already existed before
    // the macOS relay-client migration plan landed (introduced by plan 04),
    // so these duplicate intent with the tests above. Kept under the plan's
    // chosen names so the plan's acceptance criteria are visible in tree.
    #[test]
    fn device_secret_round_trips_as_string() {
        let s = DeviceSecret("hunter2-the-32-byte-base64-secret".into());
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, r#""hunter2-the-32-byte-base64-secret""#);
        let back: DeviceSecret = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn device_secret_debug_redacts() {
        let s = DeviceSecret("super-secret".into());
        let d = format!("{s:?}");
        assert!(
            !d.contains("super-secret"),
            "DeviceSecret Debug must not leak"
        );
        assert!(d.contains("DeviceSecret"));
    }
}
