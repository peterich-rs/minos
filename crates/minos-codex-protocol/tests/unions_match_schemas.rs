//! Asserts that every schema `oneOf` arm has a matching variant in the
//! generated dispatch enum. Catches "schema added a method but codegen wasn't
//! re-run" before it becomes a silent runtime gap.
//!
//! We assert presence of the `#[serde(rename = "<method>")]` annotation in the
//! generated `methods.rs` source by string search. That's cheap and reliable;
//! synthesising `{ "method": ..., "params": {} }` frames would fail when the
//! variant's typed Params requires fields, which is unrelated to coverage.

use std::collections::BTreeSet;

fn schema_methods(path: &str) -> BTreeSet<String> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    let arms = value
        .get("oneOf")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("{path}: missing top-level oneOf"));
    arms.iter()
        .map(|arm| {
            arm.pointer("/properties/method/enum/0")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{path}: arm missing method.enum[0]"))
                .to_string()
        })
        .collect()
}

fn assert_enum_renames_present(path: &str, generated_src: &str, kind: &str) {
    let expected = schema_methods(path);
    let src = std::fs::read_to_string(generated_src)
        .unwrap_or_else(|e| panic!("read {generated_src}: {e}"));
    let mut missing = Vec::new();
    for method in &expected {
        let needle = format!("#[serde(rename = \"{method}\")]");
        if !src.contains(&needle) {
            missing.push(method.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "{kind}: generated source missing rename annotations for: {missing:#?}"
    );
}

#[test]
fn server_request_renames_present() {
    assert_enum_renames_present(
        "../../schemas/ServerRequest.json",
        "src/generated/methods.rs",
        "ServerRequest",
    );
}

#[test]
fn server_notification_renames_present() {
    assert_enum_renames_present(
        "../../schemas/ServerNotification.json",
        "src/generated/methods.rs",
        "ServerNotification",
    );
}
