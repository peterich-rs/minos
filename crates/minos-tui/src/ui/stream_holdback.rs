//! Streaming markdown holdback (Codex-inspired).
//!
//! While an assistant/reasoning item is still streaming, unstable trailing
//! structure (open fences, pending/confirmed pipe tables) is omitted from the
//! rendered segment so layout does not thrash frame-to-frame.

use crate::render::table_detect::{
    is_table_delimiter_line, is_table_header_line, parse_table_segments, strip_blockquote_prefix,
    FenceKind, FenceTracker,
};

/// Incremental pipe-table holdback scanner (Codex `TableHoldbackScanner`).
pub(crate) struct TableHoldbackScanner {
    source_offset: usize,
    fence_tracker: FenceTracker,
    previous_line: Option<PreviousLineState>,
    pending_header_start: Option<usize>,
    confirmed_table_start: Option<usize>,
}

#[derive(Clone, Copy)]
struct PreviousLineState {
    source_start: usize,
    fence_kind: FenceKind,
    is_header: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableHoldbackState {
    None,
    PendingHeader { header_start: usize },
    Confirmed { table_start: usize },
}

impl TableHoldbackScanner {
    pub(crate) fn new() -> Self {
        Self {
            source_offset: 0,
            fence_tracker: FenceTracker::new(),
            previous_line: None,
            pending_header_start: None,
            confirmed_table_start: None,
        }
    }

    pub(crate) fn state(&self) -> TableHoldbackState {
        if let Some(table_start) = self.confirmed_table_start {
            TableHoldbackState::Confirmed { table_start }
        } else if let Some(header_start) = self.pending_header_start {
            TableHoldbackState::PendingHeader { header_start }
        } else {
            TableHoldbackState::None
        }
    }

    /// Feed complete source lines (prefer newline-terminated chunks).
    pub(crate) fn push_source_chunk(&mut self, source_chunk: &str) {
        if source_chunk.is_empty() {
            return;
        }
        for source_line in source_chunk.split_inclusive('\n') {
            self.push_line(source_line);
        }
    }

    fn push_line(&mut self, source_line: &str) {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let source_start = self.source_offset;
        let fence_kind = self.fence_tracker.kind();

        let candidate_text = if fence_kind == FenceKind::Other {
            None
        } else {
            table_candidate_text(line)
        };
        let is_header = candidate_text.is_some_and(is_table_header_line);
        let is_delimiter = candidate_text.is_some_and(is_table_delimiter_line);

        if self.confirmed_table_start.is_none() {
            if let Some(previous_line) = self.previous_line {
                if previous_line.fence_kind != FenceKind::Other
                    && fence_kind != FenceKind::Other
                    && previous_line.is_header
                    && is_delimiter
                {
                    self.confirmed_table_start = Some(previous_line.source_start);
                    self.pending_header_start = None;
                }
            }
        }

        if self.confirmed_table_start.is_none() && !line.trim().is_empty() {
            if fence_kind != FenceKind::Other && is_header {
                self.pending_header_start = Some(source_start);
            } else {
                self.pending_header_start = None;
            }
        }

        self.previous_line = Some(PreviousLineState {
            source_start,
            fence_kind,
            is_header,
        });

        self.fence_tracker.advance(line);
        self.source_offset = self.source_offset.saturating_add(source_line.len());
    }
}

fn table_candidate_text(line: &str) -> Option<&str> {
    let stripped = strip_blockquote_prefix(line).trim();
    parse_table_segments(stripped).map(|_| stripped)
}

/// Compute the stable render prefix of streaming markdown source.
///
/// Holds back:
/// - open non-markdown fences from the opening marker
/// - pending table headers and confirmed tables through EOF
/// - a trailing partial line (no final newline) that looks like table/fence start
pub(crate) fn holdback_streaming_source(text: &str) -> &str {
    if text.is_empty() {
        return text;
    }

    let (complete, partial) = split_complete_and_partial(text);
    let mut scanner = TableHoldbackScanner::new();
    scanner.push_source_chunk(complete);

    let mut cut = match scanner.state() {
        TableHoldbackState::None => None,
        TableHoldbackState::PendingHeader { header_start }
        | TableHoldbackState::Confirmed { table_start: header_start } => Some(header_start),
    };

    // Open fence: odd fence markers outside Other? FenceTracker ends open.
    // Scan complete region for last open fence start when fence still open.
    if matches!(scanner.fence_tracker.kind(), FenceKind::Other | FenceKind::Markdown) {
        if let Some(fence_start) = last_open_fence_start(complete) {
            cut = Some(match cut {
                Some(c) => c.min(fence_start),
                None => fence_start,
            });
        }
    }

    if !partial.is_empty() && partial_line_is_unstable(partial) {
        let partial_start = complete.len();
        cut = Some(match cut {
            Some(c) => c.min(partial_start),
            None => partial_start,
        });
    }

    match cut {
        None => text,
        Some(0) => "",
        Some(at) => text[..at.min(text.len())].trim_end_matches(['\r', '\n']),
    }
}

fn split_complete_and_partial(text: &str) -> (&str, &str) {
    if text.ends_with('\n') {
        (text, "")
    } else if let Some(idx) = text.rfind('\n') {
        (&text[..=idx], &text[idx + 1..])
    } else {
        ("", text)
    }
}

fn partial_line_is_unstable(line: &str) -> bool {
    let trimmed = strip_blockquote_prefix(line).trim();
    if trimmed.is_empty() {
        return false;
    }
    if parse_fence_marker_line(trimmed).is_some() {
        return true;
    }
    // Incomplete pipe row mid-type.
    trimmed.contains('|')
}

fn parse_fence_marker_line(line: &str) -> Option<(char, usize)> {
    crate::render::table_detect::parse_fence_marker(line)
}

fn last_open_fence_start(text: &str) -> Option<usize> {
    let mut tracker = FenceTracker::new();
    let mut open_start = None;
    let mut offset = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let before = tracker.kind();
        let leading = line.bytes().take_while(|&b| b == b' ').count();
        let candidate = if leading <= 3 {
            strip_blockquote_prefix(&line[leading.min(line.len())..])
        } else {
            ""
        };
        let is_open_marker =
            before == FenceKind::Outside && parse_fence_marker_line(candidate).is_some();
        tracker.advance(line);
        if is_open_marker && matches!(tracker.kind(), FenceKind::Other | FenceKind::Markdown) {
            open_start = Some(offset);
        }
        if tracker.kind() == FenceKind::Outside {
            open_start = None;
        }
        offset = offset.saturating_add(raw_line.len());
    }
    open_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_pending_table_header() {
        let text = "intro\n| a | b |\n";
        let held = holdback_streaming_source(text);
        assert_eq!(held, "intro");
    }

    #[test]
    fn holds_confirmed_table_body() {
        let text = "intro\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let held = holdback_streaming_source(text);
        assert_eq!(held, "intro");
    }

    #[test]
    fn keeps_stable_prose() {
        let text = "hello world\nsecond line\n";
        // No unstable structure: return source as-is (including trailing newline).
        assert_eq!(holdback_streaming_source(text), text);
    }

    #[test]
    fn holds_open_rust_fence() {
        let text = "intro\n```rust\nfn main() {\n";
        let held = holdback_streaming_source(text);
        assert_eq!(held, "intro");
    }

    #[test]
    fn ignores_pipes_inside_code_fence() {
        let text = "intro\n```\n| not | table |\n```\nafter\n";
        // Closed fence — full text stable (trim trailing newline style).
        let held = holdback_streaming_source(text);
        assert!(held.contains("after"));
        assert!(held.contains("| not | table |"));
    }

    #[test]
    fn holds_partial_pipe_line() {
        let text = "intro\n| incomplete";
        let held = holdback_streaming_source(text);
        assert_eq!(held, "intro");
    }
}
