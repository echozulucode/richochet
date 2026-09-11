//! Writing HTML and a plain-text fallback together, atomically.

use super::ClipError;

/// Place `html` (when given) and `text` on the clipboard in one open/close cycle.
pub fn write(html: Option<&str>, text: &str) -> Result<(), ClipError> {
    todo!("1.x")
}
