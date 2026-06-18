use super::*;

#[test]
fn cycles_all_panes() {
    let mut focus = FocusManager::new(false);

    let order = (0..4).map(|_| focus.cycle_next()).collect::<Vec<_>>();

    assert_eq!(
        order,
        vec![
            PaneId::MainChat,
            PaneId::Sidebar,
            PaneId::Input,
            PaneId::MainList,
        ]
    );
}

#[test]
fn cycles_previous() {
    let mut focus = FocusManager::new(false);

    focus.cycle_prev();

    assert_eq!(focus.current(), PaneId::Input);
}

#[test]
fn focus_specific_pane() {
    let mut focus = FocusManager::new(true);

    focus.focus(PaneId::MainChat);

    assert_eq!(focus.current(), PaneId::MainChat);
}

#[test]
fn switch_layout_keeps_current_pane() {
    let mut focus = FocusManager::new(true);
    focus.focus(PaneId::MainChat);

    focus.switch_layout(false);

    assert_eq!(focus.current(), PaneId::MainChat);
}
