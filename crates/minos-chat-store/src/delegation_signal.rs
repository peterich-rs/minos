//! In-process wakeups for `wait_delegation`.
//!
//! Completions and cancels publish a per-delegation generation bump so waiters
//! wake immediately instead of busy-polling SQLite. Buses are keyed by absolute
//! DB path so independently opened [`crate::TeamworkStore`] handles on the same
//! file share the same notifier (MCP handler + conversation completion).
//!
//! A slow fallback poll remains in `wait_delegation` for durability if another
//! process mutates the DB without going through this bus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;

#[derive(Debug, Default)]
pub struct DelegationSignalBus {
    keys: Mutex<HashMap<(String, String), Arc<KeyState>>>,
}

#[derive(Debug)]
struct KeyState {
    generation: AtomicU64,
    notify: Notify,
}

impl Default for KeyState {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            notify: Notify::new(),
        }
    }
}

impl DelegationSignalBus {
    /// Shared bus for all stores opened against `db_path` in this process.
    pub fn for_db_path(db_path: &Path) -> Arc<Self> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<DelegationSignalBus>>>> =
            OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let key = normalize_db_path(db_path);
        let mut guard = registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(Self::default()))
            .clone()
    }

    pub fn current_generation(&self, conversation_id: &str, delegation_id: &str) -> u64 {
        self.key_state(conversation_id, delegation_id)
            .generation
            .load(Ordering::SeqCst)
    }

    /// Mark a terminal transition and wake every waiter on this key.
    pub fn notify_terminal(&self, conversation_id: &str, delegation_id: &str) {
        let state = self.key_state(conversation_id, delegation_id);
        state.generation.fetch_add(1, Ordering::SeqCst);
        state.notify.notify_waiters();
    }

    /// Wait until the generation changes from `seen_generation` or `timeout` elapses.
    ///
    /// Returns `true` when a generation change was observed (including one that
    /// raced ahead of the wait). Returns `false` on pure timeout of this slice.
    pub async fn wait_change(
        &self,
        conversation_id: &str,
        delegation_id: &str,
        seen_generation: u64,
        timeout: Duration,
    ) -> bool {
        let state = self.key_state(conversation_id, delegation_id);
        if state.generation.load(Ordering::SeqCst) != seen_generation {
            return true;
        }
        // Subscribe before re-checking generation so a notify between the load
        // above and notified() cannot be lost.
        let notified = state.notify.notified();
        if state.generation.load(Ordering::SeqCst) != seen_generation {
            return true;
        }
        match tokio::time::timeout(timeout, notified).await {
            Ok(()) => true,
            Err(_) => state.generation.load(Ordering::SeqCst) != seen_generation,
        }
    }

    fn key_state(&self, conversation_id: &str, delegation_id: &str) -> Arc<KeyState> {
        let mut guard = self
            .keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .entry((conversation_id.to_owned(), delegation_id.to_owned()))
            .or_insert_with(|| Arc::new(KeyState::default()))
            .clone()
    }
}

fn normalize_db_path(db_path: &Path) -> PathBuf {
    // Best-effort absolute path so relative opens of the same file collide.
    std::fs::canonicalize(db_path).unwrap_or_else(|_| {
        if db_path.is_absolute() {
            db_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(db_path))
                .unwrap_or_else(|_| db_path.to_path_buf())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn notify_wakes_waiter_on_same_db_path() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("teamwork.sqlite");
        // Create the file so canonicalize succeeds consistently.
        std::fs::write(&db, []).unwrap();

        let bus_a = DelegationSignalBus::for_db_path(&db);
        let bus_b = DelegationSignalBus::for_db_path(&db);
        assert!(Arc::ptr_eq(&bus_a, &bus_b));

        let seen = bus_a.current_generation("c1", "d1");
        let waiter = tokio::spawn({
            let bus = bus_a.clone();
            async move {
                bus.wait_change("c1", "d1", seen, Duration::from_secs(2))
                    .await
            }
        });
        tokio::task::yield_now().await;
        bus_b.notify_terminal("c1", "d1");
        assert!(waiter.await.unwrap());
        assert_ne!(bus_a.current_generation("c1", "d1"), seen);
    }
}
