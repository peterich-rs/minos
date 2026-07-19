#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatSelectionPoint {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSelection {
    pub anchor: ChatSelectionPoint,
    pub focus: ChatSelectionPoint,
}

impl ChatSelection {
    pub fn normalized(&self) -> (ChatSelectionPoint, ChatSelectionPoint) {
        if (self.anchor.row, self.anchor.col) <= (self.focus.row, self.focus.col) {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }
}
