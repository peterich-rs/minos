//! Placeholder for future migration loader helpers.
//!
//! The current implementation relies on `sqlx::migrate!()` directly in
//! [`super::LocalStore::open`]. This module exists so that downstream
//! changes (e.g., introspection / dry-run tooling) have a stable location.
