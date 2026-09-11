//! Data-driven tests over the golden corpus in `tests/fixtures/`.
//!
//! The runner enumerates fixture directories, so **adding a case requires no code change** — drop
//! a directory containing `source.md`, `source.html` or `source.txt` into `tests/fixtures/` and it
//! is picked up. That is what makes Phase 4 fidelity work tractable: a newly discovered Teams
//! quirk becomes a fixture, not a code branch.
//!
//! Expected output is held in `insta` snapshots rather than hand-written files, so that changes to
//! the engine surface as a reviewable diff (`just review`) instead of a guessed-at literal.

use mdcore::{Format, RenderProfile};

/// Render one source through every output format, as a single reviewable snapshot.
fn report(input: &str, from: Format) -> String {
    let profile = RenderProfile::teams().pretty();
    let mut out = String::new();

    for (label, to) in [
        ("markdown", Format::Markdown),
        ("teams html", Format::Html),
        ("plain text", Format::Text),
    ] {
        let rendered = match mdcore::convert_with(input, from, to, &profile) {
            Ok(s) => s,
            Err(e) => format!("<error: {e}>"),
        };
        out.push_str(&format!("=== {label} ===\n{rendered}\n\n"));
    }
    out
}

/// Converting twice must produce the same result as converting once.
///
/// This is the property that keeps the two editor panes from drifting: the UI re-converts on every
/// keystroke, so a non-idempotent renderer would make the document creep as the user types.
fn assert_idempotent(input: &str, from: Format) {
    let profile = RenderProfile::teams();
    let once = mdcore::convert_with(input, from, from, &profile).expect("first pass");
    let twice = mdcore::convert_with(&once, from, from, &profile).expect("second pass");
    similar_asserts::assert_eq!(
        once,
        twice,
        "converting {from:?} -> {from:?} is not idempotent"
    );
}

/// The corpus root, resolved from this crate's manifest directory.
fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn markdown_fixtures() {
    insta::glob!(fixtures(), "*/source.md", |path| {
        let input = std::fs::read_to_string(path).expect("reading fixture");
        assert_idempotent(&input, Format::Markdown);
        insta::assert_snapshot!(report(&input, Format::Markdown));
    });
}

#[test]
fn html_fixtures() {
    insta::glob!(fixtures(), "*/source.html", |path| {
        let input = std::fs::read_to_string(path).expect("reading fixture");
        assert_idempotent(&input, Format::Html);
        insta::assert_snapshot!(report(&input, Format::Html));
    });
}

#[test]
fn text_fixtures() {
    insta::glob!(fixtures(), "*/source.txt", |path| {
        let input = std::fs::read_to_string(path).expect("reading fixture");
        assert_idempotent(&input, Format::Text);
        insta::assert_snapshot!(report(&input, Format::Text));
    });
}

/// Every fixture directory must document where it came from and what it proves.
///
/// The corpus is the project's memory of Teams' behaviour; an undocumented fixture is a test
/// nobody can safely change later.
#[test]
fn every_fixture_has_notes() {
    let root = fixtures().canonicalize().expect("fixtures directory");

    let mut missing = Vec::new();
    let mut count = 0;
    for entry in std::fs::read_dir(&root).expect("reading fixtures") {
        let dir = entry.expect("fixture entry").path();
        if !dir.is_dir() {
            continue;
        }
        count += 1;
        if !dir.join("notes.md").exists() {
            missing.push(dir.file_name().unwrap().to_string_lossy().to_string());
        }
        let has_source = ["source.md", "source.html", "source.txt"]
            .iter()
            .any(|f| dir.join(f).exists());
        assert!(
            has_source,
            "fixture {} has no source.md/source.html/source.txt",
            dir.display()
        );
    }

    assert!(missing.is_empty(), "fixtures missing notes.md: {missing:?}");
    assert!(count > 0, "no fixtures found in {}", root.display());
}
