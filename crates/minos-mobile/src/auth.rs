//! In-memory auth state held by [`crate::MobileClient`].
//!
//! Persistence lives in [`crate::store`] via `flutter_secure_storage` on the
//! Dart side; this module only shapes the live snapshot held in memory and
//! the watch-channel frame the Dart side observes.
//!
//! Spec §6.1.

use std::sync::Arc;
use std::time::Instant;

use minos_domain::MinosError;
pub use minos_protocol::AuthSummary;

/// Authenticated session held by the mobile client.
///
/// `access_expires_at_ms` is the wall-clock expiry mirrored from the
/// backend response; `access_expires_at` is the corresponding `Instant` so
/// the reconnect loop can compare against `Instant::now()` without going
/// back through the system clock.
#[derive(Debug, Clone)]
pub struct AuthSession {
    pub access_token: String,
    pub access_expires_at: Instant,
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
    pub account: AuthSummary,
}

/// Frame published on the auth-state watch channel. UI / reconnect loops
/// observe this rather than polling the session struct.
///
/// `MinosError` is neither `Clone` nor `PartialEq`, so the failure payload
/// is `Arc<MinosError>` to keep the frame cheaply cloneable across watch
/// receivers. Tests pattern-match by variant + `error.kind()` rather than
/// using equality.
#[derive(Debug, Clone)]
pub enum AuthStateFrame {
    Unauthenticated,
    Authenticated { account: AuthSummary },
    Refreshing,
    RefreshFailed { error: Arc<MinosError> },
}
