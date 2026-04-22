//! Dart-visible module tree. `flutter_rust_bridge_codegen` walks this tree to
//! discover `#[frb(...)]`-annotated items. Do NOT add Rust-internal helpers
//! here; put them under `crate::` roots that the codegen config excludes.

pub mod minos;
