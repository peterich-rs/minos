use super::*;

#[test]
fn detail_tree_cycles_all_panes() {
    let mut focus = FocusManager::new(true);

    let order = (0..6).map(|_| focus.cycle_next()).collect::<Vec<_>>();

    assert_eq!(
        order,
        vec![
            PaneId::AgentList,
            PaneId::AgentChat,
            PaneId::RoomInput,
            PaneId::AgentInput,
            PaneId::GroupChat,
            PaneId::AgentList,
        ]
    );
}

#[test]
fn overview_tree_cycles_all_panes() {
    let mut focus = FocusManager::new(false);

    let order = (0..4).map(|_| focus.cycle_next()).collect::<Vec<_>>();

    assert_eq!(
        order,
        vec![
            PaneId::GroupChat,
            PaneId::AgentList,
            PaneId::RoomInput,
            PaneId::RoomList,
        ]
    );
}

#[test]
fn overview_tree_cycles_previous() {
    let mut focus = FocusManager::new(false);

    focus.cycle_prev();

    assert_eq!(focus.current(), PaneId::RoomInput);
}

#[test]
fn focus_specific_pane() {
    let mut focus = FocusManager::new(true);

    focus.focus(PaneId::AgentChat);

    assert_eq!(focus.current(), PaneId::AgentChat);
}

#[test]
fn switch_layout_falls_back_when_current_pane_is_absent() {
    let mut focus = FocusManager::new(true);
    focus.focus(PaneId::AgentChat);

    focus.switch_layout(false);

    assert_eq!(focus.current(), PaneId::RoomList);
}
