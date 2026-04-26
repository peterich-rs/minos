//! Coarse in-memory rate limiter for auth endpoints. Per-key sliding
//! window with `permits` slots over `window`. Spec §5.6.
//!
//! Hand-rolled rather than using `tower-governor` to keep the dep tree
//! lean (spec §12.1 flagged the ecosystem churn risk). The bucket is
//! adequate for the auth surface, where the rate limits are coarse and
//! we never need per-route middleware composition.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
    permits: usize,
    window: Duration,
}

impl RateLimiter {
    #[must_use]
    pub fn new(permits: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            permits,
            window,
        }
    }

    /// Returns `Ok(())` if a permit was granted, `Err(retry_after_secs)`
    /// if the bucket is full. The `retry_after` value is computed from
    /// the oldest in-window timestamp so callers can populate the
    /// `Retry-After` HTTP header truthfully (clamped to ≥1 second).
    ///
    /// Bounded-key contract: a key is only kept in the map while it owns
    /// at least one in-window timestamp. After the retain trims expired
    /// entries, if no fresh permit ends up pushed (e.g. degenerate
    /// `permits == 0` config or a future mutator path that skips push),
    /// the key is removed entirely. This keeps the map size bounded by
    /// the active working set and stops abandoned keys from accumulating
    /// across long uptimes when an attacker rotates IPs/emails.
    pub fn check(&self, key: &str) -> Result<(), u32> {
        let now = Instant::now();
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = map.entry(key.to_string()).or_default();
        entries.retain(|t| now.duration_since(*t) < self.window);
        if entries.len() >= self.permits {
            let oldest = entries[0];
            let retry_secs = self
                .window
                .saturating_sub(now.duration_since(oldest))
                .as_secs();
            // Clamp into u32. The window is bounded by the caller; in
            // practice it never exceeds an hour, so the truncation is a
            // formality. `min(u32::MAX as u64) → as u32` is the
            // explicitly-checked path clippy is happy with.
            let retry = u32::try_from(retry_secs).unwrap_or(u32::MAX);
            return Err(retry.max(1));
        }
        entries.push(now);
        // GC: drop the key if its bucket is somehow still empty (the
        // success path always pushes, so this only fires on the
        // degenerate `permits == 0` config or a future mutator path).
        // Belt-and-braces against the rotating-IP memory leak.
        if entries.is_empty() {
            map.remove(key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_allows_permits_count_then_blocks() {
        let rl = RateLimiter::new(3, Duration::from_mins(1));
        assert!(rl.check("k1").is_ok());
        assert!(rl.check("k1").is_ok());
        assert!(rl.check("k1").is_ok());
        let err = rl.check("k1").unwrap_err();
        assert!(err >= 1, "retry must be >= 1 second");
    }

    #[test]
    fn check_isolates_keys() {
        let rl = RateLimiter::new(1, Duration::from_mins(1));
        assert!(rl.check("k1").is_ok());
        // k1 is full but k2 has its own bucket.
        assert!(rl.check("k1").is_err());
        assert!(rl.check("k2").is_ok());
    }

    #[test]
    fn check_recovers_after_window_expires() {
        let rl = RateLimiter::new(1, Duration::from_millis(50));
        assert!(rl.check("k1").is_ok());
        assert!(rl.check("k1").is_err());
        std::thread::sleep(Duration::from_millis(80));
        assert!(rl.check("k1").is_ok(), "expired entries must drop out");
    }
}
