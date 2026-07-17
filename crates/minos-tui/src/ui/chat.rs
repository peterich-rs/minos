use crate::render::{
    markdown::{
        looks_like_diff, render_code_block, render_markdown, render_tool_diff,
        render_tool_preformatted, render_tool_read_body, MarkdownStyles,
    },
    Renderable,
};
use crate::translation::{
    find_runs, header_label, paint_mode_with_runs, parse_diffstat, ChatItem, ChatSelection,
    ChatState, PaintMode, TextPart, ToolKind, VerbGroupRun,
};
use std::collections::HashSet;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::{
    ASSISTANT_BODY, BORDER_FG, DIFF_ADD, DIFF_DEL, DIM, ERROR_STYLE, FOCUSED_BORDER, MUTED,
    PROMPT_ARROW, REASONING_STYLE, STREAMING_CURSOR, THINKING_BAR, THINKING_BODY, THINKING_LABEL,
    TOOL_ERROR, TOOL_PATH, TOOL_PATH_MUTED, TOOL_RUNNING, TOOL_SUCCESS, TOOL_VERB, TOOL_VERB_MUTED,
    USER_BODY, USER_PREFIX,
};

mod cache;
pub use cache::{LayoutPass, RenderCache};

trait LineSink {
    fn push_line(&mut self, line: Line<'static>);
}

struct VecSink(Vec<Line<'static>>);
impl LineSink for VecSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}

struct VecSinkRef<'a>(&'a mut Vec<Line<'static>>);
impl<'a> LineSink for VecSinkRef<'a> {
    fn push_line(&mut self, line: Line<'static>) {
        self.0.push(line);
    }
}

#[cfg(test)]
struct CountingSink {
    width: u16,
    count: usize,
}
#[cfg(test)]
impl LineSink for CountingSink {
    fn push_line(&mut self, line: Line<'static>) {
        self.count += visual_line_count(&line, self.width);
    }
}

#[cfg(test)]
fn visual_line_count(line: &Line<'static>, width: u16) -> usize {
    // Must match `visual_lines` wrap rules (including flush when width is filled).
    visual_lines(vec![line.clone()], width).len()
}

pub enum AgentChatTarget<'a> {
    Chat {
        chat: &'a mut ChatState,
        cache: &'a mut RenderCache,
    },
    Empty,
}

pub struct AgentChatRenderable<'a> {
    target: AgentChatTarget<'a>,
    focused: bool,
}

impl<'a> AgentChatRenderable<'a> {
    pub fn new(target: AgentChatTarget<'a>, focused: bool) -> Self {
        Self { target, focused }
    }
}

impl Renderable for AgentChatRenderable<'_> {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        match &mut self.target {
            AgentChatTarget::Chat { chat, cache } => {
                render_chat(f, area, chat, self.focused, cache);
            }
            AgentChatTarget::Empty => {
                render_agent_chat_placeholder(f, area, self.focused);
            }
        }
    }

    fn desired_height(&self, _width: u16) -> u16 {
        match &self.target {
            AgentChatTarget::Chat { chat, .. } => {
                u16::try_from(chat.items.len().saturating_add(2)).unwrap_or(u16::MAX)
            }
            AgentChatTarget::Empty => 4,
        }
    }
}

pub fn render_agent_chat_placeholder(f: &mut Frame, area: Rect, focused: bool) {
    let paragraph = Paragraph::new(
        "No agent selected\n\nChoose an agent from the list to inspect its detailed transcript.",
    )
    .block(
        super::theme::border_block()
            .title("Agent Detail")
            .border_style(if focused {
                FOCUSED_BORDER
            } else {
                ratatui::style::Style::new().fg(BORDER_FG)
            }),
    );
    f.render_widget(paragraph, area);
}

pub fn render_chat(
    f: &mut Frame,
    area: Rect,
    chat: &mut ChatState,
    focused: bool,
    cache: &mut RenderCache,
) {
    let title = format!(
        "Chat: {} #{}{}",
        chat.agent.bin_name(),
        crate::agent_route::short_thread_id(&chat.thread_id),
        if chat.auto_scroll {
            ""
        } else {
            " [manual scroll]"
        }
    );
    let block = super::theme::border_block()
        .title(title)
        .border_style(if focused {
            FOCUSED_BORDER
        } else {
            ratatui::style::Style::new().fg(BORDER_FG)
        });
    let inner = block.inner(area);

    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return;
    }

    let _span = tracing::trace_span!(
        "render_chat",
        item_count = chat.items.len(),
        version = chat.version,
    )
    .entered();

    let height = usize::from(inner.height);
    // Grok-style prepare: estimate full transcript, exact-measure only the
    // viewport window (+ below margin). No above-margin so the top stays anchored.
    let scroll = cache.prepare_layout(cache::LayoutPass {
        thread_id: chat.thread_id.as_str(),
        items: &chat.items,
        version: chat.version,
        structure_version: chat.structure_version,
        width: inner.width,
        verb_group_expanded: &chat.verb_group_expanded,
        viewport_height: inner.height,
        follow_mode: chat.auto_scroll,
        scroll_offset: chat.active_scroll(),
    });
    if !chat.auto_scroll {
        chat.scroll_offset = scroll;
    }

    let max_scroll = u32::try_from(
        cache
            .total_lines()
            .saturating_sub(usize::from(inner.height)),
    )
    .unwrap_or(u32::MAX);
    chat.update_max_scroll(max_scroll);

    let base_row = chat.active_scroll() as usize;

    if cache.total_lines() == 0 {
        let lines = vec![Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        ))];
        f.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    let mut visible_visual_lines = cache.visible_visual_lines(base_row, height);
    if visible_visual_lines.is_empty() {
        visible_visual_lines.push(VisualLine {
            line: Line::from(Span::styled(
                "No messages yet. Press `n` to start another agent, then type below.",
                REASONING_STYLE,
            )),
            text: "No messages yet. Press `n` to start another agent, then type below.".to_owned(),
        });
    }

    apply_selection_with_offset(
        visible_visual_lines.as_mut_slice(),
        chat.selection.as_ref(),
        base_row,
    );

    let max_cols = usize::from(inner.width);
    let lines: Vec<Line<'static>> = visible_visual_lines
        .into_iter()
        .map(|vl| truncate_line_to_width(vl.line, max_cols))
        .collect();

    // Pre-wrapped + hard-truncated lines: no Paragraph wrap (would re-flow).
    // Truncation guarantees we never paint past the chat column into the sidebar.
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Extracts selected text. Uses full-rebuild (not the cache) because copy is
/// infrequent and the selection range may span items unpredictably. The cache
/// parameter is reserved for a future optimization that localizes the build.
pub fn selected_text(chat: &ChatState, width: u16, _cache: &RenderCache) -> Option<String> {
    let selection = chat.selection.as_ref()?;
    if selection.is_empty() || width == 0 {
        return None;
    }

    let lines = visual_lines(
        build_lines(
            chat.items.as_slice(),
            &chat.verb_group_expanded,
            width,
        ),
        width,
    );
    selected_text_from_lines(lines.as_slice(), selection)
}

fn build_lines(
    items: &[ChatItem],
    expanded_ids: &HashSet<String>,
    _width: u16,
) -> Vec<Line<'static>> {
    let mut sink = VecSink(Vec::new());
    let runs = find_runs(items, expanded_ids);
    let mut saw_visible = false;

    for (idx, item) in items.iter().enumerate() {
        let mode = paint_mode_with_runs(items, idx, &runs);
        if matches!(mode, PaintMode::Hidden) {
            continue;
        }
        if saw_visible {
            // Grok-style gap: blank row between blocks (no full-width ─ rules).
            sink.push_line(item_gap_line());
        }
        saw_visible = true;
        push_segment_content(&mut sink, items, idx, item, mode, &runs);
    }

    if sink.0.is_empty() {
        sink.push_line(Line::from(Span::styled(
            "No messages yet. Press `n` to start another agent, then type below.",
            REASONING_STYLE,
        )));
    }

    sink.0
}

/// Build one cache segment for `item_index`, respecting verb-group hide/header.
///
/// Callers must pass precomputed `runs` (from a single `find_runs` / cache hit).
/// Recomputing runs here used to cost an extra O(n) scan per exact measure.
pub(super) fn build_segment_visual_lines(
    item_index: usize,
    item: &ChatItem,
    items: &[ChatItem],
    width: u16,
    runs: &[VerbGroupRun],
) -> Vec<VisualLine> {
    let mode = paint_mode_with_runs(items, item_index, runs);
    if matches!(mode, PaintMode::Hidden) {
        return Vec::new();
    }

    let mut lines = Vec::new();
    // Gap only when a previous *visible* segment exists.
    let has_prior_visible = (0..item_index).any(|i| {
        !matches!(
            paint_mode_with_runs(items, i, runs),
            PaintMode::Hidden
        )
    });
    if has_prior_visible {
        lines.push(item_gap_line());
    }
    push_segment_content(
        &mut VecSinkRef(&mut lines),
        items,
        item_index,
        item,
        mode,
        runs,
    );
    visual_lines(lines, width)
}

fn push_segment_content<S: LineSink>(
    sink: &mut S,
    items: &[ChatItem],
    idx: usize,
    item: &ChatItem,
    mode: PaintMode,
    runs: &[crate::translation::VerbGroupRun],
) {
    match mode {
        PaintMode::Hidden => {}
        PaintMode::CollapsedHeader => {
            if let Some(run) = runs.iter().find(|r| r.start == idx) {
                let label = header_label(items, run.start, run.end);
                sink.push_line(label.line);
            } else {
                build_item_lines(sink, item);
            }
        }
        PaintMode::ExpandedHeader => {
            if let Some(run) = runs.iter().find(|r| r.start == idx) {
                let label = header_label(items, run.start, run.end);
                sink.push_line(label.line);
            }
            build_item_lines(sink, item);
        }
        PaintMode::Normal | PaintMode::ExpandedMember => {
            build_item_lines(sink, item);
        }
    }
}

/// Plain source + streaming flag for items that support commit-style holdback.
pub(super) fn streaming_text_source(item: &ChatItem) -> Option<(String, bool)> {
    match item {
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        }
        | ChatItem::UserMessage {
            text_parts,
            is_streaming,
            ..
        } => Some((plain_parts_to_string(text_parts), *is_streaming)),
        ChatItem::Reasoning {
            text,
            is_streaming,
            ..
        } => {
            // Collapsed thinking skips the streaming commit path; header-only render.
            if !item.is_fold_expanded() {
                return None;
            }
            Some((text.clone(), *is_streaming))
        }
        _ => None,
    }
}

fn plain_parts_to_string(parts: &[TextPart]) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            TextPart::Plain(text) => out.push_str(text),
            TextPart::Code { lang, code } => {
                out.push_str("```");
                out.push_str(lang);
                out.push('\n');
                out.push_str(code);
                out.push_str("\n```");
            }
        }
    }
    out
}

/// Role label line for streaming text items (before markdown body).
fn streaming_role_label(item: &ChatItem) -> Option<Line<'static>> {
    match item {
        // User prefix is painted with the first body line in the non-streaming path;
        // streaming commit path uses a bare arrow row before the body.
        ChatItem::UserMessage { .. } => {
            Some(Line::from(Span::styled(PROMPT_ARROW, USER_PREFIX)))
        }
        // Grok agent messages have no role chrome.
        ChatItem::AssistantText { .. } => None,
        ChatItem::Reasoning { is_streaming, .. } => Some(thinking_header_line(*is_streaming)),
        _ => None,
    }
}

fn thinking_header_line(is_streaming: bool) -> Line<'static> {
    if is_streaming {
        Line::from(Span::styled("Thinking…", THINKING_LABEL))
    } else {
        Line::from(Span::styled("Thought", THINKING_LABEL))
    }
}

/// Build visual lines for a streaming item, reusing a frozen stable-source prefix
/// when the holdback region only grew (lightweight commit queue).
///
/// `runs` must match `items` (same precomputed fold groups as non-streaming paths).
pub(super) fn build_streaming_segment_with_commit(
    item_index: usize,
    item: &ChatItem,
    items: &[ChatItem],
    width: u16,
    runs: &[VerbGroupRun],
    previous: Option<&StreamCommitSnapshot>,
) -> (Vec<VisualLine>, Option<StreamCommitSnapshot>) {
    let Some((source, is_streaming)) = streaming_text_source(item) else {
        return (
            build_segment_visual_lines(item_index, item, items, width, runs),
            None,
        );
    };

    if !is_streaming {
        return (
            build_segment_visual_lines(item_index, item, items, width, runs),
            None,
        );
    }

    let stable = crate::ui::stream_holdback::holdback_streaming_source(&source);
    let style = match item {
        ChatItem::Reasoning { .. } => THINKING_BODY,
        ChatItem::UserMessage { .. } => USER_BODY,
        ChatItem::AssistantText { .. } => ASSISTANT_BODY,
        _ => Style::default(),
    };

    let mut header_logical: Vec<Line<'static>> = Vec::new();
    let has_prior_visible = (0..item_index).any(|i| {
        !matches!(
            paint_mode_with_runs(items, i, runs),
            PaintMode::Hidden
        )
    });
    if has_prior_visible {
        header_logical.push(item_gap_line());
    }
    if let Some(label) = streaming_role_label(item) {
        header_logical.push(label);
    }
    let header_visual = visual_lines(header_logical, width);
    let header_line_count = header_visual.len();

    // Always re-render the full holdback-stable source as one markdown document.
    // Appending a delta fragment as its own `render_markdown` input forces each
    // fragment into a Paragraph → fake visual line breaks mid-prose, and cannot
    // re-wrap the previous last visual line when width still has room.
    // `previous` is only used to detect whether the stable prefix grew (for
    // cache invalidation call sites); body lines are never delta-appended.
    let _ = previous;
    let mut body_logical = Vec::new();
    push_markdown_lines(&mut VecSinkRef(&mut body_logical), stable, style);
    let body_visual = visual_lines(body_logical, width);
    let snapshot = Some(StreamCommitSnapshot {
        stable_source: stable.to_owned(),
        body_visual_lines: body_visual.clone(),
    });

    let mut out = header_visual;
    out.extend(body_visual);
    out.push(VisualLine {
        line: Line::from(Span::styled("█", STREAMING_CURSOR)),
        text: "█".to_owned(),
    });
    let _ = header_line_count;
    (out, snapshot)
}

/// Frozen stable-source commit state for one streaming chat segment.
#[derive(Clone, Default)]
pub(super) struct StreamCommitSnapshot {
    pub stable_source: String,
    pub body_visual_lines: Vec<VisualLine>,
}

/// Inter-item spacing (blank line). Full-width rule lines are intentionally not
/// used — they are noisy and can paint past the chat column into the sidebar.
fn item_gap_line() -> Line<'static> {
    Line::from("")
}

#[cfg(test)]
fn separator_line(_separator_width: u16) -> Line<'static> {
    item_gap_line()
}

fn build_item_lines<S: LineSink>(sink: &mut S, item: &ChatItem) {
    match item {
        ChatItem::UserMessage {
            text_parts,
            is_streaming,
            ..
        } => {
            // Grok user prompt: `❯ ` prefix + body (no [You] chrome).
            push_user_text_parts(sink, text_parts, *is_streaming);
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▍", STREAMING_CURSOR)));
            }
        }
        ChatItem::AssistantText {
            text_parts,
            is_streaming,
            ..
        } => {
            // Grok agent message: bare markdown, no role label.
            push_text_parts(sink, text_parts, ASSISTANT_BODY, *is_streaming);
            if *is_streaming {
                sink.push_line(Line::from(Span::styled("▍", STREAMING_CURSOR)));
            }
        }
        ChatItem::Reasoning {
            text,
            is_streaming,
            ..
        } => {
            let expanded = item.is_fold_expanded();
            sink.push_line(thinking_header_line(*is_streaming));
            if !expanded {
                // Collapsed: header only (Grok). Optional one-line dim preview.
                let summary = collapsed_thinking_summary(text);
                if !summary.is_empty() {
                    sink.push_line(Line::from(vec![
                        Span::styled("│ ", THINKING_BAR),
                        Span::styled(summary, MUTED),
                    ]));
                }
            } else {
                let render_text = if *is_streaming {
                    holdback_streaming_unstable_suffix(text)
                } else {
                    text.as_str()
                };
                push_thinking_body(sink, render_text);
                if *is_streaming {
                    sink.push_line(Line::from(vec![
                        Span::styled("│ ", THINKING_BAR),
                        Span::styled("▍", STREAMING_CURSOR),
                    ]));
                }
            }
        }
        ChatItem::ToolCall {
            name,
            args_summary,
            args_detail,
            output_summary,
            output_detail,
            is_error,
            is_streaming,
            ..
        } => {
            let kind = ToolKind::from_tool_name(name);
            let expanded = item.is_fold_expanded();
            let muted = !expanded;
            sink.push_line(tool_header_line(
                kind,
                args_summary,
                output_summary.as_deref(),
                *is_streaming,
                *is_error,
                muted,
                expanded,
            ));
            if expanded {
                push_tool_expanded_body(
                    sink,
                    kind,
                    args_summary,
                    args_detail.as_deref(),
                    output_detail.as_deref(),
                    output_summary.as_deref(),
                );
            }
        }
        ChatItem::SubagentCall {
            sub_thread_id,
            agent,
            model,
            prompt_summary,
            status,
            is_streaming,
            ..
        } => {
            let running = matches!(status, minos_ui_protocol::SubagentStatus::Running)
                || *is_streaming;
            let verb = if running { "Running" } else { "Ran" };
            let status_style = match status {
                minos_ui_protocol::SubagentStatus::Completed => TOOL_SUCCESS,
                minos_ui_protocol::SubagentStatus::Failed
                | minos_ui_protocol::SubagentStatus::Interrupted => TOOL_ERROR,
                minos_ui_protocol::SubagentStatus::Running => TOOL_RUNNING,
            };
            let id_short = crate::agent_route::short_thread_id(sub_thread_id);
            let mut spans = vec![
                Span::styled(format!("{verb} "), TOOL_VERB_MUTED),
                Span::styled(
                    format!("subagent {} #{id_short}", agent.bin_name()),
                    TOOL_PATH_MUTED,
                ),
            ];
            if let Some(model) = model.as_ref().filter(|value| !value.is_empty()) {
                spans.push(Span::styled(format!(" · {model}"), MUTED));
            }
            spans.push(Span::styled(
                format!(" · {status:?}").to_ascii_lowercase(),
                status_style,
            ));
            sink.push_line(Line::from(spans));
            if let Some(prompt) = prompt_summary.as_ref().filter(|value| !value.is_empty()) {
                sink.push_line(Line::from(Span::styled(prompt.clone(), MUTED)));
            }
        }
        ChatItem::SystemMessage { text } => {
            sink.push_line(Line::from(Span::styled(text.clone(), MUTED)));
        }
        ChatItem::Error { text, .. } => {
            sink.push_line(Line::from(Span::styled(text.clone(), ERROR_STYLE)));
        }
    }
}

/// Grok-style tool header: `Read path`, `Edited path +N/-M`, `Ran cmd`.
fn tool_header_line(
    kind: ToolKind,
    args_summary: &str,
    output_summary: Option<&str>,
    is_streaming: bool,
    is_error: bool,
    muted: bool,
    expanded: bool,
) -> Line<'static> {
    let running = is_streaming || output_summary.is_none();
    let verb = kind.header_verb(running);
    let verb_style = if muted { TOOL_VERB_MUTED } else { TOOL_VERB };
    let path_style = if muted { TOOL_PATH_MUTED } else { TOOL_PATH };

    let target = if !args_summary.is_empty() {
        args_summary.to_owned()
    } else {
        "…".to_owned()
    };

    let mut spans = vec![
        Span::styled(format!("{verb} "), verb_style),
        Span::styled(target, path_style),
    ];

    if is_error {
        spans.push(Span::styled("  failed", TOOL_ERROR));
    } else if running {
        spans.push(Span::styled("  …", TOOL_RUNNING));
    } else if !expanded {
        // Collapsed-only: colored diffstat for edits; dim line-count / one-liner otherwise.
        if let Some(summary) = output_summary.filter(|s| !s.is_empty()) {
            if let Some((ins, del)) = parse_diffstat(summary) {
                if ins > 0 || del > 0 {
                    spans.push(Span::styled(format!(" +{ins}"), DIFF_ADD));
                    spans.push(Span::styled("/", DIM));
                    spans.push(Span::styled(format!("-{del}"), DIFF_DEL));
                }
            } else if matches!(kind, ToolKind::Edit) {
                // no-op
            } else if !args_summary.contains(summary) {
                spans.push(Span::styled(format!("  {summary}"), MUTED));
            }
        }
    }

    Line::from(spans)
}

/// First/last line caps for expanded tool bodies (Grok-inspired truncation).
const TOOL_BODY_FIRST: usize = 12;
const TOOL_BODY_LAST: usize = 8;

fn push_tool_expanded_body<S: LineSink>(
    sink: &mut S,
    kind: ToolKind,
    args_summary: &str,
    args_detail: Option<&str>,
    output_detail: Option<&str>,
    output_summary: Option<&str>,
) {
    let output = output_detail.or(output_summary);

    match kind {
        ToolKind::Edit => {
            // Prefer output patch; some agents put the patch in args.
            let body = output
                .filter(|t| looks_like_diff(t))
                .or_else(|| args_detail.filter(|t| looks_like_diff(t)))
                .or(output)
                .or(args_detail);
            if let Some(text) = body {
                if looks_like_diff(text) {
                    for line in render_tool_diff(text, markdown_styles(ASSISTANT_BODY)) {
                        sink.push_line(line);
                    }
                } else {
                    push_tool_detail_lines(sink, text);
                }
            }
        }
        ToolKind::Execute => {
            if !args_summary.is_empty() {
                sink.push_line(Line::from(vec![
                    Span::styled("$ ", DIM),
                    Span::styled(args_summary.to_owned(), MUTED),
                ]));
            }
            if let Some(text) = output.filter(|t| !t.is_empty() && *t != "ok") {
                for line in render_tool_preformatted(
                    text,
                    markdown_styles(MUTED),
                    TOOL_BODY_FIRST,
                    TOOL_BODY_LAST,
                ) {
                    sink.push_line(line);
                }
            }
        }
        ToolKind::Read => {
            if let Some(text) = output.filter(|t| !t.is_empty()) {
                if looks_like_diff(text) {
                    for line in render_tool_diff(text, markdown_styles(ASSISTANT_BODY)) {
                        sink.push_line(line);
                    }
                } else if text.contains('\n') || text.len() > 220 {
                    for line in render_tool_read_body(
                        text,
                        args_summary,
                        markdown_styles(ASSISTANT_BODY),
                        TOOL_BODY_FIRST,
                        TOOL_BODY_LAST,
                    ) {
                        sink.push_line(line);
                    }
                } else {
                    sink.push_line(Line::from(Span::styled(format!("  {text}"), MUTED)));
                }
            } else if let Some(args) = args_detail {
                push_tool_detail_lines(sink, args);
            }
        }
        ToolKind::Search
        | ToolKind::List
        | ToolKind::WebFetch
        | ToolKind::WebSearch
        | ToolKind::Skill
        | ToolKind::Other => {
            if let Some(args) = args_detail {
                // Skip args if they're just a duplicate of the header target.
                if !args.trim().is_empty() && args.trim() != args_summary.trim() {
                    push_tool_detail_lines(sink, args);
                }
            }
            if let Some(text) = output.filter(|t| !t.is_empty()) {
                push_tool_detail_lines(sink, text);
            }
        }
    }
}

fn push_user_text_parts<S: LineSink>(sink: &mut S, text_parts: &[TextPart], is_streaming: bool) {
    let plain = plain_parts_to_string(text_parts);
    let render_text = if is_streaming {
        holdback_streaming_unstable_suffix(&plain)
    } else {
        plain.as_str()
    };
    let mut first = true;
    for line in render_markdown(render_text, markdown_styles(USER_BODY)) {
        if first {
            let mut spans = vec![Span::styled(PROMPT_ARROW, USER_PREFIX)];
            spans.extend(line.spans);
            sink.push_line(Line::from(spans));
            first = false;
        } else {
            // Continuation indent matches arrow visual width (2 cells).
            let mut spans = vec![Span::raw("  ")];
            spans.extend(line.spans);
            sink.push_line(Line::from(spans));
        }
    }
    if first {
        sink.push_line(Line::from(Span::styled(PROMPT_ARROW, USER_PREFIX)));
    }
}

fn push_thinking_body<S: LineSink>(sink: &mut S, text: &str) {
    for line in render_markdown(text, markdown_styles(THINKING_BODY)) {
        let mut spans = vec![Span::styled("│ ", THINKING_BAR)];
        spans.extend(line.spans);
        sink.push_line(Line::from(spans));
    }
}

fn push_text_parts<S: LineSink>(
    sink: &mut S,
    text_parts: &[TextPart],
    base_style: Style,
    is_streaming: bool,
) {
    let last = text_parts.len().saturating_sub(1);
    for (idx, part) in text_parts.iter().enumerate() {
        let holdback = is_streaming && idx == last;
        match part {
            TextPart::Plain(text) => {
                let render_text = if holdback {
                    holdback_streaming_unstable_suffix(text)
                } else {
                    text.as_str()
                };
                push_markdown_lines(sink, render_text, base_style);
            }
            TextPart::Code { lang, code } => {
                // Incomplete fenced code is already split into Code parts by the
                // projection layer when complete; while streaming plain text may
                // still contain open fences handled by holdback above.
                push_code_block(sink, lang, code);
            }
        }
    }
}

/// While streaming, omit unstable trailing markdown (open fences / incomplete tables)
/// so column widths and fence layout do not thrash frame-to-frame.
///
/// Codex-style table/fence holdback via [`crate::ui::stream_holdback`].
pub(crate) fn holdback_streaming_unstable_suffix(text: &str) -> &str {
    crate::ui::stream_holdback::holdback_streaming_source(text)
}

fn push_markdown_lines<S: LineSink>(sink: &mut S, text: &str, base_style: Style) {
    for line in render_markdown(text, markdown_styles(base_style)) {
        sink.push_line(line);
    }
}

fn push_code_block<S: LineSink>(sink: &mut S, lang: &str, code: &str) {
    for line in render_code_block(lang, code, markdown_styles(Style::default())) {
        sink.push_line(line);
    }
}

fn push_tool_detail_lines<S: LineSink>(sink: &mut S, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    // Diffs: Grok edit surface (unbordered, single gutter, insert/delete bg).
    if looks_like_diff(trimmed) {
        for line in render_tool_diff(trimmed, markdown_styles(ASSISTANT_BODY)) {
            sink.push_line(line);
        }
        return;
    }

    // Pretty-print JSON args/output when the body is a single JSON value.
    if let Some(pretty) = pretty_json_block(trimmed) {
        for line in render_tool_preformatted(
            &pretty,
            markdown_styles(MUTED),
            TOOL_BODY_FIRST,
            TOOL_BODY_LAST,
        ) {
            sink.push_line(line);
        }
        return;
    }

    // Multi-line shell / log output: unbordered preformatted (less chrome noise).
    if trimmed.contains('\n') {
        for line in render_tool_preformatted(
            trimmed,
            markdown_styles(MUTED),
            TOOL_BODY_FIRST,
            TOOL_BODY_LAST,
        ) {
            sink.push_line(line);
        }
        return;
    }

    sink.push_line(Line::from(Span::styled(format!("  {trimmed}"), MUTED)));
}

fn pretty_json_block(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    // Only pretty-print when it actually gains structure (object/array with content).
    match &value {
        serde_json::Value::Object(map) if !map.is_empty() => {}
        serde_json::Value::Array(items) if !items.is_empty() => {}
        _ => return None,
    }
    serde_json::to_string_pretty(&value).ok()
}

fn collapsed_thinking_summary(text: &str) -> String {
    let one_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if one_line.is_empty() {
        return String::new();
    }
    const MAX: usize = 72;
    if one_line.chars().count() <= MAX {
        return one_line.to_owned();
    }
    let mut out = String::new();
    for (i, ch) in one_line.chars().enumerate() {
        if i >= MAX.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn markdown_styles(base_style: Style) -> MarkdownStyles {
    let mut bold = base_style;
    bold.add_modifier |= Modifier::BOLD;
    let mut italic = base_style;
    italic.add_modifier |= Modifier::ITALIC;

    MarkdownStyles {
        text: base_style,
        heading: super::theme::MARKDOWN_HEADING,
        bold,
        italic,
        code_inline: super::theme::MARKDOWN_CODE,
        code_block: super::theme::MARKDOWN_CODE,
        code_block_border: Style::new().fg(BORDER_FG),
        quote: super::theme::MARKDOWN_QUOTE,
        link: super::theme::MARKDOWN_LINK,
        list_marker: super::theme::MARKDOWN_LIST,
        diff_add: super::theme::DIFF_ADD_BG,
        diff_del: super::theme::DIFF_DEL_BG,
        diff_hunk: super::theme::DIFF_HUNK,
        diff_gutter: super::theme::DIFF_GUTTER,
    }
}

#[derive(Clone)]
pub(super) struct VisualLine {
    pub(super) line: Line<'static>,
    pub(super) text: String,
}

fn visual_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<VisualLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();

    for line in lines {
        let mut current = VisualLine {
            line: Line::default(),
            text: String::new(),
        };
        let mut current_width = 0usize;

        for span in line.spans {
            let style = span.style;
            let mut span_buf = String::new();
            for ch in span.content.into_owned().chars() {
                let ch_width = char_width(ch);
                // Wrap before placing a char that would exceed the column width.
                // When the row is empty but the char itself is wider than `width`
                // (e.g. emoji on a 1-col pane), place it alone then wrap after.
                if ch_width > 0 && current_width + ch_width > width {
                    if current_width > 0 {
                        push_span(&mut current.line, &mut span_buf, style);
                        out.push(current);
                        current = VisualLine {
                            line: Line::default(),
                            text: String::new(),
                        };
                        current_width = 0;
                    } else if !current.text.is_empty() || !span_buf.is_empty() {
                        push_span(&mut current.line, &mut span_buf, style);
                        out.push(current);
                        current = VisualLine {
                            line: Line::default(),
                            text: String::new(),
                        };
                        current_width = 0;
                    }
                }

                span_buf.push(ch);
                current.text.push(ch);
                current_width = current_width.saturating_add(ch_width);

                if current_width >= width {
                    push_span(&mut current.line, &mut span_buf, style);
                    out.push(current);
                    current = VisualLine {
                        line: Line::default(),
                        text: String::new(),
                    };
                    current_width = 0;
                }
            }
            push_span(&mut current.line, &mut span_buf, style);
        }

        out.push(current);
    }

    out
}

/// Hard-cap a logical line's display width so ratatui never paints past the chat
/// column (adjacent sidebar). Prefers grapheme boundaries via char widths.
fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    if max_width == 0 {
        return Line::default();
    }
    let mut out = Line::default();
    let mut used = 0usize;
    for span in line.spans {
        if used >= max_width {
            break;
        }
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let ch_width = char_width(ch);
            if used + ch_width > max_width {
                break;
            }
            buf.push(ch);
            used = used.saturating_add(ch_width);
        }
        if !buf.is_empty() {
            out.spans.push(Span::styled(buf, style));
        }
        if used >= max_width {
            break;
        }
    }
    out
}

fn push_span(line: &mut Line<'static>, buf: &mut String, style: Style) {
    if buf.is_empty() {
        return;
    }
    line.spans.push(Span::styled(std::mem::take(buf), style));
}

fn apply_selection_with_offset(
    lines: &mut [VisualLine],
    selection: Option<&ChatSelection>,
    base_row: usize,
) {
    let Some(selection) = selection.filter(|selection| !selection.is_empty()) else {
        return;
    };

    for (local_row, visual) in lines.iter_mut().enumerate() {
        let absolute_row = base_row + local_row;
        if let Some((start_col, end_col)) =
            selected_cols_for_row(selection, absolute_row, &visual.text)
        {
            visual.line = highlight_line(std::mem::take(&mut visual.line), start_col, end_col);
        }
    }
}

fn selected_text_from_lines(lines: &[VisualLine], selection: &ChatSelection) -> Option<String> {
    let (start, end) = selection.normalized();
    if start.row >= lines.len() {
        return None;
    }

    let last_row = end.row.min(lines.len().saturating_sub(1));
    let mut selected = Vec::new();
    for row in start.row..=last_row {
        if let Some((start_col, end_col)) = selected_cols_for_row(selection, row, &lines[row].text)
        {
            selected.push(cell_slice(&lines[row].text, start_col, end_col));
        } else {
            selected.push(String::new());
        }
    }

    let text = selected.join("\n");
    (!text.is_empty()).then_some(text)
}

fn selected_cols_for_row(
    selection: &ChatSelection,
    row: usize,
    line_text: &str,
) -> Option<(usize, usize)> {
    let (start, end) = selection.normalized();
    if row < start.row || row > end.row {
        return None;
    }

    let line_width = UnicodeWidthStr::width(line_text);
    let start_col = if row == start.row { start.col } else { 0 };
    let end_col = if row == end.row {
        end.col.saturating_add(1)
    } else {
        line_width
    }
    .min(line_width);

    (start_col < end_col).then_some((start_col, end_col))
}

fn highlight_line(line: Line<'static>, start_col: usize, end_col: usize) -> Line<'static> {
    let mut out = Line::default();
    let mut col = 0usize;
    for span in line.spans {
        let base_style = span.style;
        let mut plain = String::new();
        let mut selected = String::new();
        let mut selected_mode = None;

        for ch in span.content.into_owned().chars() {
            let ch_width = char_width(ch);
            let ch_end = col.saturating_add(ch_width);
            let is_selected = ch_end > start_col && col < end_col;
            match selected_mode {
                Some(mode) if mode == is_selected => {}
                Some(true) => {
                    push_span(&mut out, &mut selected, selection_style(base_style));
                    selected_mode = Some(is_selected);
                }
                Some(false) => {
                    push_span(&mut out, &mut plain, base_style);
                    selected_mode = Some(is_selected);
                }
                None => selected_mode = Some(is_selected),
            }

            if is_selected {
                selected.push(ch);
            } else {
                plain.push(ch);
            }
            col = ch_end;
        }

        match selected_mode {
            Some(true) => push_span(&mut out, &mut selected, selection_style(base_style)),
            Some(false) => push_span(&mut out, &mut plain, base_style),
            None => {}
        }
    }

    out
}

fn cell_slice(text: &str, start_col: usize, end_col: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let ch_width = char_width(ch);
        let ch_end = col.saturating_add(ch_width);
        if ch_end > start_col && col < end_col {
            out.push(ch);
        }
        col = ch_end;
    }
    out
}

fn selection_style(style: Style) -> Style {
    style.bg(Color::Blue).fg(Color::White)
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
