//! Minos backend server library surface.
//!
//! Module layout follows hexagonal "Infrastructure" concerns:
//! - `config` — CLI + env parsing for the binary.
//! - `error` — backend-local error type (mapped to `MinosError` at the API
//!   boundary).
//! - `store` — SQLite pool + embedded migrations.
//! - `host_link` — same-account host link (bind/unbind host installations).
//! - `realtime` — formal `/ws/client|host` gateway, connection registry,
//!   topic subscriptions, and durable fanout.
//! - `http` — axum router + `/health/*` + `/ws/client|host` gateways
//!   (consumed by `main.rs`).
//!
//! The binary entry point lives in `src/main.rs` and composes the above
//! modules into a running backend.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(deprecated))]

pub mod agent_inbox;
pub mod agent_sessions;
pub mod app;
pub mod approvals;
pub mod auth;
pub mod completion_watch;
pub mod config;
pub mod conversations;
pub mod error;
pub mod friends;
pub mod host_commands;
pub mod host_link;
pub mod http;
pub mod ingest;
pub mod jobs;
pub mod media;
pub mod notifications;
pub mod profiles;
pub mod project;
pub mod realtime;
pub mod runtime;
pub mod store;
pub mod telemetry;
pub mod turn_completion;
