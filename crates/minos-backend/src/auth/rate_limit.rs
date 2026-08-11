//! Coarse in-memory rate limiter for auth endpoints. Per-key sliding
//! window with `permits` slots over `window`.
//!
//! Hand-rolled rather than using `tower-governor` to keep the dep tree
//! lean. The bucket is adequate for the auth surface, where the rate
//! limits are coarse and we never need per-route middleware composition.
//!
//! Key-count is bounded by `max_keys` to prevent memory exhaustion from
//! rotating-IP attacks. When the limit is reached, the oldest-accessed
//! key is evicted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Maximum number of distinct keys a single `RateLimiter` will track.
/// Beyond this, the least-recently-used key is evicted. 100k keys ×
/// ~256 bytes each ≈ 25 MB worst case — acceptable for the auth surface.
const DEFAULT_MAX_KEYS: usize = 100_000;

#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
    permits: usize,
    window: Duration,
    max_keys: usize,
}

impl RateLimiter {
    #[must_use]
    pub fn new(permits: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            permits,
            window,
            max_keys: DEFAULT_MAX_KEYS,
        }
    }

    /// Returns `Ok(())` if a permit was granted, `Err(retry_after_secs)`
    /// if the bucket is full. The `retry_after` value is computed from
    /// the oldest in-window timestamp so callers can populate the
    /// `Retry-After` HTTP header truthfully (clamped to ≥1 second).
    ///
    /// Bounded-key contract: the map is capped at `max_keys`. When a new
    /// key would exceed the cap, we evict the key whose most-recent
    /// timestamp is oldest (least recently active). This keeps memory
    /// bounded even under rotating-IP floods.
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
            let remaining = self.window.saturating_sub(now.duration_since(oldest));
            let retry_secs = remaining
                .as_millis()
                .saturating_add(999)
                .saturating_div(1_000);
            let retry = u32::try_from(retry_secs).unwrap_or(u32::MAX);
            return Err(retry.max(1));
        }
        entries.push(now);
        // GC: drop the key if its bucket is somehow still empty.
        if entries.is_empty() {
            map.remove(key);
        }

        // Evict oldest-accessed keys when we exceed the cap.
        if map.len() > self.max_keys {
            // Find the key with the oldest most-recent timestamp.
            let victim = map
                .iter()
                .filter(|(k, _)| k.as_str() != key) // don't evict the one we just inserted
                .min_by_key(|(_, timestamps)| timestamps.last().copied().unwrap_or(now))
                .map(|(k, _)| k.clone());
            if let Some(victim_key) = victim {
                map.remove(&victim_key);
            }
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

    #[test]
    fn check_rounds_retry_after_up_for_partial_seconds() {
        let rl = RateLimiter::new(1, Duration::from_millis(1_500));
        assert!(rl.check("k1").is_ok());

        let retry = rl.check("k1").unwrap_err();
        assert_eq!(
            retry, 2,
            "retry-after must not under-report remaining wait time"
        );
    }
}
