//! Reading every representation the clipboard offers, richest first.

use super::ClipError;
use crate::commands::ClipboardPayload;

/// Read the clipboard, preferring HTML over plain text.
pub fn read() -> Result<ClipboardPayload, ClipError> {
    todo!("1.x")
}
