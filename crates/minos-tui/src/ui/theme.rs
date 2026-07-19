//! AgentDetail / chat palette aligned with Grok Build's GrokNight theme.
//!
//! Neutral gray base + TokyoNight accents (see grok-build `xai-grok-pager-render`
//! theme/groknight.rs). Colors are RGB so truecolor terminals match Grok closely;
//! 256-color terminals approximate.

use ratatui::style::{Color, Modifier, Style};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// ── GrokNight-inspired palette ──────────────────────────────────────────────
// Unused base tokens kept as the shared design palette (match GrokNight).
#[allow(dead_code)]
pub const BG_BASE: Color = rgb(20, 20, 20);
#[allow(dead_code)]
pub const BG_CODE: Color = rgb(28, 28, 28);
#[allow(dead_code)]
pub const BG_HIGHLIGHT: Color = rgb(36, 36, 36);

pub const FG: Color = rgb(225, 225, 225);
pub const FG_DARK: Color = rgb(200, 200, 200);
#[allow(dead_code)]
pub const FG_GUTTER: Color = rgb(65, 65, 65);
pub const COMMENT: Color = rgb(108, 108, 108);
pub const DARK3: Color = rgb(90, 90, 90);
pub const DARK5: Color = rgb(120, 120, 120);

pub const BLUE: Color = rgb(122, 162, 247);
pub const BLUE1: Color = rgb(58, 149, 171);
pub const CYAN: Color = rgb(125, 207, 255);
pub const GREEN: Color = rgb(158, 206, 106);
#[allow(dead_code)]
pub const GREEN1: Color = rgb(115, 218, 202);
pub const MAGENTA: Color = rgb(187, 154, 247);
pub const ORANGE: Color = rgb(255, 158, 100);
#[allow(dead_code)]
pub const PURPLE: Color = rgb(157, 124, 216);
pub const RED: Color = rgb(247, 118, 142);
pub const TEAL: Color = rgb(26, 188, 156);
#[allow(dead_code)]
pub const YELLOW: Color = rgb(224, 175, 104);

pub const RED_DARK: Color = rgb(66, 14, 20);
pub const GREEN_DARK: Color = rgb(6, 56, 6);

// ── Semantic roles (AgentDetail transcript) ─────────────────────────────────

/// User prompt pointer (`❯ `) — Grok uses accent_user / cyan fallback.
pub const USER_PREFIX: Style = Style::new().fg(FG_DARK);
/// User prompt body.
pub const USER_BODY: Style = Style::new().fg(FG);

/// Assistant markdown body (no loud role label — Grok agent blocks are bare).
pub const ASSISTANT_BODY: Style = Style::new().fg(FG_DARK);
/// Kept for conversation timeline labels that still need a role chip.
pub const USER_LABEL: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
pub const ASSISTANT_LABEL: Style = Style::new().fg(MAGENTA).add_modifier(Modifier::BOLD);

#[allow(dead_code)]
pub const SYSTEM_LABEL: Style = Style::new().fg(BLUE);
pub const ERROR_STYLE: Style = Style::new().fg(RED).add_modifier(Modifier::BOLD);

/// Dim / secondary chrome (collapsed tools, details).
pub const MUTED: Style = Style::new().fg(COMMENT);
#[allow(dead_code)]
pub const MUTED_BOLD: Style = Style::new().fg(COMMENT).add_modifier(Modifier::BOLD);
#[allow(dead_code)]
pub const PRIMARY: Style = Style::new().fg(FG);
#[allow(dead_code)]
pub const PRIMARY_BOLD: Style = Style::new().fg(FG).add_modifier(Modifier::BOLD);
pub const DIM: Style = Style::new().fg(DARK3);

/// Legacy alias used by status / placeholders.
pub const REASONING_STYLE: Style = MUTED;

/// Thinking header: "Thinking…" / "Thought" — muted bold (Grok default).
pub const THINKING_LABEL: Style = Style::new().fg(COMMENT).add_modifier(Modifier::BOLD);
/// Thinking body when expanded — dim secondary text.
pub const THINKING_BODY: Style = Style::new().fg(DARK5).add_modifier(Modifier::ITALIC);
/// Quote bar on thinking body lines (`│ `).
pub const THINKING_BAR: Style = Style::new().fg(COMMENT).add_modifier(Modifier::DIM);

/// Tool verb ("Read ", "Ran ") when expanded / focused.
pub const TOOL_VERB: Style = Style::new().fg(FG).add_modifier(Modifier::BOLD);
/// Tool verb when collapsed (muted).
pub const TOOL_VERB_MUTED: Style = Style::new().fg(COMMENT).add_modifier(Modifier::BOLD);
/// Tool path / command target.
pub const TOOL_PATH: Style = Style::new().fg(ORANGE);
pub const TOOL_PATH_MUTED: Style = Style::new().fg(COMMENT);
/// Tool name for unknown tools.
#[allow(dead_code)]
pub const TOOL_NAME_STYLE: Style = Style::new().fg(DARK5).add_modifier(Modifier::BOLD);
pub const TOOL_SUCCESS: Style = Style::new().fg(GREEN);
pub const TOOL_ERROR: Style = Style::new().fg(RED);
pub const TOOL_RUNNING: Style = Style::new().fg(CYAN);

pub const MARKDOWN_HEADING: Style = Style::new().fg(TEAL).add_modifier(Modifier::BOLD);
#[allow(dead_code)]
pub const MARKDOWN_HEADING_H2: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
pub const MARKDOWN_CODE: Style = Style::new().fg(BLUE1);
pub const MARKDOWN_QUOTE: Style = Style::new().fg(COMMENT);
pub const MARKDOWN_LINK: Style = Style::new()
    .fg(rgb(122, 166, 218))
    .add_modifier(Modifier::UNDERLINED);
pub const MARKDOWN_LIST: Style = Style::new().fg(FG_DARK);

pub const DIFF_ADD: Style = Style::new().fg(GREEN);
pub const DIFF_DEL: Style = Style::new().fg(RED);
pub const DIFF_HUNK: Style = Style::new().fg(CYAN);
pub const DIFF_GUTTER: Style = Style::new().fg(COMMENT);
pub const DIFF_ADD_BG: Style = Style::new().fg(GREEN).bg(GREEN_DARK);
pub const DIFF_DEL_BG: Style = Style::new().fg(RED).bg(RED_DARK);

pub const STREAMING_CURSOR: Style = Style::new().fg(MAGENTA).add_modifier(Modifier::SLOW_BLINK);

pub const CLI_OK: Style = Style::new().fg(GREEN);
pub const CLI_MISSING: Style = Style::new().fg(RED);
pub const DAEMON_CONNECTED: Style = Style::new().fg(GREEN);
pub const DAEMON_DISCONNECTED: Style = Style::new().fg(RED);
pub const INPUT_PROMPT: Style = Style::new().fg(CYAN);
pub const HIGHLIGHTED: Style = Style::new().add_modifier(Modifier::REVERSED);
pub const FOCUSED_BORDER: Style = Style::new().fg(rgb(80, 80, 88));
pub const BORDER_FG: Color = rgb(50, 50, 55);

/// Grok-style user prompt arrow (❯).
pub const PROMPT_ARROW: &str = "❯ ";

pub fn border_block<'a>() -> ratatui::widgets::Block<'a> {
    ratatui::widgets::Block::bordered().border_style(Style::new().fg(BORDER_FG))
}
