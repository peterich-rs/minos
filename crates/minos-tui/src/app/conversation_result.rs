use super::*;

impl App {
    pub(super) async fn record_agent_conversation_result_if_done(&mut self, session_id: &str) {
        self.record_agent_conversation_result(session_id, false)
            .await;
    }

    pub(super) async fn record_agent_conversation_result_if_ingest_done(
        &mut self,
        session_id: &str,
        allow_ingest_done: bool,
    ) {
        let is_opencode = self
            .ui
            .session_panel
            .list
            .items
            .iter()
            .find(|thread| thread.session_id == session_id)
            .is_some_and(|thread| thread.agent == AgentName::Opencode);
        if is_opencode && !allow_ingest_done {
            return;
        }
        self.record_agent_conversation_result(session_id, allow_ingest_done)
            .await;
    }

    /// No-op: daemon `conversation_completion` owns local `agent-result:…`
    /// writeback (canonical origin formula). TUI must not dual-write bubbles.
    async fn record_agent_conversation_result(
        &mut self,
        session_id: &str,
        allow_ingest_done: bool,
    ) {
        let _ = (session_id, allow_ingest_done);
    }
}
