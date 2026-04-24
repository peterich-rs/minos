//! Regression anchor for Swift-threads-without-a-runtime calling
//! `DaemonHandle::subscribe`. Phase F.1 replaced the old connection-
//! state `subscribe` with dual `subscribe_relay_link` /
//! `subscribe_peer`; the same "capture rt_handle at start, enter on
//! subscribe" trick guards the new observers.
//!
//! We leave an `#[ignore]`d placeholder until Phase F.6 rewrites this
//! against the relay-backed start path (which needs an in-process
//! relay fixture to stand up a `DaemonHandle`).

#[test]
#[ignore = "Phase F rewrite pending: subscribe_relay_link / subscribe_peer regression needs relay fixture in F.6"]
fn subscribe_from_thread_without_current_runtime_placeholder() {}
