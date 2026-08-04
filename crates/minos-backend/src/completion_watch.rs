//! In-memory registry for agent turn completion projection.
//!
//! Armed when Mobile/`client_live` dispatches an agent; drained on host ingest
//! (event-driven). Keyed by **origin_message_id** (one watch per user turn);
//! session has a secondary index so multi-turn dispatches never overwrite.
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

#[derive(Debug, Default)]
pub struct CompletionWatchRegistry {
    /// Primary: one unfinished watch per origin user message.
    by_origin: Mutex<HashMap<String, CompletionWatch>>,
    /// Secondary: session_id → set of origin_message_ids.
    by_session: Mutex<HashMap<String, HashSet<String>>>,
}

impl CompletionWatchRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm a watch for `origin_message_id`. Does **not** overwrite an existing
    /// unfinished watch for a different origin on the same session.
    ///
    /// If the same origin is re-armed (idempotent re-dispatch), the watch is
    /// replaced.
    pub fn arm(&self, watch: CompletionWatch) {
        let origin = watch.origin_message_id.clone();
        let session_id = watch.session_id.clone();
        if let (Ok(mut by_origin), Ok(mut by_session)) =
            (self.by_origin.lock(), self.by_session.lock())
        {
            // Drop previous session index entry if origin was re-armed on a
            // different session (unlikely but keep indexes consistent).
            if let Some(prev) = by_origin.get(&origin) {
                if prev.session_id != session_id {
                    if let Some(set) = by_session.get_mut(&prev.session_id) {
                        set.remove(&origin);
                        if set.is_empty() {
                            by_session.remove(&prev.session_id);
                        }
                    }
                }
            }
            by_origin.insert(origin.clone(), watch);
            by_session
                .entry(session_id)
                .or_default()
                .insert(origin);
        }
    }

    pub fn get(&self, origin_message_id: &str) -> Option<CompletionWatch> {
        self.by_origin
            .lock()
            .ok()
            .and_then(|map| map.get(origin_message_id).cloned())
    }

    /// All unfinished watches for a session (multi-turn).
    pub fn list_for_session(&self, session_id: &str) -> Vec<CompletionWatch> {
        let origins = self
            .by_session
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned())
            .unwrap_or_default();
        let by_origin = match self.by_origin.lock() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        origins
            .into_iter()
            .filter_map(|o| by_origin.get(&o).cloned())
            .collect()
    }

    pub fn remove(&self, origin_message_id: &str) -> Option<CompletionWatch> {
        let (Ok(mut by_origin), Ok(mut by_session)) =
            (self.by_origin.lock(), self.by_session.lock())
        else {
            return None;
        };
        let watch = by_origin.remove(origin_message_id)?;
        if let Some(set) = by_session.get_mut(&watch.session_id) {
            set.remove(origin_message_id);
            if set.is_empty() {
                by_session.remove(&watch.session_id);
            }
        }
        Some(watch)
    }

    /// Drain watches whose `deadline_at_ms` is at or before `now_ms`.
    ///
    /// Used by SessionLifecycle TTL: each drained watch must surface a
    /// user-visible failure and never leak in the registry.
    pub fn drain_expired(&self, now_ms: i64) -> Vec<CompletionWatch> {
        let Ok(mut by_origin) = self.by_origin.lock() else {
            return Vec::new();
        };
        let expired_origins: Vec<String> = by_origin
            .iter()
            .filter(|(_, w)| w.deadline_at_ms > 0 && w.deadline_at_ms <= now_ms)
            .map(|(k, _)| k.clone())
            .collect();
        if expired_origins.is_empty() {
            return Vec::new();
        }
        let Ok(mut by_session) = self.by_session.lock() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(expired_origins.len());
        for origin in expired_origins {
            if let Some(watch) = by_origin.remove(&origin) {
                if let Some(set) = by_session.get_mut(&watch.session_id) {
                    set.remove(&origin);
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
        self.by_origin.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent() -> AgentRow {
        AgentRow {
            agent_id: "a1".into(),
            owner_account_id: "acc".into(),
            name: "Codex".into(),
            description: String::new(),
            source: "host_runtime".into(),
            runtime_agent: "codex".into(),
            model: String::new(),
            workspace_path: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn watch(origin: &str, session: &str, floor: u64) -> CompletionWatch {
        CompletionWatch {
            dispatch_id: format!("d-{origin}"),
            origin_message_id: origin.into(),
            conversation_id: "c1".into(),
            session_id: session.into(),
            agent: sample_agent(),
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
        assert_eq!(reg.get("o1").map(|w| w.raw_seq_floor), Some(1));
        assert_eq!(reg.get("o2").map(|w| w.raw_seq_floor), Some(10));
        assert!(reg.remove("o1").is_some());
        assert_eq!(reg.list_for_session("sess").len(), 1);
        assert!(reg.get("o1").is_none());
        assert!(reg.remove("o2").is_some());
        assert!(reg.list_for_session("sess").is_empty());
    }

    #[test]
    fn rearm_same_origin_replaces() {
        let reg = CompletionWatchRegistry::new();
        reg.arm(watch("o1", "sess", 1));
        reg.arm(watch("o1", "sess", 5));
        assert_eq!(reg.list_for_session("sess").len(), 1);
        assert_eq!(reg.get("o1").map(|w| w.raw_seq_floor), Some(5));
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
        assert!(reg.get("dead").is_none());
        assert!(reg.get("live").is_some());
        assert_eq!(reg.list_for_session("sess").len(), 1);
    }
}
