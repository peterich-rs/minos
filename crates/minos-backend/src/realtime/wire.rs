//! Re-exports of realtime wire types from `minos_protocol`.
//!
//! The canonical definitions live in `minos_protocol::realtime`; this module
//! preserves the existing import paths within the backend crate.

pub use minos_protocol::realtime::{ClientFrame, ServerFrame};
