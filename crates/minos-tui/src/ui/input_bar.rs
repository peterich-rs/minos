use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::theme::INPUT_PROMPT;

pub struct InputState {
    pub content: String,
    pub cursor_pos: usize,
    pub focused: bool,
    pub readonly: bool,
}

impl InputState {
    pub fn new(readonly: bool) -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            focused: true,
            readonly,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.readonly {
            return;
        }
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.readonly || self.cursor_pos == 0 {
            return;
        }
        let prev = self.content[..self.cursor_pos]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.content.drain(prev..self.cursor_pos);
        self.cursor_pos = prev;
    }

    pub fn take_input(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        taken
    }
}

pub fn render_input_bar(f: &mut Frame, area: Rect, state: &InputState) {
    let line = if state.readonly {
        Line::from(Span::styled(
            "[readonly mode]",
            ratatui::style::Style::new().fg(ratatui::style::Color::DarkGray),
        ))
    } else {
        let mut spans: Vec<Span> = Vec::new();
        if state.focused {
            spans.push(Span::styled("> ", INPUT_PROMPT));
        }
        spans.push(Span::raw(state.content.clone()));
        if state.focused {
            spans.push(Span::raw("▎"));
        }
        Line::from(spans)
    };

    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);
}
