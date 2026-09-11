//! Export the fixture corpus as a JSON conversion oracle for the frontend E2E tests.
//!
//! Playwright drives the UI in a real browser, where the Rust engine is not available. Rather than
//! reimplementing conversion in JavaScript — which would test nothing — the E2E suite is backed by
//! this table of *real engine output*, precomputed from the same fixtures the Rust tests use.
//! `just oracle` regenerates it, and CI regenerates it too, so the table can never drift from the
//! engine.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use mdcore::{Format, RenderProfile};

/// What the frontend may ask the engine for, precomputed.
///
/// Conversions are keyed `"<from>:<to>:<input>"` and hold a string. Outlines are keyed
/// `"outline:<markdown>"` and hold an array of source line numbers — the scroll-sync map. One flat
/// table with mixed value types keeps the frontend's lookup a single code path.
type Oracle = BTreeMap<String, serde_json::Value>;

fn key(from: Format, to: Format, input: &str) -> String {
    format!("{}:{}:{}", fmt_name(from), fmt_name(to), input)
}

fn fmt_name(f: Format) -> &'static str {
    match f {
        Format::Markdown => "markdown",
        Format::Html => "html",
        Format::Text => "text",
    }
}

/// A document long enough to actually scroll, for the scroll-sync specs.
///
/// 24 top-level blocks over 60 source lines, one of them a 12-line code block so the two panes
/// have genuinely different heights — the case proportional scrolling gets wrong. Must match
/// `SCROLL_SAMPLE` in `e2e/support.ts` byte for byte.
const SCROLL_SAMPLE: &str = "# Block 01 heading\n\nBlock 02 paragraph text.\n\nBlock 03 paragraph text.\n\n```text\nBlock 04 code line 01\nBlock 04 code line 02\nBlock 04 code line 03\nBlock 04 code line 04\nBlock 04 code line 05\nBlock 04 code line 06\nBlock 04 code line 07\nBlock 04 code line 08\nBlock 04 code line 09\nBlock 04 code line 10\nBlock 04 code line 11\nBlock 04 code line 12\n```\n\nBlock 05 paragraph text.\n\nBlock 06 paragraph text.\n\nBlock 07 paragraph text.\n\nBlock 08 paragraph text.\n\nBlock 09 paragraph text.\n\nBlock 10 paragraph text.\n\nBlock 11 paragraph text.\n\nBlock 12 paragraph text.\n\nBlock 13 paragraph text.\n\nBlock 14 paragraph text.\n\nBlock 15 paragraph text.\n\nBlock 16 paragraph text.\n\nBlock 17 paragraph text.\n\nBlock 18 paragraph text.\n\nBlock 19 paragraph text.\n\nBlock 20 paragraph text.\n\nBlock 21 paragraph text.\n\nBlock 22 paragraph text.\n\nBlock 23 paragraph text.\n\nBlock 24 paragraph text.";

/// Markdown the Playwright specs type into the Markdown pane.
const E2E_INPUTS: &[&str] = &[
    "**Hello Eric**",
    "**bold** and _italic_",
    "**bold**",
    "**Important**",
    "- one\n- two",
    "plain words",
    "abcdef",
    "typed",
    "the quick brown fox jumps over the lazy dog",
    SCROLL_SAMPLE,
];

/// HTML the specs stage on the clipboard before a simulated paste.
const E2E_HTML_INPUTS: &[&str] =
    &[r#"<div><span style="font-weight:600"> Important </span></div>"#];

/// Plain text the specs stage on the clipboard before a simulated paste.
const E2E_TEXT_INPUTS: &[&str] = &["just words", "plain words"];

/// Every conversion direction the UI can trigger.
const PAIRS: &[(Format, Format)] = &[
    (Format::Markdown, Format::Html),
    (Format::Markdown, Format::Text),
    (Format::Markdown, Format::Markdown),
    (Format::Html, Format::Markdown),
    (Format::Html, Format::Text),
    (Format::Html, Format::Html),
    (Format::Text, Format::Markdown),
];

/// Walk `tests/fixtures/`, convert every source in every direction, and write the table.
pub fn export(out: &Path) -> Result<()> {
    let root = fixture_root()?;
    let profile = RenderProfile::teams();
    let mut oracle = Oracle::new();

    let mut inputs: Vec<(Format, String)> = Vec::new();
    for entry in std::fs::read_dir(&root).with_context(|| format!("reading {}", root.display()))? {
        let dir = entry?.path();
        if !dir.is_dir() {
            continue;
        }
        for (file, format) in [
            ("source.md", Format::Markdown),
            ("source.html", Format::Html),
            ("source.txt", Format::Text),
        ] {
            let p = dir.join(file);
            if p.exists() {
                inputs.push((format, std::fs::read_to_string(&p)?));
            }
        }
    }

    // Include the UI's own starter content so the first render in a test is covered.
    inputs.push((Format::Markdown, String::new()));

    // Strings the Playwright specs type or stage. They are not fixtures — they prove nothing about
    // Teams — but the mock backend can only answer with real engine output for inputs listed here,
    // and an E2E test asserting on a passthrough would prove nothing either. Keep in sync with
    // `e2e/*.spec.ts`; a miss shows up as a failing assertion, not a silent pass.
    for s in E2E_INPUTS {
        inputs.push((Format::Markdown, (*s).to_owned()));
    }
    for s in E2E_HTML_INPUTS {
        inputs.push((Format::Html, (*s).to_owned()));
    }
    for s in E2E_TEXT_INPUTS {
        inputs.push((Format::Text, (*s).to_owned()));
    }

    for (from, input) in &inputs {
        for (pf, pt) in PAIRS {
            if pf != from {
                continue;
            }
            let rendered = mdcore::convert_with(input, *pf, *pt, &profile)?;
            oracle.insert(key(*pf, *pt, input), serde_json::Value::from(rendered));
        }
    }

    // The scroll-sync map: which source line each top-level block starts on. Only Markdown has
    // one — it is the only format the user scrolls as text.
    for (from, input) in &inputs {
        if *from != Format::Markdown {
            continue;
        }
        let lines = mdcore::outline(input).lines;
        oracle.insert(format!("outline:{input}"), serde_json::Value::from(lines));
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&oracle)?;
    std::fs::write(out, json).with_context(|| format!("writing {}", out.display()))?;
    eprintln!("wrote {} conversions to {}", oracle.len(), out.display());
    Ok(())
}

/// Locate `tests/fixtures/` relative to the workspace root.
fn fixture_root() -> Result<std::path::PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .context("locating workspace root")?;
    Ok(root.join("tests").join("fixtures"))
}
