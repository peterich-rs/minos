use super::*;

impl App {
    pub(super) async fn close_current_thread(&mut self) -> bool {
        if self.ui.current_thread_is_subagent() {
            self.ui
                .set_error("Subagent transcripts are read-only.".into());
            return true;
        }
        if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
            if let Err(error) = self.backend.close_thread(&thread_id).await {
                self.ui
                    .set_error(format!("Failed to close thread: {error}"));
            }
            return true;
        }
        false
    }

    pub(super) async fn confirm_delete_thread(&mut self) -> bool {
        let Some(pending) = self.ui.delete_confirm.take() else {
            return false;
        };

        if let Err(error) = self.backend.delete_thread(&pending.thread_id).await {
            self.ui
                .set_error(format!("Failed to delete thread: {error}"));
            return true;
        }

        self.remove_thread_from_ui(pending.selected_index, &pending.thread_id);
        true
    }

    pub(super) fn remove_thread_from_ui(&mut self, selected: usize, thread_id: &str) {
        let index = self
            .ui
            .threads
            .get(selected)
            .filter(|entry| entry.thread_id == thread_id)
            .map(|_| selected)
            .or_else(|| {
                self.ui
                    .threads
                    .iter()
                    .position(|entry| entry.thread_id == thread_id)
            });
        let Some(index) = index else {
            return;
        };

        self.ui.threads.remove(index);
        self.ui.chat_states.remove(thread_id);
        self.state.hydrated_threads.remove(thread_id);
        self.state.thread_watermarks.remove(thread_id);
        self.state
            .applied_ingest_fingerprints
            .retain(|fingerprint| !fingerprint.starts_with(&format!("{thread_id}:")));

        if self.ui.threads.is_empty() {
            self.ui.selected_thread = None;
            self.ui.agent_list_state.select(None);
            self.ui.focus.switch_layout(false);
        } else {
            self.select_thread(index.min(self.ui.threads.len().saturating_sub(1)));
        }
        self.sync_input_agent_picker();
    }

    pub(super) async fn start_agent_at(&mut self, index: usize) -> bool {
        let Some(agent_name) = self.ui.status.agents.get(index).map(|desc| desc.name) else {
            return false;
        };

        match self.start_new_thread(agent_name).await {
            Ok(_) => {
                self.ui.agent_picker = None;
                true
            }
            Err(error) => {
                self.ui.set_error(error);
                true
            }
        }
    }

    pub(super) fn select_thread(&mut self, index: usize) {
        self.ui.selected_thread = Some(index);
        self.ui.agent_list_state.select(Some(index));
    }

    pub(super) fn current_thread_is_interruptible(&self) -> bool {
        self.ui
            .selected_thread
            .and_then(|index| self.ui.threads.get(index))
            .is_some_and(|thread| {
                matches!(
                    thread.state,
                    ThreadState::Starting | ThreadState::Running { .. } | ThreadState::Resuming
                )
            })
    }
}
