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

/// One conversion the frontend may be asked to perform, keyed by `"<from>:<to>:<input>"`.
type Oracle = BTreeMap<String, String>;

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

    for (from, input) in &inputs {
        for (pf, pt) in PAIRS {
            if pf != from {
                continue;
            }
            let rendered = mdcore::convert_with(input, *pf, *pt, &profile)?;
            oracle.insert(key(*pf, *pt, input), rendered);
        }
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
