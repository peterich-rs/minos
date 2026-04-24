//! Golden-fixture tests for the codex translator.
//!
//! One file pair per scenario:
//! - `<name>.input.json`     — a JSON array of raw codex notifications.
//! - `<name>.expected.json`  — a JSON array of the full concatenated
//!   `Vec<UiEventMessage>` produced by feeding all inputs through a fresh
//!   [`CodexTranslatorState`].
//!
//! Why golden-diff here rather than inline asserts:
//! the translator is the entire wire contract between `minos-agent-runtime`
//! and `minos-backend`. A single-character change to a variant name or a
//! field type changes mobile's display without loud failure at the unit
//! level. The fixtures make that surface explicit and diff-reviewable.
//!
//! UUID handling: `item/started` + `item/toolCall/started` each mint fresh
//! v4 UUIDs via `Uuid::new_v4()`. The fixture uses the literal string
//! `"<uuid>"` where a translator-assigned id appears; the harness rewrites
//! every actual UUID match in the serialised `got` to `"<uuid>"` before
//! deserialising, so the comparison is deterministic without a regex dep.

use std::fs;
use std::path::PathBuf;

use minos_ui_protocol::{translate_codex, CodexTranslatorState, UiEventMessage};
use rstest::rstest;

/// Replace every UUID-shaped token in `s` (32 hex chars split 8-4-4-4-12
/// by dashes) with the literal placeholder `<uuid>`. Hand-rolled to avoid
/// pulling a regex crate into dev-deps.
///
/// Implementation note: UUIDs are pure ASCII, so we can safely inspect the
/// UTF-8 bytes starting at every `char_indices()` position without
/// corrupting multi-byte sequences elsewhere in the string (e.g. non-ASCII
/// text deltas). Iterating by `char_indices()` instead of raw byte indices
/// guarantees we advance by full code points, so `"Thinking…"` stays
/// intact and only literal UUID substrings get rewritten.
fn normalise_uuids(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if i + 36 <= bytes.len() && is_uuid(&bytes[i..i + 36]) {
            out.push_str("<uuid>");
            // Advance past the UUID.
            while let Some(&(j, _)) = chars.peek() {
                if j >= i + 36 {
                    break;
                }
                chars.next();
            }
        } else {
            out.push(c);
            chars.next();
        }
    }
    out
}

fn is_uuid(b: &[u8]) -> bool {
    if b.len() != 36 {
        return false;
    }
    // Expected dashes at positions 8, 13, 18, 23.
    if !(b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-') {
        return false;
    }
    for (idx, c) in b.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            continue;
        }
        if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[rstest]
fn codex_golden(#[files("tests/golden/codex/*.input.json")] input_path: PathBuf) {
    let expected_path = PathBuf::from(
        input_path
            .to_string_lossy()
            .replace(".input.json", ".expected.json"),
    );

    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&input_path).unwrap())
            .unwrap_or_else(|e| panic!("parse {}: {e}", input_path.display()));
    let expected: Vec<UiEventMessage> =
        serde_json::from_str(&fs::read_to_string(&expected_path).unwrap())
            .unwrap_or_else(|e| panic!("parse {}: {e}", expected_path.display()));

    let mut state = CodexTranslatorState::new("thr_fixture".into());
    let mut got = Vec::new();
    for ev in &inputs {
        got.extend(translate_codex(&mut state, ev).unwrap());
    }

    let got_json = normalise_uuids(&serde_json::to_string(&got).unwrap());
    let got_norm: Vec<UiEventMessage> = serde_json::from_str(&got_json).unwrap();

    pretty_assertions::assert_eq!(got_norm, expected, "fixture {}", input_path.display());
}
