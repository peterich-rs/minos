use std::sync::OnceLock;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color as SyntectColor, FontStyle, Style as SyntectStyle, Theme},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};
use two_face::theme::EmbeddedThemeName;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        two_face::theme::extra()
            .get(EmbeddedThemeName::TwoDark)
            .clone()
    })
}

pub(crate) fn highlight_to_lines(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    if code.is_empty()
        || code.len() > MAX_HIGHLIGHT_BYTES
        || code.lines().count() > MAX_HIGHLIGHT_LINES
    {
        return None;
    }

    let syntax = find_syntax(lang)?;
    let mut highlighter = HighlightLines::new(syntax, theme());
    let mut lines = Vec::new();

    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, syntax_set()).ok()?;
        let mut spans = Vec::new();
        for (style, text) in ranges {
            let text = text.trim_end_matches(['\n', '\r']);
            if !text.is_empty() {
                spans.push(Span::styled(text.to_owned(), convert_style(style)));
            }
        }
        if spans.is_empty() {
            spans.push(Span::raw(String::new()));
        }
        lines.push(spans);
    }

    (!lines.is_empty()).then_some(lines)
}

fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let lang = lang
        .trim()
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .next()
        .unwrap_or_default();
    if lang.is_empty() {
        return None;
    }

    let normalized = lang.to_ascii_lowercase();
    let patched = match normalized.as_str() {
        "csharp" | "c-sharp" => "c#",
        "cppm" | "cxxm" | "ixx" => "cpp",
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        _ => lang,
    };

    let syntaxes = syntax_set();
    syntaxes
        .find_syntax_by_token(patched)
        .or_else(|| syntaxes.find_syntax_by_name(patched))
        .or_else(|| {
            let lower = patched.to_ascii_lowercase();
            syntaxes
                .syntaxes()
                .iter()
                .find(|syntax| syntax.name.to_ascii_lowercase() == lower)
        })
        .or_else(|| syntaxes.find_syntax_by_extension(lang))
}

#[allow(clippy::disallowed_methods)]
fn convert_style(style: SyntectStyle) -> Style {
    let mut out = Style::new();

    if let Some(color) = convert_color(style.foreground) {
        out = out.fg(color);
    }
    if style.font_style.contains(FontStyle::BOLD) {
        out.add_modifier |= Modifier::BOLD;
    }

    out
}

#[allow(clippy::disallowed_methods)]
fn convert_color(color: SyntectColor) -> Option<Color> {
    match color.a {
        0x00 => Some(ansi_palette_color(color.r)),
        0x01 => None,
        _ => Some(Color::Rgb(color.r, color.g, color.b)),
    }
}

fn ansi_palette_color(index: u8) -> Color {
    match index {
        0x00 => Color::Black,
        0x01 => Color::Red,
        0x02 => Color::Green,
        0x03 => Color::Yellow,
        0x04 => Color::Blue,
        0x05 => Color::Magenta,
        0x06 => Color::Cyan,
        0x07 => Color::Gray,
        n => Color::Indexed(n),
    }
}

#[cfg(test)]
mod tests {
    use super::highlight_to_lines;

    #[test]
    fn highlights_known_language() {
        let lines = highlight_to_lines("fn main() {}\n", "rust").expect("rust highlights");

        assert_eq!(lines.len(), 1);
        assert!(lines[0].iter().any(|span| span.content.contains("fn")));
    }

    #[test]
    fn unknown_language_returns_none() {
        assert!(highlight_to_lines("hello", "definitely-not-a-language").is_none());
    }
}
