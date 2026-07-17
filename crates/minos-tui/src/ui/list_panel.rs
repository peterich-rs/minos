use ratatui::widgets::ListState;

/// Shared list triad: items + selection index + ratatui `ListState`.
///
/// `selected` is the keyboard/mouse index for the *rendered* list. For most
/// panels that equals `items[i]`. For conversation agent sessions it is the
/// flat tree index — never index `items[selected]` without a flat map.
#[derive(Debug, Default)]
pub struct ListPanel<T> {
    pub items: Vec<T>,
    pub selected: Option<usize>,
    pub list_state: ListState,
}

impl<T> ListPanel<T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected: None,
            list_state: ListState::default(),
        }
    }

    /// Sets selection and syncs `list_state` in one place.
    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index;
        self.list_state.select(index);
    }

    /// Navigate within `len` (wrap at ends). Empty `len` deselects.
    ///
    /// Agent sessions pass `flat_agent_session_count()`, not `items.len()`.
    pub fn navigate_with_len(&mut self, len: usize, delta: i32) {
        if len == 0 {
            self.select(None);
            return;
        }
        let current = self.selected.unwrap_or(0) as i32;
        let mut next = current + delta;
        if next < 0 {
            next = len as i32 - 1;
        }
        if next >= len as i32 {
            next = 0;
        }
        self.select(Some(next as usize));
    }

    pub fn navigate(&mut self, delta: i32) {
        self.navigate_with_len(self.items.len(), delta);
    }

    /// Replace items and select first (or None if empty). Syncs list_state.
    pub fn replace_items(&mut self, items: Vec<T>) {
        self.items = items;
        if self.items.is_empty() {
            self.select(None);
        } else {
            self.select(Some(0));
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.select(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_empty_list_deselects() {
        let mut panel = ListPanel::<u8>::new();
        panel.select(Some(3));
        panel.navigate_with_len(0, 1);
        assert_eq!(panel.selected, None);
        assert_eq!(panel.list_state.selected(), None);
    }

    #[test]
    fn navigate_wraps_and_syncs_list_state() {
        let mut panel = ListPanel::new();
        panel.items = vec!["a", "b", "c"];
        panel.select(Some(0));
        panel.navigate(1);
        assert_eq!(panel.selected, Some(1));
        assert_eq!(panel.list_state.selected(), Some(1));
        panel.navigate(-1);
        assert_eq!(panel.selected, Some(0));
        panel.navigate(-1);
        assert_eq!(panel.selected, Some(2));
        panel.navigate(1);
        assert_eq!(panel.selected, Some(0));
    }

    #[test]
    fn replace_items_selects_first_or_none() {
        let mut panel = ListPanel::new();
        panel.replace_items(vec![1, 2]);
        assert_eq!(panel.selected, Some(0));
        panel.replace_items(Vec::<i32>::new());
        assert_eq!(panel.selected, None);
    }
}
