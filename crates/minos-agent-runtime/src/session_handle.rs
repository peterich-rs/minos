use crate::state_machine::SessionState;
use crate::AgentKind;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone)]
pub struct SessionHandle {
    pub session_id: String,
    pub workspace: PathBuf,
    pub agent: AgentKind,
    pub codex_session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub mcp_conversation_id: Option<String>,
    /// Create-time model binding (fixed for the life of this session).
    pub model: Option<String>,
    /// Create-time reasoning effort (when the runtime supports it).
    pub reasoning_effort: Option<String>,
    /// Extra system / developer instructions for this session.
    pub instructions: Option<String>,
    pub active_turn_id: Arc<Mutex<Option<String>>>,
    pub state_tx: Arc<watch::Sender<SessionState>>,
    pub state_rx: watch::Receiver<SessionState>,
    pub last_seq: Arc<AtomicU64>,
}

impl SessionHandle {
    pub fn new(
        session_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        initial: SessionState,
        last_seq: u64,
    ) -> Self {
        let (tx, rx) = watch::channel(initial);
        Self {
            session_id,
            workspace,
            agent,
            codex_session_id: None,
            parent_session_id: None,
            mcp_conversation_id: None,
            model: None,
            reasoning_effort: None,
            instructions: None,
            active_turn_id: Arc::new(Mutex::new(None)),
            state_tx: Arc::new(tx),
            state_rx: rx,
            last_seq: Arc::new(AtomicU64::new(last_seq)),
        }
    }

    #[must_use]
    pub fn with_launch_options(
        mut self,
        model: Option<String>,
        reasoning_effort: Option<String>,
    ) -> Self {
        self.model = model;
        self.reasoning_effort = reasoning_effort;
        self
    }

    #[must_use]
    pub fn with_full_launch_options(
        mut self,
        model: Option<String>,
        reasoning_effort: Option<String>,
        instructions: Option<String>,
    ) -> Self {
        self.model = model;
        self.reasoning_effort = reasoning_effort;
        self.instructions = instructions;
        self
    }

    pub fn new_subagent(
        session_id: String,
        workspace: PathBuf,
        agent: AgentKind,
        parent_session_id: String,
        provider_session_id: Option<String>,
        initial: SessionState,
        last_seq: u64,
    ) -> Self {
        let mut handle = Self::new(session_id, workspace, agent, initial, last_seq);
        handle.parent_session_id = Some(parent_session_id);
        handle.codex_session_id = provider_session_id;
        handle
    }

    pub fn current_state(&self) -> SessionState {
        self.state_rx.borrow().clone()
    }

    pub fn transition(
        &self,
        new: SessionState,
    ) -> Result<(), crate::state_machine::IllegalTransition> {
        let _ = self.transition_if(|_| true, new)?;
        Ok(())
    }

    /// Compare-and-swap session state under the active-turn mutex so concurrent
    /// claimers cannot both succeed on the same transition (e.g. Idle→Running).
    ///
    /// On success returns the previous state. On predicate failure or illegal
    /// transition, leaves state unchanged and returns `IllegalTransition` with
    /// the observed `from`.
    pub fn transition_if<F>(
        &self,
        predicate: F,
        new: SessionState,
    ) -> Result<SessionState, crate::state_machine::IllegalTransition>
    where
        F: FnOnce(&SessionState) -> bool,
    {
        // Serialize CAS with active_turn_id mutations so claim + turn-id clear
        // cannot race mid-transition.
        let _turn_guard = self.active_turn_id.lock().unwrap();
        let from = self.state_rx.borrow().clone();
        if !predicate(&from) {
            return Err(crate::state_machine::IllegalTransition { from, to: new });
        }
        crate::state_machine::validate_transition(&from, &new)?;
        let _ = self.state_tx.send(new);
        Ok(from)
    }

    /// Atomic Idle→Running claim for a new turn. Returns previous state (Idle)
    /// on success; rejects if not Idle or if another claimer already won.
    pub fn try_begin_turn(
        &self,
        turn_started_at_ms: i64,
    ) -> Result<SessionState, crate::state_machine::IllegalTransition> {
        self.transition_if(
            |from| matches!(from, SessionState::Idle),
            SessionState::Running { turn_started_at_ms },
        )
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

    /// Bump the in-memory last_seq watermark when durable/live events are
    /// associated with this session (monotonic).
    pub fn note_event_seq(&self, seq: u64) {
        use std::sync::atomic::Ordering;
        let mut cur = self.last_seq.load(Ordering::SeqCst);
        while seq > cur {
            match self
                .last_seq
                .compare_exchange_weak(cur, seq, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::SessionState;

    #[test]
    fn rejects_illegal_transition() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            0,
        );
        let err = h.transition(SessionState::Starting).unwrap_err();
        assert!(format!("{err}").contains("illegal"));
        assert_eq!(h.current_state(), SessionState::Idle);
    }

    #[test]
    fn accepts_legal_transition() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            0,
        );
        h.transition(SessionState::Running {
            turn_started_at_ms: 1,
        })
        .unwrap();
        assert!(matches!(h.current_state(), SessionState::Running { .. }));
    }

    #[test]
    fn active_turn_id_is_shared_across_clones() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            0,
        );
        let clone = h.clone();
        h.set_active_turn_id(Some("turn-1".into()));
        assert_eq!(clone.active_turn_id().as_deref(), Some("turn-1"));
        clone.set_active_turn_id(None);
        assert_eq!(h.active_turn_id(), None);
    }

    #[test]
    fn try_begin_turn_rejects_double_claim() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            0,
        );
        let prev = h.try_begin_turn(1).unwrap();
        assert_eq!(prev, SessionState::Idle);
        assert!(matches!(h.current_state(), SessionState::Running { .. }));
        let err = h.try_begin_turn(2).unwrap_err();
        assert!(format!("{err}").contains("illegal"));
        assert!(matches!(
            h.current_state(),
            SessionState::Running {
                turn_started_at_ms: 1
            }
        ));
    }

    #[test]
    fn transition_if_predicate_rejects_without_mutating() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            0,
        );
        let err = h
            .transition_if(
                |from| matches!(from, SessionState::Running { .. }),
                SessionState::Suspended {
                    reason: crate::state_machine::PauseReason::CodexCrashed,
                },
            )
            .unwrap_err();
        assert_eq!(err.from, SessionState::Idle);
        assert_eq!(h.current_state(), SessionState::Idle);
    }

    #[test]
    fn note_event_seq_is_monotonic() {
        let h = SessionHandle::new(
            "t".into(),
            "/w".into(),
            AgentKind::Codex,
            SessionState::Idle,
            3,
        );
        h.note_event_seq(2);
        assert_eq!(h.last_seq.load(std::sync::atomic::Ordering::SeqCst), 3);
        h.note_event_seq(5);
        assert_eq!(h.last_seq.load(std::sync::atomic::Ordering::SeqCst), 5);
    }
}
