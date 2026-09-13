//! Reading every representation the clipboard offers, richest first.
//!
//! The whole read happens inside **one** `OpenClipboard`/`CloseClipboard` cycle. Windows lets
//! exactly one process own the clipboard at a time, so re-opening it per format is both slower and
//! racy — another process can win the handle between two of our reads and we would come back with
//! HTML from one copy and text from the next.

use super::ClipError;
use crate::commands::ClipboardPayload;

/// The registered clipboard format name RTF lives under.
#[cfg(windows)]
const RTF_FORMAT_NAME: &str = "Rich Text Format";

/// Read the clipboard, preferring HTML over plain text.
///
/// `kind` is `"rich"` whenever an `HTML Format` payload was present and `"text"` otherwise. RTF is
/// captured verbatim into [`ClipboardPayload::rtf`] when present and is never parsed. The
/// plain-text fallback is always read, so a rich payload still carries something a plain-text
/// consumer can use.
///
/// # Errors
///
/// Returns [`ClipError::Busy`] when another application kept the clipboard locked through the whole
/// retry window, and [`ClipError::Io`] when the clipboard was open but held neither text nor HTML. A
/// format that is present but unreadable fails with [`ClipError::Io`] rather than being silently
/// dropped; a format that is simply absent is not an error.
#[cfg(windows)]
pub fn read() -> Result<ClipboardPayload, ClipError> {
    // Held for the rest of the function; dropping it closes the clipboard.
    let _session = super::open::open()?;

    let html = read_html()?;
    let rtf = read_rtf()?;
    let text = read_text()?;

    let (kind, text) = match (html.is_some(), text) {
        (true, Some(text)) => ("rich", text),
        // HTML with no CF_UNICODETEXT companion is rare but legal; the contract says `text` is
        // always a String, so an empty fallback is the honest answer.
        (true, None) => ("rich", String::new()),
        (false, Some(text)) => ("text", text),
        (false, None) => {
            return Err(ClipError::Io {
                op: "read",
                detail: "the clipboard holds neither text nor HTML".to_owned(),
            })
        }
    };

    Ok(ClipboardPayload {
        kind,
        html,
        rtf,
        text,
    })
}

/// Read the `HTML Format` payload, with its CF_HTML header stripped.
///
/// `clipboard_win::raw::get_html` does the stripping using the block's `StartFragment` /
/// `EndFragment` offsets. When those offsets are missing or unparseable it falls back to returning
/// the *entire* block, header and all — which would put a raw CF_HTML header into
/// [`ClipboardPayload::html`]. So the result is checked, and anything that still looks wrapped is
/// handed to [`super::cf_html::decode`], which tolerates far more producer sloppiness.
#[cfg(windows)]
fn read_html() -> Result<Option<String>, ClipError> {
    use clipboard_win::{formats, raw};

    // Registering "HTML Format" is idempotent; it returns the same id the producer used.
    let Some(html) = formats::Html::new() else {
        return Ok(None);
    };
    let code = html.code();
    if !raw::is_format_avail(code) {
        return Ok(None);
    }

    let mut buf = Vec::new();
    raw::get_html(code, &mut buf).map_err(|e| ClipError::Io {
        op: "read",
        detail: format!("HTML Format: {}", err_text(&e)),
    })?;

    let fragment = String::from_utf8(buf).map_err(|e| ClipError::Encoding {
        encoding: "UTF-8",
        detail: format!("HTML Format: {e}"),
    })?;
    let fragment = trim_trailing_nuls(&fragment);

    if looks_wrapped(fragment) {
        return super::cf_html::decode(fragment).map(Some);
    }
    Ok(Some(fragment.to_owned()))
}

/// Does this string still carry a CF_HTML wrapper that `get_html` failed to strip?
#[cfg(windows)]
fn looks_wrapped(s: &str) -> bool {
    s.starts_with("Version:")
        || s.starts_with("StartHTML:")
        || s.starts_with("StartFragment:")
        || s.contains("<!--StartFragment")
}

/// Read the RTF payload verbatim, without parsing it. Unused until Phase 6.
///
/// RTF is a 7-bit ASCII stream (non-ASCII is escaped as `\'xx`), so it is valid UTF-8 in practice.
/// Decoding is lossy anyway rather than fatal: a stray byte in a representation nothing reads yet
/// must not fail the whole clipboard read.
#[cfg(windows)]
fn read_rtf() -> Result<Option<String>, ClipError> {
    use clipboard_win::raw;

    let Some(code) = raw::register_format(RTF_FORMAT_NAME) else {
        return Ok(None);
    };
    let code = code.get();
    if !raw::is_format_avail(code) {
        return Ok(None);
    }

    let mut buf = Vec::new();
    raw::get_vec(code, &mut buf).map_err(|e| ClipError::Io {
        op: "read",
        detail: format!("{RTF_FORMAT_NAME}: {}", err_text(&e)),
    })?;
    // The global memory block is NUL-terminated and may be padded.
    while buf.last() == Some(&0) {
        buf.pop();
    }
    if buf.is_empty() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Read the `CF_UNICODETEXT` fallback, converted from UTF-16 to UTF-8.
#[cfg(windows)]
fn read_text() -> Result<Option<String>, ClipError> {
    use clipboard_win::{formats, raw};

    if !raw::is_format_avail(formats::CF_UNICODETEXT) {
        return Ok(None);
    }

    let mut buf = Vec::new();
    raw::get_string(&mut buf).map_err(|e| ClipError::Io {
        op: "read",
        detail: format!("CF_UNICODETEXT: {}", err_text(&e)),
    })?;
    // `get_string` already drops the terminating NUL, but never trust that for a buffer that came
    // from another process.
    let text = String::from_utf8(buf).map_err(|e| ClipError::Encoding {
        encoding: "UTF-8",
        detail: format!("CF_UNICODETEXT: {e}"),
    })?;
    Ok(Some(trim_trailing_nuls(&text).to_owned()))
}

/// Drop the NUL padding Windows leaves on the end of a global memory block.
#[cfg(windows)]
fn trim_trailing_nuls(s: &str) -> &str {
    s.trim_end_matches('\0')
}

/// Render a `clipboard-win` error without leaking its type into the signature.
#[cfg(windows)]
fn err_text(e: &clipboard_win::ErrorCode) -> String {
    e.to_string()
}

/// Reading the clipboard is Windows-only; Richochet ships as a Windows desktop utility.
///
/// This arm exists so the crate builds and its tests run on a non-Windows CI box.
#[cfg(not(windows))]
pub fn read() -> Result<ClipboardPayload, ClipError> {
    Err(ClipError::Io {
        op: "read",
        detail: "unsupported on this platform: Richochet reads the clipboard through the \
                 Windows HTML Format, which only exists on Windows"
            .to_owned(),
    })
}
