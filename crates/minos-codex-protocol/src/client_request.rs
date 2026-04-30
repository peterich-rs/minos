//! `ClientRequest` / `ClientNotification` — marker traits implemented by the
//! generated `*Params` / `*Notification` types. Drive `CodexClient::call_typed`
//! and `notify_typed` in the runtime.
//!
//! Implementations are auto-generated in [`crate::generated::methods`]. Do not
//! hand-write impls; add a new method to the schema, regenerate.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A JSON-RPC method codex accepts as a request (expects a response).
///
/// `METHOD` is the wire-format method string (e.g. `"thread/start"`).
/// `Response` is the `*Response` type the codex app-server returns. The
/// trait is not object-safe (associated `const` + associated type) — by
/// design, callers use `client.call_typed::<XxxParams>(params)` generically.
pub trait ClientRequest: Serialize {
    const METHOD: &'static str;
    type Response: DeserializeOwned;
}

/// A JSON-RPC notification codex accepts (no response).
///
/// `METHOD` is the wire-format method string. Notifications carry no
/// response, so the trait has no associated type. Currently implemented by
/// `InitializedNotification` only (the schema's only `ClientNotification`
/// variant), but the trait scales to future additions.
pub trait ClientNotification: Serialize {
    const METHOD: &'static str;
}
