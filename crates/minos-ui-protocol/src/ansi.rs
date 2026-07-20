//! Strip terminal ANSI escape sequences from tool / command output.
//!
//! Grok ACP puts **raw** bash bytes (with SGR colors) into `tool_call` content.
//! Minos UIs are not a terminal: the ESC character is invisible / dropped, so
//! users see garbage like `[90m`, `[31m`, `[39m`. Strip at the projection
//! boundary so TUI/Desktop get plain text.

/// Remove CSI / OSC / 2-byte ESC sequences from `input`.
#[must_use]
pub fn strip_ansi_escapes(input: &str) -> String {
    if !input.contains('\u{1b}') && !input.contains('\u{9b}') {
        return input.to_owned();
    }

    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    // CSI: ESC [ … final-byte in 0x40..=0x7E
                    chars.next();
                    consume_csi_params(&mut chars);
                }
                Some(']') => {
                    // OSC: ESC ] … BEL or ESC \
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\u{07}' {
                            break;
                        }
                        if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    // Two-char escapes (charset, etc.): drop the next byte.
                    chars.next();
                }
                None => {}
            },
            // 8-bit CSI (0x9B)
            '\u{9b}' => consume_csi_params(&mut chars),
            _ => out.push(c),
        }
    }
    out
}

fn consume_csi_params(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(&n) = chars.peek() {
        chars.next();
        // Final byte of CSI sequence.
        if matches!(n, '\u{40}'..='\u{7e}') {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sgr_color_codes() {
        let raw = "\u{1b}[31m✖ fail\u{1b}[39m \u{1b}[90m(12ms)\u{1b}[39m";
        assert_eq!(strip_ansi_escapes(raw), "✖ fail (12ms)");
    }

    #[test]
    fn strips_node_test_style_output() {
        let raw = "\u{1b}[31m✖ src/lib/stick-to-bottom.test.ts \u{1b}[90m(47.469042ms)\u{1b}[39m\u{1b}[39m";
        let clean = strip_ansi_escapes(raw);
        assert!(!clean.contains('['), "got: {clean:?}");
        assert!(clean.contains("stick-to-bottom.test.ts"), "got: {clean:?}");
        assert!(clean.contains("47.469042ms"), "got: {clean:?}");
    }

    #[test]
    fn leaves_plain_text_unchanged() {
        let s = "hello\nworld +1/-2";
        assert_eq!(strip_ansi_escapes(s), s);
    }

    #[test]
    fn strips_osc_title_sequence() {
        let raw = "\u{1b}]0;title\u{07}ok";
        assert_eq!(strip_ansi_escapes(raw), "ok");
    }
}
