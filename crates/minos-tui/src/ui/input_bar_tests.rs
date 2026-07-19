use super::*;

#[test]
fn supports_mid_string_editing() {
    let mut state = InputState::new(false);
    state.content = "helo".into();
    state.cursor_pos = 3;

    assert!(state.move_left());
    state.insert_char('l');

    assert_eq!(state.content, "hello");
    assert_eq!(state.cursor_pos, 3);
}

#[test]
fn word_motion_and_word_delete_follow_whitespace_chunks() {
    let mut state = InputState::new(false);
    state.content = "hello   brave new world".into();
    state.cursor_pos = state.content.len();

    assert!(state.delete_prev_word());
    assert_eq!(state.content, "hello   brave new ");
    assert_eq!(state.cursor_pos, "hello   brave new ".len());

    assert!(state.move_word_left());
    assert_eq!(state.cursor_pos, "hello   brave ".len());

    assert!(state.delete_next_word());
    assert_eq!(state.content, "hello   brave  ");
}

#[test]
fn vertical_motion_preserves_column_when_possible() {
    let mut state = InputState::new(false);
    state.content = "alpha\nbravo\ncar".into();
    state.cursor_pos = "alpha\nbra".len();

    assert!(state.move_down());
    assert_eq!(state.cursor_pos, "alpha\nbravo\ncar".len());

    assert!(state.move_up());
    assert_eq!(state.cursor_pos, "alpha\nbra".len());
}

#[test]
fn required_height_grows_with_multiline_input_and_caps() {
    let mut state = InputState::new(false);
    state.content = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine".into();
    state.cursor_pos = state.content.len();

    assert_eq!(required_height(&state, 40), 10);
}

#[test]
fn existing_agent_candidate_has_no_session_status_label() {
    let installed = AgentMentionCandidate::installed(AgentName::Codex, AgentStatus::Ok);
    let existing = AgentMentionCandidate::existing(
        AgentName::Codex,
        "thread-codex-1234".into(),
        "thread-c".into(),
    );

    assert_eq!(
        agent_picker_status_label(&installed).map(|(label, _)| label),
        Some("install".to_owned())
    );
    assert!(agent_picker_status_label(&existing).is_none());
}

#[test]
fn prompt_history_prev_loads_last_entry() {
    let mut h = PromptHistory::new();
    h.record("first");
    h.record("second");
    assert_eq!(h.previous(""), Some("second"));
}

#[test]
fn prompt_history_prev_walks_backward_then_clamps() {
    let mut h = PromptHistory::new();
    h.record("a");
    h.record("b");
    h.record("c");
    assert_eq!(h.previous(""), Some("c"));
    assert_eq!(h.previous(""), Some("b"));
    assert_eq!(h.previous(""), Some("a"));
    // At oldest entry — returns None (clamp); cursor stays at index 0.
    assert_eq!(h.previous(""), None);
    assert!(h.is_browsing());
}

#[test]
fn prompt_history_next_past_end_returns_none_and_clears_browsing() {
    let mut h = PromptHistory::new();
    h.record("entry");
    let _ = h.previous("my draft");
    assert!(h.is_browsing());
    assert_eq!(h.next(), None);
    assert!(!h.is_browsing());
}

#[test]
fn prompt_history_cancel_restores_draft_and_clears_browsing() {
    let mut h = PromptHistory::new();
    h.record("entry");
    let _ = h.previous("original draft");
    assert!(h.is_browsing());
    assert_eq!(h.cancel(), "original draft");
    assert!(!h.is_browsing());
}

#[test]
fn prompt_history_cancel_without_browsing_returns_empty_draft() {
    let mut h = PromptHistory::new();
    h.record("entry");
    assert!(!h.is_browsing());
    assert_eq!(h.cancel(), "");
}

#[test]
fn prompt_history_record_clears_browsing_state() {
    let mut h = PromptHistory::new();
    h.record("a");
    let _ = h.previous("draft");
    assert!(h.is_browsing());
    h.record("b");
    assert!(!h.is_browsing());
}

#[test]
fn prompt_history_ignores_blank_submissions() {
    let mut h = PromptHistory::new();
    h.record("   ");
    h.record("");
    h.record("\t\n");
    assert_eq!(h.entries.len(), 0);
    assert_eq!(h.previous("current"), None);
}

#[test]
fn prompt_history_empty_returns_none_from_previous() {
    let mut h = PromptHistory::new();
    assert_eq!(h.previous("anything"), None);
    assert!(!h.is_browsing());
}

#[test]
fn prompt_history_next_walks_forward_through_entries() {
    let mut h = PromptHistory::new();
    h.record("a");
    h.record("b");
    h.record("c");
    assert_eq!(h.previous(""), Some("c"));
    assert_eq!(h.previous(""), Some("b"));
    assert_eq!(h.next(), Some("c"));
    assert_eq!(h.next(), None);
}

#[test]
fn last_visual_row_returns_last_row_index_for_soft_wrapped() {
    // 15 chars at width 5 → 3 visual rows (indices 0, 1, 2).
    assert_eq!(last_visual_row("abcdefghijklmno", 5), 2);
}

#[test]
fn last_visual_row_returns_zero_for_single_line() {
    assert_eq!(last_visual_row("hello", 80), 0);
}

#[test]
fn last_visual_row_returns_last_row_for_explicit_newlines() {
    // "hello\nworld" at width 80 → row 0, then \n → row 1, "world" stays → returns 1.
    assert_eq!(last_visual_row("hello\nworld", 80), 1);
}

#[test]
fn last_visual_row_returns_zero_for_empty_string() {
    assert_eq!(last_visual_row("", 80), 0);
}

#[test]
fn visual_cursor_row_at_start_is_zero() {
    assert_eq!(visual_cursor_row("hello world", 0, 80), 0);
}

#[test]
fn visual_cursor_row_after_newline_is_one() {
    let content = "hello\nworld";
    let pos = "hello\n".len();
    assert_eq!(visual_cursor_row(content, pos, 80), 1);
}

#[test]
fn visual_cursor_row_after_soft_wrap_is_one() {
    // 5 chars fill row 0; the 6th char 'f' triggers a wrap to row 1.
    let content = "abcdefghijklmno";
    // cursor at byte 6 (start of 'g', i.e. after 'f' has wrapped) → row 1.
    assert_eq!(visual_cursor_row(content, 6, 5), 1);
    // cursor at byte 11 (start of 'l', after 'k' wrapped) → row 2.
    assert_eq!(visual_cursor_row(content, 11, 5), 2);
    // cursor at byte 5 (boundary) is still on row 0 — no wrap processed yet.
    assert_eq!(visual_cursor_row(content, 5, 5), 0);
}

#[test]
fn active_path_range_extracts_path_token() {
    assert_eq!(active_path_range("hello src/foo", 12), Some(6..12));
    assert_eq!(active_path_range("~/foo", 5), Some(0..5));
    assert_eq!(active_path_range("no path here", 12), None);
    assert_eq!(active_path_range("./bar", 5), Some(0..5));
}

#[test]
fn active_path_range_rejects_plain_words() {
    // No slash present and not starting with ~/ → None
    assert_eq!(active_path_range("hello world", 11), None);
    assert_eq!(active_path_range("word", 4), None);
}

#[test]
fn sync_path_picker_lists_directory_entries() {
    use std::fs;
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    fs::write(root.join("src/foo.rs"), "fn main() {}").expect("write foo.rs");
    fs::write(root.join("src/bar.rs"), "fn other() {}").expect("write bar.rs");

    let mut state = InputState::new(false);
    state.content = "edit src/f".to_owned();
    state.cursor_pos = state.content.len();

    let (sequence, token) = state.sync_path_picker().expect("request");
    let candidates = list_path_candidates(&token, root).expect("candidates");
    assert!(state.apply_path_candidates(sequence, candidates));

    match &state.picker {
        InputPicker::Path(p) => {
            assert_eq!(p.candidates.len(), 1);
            assert_eq!(p.candidates[0].name, "foo.rs");
            assert!(!p.candidates[0].is_dir);
        }
        _ => panic!("expected path picker"),
    }
}

#[test]
fn accept_path_completion_inserts_file_name() {
    use std::fs;
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    fs::write(root.join("src/foo.rs"), "").expect("write foo.rs");

    let mut state = InputState::new(false);
    state.content = "edit src/f".to_owned();
    state.cursor_pos = state.content.len();
    let (sequence, token) = state.sync_path_picker().expect("request");
    let candidates = list_path_candidates(&token, root).expect("candidates");
    assert!(state.apply_path_candidates(sequence, candidates));

    let completed = state.accept_path_completion();
    assert!(completed);
    assert!(state.content.ends_with("src/foo.rs"));
    assert_eq!(state.cursor_pos, state.content.len());
    assert!(matches!(state.picker, InputPicker::None));
}

#[test]
fn accept_path_completion_dir_triggers_re_sync() {
    use std::fs;
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    fs::create_dir_all(root.join("src/sub")).expect("mkdir src/sub");
    fs::write(root.join("src/sub/main.rs"), "").expect("write main.rs");

    let mut state = InputState::new(false);
    state.content = "edit src/su".to_owned();
    state.cursor_pos = state.content.len();
    let (sequence, token) = state.sync_path_picker().expect("request");
    let candidates = list_path_candidates(&token, root).expect("candidates");
    assert!(state.apply_path_candidates(sequence, candidates));

    let completed = state.accept_path_completion();
    assert!(!completed); // dir → caller should re-sync
    assert!(state.content.ends_with("src/sub/"));
}

#[test]
fn path_accept_clears_picker_when_selected_out_of_bounds() {
    let mut state = InputState::new(false);
    state.content = "edit foo".to_owned();
    state.cursor_pos = state.content.len();
    state.picker = InputPicker::Path(super::InputPathPickerState {
        candidates: vec![PathCandidate {
            name: "bar.rs".to_owned(),
            is_dir: false,
        }],
        selected: 5,
        replace_range: 5..8,
    });
    assert!(!state.accept_path_completion());
    assert!(matches!(state.picker, InputPicker::None));
    assert_eq!(state.content, "edit foo");
}

#[test]
fn path_accept_clears_picker_when_replace_range_is_stale() {
    let mut state = InputState::new(false);
    state.content = "x".to_owned();
    state.cursor_pos = 1;
    state.picker = InputPicker::Path(super::InputPathPickerState {
        candidates: vec![PathCandidate {
            name: "bar.rs".to_owned(),
            is_dir: false,
        }],
        selected: 0,
        replace_range: 0..10, // beyond content length
    });
    assert!(!state.accept_path_completion());
    assert!(matches!(state.picker, InputPicker::None));
    assert_eq!(state.content, "x");
}

#[test]
fn path_picker_ignores_stale_async_results() {
    let mut state = InputState::new(false);
    state.content = "edit src/f".to_owned();
    state.cursor_pos = state.content.len();
    let (old_sequence, _) = state.sync_path_picker().expect("old request");

    state.content = "edit src/b".to_owned();
    state.cursor_pos = state.content.len();
    let (new_sequence, _) = state.sync_path_picker().expect("new request");

    assert!(!state.apply_path_candidates(
        old_sequence,
        vec![PathCandidate {
            name: "foo.rs".to_owned(),
            is_dir: false,
        }],
    ));
    assert!(matches!(state.picker, InputPicker::None));

    assert!(state.apply_path_candidates(
        new_sequence,
        vec![PathCandidate {
            name: "bar.rs".to_owned(),
            is_dir: false,
        }],
    ));
    assert!(state.has_path_picker());
}

#[test]
fn path_candidates_match_case_insensitive_substrings() {
    use std::fs;
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    fs::write(root.join("src/AlphaThing.rs"), "").expect("write file");

    let candidates = list_path_candidates("src/thing", root).expect("candidates");
    assert_eq!(candidates[0].name, "AlphaThing.rs");
}

#[test]
fn agent_picker_matches_case_insensitive_substrings() {
    let candidates = vec![AgentMentionCandidate::installed(
        AgentName::Opencode,
        AgentStatus::Ok,
    )];
    let mut state = InputState::new(false);
    state.content = "@CODE".to_owned();
    state.cursor_pos = state.content.len();

    state.sync_agent_picker(&candidates, true);

    match &state.picker {
        InputPicker::Agent(p) => assert_eq!(p.candidate_indices, vec![0]),
        _ => panic!("expected agent picker"),
    }
}

#[test]
fn byte_offset_for_visual_position_clamps_to_line_end() {
    let content = "hello world";
    let offset = byte_offset_for_visual_position(content, 0, 100, 80);
    assert_eq!(offset, content.len());
}

#[test]
fn byte_offset_for_visual_position_handles_multiline() {
    let content = "hello\nworld";
    let offset = byte_offset_for_visual_position(content, 1, 0, 80);
    assert_eq!(offset, 6);
}

#[test]
fn byte_offset_for_visual_position_single_line_basic() {
    let content = "hello";
    assert_eq!(byte_offset_for_visual_position(content, 0, 0, 80), 0);
    assert_eq!(byte_offset_for_visual_position(content, 0, 2, 80), 2);
    assert_eq!(byte_offset_for_visual_position(content, 0, 5, 80), 5);
}

#[test]
fn byte_offset_for_visual_position_multiline_mid_row() {
    let content = "hello\nworld";
    assert_eq!(byte_offset_for_visual_position(content, 1, 3, 80), 9);
}

#[test]
fn byte_offset_for_visual_position_past_last_row_clamps_to_end() {
    let content = "hello\nworld";
    assert_eq!(
        byte_offset_for_visual_position(content, 5, 0, 80),
        content.len()
    );
}

#[test]
fn cursor_style_toggle_flips_between_bar_and_block() {
    let mut state = InputState::new(false);
    assert_eq!(state.cursor_style, CursorStyle::Bar);
    state.toggle_cursor_style();
    assert_eq!(state.cursor_style, CursorStyle::Block);
    state.toggle_cursor_style();
    assert_eq!(state.cursor_style, CursorStyle::Bar);
}

#[test]
fn multiline_toggle_flips_mode() {
    let mut state = InputState::new(false);
    assert!(!state.multiline);
    state.toggle_multiline();
    assert!(state.multiline);
    state.toggle_multiline();
    assert!(!state.multiline);
}
