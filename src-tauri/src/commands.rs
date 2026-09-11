//! The IPC command surface.
//!
//! **Frozen contract.** The frontend is written against these signatures. Every command is
//! registered in `generate_handler![]` in [`crate::run`] — an unregistered command fails silently
//! at runtime.

use mdcore::{Format, RenderProfile};
use serde::{Deserialize, Serialize};

use crate::clipboard::{self, ClipError};

/// A wire format, mirrored from [`mdcore::Format`] so the frontend has a stable name for it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireFormat {
    /// CommonMark with GFM strikethrough.
    Markdown,
    /// HTML in the Teams dialect.
    Html,
    /// Unformatted plain text.
    Text,
}

impl From<WireFormat> for Format {
    fn from(f: WireFormat) -> Self {
        match f {
            WireFormat::Markdown => Format::Markdown,
            WireFormat::Html => Format::Html,
            WireFormat::Text => Format::Text,
        }
    }
}

/// A conversion failure, serialized to the frontend as a message string.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The engine could not convert the input.
    #[error(transparent)]
    Convert(#[from] mdcore::ConvertError),
    /// The clipboard could not be read or written.
    #[error(transparent)]
    Clipboard(#[from] ClipError),
}

impl Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// What was found on the clipboard when the user pasted.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPayload {
    /// `"rich"` when HTML was available, `"text"` when only plain text was.
    pub kind: &'static str,
    /// The HTML fragment, with any CF_HTML header already stripped.
    pub html: Option<String>,
    /// The RTF representation, if one was present. Unused until Phase 6.
    pub rtf: Option<String>,
    /// The plain-text fallback.
    pub text: String,
}

/// Content to place on the clipboard.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundPayload {
    /// The HTML representation. When present it is written alongside `text`.
    pub html: Option<String>,
    /// The plain-text fallback. Always written.
    pub text: String,
}

/// Convert `input` from one format to another through the document model.
#[tauri::command]
pub fn convert(input: String, from: WireFormat, to: WireFormat) -> Result<String, CommandError> {
    let profile = RenderProfile::teams();
    Ok(mdcore::convert_with(
        &input,
        from.into(),
        to.into(),
        &profile,
    )?)
}

/// Where each top-level block of a Markdown document starts, for scrolling the panes together.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlinePayload {
    /// 1-based source line per top-level block, in document order.
    pub lines: Vec<u32>,
}

/// Map the Markdown source to its top-level blocks.
///
/// Separate from [`convert`] on purpose: it is only needed for scroll sync, it is cheap, and
/// keeping it out of the conversion path means the two panes' live update never waits on it.
#[tauri::command]
pub fn outline(markdown: String) -> Result<OutlinePayload, CommandError> {
    Ok(OutlinePayload {
        lines: mdcore::outline(&markdown).lines,
    })
}

/// Read every representation the clipboard offers and return the richest ones.
#[tauri::command]
pub fn read_clipboard() -> Result<ClipboardPayload, CommandError> {
    Ok(clipboard::read()?)
}

/// Write HTML and a plain-text fallback to the clipboard in a single atomic operation.
#[tauri::command]
pub fn write_clipboard(payload: OutboundPayload) -> Result<(), CommandError> {
    clipboard::write(payload.html.as_deref(), &payload.text)?;
    Ok(())
}
