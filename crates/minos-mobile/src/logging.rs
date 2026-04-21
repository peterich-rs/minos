//! mars-xlog wiring lives in Phase J Task 30. For now this module exposes a
//! noop init so frb-side bootstrap can call it without conditional compilation.

use minos_domain::MinosError;

#[allow(clippy::missing_errors_doc)]
pub fn init() -> Result<(), MinosError> {
    Ok(())
}
