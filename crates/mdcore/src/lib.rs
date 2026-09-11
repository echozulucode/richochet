//! Richochet's conversion engine.
//!
//! Everything normalizes into the [`Document`] model; Markdown, HTML and plain text each have a
//! parser into it and a renderer out of it. This crate has **no dependency on Tauri** and performs
//! no I/O, which keeps its tests fast and lets the engine be reused from a CLI or compiled to WASM.
//!
//! ```
//! use mdcore::{Format, convert};
//!
//! let html = convert("**bold**", Format::Markdown, Format::Html).unwrap();
//! assert_eq!(html, "<p><strong>bold</strong></p>");
//! ```

#![forbid(unsafe_code)]

pub mod document;
pub mod html;
pub mod markdown;
pub mod profile;
pub mod text;

pub use document::model::{Alignment, Block, Cell, Document, Inline, List, ListItem, Row, Table};
pub use profile::RenderProfile;

/// A wire format the engine can read from and write to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum Format {
    /// CommonMark with GFM strikethrough.
    Markdown,
    /// HTML, rendered through a [`RenderProfile`].
    Html,
    /// Unformatted plain text.
    Text,
}

/// Anything that can go wrong converting between formats.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// The input could not be parsed as the format it claimed to be.
    #[error("could not parse input as {format:?}: {detail}")]
    Parse {
        /// The format we tried to parse.
        format: &'static str,
        /// What went wrong.
        detail: String,
    },
}

/// Parse `input` as `from` and render it as `to`, using the Teams render profile.
///
/// Converting a format to itself still round-trips through the AST, which normalizes the input.
pub fn convert(input: &str, from: Format, to: Format) -> Result<String, ConvertError> {
    convert_with(input, from, to, &RenderProfile::teams())
}

/// Like [`convert`], but with an explicit [`RenderProfile`] for HTML output.
pub fn convert_with(
    input: &str,
    from: Format,
    to: Format,
    profile: &RenderProfile,
) -> Result<String, ConvertError> {
    let doc = parse(input, from)?;
    Ok(render(&doc, to, profile))
}

/// A parsed document alongside where each top-level block came from in the source.
///
/// This is what makes the two panes scroll together. The rendered pane has no idea what line
/// anything came from, but its top-level nodes correspond one-for-one with `document.blocks`, so
/// mapping a block index to a source line is enough to align them — the same trick VS Code's
/// Markdown preview uses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Outline {
    /// The parsed, normalized document.
    pub document: Document,
    /// Parallel to `document.blocks`: the 1-based source line each block began on.
    ///
    /// Stays aligned with `blocks` because blocks the normalizer drops have their line dropped
    /// with them.
    pub lines: Vec<u32>,
}

/// Parse Markdown, keeping the source line each top-level block started on.
pub fn outline(markdown: &str) -> Outline {
    crate::markdown::outline(markdown)
}

/// Parse `input` in the given format into the document model.
pub fn parse(input: &str, from: Format) -> Result<Document, ConvertError> {
    Ok(match from {
        Format::Markdown => markdown::parse(input),
        Format::Html => html::parse(input),
        Format::Text => text::parse(input),
    })
}

/// Render a document in the given format.
pub fn render(doc: &Document, to: Format, profile: &RenderProfile) -> String {
    match to {
        Format::Markdown => markdown::render(doc),
        Format::Html => html::render(doc, profile),
        Format::Text => text::render(doc),
    }
}
