//! Asserts `ClientRequest::METHOD` / `ClientNotification::METHOD` strings
//! match the schema's method-enum value. Spot-checks a representative subset
//! (full coverage is enforced by the codegen pipeline itself, but a
//! compile-checked assertion here catches accidental hand-edits to
//! `generated/methods.rs`).

use minos_codex_protocol::{ClientNotification, ClientRequest};

#[test]
fn initialize_method_string_matches_schema() {
    use minos_codex_protocol::InitializeParams;
    assert_eq!(<InitializeParams as ClientRequest>::METHOD, "initialize");
}

#[test]
fn thread_start_method_string_matches_schema() {
    use minos_codex_protocol::ThreadStartParams;
    assert_eq!(<ThreadStartParams as ClientRequest>::METHOD, "thread/start");
}

#[test]
fn turn_start_method_string_matches_schema() {
    use minos_codex_protocol::TurnStartParams;
    assert_eq!(<TurnStartParams as ClientRequest>::METHOD, "turn/start");
}

#[test]
fn turn_interrupt_method_string_matches_schema() {
    use minos_codex_protocol::TurnInterruptParams;
    assert_eq!(
        <TurnInterruptParams as ClientRequest>::METHOD,
        "turn/interrupt"
    );
}

#[test]
fn thread_archive_method_string_matches_schema() {
    use minos_codex_protocol::ThreadArchiveParams;
    assert_eq!(
        <ThreadArchiveParams as ClientRequest>::METHOD,
        "thread/archive"
    );
}

#[test]
fn initialized_notification_method_string_matches_schema() {
    use minos_codex_protocol::InitializedNotification;
    assert_eq!(
        <InitializedNotification as ClientNotification>::METHOD,
        "initialized"
    );
}
