//! In-memory registry for agent turn completion projection.
//!
//! Armed when Mobile/`client_live` dispatches an agent; drained on host ingest
//! (event-driven). Keyed by **watch_key** = `{origin_message_id}:{session_id}`
//! so multi-@ fan-out (same origin, distinct sessions/agents) never overwrites.
//! Session has a secondary index for multi-watch drain on ingest.
//!
//! # Multi-instance (P6)
//!
//! This registry is **process-local**. Correct multi-replica operation requires
//! co-locating host WebSocket + `agent_dispatch_worker` on the same process so
//! `arm` and host-ingest `try_project_completion_for_session` share memory.
//!
//! Recovery path (already production): host online →
//! [`crate::http::v1::social::on_host_online_force_agent_dispatch`] force-dues
//! pending dispatches → worker re-arms watches on the instance that processes
//! the batch. A shared Redis-backed watch store is deferred until multi-replica
//! host affinity is required at scale.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::store::social::AgentRow;

/// Pending watch for one agent session turn (Hub TurnCompletionProjector).
#[derive(Debug, Clone)]
pub struct CompletionWatch {
    pub dispatch_id: String,
    pub origin_message_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub agent: AgentRow,
    /// Only raw events with seq > this count toward this turn.
    pub raw_seq_floor: u64,
    pub armed_at_ms: i64,
    pub deadline_at_ms: i64,
    pub mention_account_id: Option<String>,
    pub mention_minos_id: Option<String>,
}

impl CompletionWatch {
    /// Stable registry key: one unfinished watch per (origin, session).
    /// Multi-@ fan-out arms one watch per agent session against the same origin.
    #[must_use]
    pub fn watch_key(&self) -> String {
        watch_key(&self.origin_message_id, &self.session_id)
    }
}

/// `origin_message_id` + `session_id` composite key.
#[must_use]
pub fn watch_key(origin_message_id: &str, session_id: &str) -> String {
    format!("{origin_message_id}:{session_id}")
}

#[derive(Debug, Default)]
pub struct CompletionWatchRegistry {
    /// Primary: unfinished watches by [`CompletionWatch::watch_key`].
    by_key: Mutex<HashMap<String, CompletionWatch>>,
    /// Secondary: session_id → set of watch keys.
    by_session: Mutex<HashMap<String, HashSet<String>>>,
}

impl CompletionWatchRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm a watch. Re-arming the same (origin, session) replaces the prior row
    /// (idempotent re-dispatch). Distinct sessions for the same origin coexist
    /// (multi-@ fan-out).
    pub fn arm(&self, watch: CompletionWatch) {
        let key = watch.watch_key();
        let session_id = watch.session_id.clone();
        if let (Ok(mut by_key), Ok(mut by_session)) = (self.by_key.lock(), self.by_session.lock()) {
            if let Some(prev) = by_key.get(&key) {
                if prev.session_id != session_id {
                    if let Some(set) = by_session.get_mut(&prev.session_id) {
                        set.remove(&key);
                        if set.is_empty() {
                            by_session.remove(&prev.session_id);
                        }
                    }
                }
            }
            by_key.insert(key.clone(), watch);
            by_session.entry(session_id).or_default().insert(key);
        }
    }

    pub fn get(&self, watch_key: &str) -> Option<CompletionWatch> {
        self.by_key
            .lock()
            .ok()
            .and_then(|map| map.get(watch_key).cloned())
    }

    /// All unfinished watches for a session (multi-turn / multi-origin).
    pub fn list_for_session(&self, session_id: &str) -> Vec<CompletionWatch> {
        let keys = self
            .by_session
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned())
            .unwrap_or_default();
        let by_key = match self.by_key.lock() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        keys.into_iter()
            .filter_map(|k| by_key.get(&k).cloned())
            .collect()
    }

    pub fn remove(&self, watch_key: &str) -> Option<CompletionWatch> {
        let (Ok(mut by_key), Ok(mut by_session)) = (self.by_key.lock(), self.by_session.lock())
        else {
            return None;
        };
        let watch = by_key.remove(watch_key)?;
        if let Some(set) = by_session.get_mut(&watch.session_id) {
            set.remove(watch_key);
            if set.is_empty() {
                by_session.remove(&watch.session_id);
            }
        }
        Some(watch)
    }

    /// Drain watches whose `deadline_at_ms` is at or before `now_ms`.
    pub fn drain_expired(&self, now_ms: i64) -> Vec<CompletionWatch> {
        let Ok(mut by_key) = self.by_key.lock() else {
            return Vec::new();
        };
        let expired_keys: Vec<String> = by_key
            .iter()
            .filter(|(_, w)| w.deadline_at_ms > 0 && w.deadline_at_ms <= now_ms)
            .map(|(k, _)| k.clone())
            .collect();
        if expired_keys.is_empty() {
            return Vec::new();
        }
        let Ok(mut by_session) = self.by_session.lock() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(expired_keys.len());
        for key in expired_keys {
            if let Some(watch) = by_key.remove(&key) {
                if let Some(set) = by_session.get_mut(&watch.session_id) {
                    set.remove(&key);
                    if set.is_empty() {
                        by_session.remove(&watch.session_id);
                    }
                }
                out.push(watch);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.by_key.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent(id: &str, runtime: &str) -> AgentRow {
        AgentRow {
            agent_id: id.into(),
            owner_account_id: "acc".into(),
            name: runtime.to_string(),
            description: String::new(),
            source: "host_runtime".into(),
            runtime_agent: runtime.into(),
            model: String::new(),
            workspace_path: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn watch(origin: &str, session: &str, floor: u64) -> CompletionWatch {
        CompletionWatch {
            dispatch_id: format!("d-{origin}-{session}"),
            origin_message_id: origin.into(),
            conversation_id: "c1".into(),
            session_id: session.into(),
            agent: sample_agent("a1", "codex"),
            raw_seq_floor: floor,
            armed_at_ms: 0,
            deadline_at_ms: 0,
            mention_account_id: None,
            mention_minos_id: None,
        }
    }

    #[test]
    fn two_origins_same_session_do_not_overwrite() {
        let reg = CompletionWatchRegistry::new();
        reg.arm(watch("o1", "sess", 1));
        reg.arm(watch("o2", "sess", 10));
        assert_eq!(reg.list_for_session("sess").len(), 2);
        assert_eq!(
            reg.get(&watch_key("o1", "sess")).map(|w| w.raw_seq_floor),
            Some(1)
        );
        assert_eq!(
            reg.get(&watch_key("o2", "sess")).map(|w| w.raw_seq_floor),
            Some(10)
        );
        assert!(reg.remove(&watch_key("o1", "sess")).is_some());
        assert_eq!(reg.list_for_session("sess").len(), 1);
        assert!(reg.get(&watch_key("o1", "sess")).is_none());
        assert!(reg.remove(&watch_key("o2", "sess")).is_some());
        assert!(reg.list_for_session("sess").is_empty());
    }

    #[test]
    fn multi_agent_same_origin_coexist() {
        let reg = CompletionWatchRegistry::new();
        reg.arm(watch("origin-1", "sess-codex", 1));
        reg.arm(watch("origin-1", "sess-claude", 1));
        assert_eq!(reg.len(), 2);
        assert!(reg.get(&watch_key("origin-1", "sess-codex")).is_some());
        assert!(reg.get(&watch_key("origin-1", "sess-claude")).is_some());
    }

    #[test]
    fn rearm_same_origin_session_replaces() {
        let reg = CompletionWatchRegistry::new();
        reg.arm(watch("o1", "sess", 1));
        reg.arm(watch("o1", "sess", 5));
        assert_eq!(reg.list_for_session("sess").len(), 1);
        assert_eq!(
            reg.get(&watch_key("o1", "sess")).map(|w| w.raw_seq_floor),
            Some(5)
        );
    }

    #[test]
    fn drain_expired_removes_past_deadline_only() {
        let reg = CompletionWatchRegistry::new();
        let mut live = watch("live", "sess", 1);
        live.deadline_at_ms = 1_000;
        let mut dead = watch("dead", "sess", 2);
        dead.deadline_at_ms = 50;
        reg.arm(live);
        reg.arm(dead);
        let drained = reg.drain_expired(100);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].origin_message_id, "dead");
        assert!(reg.get(&watch_key("dead", "sess")).is_none());
        assert!(reg.get(&watch_key("live", "sess")).is_some());
        assert_eq!(reg.list_for_session("sess").len(), 1);
    }
}
