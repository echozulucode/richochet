//! The HTML dialect to emit, expressed as data.
//!
//! Teams supports a Markdown-*style* syntax that Microsoft explicitly documents as not being
//! standard Markdown, and the rich clipboard representation it accepts is not part of any public
//! contract. So rather than hard-coding a dialect into the renderer, the dialect is a value.
//! Tuning Teams fidelity (Phase 4) means editing a struct literal and adding a fixture, not
//! rewriting [`crate::html::render`].
//!
//! Defaults here are the Phase 1 starting hypothesis. They are expected to change once
//! `docs/clipboard-findings.md` exists.

/// How to encode an inline mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkStyle {
    /// Wrap in a semantic tag, e.g. `<strong>`.
    Tag(&'static str),
    /// Wrap in `<span>` carrying an inline style, e.g. `font-weight:bold`.
    Style(&'static str),
}

/// How to encode headings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadingStrategy {
    /// Native `<h1>`..`<h3>`.
    Native,
    /// A paragraph of bold text, for targets that strip heading tags.
    BoldParagraph,
}

/// How to encode fenced code blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeBlockStrategy {
    /// `<pre><code>`, the standard encoding.
    PreCode,
    /// A `<div>` with a monospace font, for targets that strip `<pre>`.
    MonospaceDiv,
}

/// How to encode nested lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestingStrategy {
    /// A nested `<ul>`/`<ol>` inside the parent `<li>`.
    Native,
    /// A flat list with `margin-left` indentation, for targets that flatten nesting.
    MarginIndent,
}

/// How to encode block quotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStrategy {
    /// A native `<blockquote>`.
    Blockquote,
    /// Paragraphs prefixed with `> `.
    TextPrefix,
}

/// A complete HTML dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderProfile {
    /// How to mark strong emphasis.
    pub bold: MarkStyle,
    /// How to mark emphasis.
    pub italic: MarkStyle,
    /// How to mark strikethrough.
    pub strike: MarkStyle,
    /// How to mark inline code.
    pub code: MarkStyle,
    /// How to encode headings.
    pub headings: HeadingStrategy,
    /// How to encode code blocks.
    pub code_block: CodeBlockStrategy,
    /// How to encode nested lists.
    pub nested_lists: NestingStrategy,
    /// How to encode block quotes.
    pub blockquote: QuoteStrategy,
    /// Emit a newline between block elements. Off for clipboard payloads, on for readable output.
    pub pretty: bool,
}

impl RenderProfile {
    /// Plain semantic HTML. The baseline every other profile is a deviation from.
    pub fn standard() -> Self {
        RenderProfile {
            bold: MarkStyle::Tag("strong"),
            italic: MarkStyle::Tag("em"),
            strike: MarkStyle::Tag("s"),
            code: MarkStyle::Tag("code"),
            headings: HeadingStrategy::Native,
            code_block: CodeBlockStrategy::PreCode,
            nested_lists: NestingStrategy::Native,
            blockquote: QuoteStrategy::Blockquote,
            pretty: false,
        }
    }

    /// The dialect written to the clipboard for Teams.
    ///
    /// Currently identical to [`RenderProfile::standard`]. This is the Phase 1 hypothesis: that
    /// Teams accepts well-formed semantic HTML. Every deviation discovered by the clipboard spike
    /// gets encoded here, with a fixture and a line in `docs/clipboard-findings.md`.
    pub fn teams() -> Self {
        RenderProfile::standard()
    }

    /// A readable variant with newlines between blocks, for tests and CLI output.
    pub fn pretty(mut self) -> Self {
        self.pretty = true;
        self
    }
}

impl Default for RenderProfile {
    fn default() -> Self {
        RenderProfile::standard()
    }
}
