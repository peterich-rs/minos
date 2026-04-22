//! flutter_rust_bridge v2 adapter for `minos-mobile::MobileClient`.
//!
//! The Dart-visible surface lives under [`api`]; the codegen tool scans that
//! module tree and emits:
//!   * `frb_generated.rs` here (re-exported below), containing the wire
//!     handlers that bridge Dart ↔ Rust.
//!   * `apps/mobile/lib/src/rust/**.dart`, the Dart-side API mirror.
//!
//! See `flutter_rust_bridge.yaml` at the repo root for the codegen config.

// frb's `#[frb(...)]` macros expand a `#[cfg(frb_expand)]` branch that's
// only set by the codegen-side evaluator. Whitelisting the name at the crate
// root stops cargo from warning on every annotated declaration. This cannot
// live in `[lints.rust]` because that conflicts with `lints.workspace = true`.
#![allow(unexpected_cfgs)]

pub mod api;

// `frb_generated.rs` is produced by `flutter_rust_bridge_codegen generate`
// and checked in so CI's Dart leg does not need a Rust toolchain. It must
// exist for this crate to compile at all.
//
// The generated file ships its own `#![allow(...)]` preamble but that only
// covers a curated subset of lints. Our workspace turns on `clippy::pedantic`
// crate-wide, which fires on idioms the codegen emits intentionally
// (wildcard imports, `format!` substitutions, `as` casts, unsafe-pointer
// shapes required for the Dart FFI boundary, etc.). Silence them broadly at
// the module declaration — this lint mask applies only to the generated
// file, not to anything else we ship.
#[allow(
    unused,
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction,
    clippy::cargo
)]
mod frb_generated;

#[allow(unused_imports)]
pub use frb_generated::*;
