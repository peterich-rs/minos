use super::*;
use crate::translation::ChatSelectionPoint;
use minos_domain::AgentName;
use std::collections::HashSet;

fn empty_groups() -> HashSet<String> {
    HashSet::new()
}

fn lines_of(items: &[ChatItem], width: u16) -> Vec<Line<'static>> {
    build_lines(items, &empty_groups(), width)
}

fn user_item(text: &str, is_streaming: bool) -> ChatItem {
    ChatItem::UserMessage {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain(text.into())],
        is_streaming,
    }
}

fn assistant_item(text: &str, is_streaming: bool) -> ChatItem {
    ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain(text.into())],
        is_streaming,
    }
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn user_streaming_item_renders_cursor() {
    let lines = lines_of(&[user_item("sent", true)], 80);

    assert!(lines.iter().any(|line| line_text(line).contains('▍')));
    assert!(lines.iter().any(|line| line_text(line).contains('❯')));
}

#[test]
fn assistant_streaming_item_renders_cursor() {
    let lines = lines_of(&[assistant_item("thinking", true)], 80);

    assert!(lines.iter().any(|line| line_text(line).contains('▍')));
    // Grok-style: no [Agent] role chrome on assistant messages.
    assert!(!lines.iter().any(|line| line_text(line).contains("[Agent]")));
}

#[test]
fn markdown_headings_lists_inline_code_and_fences_render_structurally() {
    let lines = lines_of(
        &[assistant_item(
            "# Plan\n- run `cargo test`\n```rust\nfn main() {}\n```",
            false,
        )],
        80,
    );

    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
    assert!(rendered.iter().any(|line| line == "Plan"));
    assert!(rendered.iter().any(|line| line.contains("• run ")));
    assert!(rendered.iter().any(|line| line.contains("┌─ rust ─")));
    assert!(rendered.iter().any(|line| line.contains("fn main() {}")));
}

#[test]
fn reasoning_renders_as_thinking_with_markdown() {
    let lines = lines_of(
        &[
            ChatItem::Reasoning {
                message_id: "m1".into(),
                text: "# Inspect\n- read `app.rs`".into(),
                is_streaming: false,
                is_user_toggled: Some(true),
            },
            assistant_item("final answer", false),
        ],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("Thought")));
    assert!(rendered.iter().any(|line| line.contains("Inspect")));
    assert!(rendered.iter().any(|line| line.contains("• read ")));
}

#[test]
fn reasoning_defaults_to_collapsed_when_idle() {
    let lines = lines_of(
        &[ChatItem::Reasoning {
            message_id: "m1".into(),
            text: "# Inspect\n- read `app.rs`".into(),
            is_streaming: false,
            is_user_toggled: None,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "Thought"));
    // Full markdown body stays folded; only a one-line dim preview may show.
    assert!(!rendered.iter().any(|line| line.contains("• read ")));
}

#[test]
fn reasoning_auto_expands_while_streaming() {
    let lines = lines_of(
        &[ChatItem::Reasoning {
            message_id: "m1".into(),
            text: "partial thought\n".into(),
            is_streaming: true,
            is_user_toggled: None,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
    assert!(rendered.iter().any(|line| line.contains("Thinking…")));
    assert!(rendered.iter().any(|line| line.contains("partial thought")));
    assert!(rendered.iter().any(|line| line.contains('▍')));
}

#[test]
fn foldable_header_hit_test_finds_tool_and_thinking() {
    let items = vec![
        ChatItem::Reasoning {
            message_id: "m1".into(),
            text: "think".into(),
            is_streaming: false,
            is_user_toggled: None,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_summary: "ls".into(),
            args_detail: Some("detail".into()),
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
    ];
    let mut cache = RenderCache::default();
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());

    // item 0 header at row 0
    assert_eq!(
        cache.foldable_header_item_at_row(&items, 0, &empty_groups()),
        Some(0)
    );
    // item 1: separator at item_starts[1], header at +1
    let tool_start = cache.item_starts()[1];
    assert_eq!(
        cache.foldable_header_item_at_row(&items, tool_start + 1, &empty_groups()),
        Some(1)
    );
    // body/separator rows are not fold headers
    assert_eq!(
        cache.foldable_header_item_at_row(&items, tool_start, &empty_groups()),
        None
    );
}

#[test]
fn item_gap_is_blank_not_full_width_rule() {
    let lines = lines_of(
        &[
            assistant_item("first", false),
            assistant_item("second", false),
        ],
        40,
    );
    // Exactly one blank gap line between the two items' content blocks.
    let texts: Vec<String> = lines.iter().map(line_text).collect();
    assert!(
        !texts
            .iter()
            .any(|t| t.contains('─') && t.chars().count() > 4),
        "full-width rule separators should be gone: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.is_empty()),
        "expected blank gap between items: {texts:?}"
    );
}

#[test]
fn truncate_line_never_exceeds_column_width() {
    let long = Line::from(Span::raw("abcdefghijklmnopqrstuvwxyz0123456789"));
    let clipped = truncate_line_to_width(long, 10);
    let width: usize = clipped
        .spans
        .iter()
        .flat_map(|s| s.content.chars())
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0).max(1))
        .sum();
    assert!(width <= 10, "clipped width {width} > 10");
}

#[test]
fn toggle_fold_at_tool_index_overrides_auto_expand() {
    let mut chat = ChatState::new("t1".into(), AgentName::Codex);
    chat.items.push(ChatItem::ToolCall {
        message_id: "m1".into(),
        tool_call_id: "tc1".into(),
        name: "bash".into(),
        args_summary: "ls".into(),
        args_detail: Some("hidden".into()),
        output_summary: Some("ok".into()),
        output_detail: None,
        is_error: false,
        is_expanded: true,
        is_user_toggled: None,
        is_streaming: false,
    });
    assert!(chat.items[0].is_fold_expanded());
    assert!(chat.toggle_fold_at(0));
    assert!(!chat.items[0].is_fold_expanded());
}

#[test]
fn diff_lines_get_diff_styles_without_treating_markdown_bullets_as_diff() {
    let lines = lines_of(
        &[assistant_item(
            "- markdown bullet\n```diff\n@@ -1 +1\n-old\n+new\n```",
            false,
        )],
        80,
    );

    let bullet = lines
        .iter()
        .find(|line| line_text(line).contains("markdown bullet"))
        .expect("bullet line");
    assert!(line_text(bullet).starts_with("• "));
    let added = lines
        .iter()
        .find(|line| line_text(line).contains("+new"))
        .expect("added diff line");
    assert!(
        added.spans.iter().any(|span| {
            span.content.contains("+new")
                && (span.style == super::super::theme::DIFF_ADD
                    || span.style == super::super::theme::DIFF_ADD_BG)
        }),
        "expected DIFF_ADD on +new content: {:?}",
        added.spans
    );
}

#[test]
fn plain_diff_lines_keep_diff_styles() {
    let lines = lines_of(&[assistant_item("+added\n-removed\n@@ hunk", false)], 80);

    let added = lines
        .iter()
        .find(|line| line_text(line).contains("+added"))
        .expect("added diff line");
    assert!(
        added.spans[0].style == super::super::theme::DIFF_ADD
            || added.spans[0].style == super::super::theme::DIFF_ADD_BG
    );

    let removed = lines
        .iter()
        .find(|line| line_text(line).contains("-removed"))
        .expect("removed diff line");
    assert!(
        removed.spans[0].style == super::super::theme::DIFF_DEL
            || removed.spans[0].style == super::super::theme::DIFF_DEL_BG
    );
}

#[test]
fn non_diff_code_blocks_do_not_color_markdown_lists_as_diff() {
    let lines = lines_of(
        &[assistant_item("```text\n- markdown bullet\n```", false)],
        80,
    );

    let bullet = lines
        .iter()
        .find(|line| line_text(line).contains("- markdown bullet"))
        .expect("code line");
    assert_eq!(bullet.spans[1].style, super::super::theme::MARKDOWN_CODE);
}

#[test]
fn tool_call_item_renders_status_and_summary() {
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "read_file".into(),
            args_summary: "src/main.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    // Singleton still verb-folds (Grok: compact label even for 1 member).
    assert!(
        rendered.iter().any(|line| line.contains("Read 1 file")),
        "expected Grok-style Read group header: {rendered:?}"
    );
}

#[test]
fn verb_group_collapses_multiple_reads() {
    let items = vec![
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "1".into(),
            name: "read_file".into(),
            args_summary: "a.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "2".into(),
            name: "read_file".into(),
            args_summary: "b.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "3".into(),
            name: "read_file".into(),
            args_summary: "c.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
    ];
    let rendered = lines_of(&items, 80)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>();
    assert_eq!(
        rendered.iter().filter(|l| l.contains("Read")).count(),
        1,
        "collapsed group is one header: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|l| l.contains("Read 3 files")),
        "expected aggregate label: {rendered:?}"
    );
    assert!(!rendered.iter().any(|l| l.contains("a.rs")));
}

#[test]
fn verb_group_expand_reveals_member_paths() {
    let mut chat = ChatState::new("t1".into(), AgentName::Codex);
    chat.items = vec![
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "1".into(),
            name: "read_file".into(),
            args_summary: "a.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "2".into(),
            name: "read_file".into(),
            args_summary: "b.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
    ];
    assert!(chat.toggle_fold_at(0)); // expand group
    let rendered = build_lines(&chat.items, &chat.verb_group_expanded, 80)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>();
    assert!(
        rendered.iter().any(|l| l.contains("Read 2 files")),
        "header remains when expanded: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|l| l.contains("a.rs")),
        "members visible: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|l| l.contains("b.rs")),
        "members visible: {rendered:?}"
    );
}

#[test]
fn tool_call_user_toggle_overrides_auto_expansion() {
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_summary: "patch".into(),
            args_detail: Some("hidden detail".into()),
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: true,
            is_user_toggled: Some(false),
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(!rendered.iter().any(|line| line.contains("hidden detail")));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ran") && line.contains("patch")),
        "expected collapsed Ran header: {rendered:?}"
    );
}

#[test]
fn tool_call_expanded_diff_uses_diff_block_not_markdown_lists() {
    let diff = "diff --git a/x.rs b/x.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "apply_patch".into(),
            args_summary: "x.rs".into(),
            args_detail: None,
            output_summary: Some("+1/-1".into()),
            output_detail: Some(diff.into()),
            is_error: false,
            is_expanded: true,
            is_user_toggled: None,
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Edited") && line.contains("x.rs")),
        "expected Edited header: {rendered:?}"
    );
    // Grok edit surface: no boxed ```diff chrome on tool expand.
    assert!(!rendered.iter().any(|line| line.contains("┌─ diff ─")));
    let added = lines
        .iter()
        .find(|line| line_text(line).contains("+new"))
        .expect("+new line");
    assert!(
        added.spans.iter().any(|span| {
            span.content.contains("+new")
                && (span.style == super::super::theme::DIFF_ADD
                    || span.style == super::super::theme::DIFF_ADD_BG)
        }),
        "expected DIFF_ADD style on +new, spans={:?}",
        added.spans
    );
    // Indent + single gutter (no dual │ border box).
    assert!(
        line_text(added).starts_with("  "),
        "expected indented tool diff line: {}",
        line_text(added)
    );
}

#[test]
fn tool_call_collapsed_edit_shows_colored_diffstat() {
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "apply_patch".into(),
            args_summary: "x.rs".into(),
            args_detail: None,
            output_summary: Some("+3/-1".into()),
            output_detail: Some("diff --git a/x.rs b/x.rs\n".into()),
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        }],
        80,
    );
    let header = lines
        .iter()
        .find(|line| line_text(line).contains("Edited"))
        .expect("header");
    let text = line_text(header);
    assert!(
        text.contains("x.rs") && text.contains("+3") && text.contains("-1"),
        "{text}"
    );
    assert!(
        header
            .spans
            .iter()
            .any(|s| s.content.contains("+3") && s.style == super::super::theme::DIFF_ADD),
        "expected green +N span: {:?}",
        header.spans
    );
}

#[test]
fn tool_call_expanded_json_args_pretty_print() {
    // Unknown/other tools still pretty-print JSON args when expanded.
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "custom_tool".into(),
            args_summary: "a.rs".into(),
            args_detail: Some(r#"{"path":"a.rs","offset":1}"#.into()),
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: true,
            is_user_toggled: None,
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
    // Unbordered preformatted JSON (Grok tool chrome is quieter than ``` boxes).
    assert!(!rendered.iter().any(|line| line.contains("┌─ json ─")));
    assert!(rendered.iter().any(|line| line.contains("\"path\"")));
}

#[test]
fn tool_call_execute_expanded_shows_shell_line() {
    let lines = lines_of(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_summary: "cargo test".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: Some("running 3 tests\nok\n".into()),
            is_error: false,
            is_expanded: true,
            is_user_toggled: None,
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("$ ") && line.contains("cargo test")),
        "expected $ command line: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| line.contains("running 3 tests")),
        "expected stdout body: {rendered:?}"
    );
}

#[test]
fn markdown_table_renders_in_assistant_text() {
    let lines = lines_of(
        &[assistant_item(
            "| A | B |\n| --- | --- |\n| 1 | 2 |\n",
            false,
        )],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();
    assert!(rendered
        .iter()
        .any(|line| line.contains('A') && line.contains('B')));
    assert!(rendered
        .iter()
        .any(|line| line.contains('1') && line.contains('2')));
}

#[test]
fn selected_text_copies_after_wrapping_model() {
    let mut chat = ChatState::new("t1".into(), AgentName::Codex);
    // User lines are `❯ hello` / `  world` — select within the first body row.
    chat.items.push(user_item("hello\nworld", false));
    chat.selection = Some(ChatSelection {
        anchor: ChatSelectionPoint { row: 0, col: 2 },
        focus: ChatSelectionPoint { row: 0, col: 6 },
    });

    let cache = RenderCache::default();
    let text = selected_text(&chat, 80, &cache).expect("selection");
    assert!(
        text.contains("hello") || text.contains("ello"),
        "unexpected selection {text:?}"
    );
}

#[test]
fn apply_selection_with_offset_highlights_correct_absolute_rows() {
    let mut lines: Vec<VisualLine> = (0..10)
        .map(|i| VisualLine {
            line: Line::from(Span::raw(format!("line{i}"))),
            text: format!("line{i}"),
        })
        .collect();

    let selection = ChatSelection {
        anchor: ChatSelectionPoint { row: 5, col: 0 },
        focus: ChatSelectionPoint { row: 6, col: 5 },
    };

    // base_row=3, so absolute rows 5,6 = local rows 2,3
    apply_selection_with_offset(&mut lines, Some(&selection), 3);
    // Should not panic; lines 2 and 3 should have been processed
    assert!(!lines[2].line.spans.is_empty());
    assert!(!lines[3].line.spans.is_empty());
}

#[test]
fn counting_sink_matches_vec_sink_line_count() {
    let items = [
        ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain(
                "# Heading\n\nA long line that will definitely wrap at width 20: the quick brown fox jumps over the lazy dog repeatedly\n```\ncode line\ncode line 2\n```".into(),
            )],
            is_streaming: false,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "bash".into(),
            args_summary: "ls -la".into(),
            args_detail: Some("detailed args".into()),
            output_summary: Some("file1.txt\nfile2.txt".into()),
            output_detail: None,
            is_error: false,
            is_expanded: true,
            is_user_toggled: None,
            is_streaming: false,
        },
    ];

    for width in [10u16, 20, 40, 80] {
        // Build via VecSink, then wrap to count visual lines
        let mut vec_sink = VecSink(Vec::new());
        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                vec_sink.push_line(separator_line(width));
            }
            build_item_lines(&mut vec_sink, item);
        }
        let actual_count = visual_lines(vec_sink.0, width).len();

        // Build via CountingSink
        let mut counting_sink = CountingSink { width, count: 0 };
        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                counting_sink.push_line(separator_line(width));
            }
            build_item_lines(&mut counting_sink, item);
        }

        assert_eq!(
            counting_sink.count, actual_count,
            "CountingSink mismatch at width {width}"
        );
    }
}

#[test]
fn counting_sink_counts_soft_wrapped_visual_lines() {
    let text = "abcdefghijklmno"; // 15 chars, wraps to 2 rows at width 10
    let mut vec_sink = VecSink(Vec::new());
    vec_sink.push_line(Line::from(Span::raw(text)));
    let wrapped = visual_lines(vec_sink.0, 10);
    assert_eq!(wrapped.len(), 2);

    let mut counting_sink = CountingSink {
        width: 10,
        count: 0,
    };
    counting_sink.push_line(Line::from(Span::raw(text)));
    assert_eq!(counting_sink.count, 2);
}

#[test]
fn render_cache_settles_viewport_and_leaves_far_history_estimated() {
    let mut cache = RenderCache::default();
    // Enough short rows that warm-up (3 viewports) cannot cover the top.
    let items: Vec<_> = (0..400)
        .map(|i| assistant_item(&format!("message body number {i} with enough text"), false))
        .collect();
    // Bottom-pinned prepare: bottom window (+ warm band) is exact; far history not.
    let _scroll = cache.prepare_layout(super::LayoutPass {
        session_id: "t1",
        items: &items,
        version: 1,
        structure_version: 1,
        width: 80,
        verb_group_expanded: &empty_groups(),
        viewport_height: 20,
        follow_mode: true,
        scroll_offset: 0,
    });
    assert!(
        cache.is_measured(399),
        "bottom entry must be exact after follow prepare"
    );
    // Warm-up measures a few viewports above the bottom; well above that
    // band must stay estimated (Grok deliberately skips above-margin).
    assert!(
        !cache.is_measured(50),
        "history above the warm band stays estimated"
    );

    // Manual scroll to top: settle amortizes exact measure across frames.
    for _ in 0..16 {
        let _ = cache.prepare_layout(super::LayoutPass {
            session_id: "t1",
            items: &items,
            version: 1,
            structure_version: 1,
            width: 80,
            verb_group_expanded: &empty_groups(),
            viewport_height: 20,
            follow_mode: false,
            scroll_offset: 0,
        });
        if cache.is_measured(0) {
            break;
        }
    }
    assert!(
        cache.is_measured(0),
        "scrolling to top exact-measures the visible window within a few frames"
    );
}

#[test]
fn render_cache_rebuilds_on_version_change() {
    let mut cache = RenderCache::default();
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello world".into())],
        is_streaming: false,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    assert!(cache.is_valid("t1", 1, 80));
    assert!(!cache.is_valid("t1", 2, 80));
    cache.rebuild_if_stale("t1", &items, 2, 2, 80, &empty_groups());
    assert!(cache.is_valid("t1", 2, 80));
}

#[test]
fn render_cache_rebuilds_on_width_change() {
    let mut cache = RenderCache::default();
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello world".into())],
        is_streaming: false,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    assert!(cache.is_valid("t1", 1, 80));
    assert!(!cache.is_valid("t1", 1, 40));
}

#[test]
fn render_cache_rebuilds_on_session_id_change() {
    let mut cache = RenderCache::default();
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello world".into())],
        is_streaming: false,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    assert!(cache.is_valid("t1", 1, 80));
    assert!(!cache.is_valid("t2", 1, 80));
}

#[test]
fn visible_window_returns_items_covering_scroll_range() {
    let items = vec![
        ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("line1".into())],
            is_streaming: false,
        },
        ChatItem::AssistantText {
            message_id: "m2".into(),
            text_parts: vec![TextPart::Plain("line2".into())],
            is_streaming: false,
        },
        ChatItem::AssistantText {
            message_id: "m3".into(),
            text_parts: vec![TextPart::Plain("line3".into())],
            is_streaming: false,
        },
    ];
    let mut cache = RenderCache::default();
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    let window = cache.visible_window(&items, 3, 3);
    assert!(window.start_item_index <= 2);
    assert!(!window.items.is_empty());
}

#[test]
fn visible_window_handles_scroll_at_boundary() {
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("line1".into())],
        is_streaming: false,
    }];
    let mut cache = RenderCache::default();
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    let window = cache.visible_window(&items, 0, 10);
    assert_eq!(window.start_item_index, 0);
    assert_eq!(window.line_offset_within_first_segment, 0);
}

#[test]
fn visible_window_handles_empty_items() {
    let cache = RenderCache::default();
    let items: Vec<ChatItem> = vec![];
    let window = cache.visible_window(&items, 0, 10);
    assert!(window.items.is_empty());
}

#[test]
fn render_cache_separator_offset_matches_full_build() {
    // Bare assistant lines (no role chrome): hello / gap+world / gap+line3.
    // item_starts: [0, 1, 3]
    let items = vec![
        ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("hello".into())],
            is_streaming: false,
        },
        ChatItem::AssistantText {
            message_id: "m2".into(),
            text_parts: vec![TextPart::Plain("world".into())],
            is_streaming: false,
        },
        ChatItem::AssistantText {
            message_id: "m3".into(),
            text_parts: vec![TextPart::Plain("line3".into())],
            is_streaming: false,
        },
    ];

    let mut cache = RenderCache::default();
    // Tall viewport settles every entry exactly so total_lines matches full build.
    let _ = cache.prepare_layout(super::LayoutPass {
        session_id: "t1",
        items: &items,
        version: 1,
        structure_version: 1,
        width: 80,
        verb_group_expanded: &empty_groups(),
        viewport_height: 80,
        follow_mode: false,
        scroll_offset: 0,
    });

    let full_lines = lines_of(&items, 80);
    let full_visual = visual_lines(full_lines, 80);
    assert_eq!(
        cache.total_lines(),
        full_visual.len(),
        "cache total_lines must match full build visual line count"
    );

    assert_eq!(cache.item_starts(), &[0, 1, 3]);

    // row 2 is "world" content inside item 1 (start=1, gap at 1, content at 2)
    let window = cache.visible_window(&items, 2, 2);
    assert_eq!(window.start_item_index, 1);
    assert_eq!(window.line_offset_within_first_segment, 1);
}

#[test]
fn render_cache_visible_lines_match_full_build_slice() {
    let items = vec![
        assistant_item(
            "alpha beta gamma delta epsilon zeta eta theta iota kappa",
            false,
        ),
        assistant_item("second item\nwith two logical lines", false),
        assistant_item("third item", false),
    ];
    let width = 16;
    let mut cache = RenderCache::default();
    let _ = cache.prepare_layout(super::LayoutPass {
        session_id: "t1",
        items: &items,
        version: 1,
        structure_version: 1,
        width,
        verb_group_expanded: &empty_groups(),
        viewport_height: 40,
        follow_mode: false,
        scroll_offset: 0,
    });

    let full = visual_lines(lines_of(&items, width), width);
    let base_row = 3;
    let height = 7;
    let cached = cache.visible_visual_lines(base_row, height);

    assert_eq!(
        cached
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        full[base_row..base_row + cached.len()]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn holdback_drops_open_code_fence_while_streaming() {
    let text = "intro\n```rust\nfn main() {\n";
    let held = holdback_streaming_unstable_suffix(text);
    assert_eq!(held, "intro");
}

#[test]
fn holdback_drops_incomplete_trailing_table() {
    let text = "before\n| col a | col b |\n| more |";
    let held = holdback_streaming_unstable_suffix(text);
    assert_eq!(held, "before");
}

#[test]
fn holdback_holds_confirmed_table_until_stream_ends() {
    // Codex-style: once header+delimiter seen, entire table stays in mutable tail
    // while streaming (final paint uses full text when is_streaming=false).
    let text = "before\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
    let held = holdback_streaming_unstable_suffix(text);
    assert_eq!(held, "before");
}

#[test]
fn render_cache_streaming_tail_keeps_structure_version() {
    let mut cache = RenderCache::default();
    let mut items = vec![
        ChatItem::AssistantText {
            message_id: "m1".into(),
            text_parts: vec![TextPart::Plain("committed".into())],
            is_streaming: false,
        },
        ChatItem::AssistantText {
            message_id: "m2".into(),
            text_parts: vec![TextPart::Plain("stream".into())],
            is_streaming: true,
        },
    ];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    let lines_before = cache.total_lines();

    // Only tail content changes; structure_version stays 1.
    if let ChatItem::AssistantText { text_parts, .. } = &mut items[1] {
        text_parts[0] = TextPart::Plain("stream more text".into());
    }
    cache.rebuild_if_stale("t1", &items, 2, 1, 80, &empty_groups());
    assert!(cache.total_lines() >= lines_before);
    assert!(cache.is_valid("t1", 2, 80));
}

#[test]
fn stream_commit_extends_stable_prefix_without_dropping_prior_body() {
    let mut cache = RenderCache::default();
    // Complete lines only so holdback keeps them as stable.
    let mut items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("line one\n".into())],
        is_streaming: true,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    // Streaming rows are exact-measured on rebuild (live tail).
    let stable1 = cache.stream_commit_stable_len(0).expect("commit");
    assert!(stable1 >= "line one\n".len() - 1); // may trim trailing newline
    let body1 = cache.segment_body_line_count(0).unwrap_or(0);

    if let ChatItem::AssistantText { text_parts, .. } = &mut items[0] {
        text_parts[0] = TextPart::Plain("line one\nline two\n".into());
    }
    cache.rebuild_if_stale("t1", &items, 2, 1, 80, &empty_groups());
    let stable2 = cache.stream_commit_stable_len(0).expect("commit grew");
    assert!(stable2 > stable1);
    let body2 = cache.segment_body_line_count(0).unwrap_or(0);
    assert!(body2 >= body1);
}

#[test]
fn stream_commit_mid_paragraph_deltas_stay_one_visual_line() {
    // Regression: delta-append path rendered each token as its own Paragraph line.
    // Full stable re-render must keep contiguous prose as one logical/visual line.
    let mut cache = RenderCache::default();
    let mut items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("thinking...".into())],
        is_streaming: true,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    let lines1 = cache.segment_body_line_count(0).unwrap_or(0);

    if let ChatItem::AssistantText { text_parts, .. } = &mut items[0] {
        text_parts[0] = TextPart::Plain("thinking...user asked me".into());
    }
    cache.rebuild_if_stale("t1", &items, 2, 1, 80, &empty_groups());
    let lines2 = cache.segment_body_line_count(0).unwrap_or(0);
    // Same width, no newline → body line count must not grow from token append.
    assert_eq!(
        lines2, lines1,
        "mid-paragraph stream must not invent visual lines (got {lines1} → {lines2})"
    );
}

#[test]
fn stream_commit_continues_last_visual_line_when_width_allows() {
    // width=20 fits "hello world!" on one line. Delta-append would keep the
    // frozen "hello wor" line and extend "ld!" as a second visual line.
    let width = 20u16;
    let mut cache = RenderCache::default();
    let mut items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello wor".into())],
        is_streaming: true,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, width, &empty_groups());
    let lines1 = cache.segment_body_line_count(0).unwrap_or(0);
    assert_eq!(lines1, 1, "prefix should be a single visual line");

    if let ChatItem::AssistantText { text_parts, .. } = &mut items[0] {
        text_parts[0] = TextPart::Plain("hello world!".into());
    }
    cache.rebuild_if_stale("t1", &items, 2, 1, width, &empty_groups());
    let lines2 = cache.segment_body_line_count(0).unwrap_or(0);
    assert_eq!(
        lines2, 1,
        "full stable re-render must keep prose on one visual line when width allows"
    );
}

#[test]
fn stream_commit_clears_when_streaming_ends() {
    let mut cache = RenderCache::default();
    let mut items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello\n".into())],
        is_streaming: true,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 1, 80, &empty_groups());
    assert!(cache.stream_commit_stable_len(0).is_some());

    if let ChatItem::AssistantText {
        is_streaming,
        text_parts,
        ..
    } = &mut items[0]
    {
        *is_streaming = false;
        text_parts[0] = TextPart::Plain("hello\nworld\n".into());
    }
    cache.rebuild_if_stale("t1", &items, 2, 1, 80, &empty_groups());
    assert!(cache.stream_commit_stable_len(0).is_none());
}

#[test]
fn render_cache_reuses_runs_across_settle_frames_into_history() {
    // Scroll into unmeasured history across multiple prepare_layout frames.
    // Previously each exact measure re-ran find_runs (O(n) × items measured).
    let mut cache = RenderCache::default();
    let items: Vec<_> = (0..200)
        .map(|i| assistant_item(&format!("history row {i} with some body text"), false))
        .collect();

    // Bottom-pinned: far history stays estimated.
    let _ = cache.prepare_layout(super::LayoutPass {
        session_id: "t1",
        items: &items,
        version: 1,
        structure_version: 1,
        width: 80,
        verb_group_expanded: &empty_groups(),
        viewport_height: 12,
        follow_mode: true,
        scroll_offset: 0,
    });
    assert!(
        !cache.is_measured(5),
        "far history stays estimated after follow"
    );

    // Manual scroll to top amortizes exact measure; layout must stay consistent.
    let mut frames = 0u32;
    for _ in 0..32 {
        frames += 1;
        let _ = cache.prepare_layout(super::LayoutPass {
            session_id: "t1",
            items: &items,
            version: 1,
            structure_version: 1,
            width: 80,
            verb_group_expanded: &empty_groups(),
            viewport_height: 12,
            follow_mode: false,
            scroll_offset: 0,
        });
        if cache.is_measured(0) {
            break;
        }
    }
    assert!(
        cache.is_measured(0),
        "top settles within amortized frames (used {frames})"
    );
    // Content-only version bump must keep structure runs cache usable.
    let lines = cache.total_lines();
    let _ = cache.prepare_layout(super::LayoutPass {
        session_id: "t1",
        items: &items,
        version: 2,
        structure_version: 1,
        width: 80,
        verb_group_expanded: &empty_groups(),
        viewport_height: 12,
        follow_mode: false,
        scroll_offset: 0,
    });
    assert_eq!(cache.total_lines(), lines);
}

#[test]
fn build_segment_visual_lines_uses_provided_runs_without_recomputing() {
    // Collapsed tool group: only the header is visible; members are Hidden.
    let items = vec![
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "t1".into(),
            name: "read_file".into(),
            args_summary: "a.rs".into(),
            args_detail: None,
            output_summary: None,
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
        ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "t2".into(),
            name: "read_file".into(),
            args_summary: "b.rs".into(),
            args_detail: None,
            output_summary: None,
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        },
    ];
    let runs = crate::translation::find_runs(&items, &empty_groups());
    assert!(
        !runs.is_empty(),
        "two sequential file tools form a fold run"
    );

    let header = build_segment_visual_lines(0, &items[0], &items, 80, &runs);
    assert!(!header.is_empty(), "collapsed header paints");
    let hidden = build_segment_visual_lines(1, &items[1], &items, 80, &runs);
    assert!(
        hidden.is_empty(),
        "collapsed member must stay hidden when runs mark it Hidden"
    );
}
