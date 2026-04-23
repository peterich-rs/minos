//! Per-connection session registry: who is online and where to ship frames.
//!
//! The registry is the runtime-side counterpart to the `pairings` SQLite
//! table. Where pairings answer "who MAY talk", the registry answers "who
//! IS connected right now, and what channel do I push frames down".
//!
//! Submodules:
//! - [`registry`] — [`SessionRegistry`], [`SessionHandle`], [`ServerFrame`]
//!   and the `insert` / `remove` / `get` / `route` surface.
//!
//! Step 8's WebSocket dispatcher (`session/heartbeat.rs`, not yet created)
//! will consume this module: each accepted socket constructs a
//! [`SessionHandle`], inserts it on `OPEN`, removes it on `CLOSE`, and
//! drives a writer task that pulls [`ServerFrame`]s out of the handle's
//! outbox and serialises them to the wire.

pub mod registry;

pub use registry::{ServerFrame, SessionHandle, SessionRegistry};
