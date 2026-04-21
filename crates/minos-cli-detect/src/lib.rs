#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

pub mod detect;
pub mod runner;

pub use detect::*;
pub use runner::*;
