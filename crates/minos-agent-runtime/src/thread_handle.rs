use crate::AgentKind;
use crate::state_machine::ThreadState;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::watch;

#[derive(Clone)]
pub struct ThreadHandle {
    pub thread_id: String,
    pub workspace: PathBuf,
    pub agent: AgentKind,
    pub codex_session_id: Option<String>,
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
            state_tx: Arc::new(tx),
            state_rx: rx,
            last_seq: Arc::new(AtomicU64::new(last_seq)),
        }
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
        assert!(matches!(
            h.current_state(),
            ThreadState::Running { .. }
        ));
    }
}
