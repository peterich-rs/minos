use crate::state_machine::ThreadState;
use crate::AgentKind;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone)]
pub struct ThreadHandle {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub agent: AgentKind,
    pub codex_session_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub active_turn_id: Arc<Mutex<Option<String>>>,
    pub state_tx: Arc<watch::Sender<ThreadState>>,
    pub state_rx: watch::Receiver<ThreadState>,
    pub last_seq: Arc<AtomicU64>,
}

impl ThreadHandle {
    pub fn new(
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        initial: ThreadState,
        last_seq: u64,
    ) -> Self {
        let (tx, rx) = watch::channel(initial);
        Self {
            thread_id,
            workspace,
            agent,
            codex_session_id: None,
            parent_thread_id: None,
            active_turn_id: Arc::new(Mutex::new(None)),
            state_tx: Arc::new(tx),
            state_rx: rx,
            last_seq: Arc::new(AtomicU64::new(last_seq)),
        }
    }

    pub fn new_subagent(
        thread_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        parent_thread_id: String,
        provider_session_id: Option<String>,
        initial: ThreadState,
        last_seq: u64,
    ) -> Self {
        let mut handle = Self::new(thread_id, workspace, agent, initial, last_seq);
        handle.parent_thread_id = Some(parent_thread_id);
        handle.codex_session_id = provider_session_id;
        handle
    }

    pub fn current_state(&self) -> ThreadState {
        self.state_rx.borrow().clone()
    }

    pub fn transition(
        &self,
        new: ThreadState,
    ) -> Result<(), crate::state_machine::IllegalTransition> {
        let from = self.current_state();
        crate::state_machine::validate_transition(&from, &new)?;
        let _ = self.state_tx.send(new);
        Ok(())
    }

    pub fn active_turn_id(&self) -> Option<String> {
        self.active_turn_id.lock().unwrap().clone()
    }

    pub fn set_active_turn_id(&self, turn_id: Option<String>) {
        *self.active_turn_id.lock().unwrap() = turn_id;
    }

    pub fn set_active_turn_id_if_absent(&self, turn_id: String) {
        let mut guard = self.active_turn_id.lock().unwrap();
        if guard.is_none() {
            *guard = Some(turn_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::ThreadState;

    #[test]
    fn rejects_illegal_transition() {
        let h = ThreadHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            ThreadState::Idle,
            0,
        );
        let err = h.transition(ThreadState::Starting).unwrap_err();
        assert!(format!("{err}").contains("illegal"));
        assert_eq!(h.current_state(), ThreadState::Idle);
    }

    #[test]
    fn accepts_legal_transition() {
        let h = ThreadHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            ThreadState::Idle,
            0,
        );
        h.transition(ThreadState::Running {
            turn_started_at_ms: 1,
        })
        .unwrap();
        assert!(matches!(h.current_state(), ThreadState::Running { .. }));
    }

    #[test]
    fn active_turn_id_is_shared_across_clones() {
        let h = ThreadHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            ThreadState::Idle,
            0,
        );
        let clone = h.clone();
        h.set_active_turn_id(Some("turn-1".into()));
        assert_eq!(clone.active_turn_id().as_deref(), Some("turn-1"));
        clone.set_active_turn_id(None);
        assert_eq!(h.active_turn_id(), None);
    }
}
