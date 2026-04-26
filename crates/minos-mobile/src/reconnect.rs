//! Auto-reconnect controller. Spec §6.3.
//!
//! The reconnect loop in [`crate::client::MobileClient`] consults this state
//! between attempts: it asks for the next backoff delay, sleeps, then asks
//! whether it's allowed to attempt at all. After each attempt it reports
//! success or failure, and the controller updates the delay accordingly.
//!
//! Behaviour:
//!
//! - Start at 1s, double on each consecutive failure, cap at 30s.
//! - On a sustained success (60s+ connected), reset to 1s; quick re-fails
//!   keep the previous backoff so we don't churn.
//! - Foregrounding resets the backoff to 1s and clears `paused`. Background
//!   sets `paused` so the loop can quickly check and exit.

use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Backoff state machine consulted by the reconnect loop. Owns no IO of
/// its own — the loop drives the actual `connect`. Spec §6.3.
#[derive(Debug)]
pub(crate) struct ReconnectController {
    state: RwLock<ReconnectState>,
}

#[derive(Debug)]
struct ReconnectState {
    delay: Duration,
    consecutive_failures: u32,
    last_connected_at: Option<Instant>,
    foreground: bool,
    paused: bool,
}

const INITIAL_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(30);
const STABLE_THRESHOLD: Duration = Duration::from_secs(60);

impl ReconnectController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ReconnectState {
                delay: INITIAL_DELAY,
                consecutive_failures: 0,
                last_connected_at: None,
                foreground: true,
                paused: false,
            }),
        }
    }

    /// How long to sleep before the next attempt. Reads the current delay;
    /// callers should take the value, sleep, then re-check `is_paused`.
    pub async fn next_delay(&self) -> Duration {
        self.state.read().await.delay
    }

    /// Mark a connect attempt failed. Doubles the delay (capped at 30s)
    /// and bumps `consecutive_failures`.
    pub async fn record_failure(&self) {
        let mut s = self.state.write().await;
        s.consecutive_failures = s.consecutive_failures.saturating_add(1);
        s.delay = (s.delay * 2).min(MAX_DELAY);
    }

    /// Mark a connect attempt succeeded. If the prior connection was
    /// stable (`STABLE_THRESHOLD`+), reset the delay to the initial value.
    /// Either way, zero out `consecutive_failures` and stamp the
    /// `last_connected_at` timestamp.
    pub async fn record_success(&self) {
        let mut s = self.state.write().await;
        let stable = s
            .last_connected_at
            .is_none_or(|t| t.elapsed() > STABLE_THRESHOLD);
        if stable {
            s.delay = INITIAL_DELAY;
        }
        s.consecutive_failures = 0;
        s.last_connected_at = Some(Instant::now());
    }

    /// App moved to foreground (Dart `WidgetsBindingObserver`). Resets
    /// the delay and clears the paused flag so the loop reconnects
    /// immediately.
    pub async fn notify_foregrounded(&self) {
        let mut s = self.state.write().await;
        s.foreground = true;
        s.delay = INITIAL_DELAY;
        s.paused = false;
    }

    /// App moved to background. Sets `paused` so the loop quickly exits
    /// after its next wakeup. We do NOT actively close the WS here (iOS
    /// will keep it warm for a few seconds), only flag the loop.
    pub async fn notify_backgrounded(&self) {
        let mut s = self.state.write().await;
        s.foreground = false;
        s.paused = true;
    }

    pub async fn is_paused(&self) -> bool {
        self.state.read().await.paused
    }

    /// Test/observability accessor: how many consecutive failures has the
    /// loop seen since the last success or foreground transition.
    #[cfg(test)]
    pub async fn consecutive_failures(&self) -> u32 {
        self.state.read().await.consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_delay_is_one_second() {
        let r = ReconnectController::new();
        assert_eq!(r.next_delay().await, INITIAL_DELAY);
    }

    #[tokio::test]
    async fn backoff_doubles_per_failure() {
        let r = ReconnectController::new();
        r.record_failure().await;
        assert_eq!(r.next_delay().await, Duration::from_secs(2));
        r.record_failure().await;
        assert_eq!(r.next_delay().await, Duration::from_secs(4));
        r.record_failure().await;
        assert_eq!(r.next_delay().await, Duration::from_secs(8));
    }

    #[tokio::test]
    async fn backoff_caps_at_30s() {
        let r = ReconnectController::new();
        for _ in 0..10 {
            r.record_failure().await;
        }
        assert_eq!(r.next_delay().await, MAX_DELAY);
    }

    #[tokio::test]
    async fn record_success_resets_consecutive_failures() {
        let r = ReconnectController::new();
        r.record_failure().await;
        r.record_failure().await;
        r.record_success().await;
        assert_eq!(r.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn record_success_resets_delay_when_first_connection() {
        let r = ReconnectController::new();
        r.record_failure().await;
        r.record_failure().await;
        // No prior `last_connected_at` → stable=true branch, delay resets.
        r.record_success().await;
        assert_eq!(r.next_delay().await, INITIAL_DELAY);
    }

    #[tokio::test]
    async fn foreground_resets_delay_and_clears_paused() {
        let r = ReconnectController::new();
        r.record_failure().await;
        r.record_failure().await;
        r.notify_backgrounded().await;
        assert!(r.is_paused().await);

        r.notify_foregrounded().await;
        assert_eq!(r.next_delay().await, INITIAL_DELAY);
        assert!(!r.is_paused().await);
    }

    #[tokio::test]
    async fn background_sets_paused() {
        let r = ReconnectController::new();
        r.notify_backgrounded().await;
        assert!(r.is_paused().await);
    }
}
