#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneId {
    MainList,
    MainChat,
    Sidebar,
    Input,
    #[allow(dead_code)]
    ApprovalOverlay,
}

pub struct FocusManager {
    order: [PaneId; 4],
    index: usize,
}

impl FocusManager {
    pub fn new(_detail: bool) -> Self {
        Self {
            order: [
                PaneId::MainList,
                PaneId::MainChat,
                PaneId::Sidebar,
                PaneId::Input,
            ],
            index: 0,
        }
    }

    pub fn current(&self) -> PaneId {
        self.order[self.index]
    }

    pub fn is(&self, pane: PaneId) -> bool {
        self.current() == pane
    }

    pub fn focus(&mut self, pane: PaneId) {
        if let Some(index) = self.order.iter().position(|candidate| *candidate == pane) {
            self.index = index;
        }
    }

    pub fn cycle_next(&mut self) -> PaneId {
        self.index = (self.index + 1) % self.order.len();
        self.current()
    }

    pub fn cycle_prev(&mut self) -> PaneId {
        self.index = if self.index == 0 {
            self.order.len() - 1
        } else {
            self.index - 1
        };
        self.current()
    }

    #[allow(clippy::unused_self)] // reserved for future layout modes
    pub fn switch_layout(&mut self, _detail: bool) {}
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
