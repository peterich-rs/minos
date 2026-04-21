#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod backoff;

pub use backoff::*;
