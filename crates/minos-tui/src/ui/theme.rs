use ratatui::style::{Color, Modifier, Style};

pub const USER_LABEL: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const ASSISTANT_LABEL: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const SYSTEM_LABEL: Style = Style::new().fg(Color::Yellow);
pub const ERROR_STYLE: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
pub const RAW_STYLE: Style = Style::new().fg(Color::Yellow);
pub const REASONING_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const TOOL_NAME_STYLE: Style = Style::new().fg(Color::Magenta);
pub const TOOL_SUCCESS: Style = Style::new().fg(Color::Green);
pub const TOOL_ERROR: Style = Style::new().fg(Color::Red);
pub const STREAMING_CURSOR: Style = Style::new()
    .fg(Color::White)
    .add_modifier(Modifier::SLOW_BLINK);
pub const THREAD_ACTIVE: Style = Style::new().fg(Color::Green);
pub const THREAD_IDLE: Style = Style::new().fg(Color::DarkGray);
pub const THREAD_RUNNING: Style = Style::new().fg(Color::Yellow);
pub const THREAD_CLOSED: Style = Style::new().fg(Color::DarkGray);
pub const CLI_OK: Style = Style::new().fg(Color::Green);
pub const CLI_MISSING: Style = Style::new().fg(Color::Red);
pub const INPUT_PROMPT: Style = Style::new().fg(Color::Cyan);
pub const HIGHLIGHTED: Style = Style::new().add_modifier(Modifier::REVERSED);
pub const FOCUSED_BORDER: Style = Style::new().fg(Color::Cyan);
pub const BORDER_FG: Color = Color::DarkGray;

pub fn border_block<'a>() -> ratatui::widgets::Block<'a> {
    ratatui::widgets::Block::bordered().border_style(Style::new().fg(BORDER_FG))
}
