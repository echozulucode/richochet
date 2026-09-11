//! The official CommonMark and GFM specification examples, used as an input corpus.
//!
//! The spec ships ~1300 machine-readable examples written by the people who designed the format.
//! That is a far broader and more adversarial body of real Markdown than any corpus we could write
//! by hand, so it is worth running the engine over all of it.
//!
//! **We deliberately do not compare our HTML against the spec's reference HTML.** Richochet is not
//! a CommonMark renderer and does not want to be: it renders through a [`RenderProfile`] aimed at
//! Teams, its document model is deliberately small (no images, headings clamped to three levels),
//! and `Block::Unsupported` degrades content on purpose. Asserting equality with the reference
//! output would be asserting we are something we are not, and every profile change would break it.
//!
//! What the examples are used for instead is the same set of invariants `roundtrip.rs` asserts
//! over *generated* documents, now driven by *real* input:
//!
//! 1. nothing panics, in any direction;
//! 2. Markdown settles under repeated conversion, and stays settled;
//! 3. settled Markdown survives a trip through HTML unchanged;
//! 4. no visible word is silently lost.
//!
//! Where an example legitimately fails (3) or (4) — because the model cannot hold what it
//! contains — the example number and a reason live in `tests/data/known-divergences.txt`, and the
//! tests assert the set of divergences is **exactly** that list. A new divergence fails the build;
//! so does a fixed one, until it is removed from the list. Known gaps are tracked data, not a
//! silent allowance.
//!
//! The spec files are vendored under `tests/data/` — see the README there. Nothing here touches
//! the network, so the suite stays offline and deterministic.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;

use mdcore::{Block, Document, Format, Inline, List, ListItem, RenderProfile, Table};

// ---------------------------------------------------------------------------
// Loading the vendored specs
// ---------------------------------------------------------------------------

/// One example lifted out of a spec document.
struct Example {
    /// Stable identifier, e.g. `CM-042` or `GFM-198`. This is what the divergence list keys on.
    id: String,
    /// The `##` section the example appeared under, for diagnostics.
    section: String,
    /// The example's Markdown input.
    markdown: String,
    /// The spec's reference HTML. Used *only* to derive the words the example makes visible.
    html: String,
}

/// The directory holding the vendored spec files.
fn data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

/// Read a vendored data file, failing loudly rather than silently running an empty corpus.
fn read_data(name: &str) -> String {
    let path = data_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading vendored spec data {}: {e}", path.display()))
}

/// Pull the examples out of a spec written in CommonMark's own `spec.txt` format.
///
/// The format is a fence of 32 backticks followed by `example` (extensions add a tag, e.g.
/// `example table`), the Markdown input, a line containing only `.`, the reference HTML, then a
/// closing fence. Tabs are written as `→` so they survive editing, exactly as the spec's own test
/// harness assumes. Examples are numbered sequentially in order of appearance, which is the
/// numbering the published spec and `spec.json` use.
///
/// `keep` decides which sections contribute examples; the number still advances for skipped ones
/// so that ids match the published spec.
fn parse_spec(source: &str, prefix: &str, keep: impl Fn(&str) -> bool) -> Vec<Example> {
    /// A fence shorter than this is ordinary code in the spec's prose.
    const FENCE: &str = "````````````````````````````````";

    let mut examples = Vec::new();
    let mut section = String::from("(preamble)");
    let mut number = 0u32;
    let mut lines = source.lines();

    while let Some(line) = lines.next() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading.trim().to_string();
            continue;
        }
        let Some(rest) = line.strip_prefix(FENCE) else {
            continue;
        };
        if !rest.trim_start().starts_with("example") {
            continue;
        }

        number += 1;
        let mut markdown = String::new();
        let mut html = String::new();
        let mut in_html = false;
        for body in lines.by_ref() {
            // The *first* bare `.` separates input from output; a later one is content.
            if !in_html && body == "." {
                in_html = true;
                continue;
            }
            if body.starts_with(FENCE) {
                break;
            }
            let target = if in_html { &mut html } else { &mut markdown };
            target.push_str(body);
            target.push('\n');
        }

        if !keep(&section) {
            continue;
        }
        examples.push(Example {
            id: format!("{prefix}-{number:03}"),
            section: section.clone(),
            markdown: markdown.replace('\u{2192}', "\t"),
            html: html.replace('\u{2192}', "\t"),
        });
    }

    examples
}

/// The whole corpus: CommonMark in full, plus GFM's extension chapters.
///
/// The GFM spec is a fork of CommonMark 0.29 with five extra sections spliced in, so running it
/// whole would re-run an older copy of everything above under a second set of numbers. Only the
/// sections GFM actually adds — tables, task lists, strikethrough, extended autolinks and the tag
/// filter, each marked `(extension)` in its heading — are new input, so those are what we take.
fn corpus() -> Vec<Example> {
    let mut examples = parse_spec(&read_data("commonmark-0.31.2.spec.txt"), "CM", |_| true);
    examples.extend(parse_spec(
        &read_data("gfm-0.29.spec.txt"),
        "GFM",
        |section| section.ends_with("(extension)"),
    ));

    assert!(
        examples.len() > 600,
        "only {} spec examples loaded — the vendored spec format has probably changed",
        examples.len()
    );
    examples
}

// ---------------------------------------------------------------------------
// The known-divergence list
// ---------------------------------------------------------------------------

/// Invariant 3: settled Markdown must survive a trip through HTML.
const HTML_ROUNDTRIP: &str = "html-roundtrip";
/// Invariant 4: the visible words of an example must all reach the plain-text rendering.
const CONTENT_LOSS: &str = "content-loss";

/// Load `known-divergences.txt` as `kind -> id -> reason`.
///
/// Lines are `<kind> <example-id> <one-line reason>`; `#` starts a comment.
fn known_divergences() -> BTreeMap<String, BTreeMap<String, String>> {
    let raw = read_data("known-divergences.txt");
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    for (n, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on runs of whitespace so the file can be column-aligned for reading.
        let mut parts = line.split_whitespace();
        let kind = parts.next().unwrap_or_default();
        let id = parts.next().unwrap_or_default();
        let reason = parts.collect::<Vec<_>>().join(" ");
        assert!(
            !id.is_empty() && !reason.is_empty(),
            "known-divergences.txt line {}: expected `<kind> <id> <reason>`, got {line:?}",
            n + 1
        );
        assert!(
            kind == HTML_ROUNDTRIP || kind == CONTENT_LOSS,
            "known-divergences.txt line {}: unknown kind {kind:?}",
            n + 1
        );
        let previous = out
            .entry(kind.to_string())
            .or_default()
            .insert(id.to_string(), reason.to_string());
        assert!(
            previous.is_none(),
            "known-divergences.txt line {}: {id} listed twice under {kind}",
            n + 1
        );
    }
    out
}

/// Assert the divergences observed are *exactly* the ones vendored for this invariant.
///
/// Both directions matter. An unexpected divergence is a regression. An entry that no longer
/// diverges is a fix nobody recorded, and leaving it listed would mask the next regression in the
/// same example — so it fails too, until the line is deleted.
fn assert_divergences_match(
    kind: &str,
    observed: &BTreeMap<String, String>,
    corpus_ids: &BTreeSet<String>,
) {
    let mut all = known_divergences();
    let known = all.remove(kind).unwrap_or_default();

    let stale: Vec<&String> = known
        .keys()
        .filter(|id| !corpus_ids.contains(*id))
        .collect();
    assert!(
        stale.is_empty(),
        "known-divergences.txt lists ids under `{kind}` that are not in the corpus: {stale:?}"
    );

    let unexpected: Vec<&String> = observed
        .keys()
        .filter(|id| !known.contains_key(*id))
        .collect();
    let fixed: Vec<&String> = known
        .keys()
        .filter(|id| !observed.contains_key(*id))
        .collect();

    if unexpected.is_empty() && fixed.is_empty() {
        return;
    }

    let mut message =
        format!("`{kind}` divergences do not match tests/data/known-divergences.txt\n");
    if !unexpected.is_empty() {
        message.push_str("\nNEW divergences (a regression, or a case to record):\n");
        for id in unexpected {
            let _ = writeln!(message, "{kind} {id} {}", observed[id]);
        }
    }
    if !fixed.is_empty() {
        message.push_str("\nNO LONGER diverging (delete these lines):\n");
        for id in fixed {
            let _ = writeln!(message, "{kind} {id} {}", known[id]);
        }
    }
    panic!("{message}");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The most passes a document may take to stop changing. Mirrors `roundtrip.rs`.
///
/// One is not always enough: factoring a mark shared by adjacent siblings can produce a shape
/// CommonMark cannot express, which the renderer then simplifies on the *next* pass. What matters
/// is that the text settles and stays settled, not that it settles instantly.
const MAX_PASSES: usize = 4;

/// Convert Markdown to itself until it stops changing.
///
/// Returns `Err` holding the last text if it never settles — a document that oscillates forever
/// would visibly churn under the user's cursor, which is the failure worth catching.
fn settle(input: &str) -> Result<String, String> {
    let profile = RenderProfile::teams();
    let mut current = mdcore::convert_with(input, Format::Markdown, Format::Markdown, &profile)
        .expect("markdown conversion is infallible");
    for _ in 0..MAX_PASSES {
        let next = mdcore::convert_with(&current, Format::Markdown, Format::Markdown, &profile)
            .expect("markdown conversion is infallible");
        if next == current {
            return Ok(current);
        }
        current = next;
    }
    Err(current)
}

/// The words a chunk of HTML puts on the screen.
///
/// Deliberately crude, and deliberately lenient: tags and attributes go (so `href`s and `alt` text
/// are never counted as visible), the four entities cmark emits are decoded, and every character
/// that is not alphanumeric is dropped from each token. That leaves a set that is blind to markup,
/// escaping and punctuation differences — which is the point. We are asking "did the words
/// survive", not "did we render it the same way".
fn visible_words(html: &str) -> BTreeSet<String> {
    let mut text = String::with_capacity(html.len());
    let mut depth = 0usize;
    for ch in html.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => {
                depth -= 1;
                // A tag boundary separates words: `a<br>b` is two words, not one.
                text.push(' ');
            }
            _ if depth == 0 => text.push(ch),
            _ => {}
        }
    }
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&");

    normalize_words(&text)
}

/// Split text into a set of lowercase alphanumeric words.
fn normalize_words(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// Every alphanumeric character of `text`, lowercased, with everything else removed.
///
/// Expected words are matched against this rather than against a word set of our own, because the
/// two sides disagree about where words *begin*, not about whether the content is there. The spec's
/// `foo<em>bar</em>` and our `foobar` are the same six letters; so are its `a<br>b` and our two
/// lines. Matching fine-grained expected words against the coarse actual stream is blind to that
/// disagreement and still catches the thing we care about — a word that is simply gone.
fn alphanumeric_stream(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Write a line that survives `cargo test`'s output capture.
///
/// The harness captures the `print!`/`eprint!` macros, so a summary printed with them is invisible
/// unless someone remembers `-- --nocapture`. Writing to the stderr handle directly bypasses the
/// capture, which is what makes the conformance level visible every time the suite runs.
fn report(line: &str) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "{line}");
    let _ = err.flush();
}

/// Replace every soft break in a document with a space.
///
/// A soft break is line *wrapping*, not formatting, and HTML has no way to express "a newline that
/// renders as a space" — so the HTML path collapses it, deliberately, and `roundtrip.rs` pins that
/// behaviour in `soft_breaks_become_spaces_through_html`. Almost every multi-line paragraph in the
/// spec would otherwise show up as a round-trip divergence, burying the divergences that carry
/// real information. Collapsing the baseline the same way the HTML trip does is the same move
/// `roundtrip.rs` makes when it leaves `SoftBreak` out of its generator.
fn collapse_soft_breaks(blocks: Vec<Block>) -> Vec<Block> {
    fn inlines(nodes: Vec<Inline>) -> Vec<Inline> {
        nodes
            .into_iter()
            .map(|node| match node {
                Inline::SoftBreak => Inline::Text(" ".into()),
                other => match other.children() {
                    Some(children) => other.with_children(inlines(children.to_vec())),
                    None => other,
                },
            })
            .collect()
    }
    fn cells(row: Vec<Vec<Inline>>) -> Vec<Vec<Inline>> {
        row.into_iter().map(inlines).collect()
    }

    blocks
        .into_iter()
        .map(|block| match block {
            Block::Paragraph(c) => Block::Paragraph(inlines(c)),
            Block::Heading { level, content } => Block::Heading {
                level,
                content: inlines(content),
            },
            Block::Unsupported { kind, fallback } => Block::Unsupported {
                kind,
                fallback: inlines(fallback),
            },
            Block::BlockQuote(inner) => Block::BlockQuote(collapse_soft_breaks(inner)),
            Block::List(list) => Block::List(List {
                items: list
                    .items
                    .into_iter()
                    .map(|item| ListItem {
                        blocks: collapse_soft_breaks(item.blocks),
                    })
                    .collect(),
                ..list
            }),
            Block::Table(table) => Block::Table(Table {
                head: cells(table.head),
                rows: table.rows.into_iter().map(cells).collect(),
                ..table
            }),
            other @ (Block::CodeBlock { .. } | Block::ThematicBreak) => other,
        })
        .collect()
}

/// What an HTML round trip is allowed to produce: the settled Markdown with its soft breaks
/// already collapsed, which is the one documented loss of the HTML path.
fn html_baseline(settled: &str) -> String {
    let doc = mdcore::parse(settled, Format::Markdown).expect("parsing is infallible");
    let collapsed = Document::from_blocks(collapse_soft_breaks(doc.blocks));
    let rendered = mdcore::render(&collapsed, Format::Markdown, &RenderProfile::teams());
    settle(&rendered).unwrap_or(rendered)
}

// ---------------------------------------------------------------------------
// Invariant 1 — nothing panics
// ---------------------------------------------------------------------------

/// Parsing and rendering every spec example, in every direction, must not panic.
///
/// Each example runs inside `catch_unwind` so a panic reports *which* example caused it instead of
/// killing the run at the first one. Markdown is the input format under test, but the reference
/// HTML is fed through the HTML and text parsers too — it is free, realistic input, and the HTML
/// parser is the one that faces untrusted content in production.
#[test]
fn nothing_panics_on_any_spec_example() {
    let examples = corpus();
    let profile = RenderProfile::teams();
    let mut panicked = Vec::new();

    for example in &examples {
        let outcome = std::panic::catch_unwind(|| {
            for (input, from) in [
                (&example.markdown, Format::Markdown),
                (&example.html, Format::Html),
                (&example.markdown, Format::Text),
            ] {
                let doc = mdcore::parse(input, from).expect("parsing is infallible");
                for to in [Format::Markdown, Format::Html, Format::Text] {
                    let _ = mdcore::render(&doc, to, &profile);
                }
            }
            let _ = mdcore::outline(&example.markdown);
        });
        if outcome.is_err() {
            panicked.push(format!("{} ({})", example.id, example.section));
        }
    }

    assert!(
        panicked.is_empty(),
        "{} of {} spec examples panicked: {panicked:#?}",
        panicked.len(),
        examples.len()
    );
    report(&format!(
        "commonmark: {} spec examples parsed and rendered in every direction without panicking",
        examples.len()
    ));
}

// ---------------------------------------------------------------------------
// Invariant 2 — Markdown settles
// ---------------------------------------------------------------------------

/// Markdown converges under repeated conversion, and stays converged.
///
/// This is the property that keeps the editor panes from creeping as the user types: the UI
/// re-converts on every keystroke, so text that never reaches a fixed point would visibly churn.
/// There is no allowance list for this one — a spec example that never settles is a bug, not a
/// model limitation.
#[test]
fn every_spec_example_settles() {
    let examples = corpus();
    let mut unsettled = Vec::new();
    let mut settled_count = 0usize;

    for example in &examples {
        match settle(&example.markdown) {
            Ok(settled) => {
                // Settled means settled: one more pass must change nothing.
                let again = mdcore::convert_with(
                    &settled,
                    Format::Markdown,
                    Format::Markdown,
                    &RenderProfile::teams(),
                )
                .expect("markdown conversion is infallible");
                if again != settled {
                    unsettled.push(format!("{} drifted after settling", example.id));
                }
                settled_count += 1;
            }
            Err(last) => unsettled.push(format!(
                "{} ({}) never settled in {MAX_PASSES} passes; last text was:\n{last}",
                example.id, example.section
            )),
        }
    }

    assert!(unsettled.is_empty(), "{unsettled:#?}");
    report(&format!(
        "commonmark: {settled_count} of {} spec examples reach a Markdown fixed point",
        examples.len()
    ));
}

// ---------------------------------------------------------------------------
// Invariant 3 — settled Markdown survives HTML
// ---------------------------------------------------------------------------

/// The app's core promise, measured against real spec input: settled Markdown makes the trip to
/// Teams HTML and back unchanged.
///
/// Measured from *settled* Markdown because that is the only thing either editor pane ever shows;
/// neither displays a render of an AST that did not come from a parser. The single allowance is
/// the soft-break collapse `html_baseline` applies, which is documented and pinned elsewhere.
#[test]
fn settled_markdown_survives_a_trip_through_html() {
    let examples = corpus();
    let profile = RenderProfile::teams();
    let mut observed = BTreeMap::new();

    for example in &examples {
        let Ok(settled) = settle(&example.markdown) else {
            // Covered by `every_spec_example_settles`; nothing to add here.
            continue;
        };
        let html = mdcore::convert_with(&settled, Format::Markdown, Format::Html, &profile)
            .expect("conversion is infallible");
        let back = mdcore::convert_with(&html, Format::Html, Format::Markdown, &profile)
            .expect("conversion is infallible");
        if back != html_baseline(&settled) {
            observed.insert(
                example.id.clone(),
                format!("{} — md/html round trip is not stable", example.section),
            );
        }
    }

    report(&format!(
        "commonmark: {} of {} spec examples survive md -> html -> md unchanged ({} known divergences)",
        examples.len() - observed.len(),
        examples.len(),
        observed.len()
    ));
    let ids = examples.iter().map(|e| e.id.clone()).collect();
    assert_divergences_match(HTML_ROUNDTRIP, &observed, &ids);
}

// ---------------------------------------------------------------------------
// Invariant 4 — no content is silently lost
// ---------------------------------------------------------------------------

/// Every word the spec says an example makes visible reaches our plain-text rendering.
///
/// The expectation comes from the spec's own reference HTML with the markup stripped, so it is the
/// spec authors' view of what the example *says*, not of how it should be marked up. Formatting we
/// deliberately drop is invisible to this test; content we accidentally drop is not.
#[test]
fn no_content_is_silently_lost() {
    let examples = corpus();
    let profile = RenderProfile::teams();
    let mut observed = BTreeMap::new();

    for example in &examples {
        let expected = visible_words(&example.html);
        if expected.is_empty() {
            continue;
        }
        let doc = mdcore::parse(&example.markdown, Format::Markdown).expect("infallible");
        let actual = alphanumeric_stream(&mdcore::render(&doc, Format::Text, &profile));

        let missing: Vec<&String> = expected
            .iter()
            .filter(|word| !actual.contains(word.as_str()))
            .collect();
        if !missing.is_empty() {
            observed.insert(
                example.id.clone(),
                format!("{} — words dropped: {missing:?}", example.section),
            );
        }
    }

    report(&format!(
        "commonmark: {} of {} spec examples keep every visible word ({} known divergences)",
        examples.len() - observed.len(),
        examples.len(),
        observed.len()
    ));
    let ids = examples.iter().map(|e| e.id.clone()).collect();
    assert_divergences_match(CONTENT_LOSS, &observed, &ids);
}

// ---------------------------------------------------------------------------
// The corpus itself
// ---------------------------------------------------------------------------

/// The vendored corpus is the shape we think it is.
///
/// Cheap insurance: if a spec file is re-vendored and the fence format or section names shift, the
/// example ids in `known-divergences.txt` would silently point at different examples, and every
/// other test here would be quietly testing the wrong thing.
#[test]
fn corpus_is_intact() {
    let examples = corpus();

    let commonmark = examples.iter().filter(|e| e.id.starts_with("CM-")).count();
    let gfm = examples.iter().filter(|e| e.id.starts_with("GFM-")).count();
    assert_eq!(commonmark, 652, "CommonMark 0.31.2 has 652 examples");
    assert_eq!(
        gfm, 24,
        "GFM 0.29 has 24 examples in its extension sections"
    );

    let ids: BTreeSet<&String> = examples.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), examples.len(), "example ids are not unique");

    // Example 1 of both specs is the tabs example, and it must have kept its tab.
    let first = examples.first().expect("a non-empty corpus");
    assert_eq!(first.id, "CM-001");
    assert!(
        first.markdown.contains('\t'),
        "`→` was not decoded back into a tab: {:?}",
        first.markdown
    );
    assert!(
        !examples.iter().any(|e| e.markdown.contains('\u{2192}')),
        "an example still contains an undecoded `→`"
    );

    // The GFM examples must be the extension chapters and nothing else.
    let sections: BTreeSet<&str> = examples
        .iter()
        .filter(|e| e.id.starts_with("GFM-"))
        .map(|e| e.section.as_str())
        .collect();
    assert!(
        sections.iter().all(|s| s.ends_with("(extension)")),
        "unexpected GFM sections: {sections:?}"
    );

    report(&format!(
        "commonmark: corpus is {commonmark} CommonMark + {gfm} GFM-extension examples"
    ));
}
