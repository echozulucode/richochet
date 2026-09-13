//! Native clipboard access.
//!
//! Tauri's official clipboard-manager exposes only text and images for reading, so Richochet
//! reaches the platform clipboard directly. See `docs/adr/0001-clipboard-strategy.md`.

pub mod cf_html;
mod open;
mod read;
mod write;

pub use read::read;
pub use write::write;

/// Anything that can go wrong talking to the platform clipboard.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClipError {
    /// The clipboard stayed locked by another application for the whole retry window.
    ///
    /// The message is written for a person, because it lands in a toast: the raw
    /// `OSError(5): Access is denied` told users nothing about what to do. `holder` names the process
    /// that had the clipboard open when Windows would say, which turns a vague failure in the field
    /// into one that identifies its own cause.
    #[error("{}", busy_message(.holder.as_deref()))]
    Busy {
        /// Executable name of the process holding the clipboard open, if it could be found.
        holder: Option<String>,
        /// The underlying platform error, kept for diagnostics.
        detail: String,
    },
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

/// The user-facing text for [`ClipError::Busy`].
fn busy_message(holder: Option<&str>) -> String {
    match holder {
        Some(name) => {
            format!("The clipboard is in use by {name}. Try again in a moment.")
        }
        None => {
            "The clipboard is in use by another application. Try again in a moment.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_clipboard_names_its_holder() {
        let error = ClipError::Busy {
            holder: Some("rdpclip.exe".into()),
            detail: "OSError(5): Access is denied".into(),
        };
        assert_eq!(
            error.to_string(),
            "The clipboard is in use by rdpclip.exe. Try again in a moment."
        );
    }

    #[test]
    fn a_busy_clipboard_without_a_known_holder_still_says_what_to_do() {
        let error = ClipError::Busy {
            holder: None,
            detail: "OSError(5): Access is denied".into(),
        };
        assert_eq!(
            error.to_string(),
            "The clipboard is in use by another application. Try again in a moment."
        );
    }

    #[test]
    fn the_raw_os_error_stays_out_of_the_message() {
        // It lands in a toast. The OS error is kept in `detail` for diagnostics, not shown.
        let error = ClipError::Busy {
            holder: None,
            detail: "OSError(5): Access is denied".into(),
        };
        assert!(!error.to_string().contains("OSError"));
    }
}
