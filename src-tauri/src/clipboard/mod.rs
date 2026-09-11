//! Native clipboard access.
//!
//! Tauri's official clipboard-manager exposes only text and images for reading, so Richochet
//! reaches the platform clipboard directly. See `docs/adr/0001-clipboard-strategy.md`.

pub mod cf_html;
mod read;
mod write;

pub use read::read;
pub use write::write;

/// Anything that can go wrong talking to the platform clipboard.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClipError {
    /// The clipboard could not be opened; another process usually holds it briefly.
    #[error("could not open the clipboard: {0}")]
    Open(String),
    /// A read or write failed.
    #[error("clipboard {op} failed: {detail}")]
    Io {
        /// `"read"` or `"write"`.
        op: &'static str,
        /// The platform error.
        detail: String,
    },
    /// The clipboard held bytes that were not valid text in the expected encoding.
    #[error("clipboard contained invalid {encoding}: {detail}")]
    Encoding {
        /// The encoding we expected.
        encoding: &'static str,
        /// What went wrong.
        detail: String,
    },
}
