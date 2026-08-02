//! IM connection liveness: heartbeat cadence and timeout policy.
//!
//! Formal gateway owns this path (not the legacy envelope session loop).
//! Clients may use protocol `Ping`/`Pong` and/or WebSocket control ping;
//! either counts as activity.

use std::time::Duration;

/// Advertised to clients in `ServerFrame::Hello.heartbeat_interval_ms`.
pub const HEARTBEAT_INTERVAL_MS: i64 = minos_protocol::realtime::DEFAULT_HEARTBEAT_INTERVAL_MS;

/// How often the gateway probes with a WebSocket `Ping` and checks expiry.
pub const HEARTBEAT_TICK: Duration = Duration::from_secs(15);

/// Close the socket when no inbound activity (text / control ping/pong /
/// client `Ping`) has been observed for this long.
///
/// Sized for ~3–4 missed client intervals at 25–30s, matching legacy
/// envelope `PAIRED_TIMEOUT`.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(90);

/// Throttle durable `last_seen_at_ms` writes during a live socket.
pub const LAST_SEEN_DB_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// WS close code for heartbeat timeout (RFC 6455 internal error).
pub const CLOSE_CODE_HEARTBEAT_TIMEOUT: u16 = 1011;

/// Pure policy: true when `elapsed` exceeds the configured liveness window.
#[must_use]
pub fn is_heartbeat_expired(elapsed: Duration, limit: Duration) -> bool {
    elapsed > limit
}

/// Whether a durable last_seen write should run given time since last DB touch.
#[must_use]
pub fn should_persist_last_seen(since_last_db_touch: Duration, min_interval: Duration) -> bool {
    since_last_db_touch >= min_interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_expired_after_limit() {
        assert!(!is_heartbeat_expired(
            Duration::from_secs(89),
            HEARTBEAT_TIMEOUT
        ));
        assert!(is_heartbeat_expired(
            Duration::from_secs(91),
            HEARTBEAT_TIMEOUT
        ));
    }

    #[test]
    fn last_seen_throttle() {
        assert!(!should_persist_last_seen(
            Duration::from_secs(10),
            LAST_SEEN_DB_MIN_INTERVAL
        ));
        assert!(should_persist_last_seen(
            Duration::from_secs(30),
            LAST_SEEN_DB_MIN_INTERVAL
        ));
    }
}
