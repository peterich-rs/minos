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
        let Some(pending) = self.ui.overlays.delete_confirm.take() else {
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
            .thread_panel
            .list
            .items
            .get(selected)
            .filter(|entry| entry.thread_id == thread_id)
            .map(|_| selected)
            .or_else(|| {
                self.ui
                    .thread_panel
                    .list
                    .items
                    .iter()
                    .position(|entry| entry.thread_id == thread_id)
            });
        let Some(index) = index else {
            return;
        };

        self.ui.thread_panel.list.items.remove(index);
        self.ui.thread_panel.chat_states.remove(thread_id);
        self.state.hydrated_threads.remove(thread_id);
        self.state.thread_watermarks.remove(thread_id);
        self.state
            .applied_ingest_fingerprints
            .retain(|fingerprint| !fingerprint.starts_with(&format!("{thread_id}:")));

        if self.ui.thread_panel.list.items.is_empty() {
            self.ui.thread_panel.list.select(None);
            self.ui.focus.switch_layout(false);
        } else {
            self.select_thread(index.min(self.ui.thread_panel.list.items.len().saturating_sub(1)));
        }
        self.sync_input_agent_picker();
    }

    pub(super) fn select_thread(&mut self, index: usize) {
        self.ui.thread_panel.list.select(Some(index));
    }

    pub(super) fn current_thread_is_interruptible(&self) -> bool {
        self.ui
            .thread_panel
            .list
            .selected
            .and_then(|index| self.ui.thread_panel.list.items.get(index))
            .is_some_and(|thread| {
                matches!(
                    thread.state,
                    ThreadState::Starting | ThreadState::Running { .. } | ThreadState::Resuming
                )
            })
    }
}
