use super::*;

impl App {
    pub(super) async fn close_current_thread(&mut self) -> bool {
        if self.ui.current_thread_is_subagent() {
            self.ui
                .set_error("Subagent transcripts are read-only.".into());
            return true;
        }
        if let Some(session_id) = self.ui.current_session_id().map(String::from) {
            if let Err(error) = self.backend.close_session(&session_id).await {
                self.ui
                    .set_error(format!("Failed to close thread: {error}"));
            }
            return true;
        }
        false
    }

    pub(super) async fn confirm_delete_session(&mut self) -> bool {
        let Some(pending) = self.ui.overlays.delete_confirm.take() else {
            return false;
        };

        if let Err(error) = self.backend.delete_session(&pending.session_id).await {
            self.ui
                .set_error(format!("Failed to delete thread: {error}"));
            return true;
        }

        self.remove_thread_from_ui(pending.selected_index, &pending.session_id);
        true
    }

    pub(super) fn remove_thread_from_ui(&mut self, selected: usize, session_id: &str) {
        let index = self
            .ui
            .session_panel
            .list
            .items
            .get(selected)
            .filter(|entry| entry.session_id == session_id)
            .map(|_| selected)
            .or_else(|| {
                self.ui
                    .session_panel
                    .list
                    .items
                    .iter()
                    .position(|entry| entry.session_id == session_id)
            });
        let Some(index) = index else {
            return;
        };

        self.ui.session_panel.list.items.remove(index);
        self.ui.session_panel.chat_states.remove(session_id);
        self.state.hydrated_threads.remove(session_id);
        self.state.session_watermarks.remove(session_id);
        self.state
            .applied_ingest_fingerprints
            .retain(|fingerprint| !fingerprint.starts_with(&format!("{session_id}:")));

        if self.ui.session_panel.list.items.is_empty() {
            self.ui.session_panel.list.select(None);
            self.ui.focus.switch_layout(false);
        } else {
            self.select_thread(index.min(self.ui.session_panel.list.items.len().saturating_sub(1)));
        }
        self.sync_input_agent_picker();
    }

    pub(super) fn select_thread(&mut self, index: usize) {
        self.ui.session_panel.list.select(Some(index));
    }

    pub(super) fn current_thread_is_interruptible(&self) -> bool {
        self.ui
            .session_panel
            .list
            .selected
            .and_then(|index| self.ui.session_panel.list.items.get(index))
            .is_some_and(|thread| {
                matches!(
                    thread.state,
                    SessionState::Starting | SessionState::Running { .. } | SessionState::Resuming
                )
            })
    }
}
