//! Writing HTML and a plain-text fallback together, atomically.
//!
//! The clipboard is a *set* of representations of one selection, not a single value. A paste
//! target picks the richest representation it understands: Teams takes `HTML Format`, Notepad
//! takes `CF_UNICODETEXT`. Both must therefore describe the same content and both must be on the
//! clipboard at the same time.
//!
//! That means **one** `OpenClipboard` … `EmptyClipboard` … write … write … `CloseClipboard` cycle.
//! Writing the two representations in separate cycles is the bug this module exists to avoid: the
//! second `EmptyClipboard` wipes the first representation, and Teams silently pastes plain text.

use super::ClipError;

/// Place `html` (when given) and `text` on the clipboard in one open/close cycle.
///
/// `text` is always written as `CF_UNICODETEXT`. When `html` is `Some`, it is additionally written
/// as `HTML Format`, wrapped in a correct CF_HTML header by `clipboard-win`. Pass the *fragment*,
/// not a whole document and not a pre-wrapped block — the header is added here.
///
/// # Errors
///
/// [`ClipError::Busy`] when another application kept the clipboard locked through the whole retries, and
/// [`ClipError::Io`] when a write failed. A failed HTML write leaves the plain-text representation
/// in place, because the text is written first: a degraded paste beats an empty one.
#[cfg(windows)]
pub fn write(html: Option<&str>, text: &str) -> Result<(), ClipError> {
    use clipboard_win::{formats, options::NoClear, raw};

    // Held for the rest of the function; dropping it closes the clipboard.
    let _session = super::open::open()?;

    // Exactly once, up front. `EmptyClipboard` also transfers ownership to us, which is what makes
    // the subsequent `SetClipboardData` calls legal.
    raw::empty().map_err(|e| ClipError::Io {
        op: "write",
        detail: format!("emptying the clipboard: {}", err_text(&e)),
    })?;

    // `set_string` would empty the clipboard again; `set_string_with(_, NoClear)` is the same write
    // without the clear. `raw::set_html` never clears, so it is safe to follow this.
    raw::set_string_with(text, NoClear).map_err(|e| ClipError::Io {
        op: "write",
        detail: format!("CF_UNICODETEXT: {}", err_text(&e)),
    })?;

    if let Some(html) = html {
        let format = formats::Html::new().ok_or_else(|| ClipError::Io {
            op: "write",
            detail: "could not register the \"HTML Format\" clipboard format".to_owned(),
        })?;
        raw::set_html(format.code(), html).map_err(|e| ClipError::Io {
            op: "write",
            detail: format!("HTML Format: {}", err_text(&e)),
        })?;
    }

    Ok(())
}

/// Render a `clipboard-win` error without leaking its type into the signature.
#[cfg(windows)]
fn err_text(e: &clipboard_win::ErrorCode) -> String {
    e.to_string()
}

/// Writing the clipboard is Windows-only; Richochet ships as a Windows desktop utility.
///
/// This arm exists so the crate builds and its tests run on a non-Windows CI box.
#[cfg(not(windows))]
pub fn write(html: Option<&str>, text: &str) -> Result<(), ClipError> {
    let _ = (html, text);
    Err(ClipError::Io {
        op: "write",
        detail: "unsupported on this platform: Richochet writes the clipboard through the \
                 Windows HTML Format, which only exists on Windows"
            .to_owned(),
    })
}
