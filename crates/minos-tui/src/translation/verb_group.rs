//! Verb-group aggregation for AgentDetail — Grok-style "Read 3 files, Searched 2 patterns".
//!
//! Mirrors `xai-grok-pager` `scrollback/state/verb_group.rs` run classification
//! and bucket labels against Minos `ChatItem`s (no Grok crate dependency).

use std::collections::HashSet;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::chat_item::ChatItem;
use super::tool_kind::ToolKind;

// Local styles (avoid ui↔translation cycle). Matches GrokNight gray_bright / error.
const HEADER_STYLE: Style = Style::new()
    .fg(Color::Rgb(120, 120, 120))
    .add_modifier(Modifier::BOLD);
const FAILED_STYLE: Style = Style::new().fg(Color::Rgb(247, 118, 142));

/// Bucket identity for aggregated headers (Grok `VerbGroupKind` subset that folds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerbBucket {
    File,
    Skill,
    Search,
    Dir,
    WebFetch,
    WebSearch,
    Subagent,
}

impl VerbBucket {
    pub fn verb(self, running: bool) -> &'static str {
        let (past, present) = match self {
            Self::File | Self::Skill => ("Read", "Reading"),
            Self::Search | Self::WebSearch => ("Searched", "Searching"),
            Self::Dir => ("Listed", "Listing"),
            Self::WebFetch => ("Fetched", "Fetching"),
            Self::Subagent => ("Ran", "Running"),
        };
        if running { present } else { past }
    }

    pub fn noun(self, count: usize) -> &'static str {
        let (one, many) = match self {
            Self::File => ("file", "files"),
            Self::Skill => ("skill", "skills"),
            Self::Search => ("pattern", "patterns"),
            Self::Dir => ("dir", "dirs"),
            Self::WebFetch | Self::WebSearch => ("website", "websites"),
            Self::Subagent => ("subagent", "subagents"),
        };
        if count == 1 { one } else { many }
    }
}

/// Whether this tool kind eagerly folds into a verb-group run.
/// Execute / Edit / Other never fold (Grok: label-only for truncation only).
pub fn verb_bucket_for_tool(name: &str) -> Option<VerbBucket> {
    match ToolKind::from_tool_name(name) {
        ToolKind::Read => Some(VerbBucket::File),
        ToolKind::Skill => Some(VerbBucket::Skill),
        ToolKind::Search => Some(VerbBucket::Search),
        ToolKind::List => Some(VerbBucket::Dir),
        ToolKind::WebFetch => Some(VerbBucket::WebFetch),
        ToolKind::WebSearch => Some(VerbBucket::WebSearch),
        ToolKind::Edit | ToolKind::Execute | ToolKind::Other => None,
    }
}

/// One step of a verb-group run walk (Grok `RunStep`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStep {
    /// Collapsed foldable tool / subagent — joins run and counts toward fold.
    Member(VerbBucket),
    /// Finished collapsed thinking — joins run, never counts or labels.
    ThoughtMember,
    /// Manually opened tool/thinking — stays visible inside run, does not break it.
    Transparent,
    /// Ends the run (user/assistant/edit/execute/error/…).
    Break,
}

/// Classify one chat item for run walking.
pub fn run_step(item: &ChatItem) -> RunStep {
    match item {
        ChatItem::ToolCall { name, .. } => {
            let Some(bucket) = verb_bucket_for_tool(name) else {
                return RunStep::Break;
            };
            if item.is_fold_expanded() {
                // Opened member keeps its own rows without splitting the run.
                RunStep::Transparent
            } else {
                RunStep::Member(bucket)
            }
        }
        ChatItem::SubagentCall { .. } => RunStep::Member(VerbBucket::Subagent),
        ChatItem::Reasoning {
            is_streaming,
            is_user_toggled,
            ..
        } => {
            let expanded = is_user_toggled.unwrap_or(*is_streaming);
            if *is_streaming || expanded {
                RunStep::Transparent
            } else {
                // Finished collapsed thought folds into the run.
                RunStep::ThoughtMember
            }
        }
        ChatItem::UserMessage { .. }
        | ChatItem::AssistantText { .. }
        | ChatItem::SystemMessage { .. }
        | ChatItem::Error { .. } => RunStep::Break,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbGroupRun {
    /// Inclusive start index (first claimed member / thought).
    pub start: usize,
    /// Exclusive end (one past last claimed entry).
    pub end: usize,
    /// Tool/subagent members counted (thoughts excluded).
    pub members: usize,
    /// Whether the group is user-expanded (shows members under the label).
    pub expanded: bool,
}

impl VerbGroupRun {
    pub fn contains(self, index: usize) -> bool {
        index >= self.start && index < self.end
    }

    pub fn folds(self) -> bool {
        self.members >= 1
    }
}

/// Stable identity of a run's first member (for expand-state keys).
pub fn run_anchor_id(items: &[ChatItem], start: usize) -> Option<String> {
    let item = items.get(start)?;
    match item {
        ChatItem::ToolCall { tool_call_id, .. } => Some(tool_call_id.clone()),
        ChatItem::SubagentCall { tool_call_id, .. } => Some(tool_call_id.clone()),
        ChatItem::Reasoning { message_id, .. } => {
            // Thought-only anchors shouldn't fold (members == 0); keep id for completeness.
            Some(format!("thought:{message_id}"))
        }
        _ => None,
    }
}

/// Scan all folding runs in `items`.
///
/// `expanded_ids` holds anchor ids of user-expanded groups.
pub fn find_runs(items: &[ChatItem], expanded_ids: &HashSet<String>) -> Vec<VerbGroupRun> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    while i < items.len() {
        match scan_run_forward(items, i) {
            Some(scan) if scan.members >= 1 => {
                let anchor = run_anchor_id(items, scan.start);
                let expanded = anchor
                    .as_ref()
                    .is_some_and(|id| expanded_ids.contains(id));
                runs.push(VerbGroupRun {
                    start: scan.start,
                    end: scan.end,
                    members: scan.members,
                    expanded,
                });
                i = scan.stop.max(scan.start + 1);
            }
            Some(scan) => {
                // Thought-only walk: advance past it.
                i = scan.stop.max(i + 1);
            }
            None => i += 1,
        }
    }
    runs
}

struct RunScan {
    start: usize,
    members: usize,
    end: usize,
    stop: usize,
}

fn scan_run_forward(items: &[ChatItem], start: usize) -> Option<RunScan> {
    match run_step(items.get(start)?) {
        RunStep::Member(_) | RunStep::ThoughtMember => {}
        RunStep::Transparent | RunStep::Break => return None,
    }
    let mut members = 0usize;
    let mut end = start;
    let mut i = start;
    while i < items.len() {
        match run_step(&items[i]) {
            RunStep::Member(_) => {
                members += 1;
                end = i + 1;
            }
            RunStep::ThoughtMember => {
                end = i + 1;
            }
            RunStep::Transparent => {}
            RunStep::Break => break,
        }
        i += 1;
    }
    Some(RunScan {
        start,
        members,
        end,
        stop: i,
    })
}

/// How one item paints relative to verb groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaintMode {
    /// Not in a folding run — normal item paint.
    Normal,
    /// Collapsed run start: only the aggregate header line.
    CollapsedHeader,
    /// Expanded run start: aggregate header then the item itself.
    ExpandedHeader,
    /// Collapsed run member/thought: emit no lines.
    Hidden,
    /// Expanded run non-start member/thought/transparent: normal item paint.
    ExpandedMember,
}

#[cfg(test)]
pub fn paint_mode(items: &[ChatItem], index: usize, expanded_ids: &HashSet<String>) -> PaintMode {
    let runs = find_runs(items, expanded_ids);
    paint_mode_with_runs(items, index, &runs)
}

pub fn paint_mode_with_runs(
    items: &[ChatItem],
    index: usize,
    runs: &[VerbGroupRun],
) -> PaintMode {
    let Some(run) = runs.iter().find(|r| r.contains(index)) else {
        return PaintMode::Normal;
    };
    if !run.folds() {
        return PaintMode::Normal;
    }
    if run.expanded {
        if index == run.start {
            PaintMode::ExpandedHeader
        } else {
            PaintMode::ExpandedMember
        }
    } else if index == run.start {
        PaintMode::CollapsedHeader
    } else {
        // Only claimable rows hide; transparent rows inside a collapsed run
        // still render (opened tools mid-group).
        match run_step(&items[index]) {
            RunStep::Member(_) | RunStep::ThoughtMember => PaintMode::Hidden,
            RunStep::Transparent => PaintMode::ExpandedMember,
            RunStep::Break => PaintMode::Normal,
        }
    }
}

/// Aggregated header label for a run (`Read 3 files, Searched 2 patterns · 1 failed`).
pub fn header_label(items: &[ChatItem], start: usize, end: usize) -> HeaderLabel {
    let end = end.min(items.len());
    let mut buckets: Vec<(VerbBucket, usize)> = Vec::new();
    let mut running = false;
    let mut failed = 0usize;
    let mut subagent_ids: HashSet<&str> = HashSet::new();

    for item in &items[start.min(end)..end] {
        let bucket = match run_step(item) {
            RunStep::Member(b) => b,
            RunStep::Break => break,
            RunStep::ThoughtMember | RunStep::Transparent => continue,
        };

        match item {
            ChatItem::ToolCall {
                is_error,
                is_streaming,
                output_summary,
                ..
            } => {
                if *is_error {
                    failed += 1;
                }
                if *is_streaming || output_summary.is_none() {
                    running = true;
                }
                push_bucket(&mut buckets, bucket);
            }
            ChatItem::SubagentCall {
                sub_thread_id,
                status,
                is_streaming,
                ..
            } => {
                // Distinct subagents (started + terminal of same id count once).
                if subagent_ids.insert(sub_thread_id.as_str()) {
                    push_bucket(&mut buckets, VerbBucket::Subagent);
                }
                if matches!(status, minos_ui_protocol::SubagentStatus::Failed) {
                    failed += 1;
                }
                if *is_streaming
                    || matches!(status, minos_ui_protocol::SubagentStatus::Running)
                {
                    running = true;
                }
            }
            _ => push_bucket(&mut buckets, bucket),
        }
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (bucket, count)) in buckets.iter().enumerate() {
        let segment = format!(
            "{}{} {} {}",
            if i == 0 { "" } else { ", " },
            bucket.verb(running),
            count,
            bucket.noun(*count)
        );
        spans.push(Span::styled(segment, HEADER_STYLE));
    }
    if failed > 0 {
        spans.push(Span::styled(format!(" · {failed} failed"), FAILED_STYLE));
    }

    HeaderLabel {
        line: Line::from(spans),
    }
}

fn push_bucket(buckets: &mut Vec<(VerbBucket, usize)>, bucket: VerbBucket) {
    if let Some((_, count)) = buckets.iter_mut().find(|(b, _)| *b == bucket) {
        *count += 1;
    } else {
        buckets.push((bucket, 1));
    }
}

#[derive(Debug, Clone)]
pub struct HeaderLabel {
    pub line: Line<'static>,
}

#[cfg(test)]
impl HeaderLabel {
    /// Flatten styled spans to plain text for assertions.
    fn plain_text(&self) -> String {
        self.line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}

/// Find the folding run containing `index`, if any.
pub fn run_containing(
    items: &[ChatItem],
    index: usize,
    expanded_ids: &HashSet<String>,
) -> Option<VerbGroupRun> {
    find_runs(items, expanded_ids)
        .into_iter()
        .find(|r| r.contains(index) && r.folds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minos_domain::AgentName;
    use minos_ui_protocol::SubagentStatus;

    fn tool(name: &str, id: &str, path: &str) -> ChatItem {
        ChatItem::ToolCall {
            message_id: "m".into(),
            tool_call_id: id.into(),
            name: name.into(),
            args_summary: path.into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        }
    }

    fn bash(id: &str) -> ChatItem {
        ChatItem::ToolCall {
            message_id: "m".into(),
            tool_call_id: id.into(),
            name: "bash".into(),
            args_summary: "ls".into(),
            args_detail: None,
            output_summary: Some("ok".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        }
    }

    fn thought() -> ChatItem {
        ChatItem::Reasoning {
            message_id: "r1".into(),
            text: "hmm".into(),
            is_streaming: false,
            is_user_toggled: None,
        }
    }

    #[test]
    fn three_reads_form_one_collapsed_run() {
        let items = vec![
            tool("read_file", "1", "a.rs"),
            tool("read_file", "2", "b.rs"),
            tool("read_file", "3", "c.rs"),
        ];
        let runs = find_runs(&items, &HashSet::new());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 3);
        assert_eq!(runs[0].members, 3);
        assert!(!runs[0].expanded);

        let label = header_label(&items, 0, 3);
        assert_eq!(label.plain_text(), "Read 3 files");
        assert_eq!(paint_mode(&items, 0, &HashSet::new()), PaintMode::CollapsedHeader);
        assert_eq!(paint_mode(&items, 1, &HashSet::new()), PaintMode::Hidden);
        assert_eq!(paint_mode(&items, 2, &HashSet::new()), PaintMode::Hidden);
    }

    #[test]
    fn bash_breaks_run_between_reads() {
        let items = vec![
            tool("read_file", "1", "a.rs"),
            bash("2"),
            tool("read_file", "3", "c.rs"),
        ];
        let runs = find_runs(&items, &HashSet::new());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].start..runs[0].end, 0..1);
        assert_eq!(runs[1].start..runs[1].end, 2..3);
    }

    #[test]
    fn collapsed_thought_joins_run_without_label_count() {
        let items = vec![
            tool("read_file", "1", "a.rs"),
            thought(),
            tool("read_file", "2", "b.rs"),
        ];
        let runs = find_runs(&items, &HashSet::new());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].members, 2);
        assert_eq!(runs[0].end, 3);
        let label = header_label(&items, 0, 3);
        assert_eq!(label.plain_text(), "Read 2 files");
    }

    #[test]
    fn edit_never_verb_folds() {
        let items = vec![ChatItem::ToolCall {
            message_id: "m".into(),
            tool_call_id: "1".into(),
            name: "apply_patch".into(),
            args_summary: "x.rs".into(),
            args_detail: None,
            output_summary: Some("+1/-1".into()),
            output_detail: None,
            is_error: false,
            is_expanded: false,
            is_user_toggled: None,
            is_streaming: false,
        }];
        assert!(find_runs(&items, &HashSet::new()).is_empty());
        assert_eq!(paint_mode(&items, 0, &HashSet::new()), PaintMode::Normal);
    }

    #[test]
    fn expanded_group_shows_members() {
        let items = vec![
            tool("read_file", "1", "a.rs"),
            tool("read_file", "2", "b.rs"),
        ];
        let mut expanded = HashSet::new();
        expanded.insert("1".into());
        let runs = find_runs(&items, &expanded);
        assert!(runs[0].expanded);
        assert_eq!(paint_mode_with_runs(&items, 0, &runs), PaintMode::ExpandedHeader);
        assert_eq!(paint_mode_with_runs(&items, 1, &runs), PaintMode::ExpandedMember);
    }

    #[test]
    fn mixed_buckets_preserve_first_appearance_order() {
        let items = vec![
            tool("read_file", "1", "a.rs"),
            tool("grep", "2", "foo"),
            tool("read_file", "3", "b.rs"),
            ChatItem::SubagentCall {
                message_id: "m".into(),
                tool_call_id: "4".into(),
                sub_thread_id: "sub-1".into(),
                agent: AgentName::Codex,
                model: None,
                prompt_summary: None,
                status: SubagentStatus::Completed,
                is_streaming: false,
            },
        ];
        let label = header_label(&items, 0, 4);
        assert_eq!(
            label.plain_text(),
            "Read 2 files, Searched 1 pattern, Ran 1 subagent"
        );
    }

    #[test]
    fn failed_members_append_suffix() {
        let mut t = tool("read_file", "1", "a.rs");
        if let ChatItem::ToolCall { is_error, .. } = &mut t {
            *is_error = true;
        }
        let items = vec![t, tool("read_file", "2", "b.rs")];
        let label = header_label(&items, 0, 2);
        assert_eq!(label.plain_text(), "Read 2 files · 1 failed");
    }

    #[test]
    fn running_member_flips_tense() {
        let mut t = tool("read_file", "1", "a.rs");
        if let ChatItem::ToolCall {
            is_streaming,
            output_summary,
            ..
        } = &mut t
        {
            *is_streaming = true;
            *output_summary = None;
        }
        let items = vec![t, tool("read_file", "2", "b.rs")];
        let label = header_label(&items, 0, 2);
        let text = label.plain_text();
        assert!(text.starts_with("Reading "), "{text}");
    }
}
