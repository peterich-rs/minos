//! mars-xlog wiring lives in Task 29 (Phase J). For now this module just
//! exposes a noop init so other modules can call it without conditional
//! compilation.

use minos_domain::MinosError;

/// Idempotent. Real implementation arrives in Phase J Task 29.
#[allow(clippy::missing_errors_doc)]
pub fn init() -> Result<(), MinosError> {
    Ok(())
}
