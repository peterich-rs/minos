use crate::backend::SessionSummaryEntry;
use crate::render::Renderable;
use crate::ui::{flat_agent_sessions, theme, SessionEntry};
use minos_agent_runtime::SessionState;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState},
    Frame,
};
use std::collections::HashMap;

pub struct AgentSessionListRenderable<'a> {
    /// Durable conversation agent-session list (from daemon list RPC).
    sessions: &'a [SessionSummaryEntry],
    /// Live runtime rows (status glyphs); key = Minos session_id.
    live_sessions: &'a [SessionEntry],
    recent_files: &'a HashMap<String, Vec<String>>,
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> AgentSessionListRenderable<'a> {
    pub fn new(
        sessions: &'a [SessionSummaryEntry],
        live_sessions: &'a [SessionEntry],
        recent_files: &'a HashMap<String, Vec<String>>,
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            sessions,
            live_sessions,
            recent_files,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for AgentSessionListRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let block = Block::bordered()
            .title("Agent Sessions")
            .border_style(border_style);
        let flat = flat_agent_sessions(self.sessions);
        let items = flat
            .iter()
            .enumerate()
            .map(|(index, flat_session)| {
                let session = &self.sessions[flat_session.source_index];
                let id_short = &session.session_id[..8.min(session.session_id.len())];
                let prefix = if self.selected == Some(index) {
                    "> "
                } else {
                    "  "
                };
                let indent = if flat_session.depth == 0 { "" } else { "  " };
                let subagent_marker = if session.parent_session_id.is_some() {
                    " sub"
                } else {
                    ""
                };
                let (status_char, status_style) = self.status_for_session(&session.session_id);
                let files = self.recent_files_for_thread(&session.session_id);
                let message_count = session.message_count;
                let mut lines = vec![Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::Cyan)),
                    Span::raw(indent),
                    Span::raw(session.agent.bin_name()),
                    Span::styled(subagent_marker, Style::new().fg(Color::DarkGray)),
                    Span::styled(format!(" #{}", id_short), Style::new().fg(Color::DarkGray)),
                    Span::styled(format!(" {}", status_char), status_style),
                ])];
                for file in files.iter().take(2) {
                    lines.push(Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(
                            format!("  {} {}", "·", shorten_path(file)),
                            Style::new().fg(Color::DarkGray),
                        ),
                    ]));
                }
                if message_count > 0 {
                    lines.push(Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(
                            format!("  msgs: {}", message_count),
                            Style::new().fg(Color::DarkGray),
                        ),
                    ]));
                }
                ListItem::new(lines)
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::HIGHLIGHTED);
        frame.render_stateful_widget(list, area, self.list_state);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

impl AgentSessionListRenderable<'_> {
    fn status_for_session(&self, session_id: &str) -> (char, Style) {
        let state = self
            .live_sessions
            .iter()
            .find(|t| t.session_id == session_id)
            .map(|t| &t.state);
        match state {
            Some(SessionState::Starting)
            | Some(SessionState::Resuming)
            | Some(SessionState::Running { .. }) => ('●', Style::new().fg(Color::Green)),
            Some(SessionState::Idle) | Some(SessionState::Suspended { .. }) => {
                ('○', Style::new().fg(Color::DarkGray))
            }
            Some(SessionState::Closed { .. }) => ('✕', Style::new().fg(Color::Red)),
            None => ('?', Style::new().fg(Color::DarkGray)),
        }
    }

    fn recent_files_for_thread(&self, session_id: &str) -> Vec<String> {
        self.recent_files
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }
}

pub fn extract_file_path(tool_name: &str, args_summary: &str) -> Option<String> {
    let kind = crate::translation::ToolKind::from_tool_name(tool_name);
    if !matches!(
        kind,
        crate::translation::ToolKind::Read | crate::translation::ToolKind::Edit
    ) {
        return None;
    }
    let path = args_summary
        .strip_prefix("file: ")
        .or_else(|| args_summary.strip_prefix("file="))
        .unwrap_or(args_summary)
        .trim();
    if path.is_empty() {
        return None;
    }
    // Bare path from Grok-style summaries, or legacy labeled forms.
    if path.contains('/') || path.contains('.') || path.starts_with('~') {
        return Some(path.to_owned());
    }
    None
}

fn shorten_path(path: &str) -> String {
    const MAX_LEN: usize = 30;
    if path.len() <= MAX_LEN {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 2 {
        return path.to_owned();
    }
    let last_two = parts[parts.len() - 2..].join("/");
    format!("…/{}", last_two)
}
