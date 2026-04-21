#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod file_store;
pub mod tailscale;

pub use file_store::*;
