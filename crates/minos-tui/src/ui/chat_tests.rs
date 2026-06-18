use super::*;
use crate::translation::ChatSelectionPoint;
use minos_domain::AgentName;

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
    let lines = build_lines(&[user_item("sent", true)], 80);

    assert!(lines.iter().any(|line| line_text(line).contains('▓')));
}

#[test]
fn assistant_streaming_item_renders_cursor() {
    let lines = build_lines(&[assistant_item("thinking", true)], 80);

    assert!(lines.iter().any(|line| line_text(line).contains('▓')));
}

#[test]
fn markdown_headings_lists_inline_code_and_fences_render_structurally() {
    let lines = build_lines(
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
    let lines = build_lines(
        &[
            ChatItem::Reasoning {
                message_id: "m1".into(),
                text: "# Inspect\n- read `app.rs`".into(),
                is_streaming: false,
            },
            assistant_item("final answer", false),
        ],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "Thinking"));
    assert!(rendered.iter().any(|line| line == "Inspect"));
    assert!(rendered.iter().any(|line| line.contains("• read ")));
}

#[test]
fn diff_lines_get_diff_styles_without_treating_markdown_bullets_as_diff() {
    let lines = build_lines(
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
    assert_eq!(added.spans[1].style, super::super::theme::DIFF_ADD);
}

#[test]
fn non_diff_code_blocks_do_not_color_markdown_lists_as_diff() {
    let lines = build_lines(
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
    let lines = build_lines(
        &[ChatItem::ToolCall {
            message_id: "m1".into(),
            tool_call_id: "tc1".into(),
            name: "read_file".into(),
            args_summary: "file=src/main.rs".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_streaming: false,
        }],
        80,
    );
    let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

    assert!(rendered
        .iter()
        .any(|line| line.contains("Tool read_file") && line.contains("done")));
}

#[test]
fn selected_text_copies_after_wrapping_model() {
    let mut chat = ChatState::new("t1".into(), AgentName::Codex);
    chat.items.push(user_item("hello\nworld", false));
    chat.selection = Some(ChatSelection {
        anchor: ChatSelectionPoint { row: 1, col: 1 },
        focus: ChatSelectionPoint { row: 2, col: 2 },
    });

    let cache = RenderCache::default();
    assert_eq!(
        selected_text(&chat, 80, &cache).as_deref(),
        Some("ello\nwor")
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
fn render_cache_rebuilds_on_version_change() {
    let mut cache = RenderCache::default();
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello world".into())],
        is_streaming: false,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 80);
    assert!(cache.is_valid("t1", 1, 80));
    assert!(!cache.is_valid("t1", 2, 80));
    cache.rebuild_if_stale("t1", &items, 2, 80);
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
    cache.rebuild_if_stale("t1", &items, 1, 80);
    assert!(cache.is_valid("t1", 1, 80));
    assert!(!cache.is_valid("t1", 1, 40));
}

#[test]
fn render_cache_rebuilds_on_thread_id_change() {
    let mut cache = RenderCache::default();
    let items = vec![ChatItem::AssistantText {
        message_id: "m1".into(),
        text_parts: vec![TextPart::Plain("hello world".into())],
        is_streaming: false,
    }];
    cache.rebuild_if_stale("t1", &items, 1, 80);
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
    cache.rebuild_if_stale("t1", &items, 1, 80);
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
    cache.rebuild_if_stale("t1", &items, 1, 80);
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
    // 3 items, each producing [Agent] label + content = 2 content lines.
    // Full build: [Agent, hello, sep, Agent, world, sep, Agent, line3] = 8 visual lines.
    // item_starts should be [0, 2, 5] where item 1 start includes its separator.
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
    cache.rebuild_if_stale("t1", &items, 1, 80);

    // Cross-check total_lines against full build
    let full_lines = build_lines(&items, 80);
    let full_visual = visual_lines(full_lines, 80);
    assert_eq!(
        cache.total_lines(),
        full_visual.len(),
        "cache total_lines must match full build visual line count"
    );

    // Cross-check item_starts: each item_start should point to where
    // [separator?, content...] begins in the full build
    assert_eq!(cache.item_starts(), &[0, 2, 5]);

    // Scroll to item 1's content (row 3 = [Agent] for item 1)
    // item_starts[1] = 2 (separator), line_offset = 3 - 2 = 1
    let window = cache.visible_window(&items, 3, 2);
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
    cache.rebuild_if_stale("t1", &items, 1, width);

    let full = visual_lines(build_lines(&items, width), width);
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
