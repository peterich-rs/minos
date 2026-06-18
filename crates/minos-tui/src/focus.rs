#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneId {
    RoomList,
    GroupChat,
    AgentList,
    AgentChat,
    RoomInput,
    AgentInput,
}

#[derive(Clone, Debug)]
pub enum FocusNode {
    Pane(PaneId),
    Group(Vec<FocusNode>),
}

pub struct FocusManager {
    tree: FocusNode,
    path: Vec<usize>,
}

impl FocusManager {
    pub fn new(detail: bool) -> Self {
        Self {
            tree: default_focus_tree(detail),
            path: vec![0],
        }
    }

    pub fn current(&self) -> PaneId {
        match self.node_at_path(&self.path) {
            Some(FocusNode::Pane(id)) => *id,
            _ => first_pane(&self.tree).expect("focus tree must contain at least one pane"),
        }
    }

    pub fn is(&self, pane: PaneId) -> bool {
        self.current() == pane
    }

    pub fn focus(&mut self, pane: PaneId) {
        if let Some(path) = find_pane_path(&self.tree, pane) {
            self.path = path;
        }
    }

    pub fn cycle_next(&mut self) -> PaneId {
        let order = self.flatten_panes();
        let current = self.current();
        let index = order.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = order[(index + 1) % order.len()];
        self.focus(next);
        next
    }

    pub fn cycle_prev(&mut self) -> PaneId {
        let order = self.flatten_panes();
        let current = self.current();
        let index = order.iter().position(|pane| *pane == current).unwrap_or(0);
        let previous = if index == 0 {
            order[order.len() - 1]
        } else {
            order[index - 1]
        };
        self.focus(previous);
        previous
    }

    pub fn switch_layout(&mut self, detail: bool) {
        let current = self.current();
        self.tree = default_focus_tree(detail);
        if let Some(path) = find_pane_path(&self.tree, current) {
            self.path = path;
        } else {
            self.path = first_pane_path(&self.tree).unwrap_or_else(|| vec![0]);
        }
    }

    fn flatten_panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        collect_panes(&self.tree, &mut panes);
        panes
    }

    fn node_at_path(&self, path: &[usize]) -> Option<&FocusNode> {
        let mut node = &self.tree;
        for index in path {
            match node {
                FocusNode::Pane(_) => return None,
                FocusNode::Group(children) => node = children.get(*index)?,
            }
        }
        Some(node)
    }
}

fn default_focus_tree(detail: bool) -> FocusNode {
    if detail {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::AgentList),
                FocusNode::Pane(PaneId::AgentChat),
            ]),
            FocusNode::Group(vec![
                FocusNode::Pane(PaneId::RoomInput),
                FocusNode::Pane(PaneId::AgentInput),
            ]),
        ])
    } else {
        FocusNode::Group(vec![
            FocusNode::Pane(PaneId::RoomList),
            FocusNode::Pane(PaneId::GroupChat),
            FocusNode::Pane(PaneId::AgentList),
            FocusNode::Pane(PaneId::RoomInput),
        ])
    }
}

fn collect_panes(node: &FocusNode, out: &mut Vec<PaneId>) {
    match node {
        FocusNode::Pane(id) => out.push(*id),
        FocusNode::Group(children) => {
            for child in children {
                collect_panes(child, out);
            }
        }
    }
}

fn find_pane_path(node: &FocusNode, target: PaneId) -> Option<Vec<usize>> {
    match node {
        FocusNode::Pane(id) if *id == target => Some(Vec::new()),
        FocusNode::Pane(_) => None,
        FocusNode::Group(children) => {
            for (index, child) in children.iter().enumerate() {
                if let Some(mut path) = find_pane_path(child, target) {
                    path.insert(0, index);
                    return Some(path);
                }
            }
            None
        }
    }
}

fn first_pane(node: &FocusNode) -> Option<PaneId> {
    match node {
        FocusNode::Pane(id) => Some(*id),
        FocusNode::Group(children) => children.iter().find_map(first_pane),
    }
}

fn first_pane_path(node: &FocusNode) -> Option<Vec<usize>> {
    match node {
        FocusNode::Pane(_) => Some(Vec::new()),
        FocusNode::Group(children) => {
            for (index, child) in children.iter().enumerate() {
                if let Some(mut path) = first_pane_path(child) {
                    path.insert(0, index);
                    return Some(path);
                }
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
