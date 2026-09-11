//! Property tests for the invariants that fixtures cannot cover exhaustively.
//!
//! Fixtures pin known cases; these pin the *rules*. Between them they catch the class of bug where
//! an escaping or nesting change looks right on every example in the corpus and is still wrong.

use mdcore::{Block, Document, Format, Inline, List, ListItem, RenderProfile};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Text that exercises the escaping rules without generating unrepresentable content.
///
/// Deliberately includes Markdown metacharacters, intraword underscores, HTML metacharacters and
/// non-ASCII — the four things that break naive implementations.
fn text() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "plain",
        "two words",
        "update_user_profile",
        "a*b",
        "a_b_c",
        "100% sure",
        "<not a tag>",
        "a & b",
        "[bracket]",
        "back\\slash",
        "tilde~thing",
        "hash # mid",
        "dot. after",
        "émoji 🎯 here",
        "trailing space ",
        " leading space",
    ])
    .prop_map(String::from)
}

fn inline() -> impl Strategy<Value = Inline> {
    let leaf = prop_oneof![
        4 => text().prop_map(Inline::Text),
        1 => prop::sample::select(vec!["code", "a b", "x`y"]).prop_map(|s| Inline::Code(s.into())),
        // `HardBreak` is representable everywhere (`<br>`, a trailing backslash). `SoftBreak` is
        // deliberately absent: it is line *wrapping*, not formatting, and HTML has no way to
        // express "a newline that renders as a space". Collapsing it to a space is the right
        // product choice — Teams sends pretty-printed HTML, and honouring its newlines would wrap
        // the user's Markdown at arbitrary points. The loss is pinned by
        // `soft_breaks_become_spaces_through_html` below rather than smuggled into a property.
        1 => Just(Inline::HardBreak),
    ];

    leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..3).prop_map(Inline::Bold),
            prop::collection::vec(inner.clone(), 1..3).prop_map(Inline::Italic),
            prop::collection::vec(inner.clone(), 1..3).prop_map(Inline::Strike),
            prop::collection::vec(inner, 1..3).prop_map(|content| Inline::Link {
                href: "https://example.com/p".into(),
                title: None,
                content,
            }),
        ]
    })
}

fn inlines() -> impl Strategy<Value = Vec<Inline>> {
    prop::collection::vec(inline(), 1..4)
}

fn block() -> impl Strategy<Value = Block> {
    let leaf = prop_oneof![
        3 => inlines().prop_map(Block::Paragraph),
        1 => (1u8..=3, inlines()).prop_map(|(level, content)| Block::Heading { level, content }),
        1 => prop::sample::select(vec![
                ("fn main() {}", Some("rust")),
                ("plain\nlines", None),
                ("has ` backtick", None),
            ])
            .prop_map(|(code, lang)| Block::CodeBlock {
                code: code.into(),
                lang: lang.map(String::from),
            }),
        1 => Just(Block::ThematicBreak),
    ];

    leaf.prop_recursive(2, 8, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..3).prop_map(Block::BlockQuote),
            (any::<bool>(), prop::collection::vec(inner, 1..3)).prop_map(|(ordered, blocks)| {
                let items = blocks
                    .into_iter()
                    .map(|b| ListItem { blocks: vec![b] })
                    .collect();
                Block::List(List {
                    ordered,
                    start: 1,
                    tight: true,
                    items,
                })
            }),
        ]
    })
}

fn document() -> impl Strategy<Value = Document> {
    prop::collection::vec(block(), 1..4).prop_map(Document::from_blocks)
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// The most passes a document may take to stop changing.
///
/// One is not always enough. Factoring a mark shared by adjacent siblings can produce a shape
/// CommonMark cannot express — `Bold([Link, Text])` sitting right after a word, whose `**` would
/// not be left-flanking — and the renderer then drops that mark on the *next* pass. What matters
/// is that the document settles and stays settled, not that it settles instantly.
const MAX_PASSES: usize = 4;

/// Convert a format to itself until the text stops changing, returning the settled text and the
/// number of passes it took. Panics if it never settles, which is the failure worth catching: a
/// document that oscillates forever would visibly churn under the user's cursor.
fn settle(input: &str, fmt: Format, profile: &RenderProfile) -> (String, usize) {
    let mut current = mdcore::convert_with(input, fmt, fmt, profile).expect("converting");
    for pass in 1..=MAX_PASSES {
        let next = mdcore::convert_with(&current, fmt, fmt, profile).expect("converting");
        if next == current {
            return (current, pass);
        }
        current = next;
    }
    panic!(
        "{fmt:?} never settled within {MAX_PASSES} passes; last text was:
{current}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        // The corpus lives in tests/, where proptest cannot find a crate root to write its
        // regression file next to.
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Normalization is a fixed point: normalizing twice changes nothing.
    ///
    /// `Document::from_blocks` normalizes, so a generated document is already normalized; feeding
    /// it back through must be a no-op.
    #[test]
    fn normalization_is_idempotent(doc in document()) {
        let again = Document::from_blocks(doc.blocks.clone());
        prop_assert_eq!(&doc, &again);
    }

    /// Markdown settles, and stays settled.
    ///
    /// Deliberately *not* `parse(render(doc)) == doc`. Markdown cannot express every document the
    /// model can hold — most notably emphasis that begins or ends with punctuation directly
    /// against a word character, which CommonMark's flanking rules make undeliverable with any
    /// delimiter. Demanding first-pass losslessness would be demanding something the format cannot
    /// give. Convergence is what actually protects the user: a rare, *stable* simplification is
    /// fine; a document that keeps shifting as they type is not.
    #[test]
    fn markdown_settles(doc in document()) {
        let profile = RenderProfile::teams();
        let (text, _) = settle(&mdcore::render(&doc, Format::Markdown, &profile), Format::Markdown, &profile);

        // Settled means settled: one more pass changes nothing.
        let again = mdcore::convert_with(&text, Format::Markdown, Format::Markdown, &profile).unwrap();
        prop_assert_eq!(&text, &again);
    }

    /// HTML settles, and stays settled.
    ///
    /// HTML's own lossy edge is `SoftBreak`: there is no way to write "a newline that renders as a
    /// space", so it converges to a space.
    #[test]
    fn html_settles(doc in document()) {
        let profile = RenderProfile::standard();
        let (text, _) = settle(&mdcore::render(&doc, Format::Html, &profile), Format::Html, &profile);

        let again = mdcore::convert_with(&text, Format::Html, Format::Html, &profile).unwrap();
        prop_assert_eq!(&text, &again);
    }

    /// The app's core promise: a Markdown -> Teams -> Markdown trip does not degrade.
    ///
    /// Measured from settled Markdown, because that is the only thing either editor pane ever
    /// displays — neither ever shows a raw render of an AST that did not come from a parser.
    #[test]
    fn markdown_survives_a_trip_through_teams_html(doc in document()) {
        let profile = RenderProfile::teams();
        let (md, _) = settle(&mdcore::render(&doc, Format::Markdown, &profile), Format::Markdown, &profile);

        let html = mdcore::convert_with(&md, Format::Markdown, Format::Html, &profile).unwrap();
        let back = mdcore::convert_with(&html, Format::Html, Format::Markdown, &profile).unwrap();
        prop_assert_eq!(
            &md, &back,
            "a Markdown -> HTML -> Markdown trip changed settled text
--- html ---
{}", html
        );
    }

    /// Plain-text rendering never panics and never invents content.
    #[test]
    fn text_rendering_is_total(doc in document()) {
        let profile = RenderProfile::teams();
        let _ = mdcore::render(&doc, Format::Text, &profile);
    }

    /// The parsers never panic on arbitrary input.
    ///
    /// Clipboard content arrives from another process and is not required to be well formed.
    #[test]
    fn parsers_never_panic(s in ".{0,200}") {
        let _ = mdcore::parse(&s, Format::Markdown);
        let _ = mdcore::parse(&s, Format::Html);
        let _ = mdcore::parse(&s, Format::Text);
    }
}

/// Whitespace must never end up immediately inside an emphasis mark.
///
/// `**  x  **` is not bold in any Markdown parser, so a mark whose first or last child begins or
/// ends with whitespace is a rendering bug waiting to happen. This walks the tree after
/// normalization and asserts it cannot occur.
#[test]
fn no_mark_ever_starts_or_ends_with_whitespace() {
    fn check(inlines: &[Inline]) {
        for node in inlines {
            if matches!(
                node,
                Inline::Bold(_) | Inline::Italic(_) | Inline::Strike(_)
            ) {
                if let Some(children) = node.children() {
                    if let Some(Inline::Text(t)) = children.first() {
                        assert!(
                            !t.starts_with(char::is_whitespace),
                            "mark starts with whitespace: {node:?}"
                        );
                    }
                    if let Some(Inline::Text(t)) = children.last() {
                        assert!(
                            !t.ends_with(char::is_whitespace),
                            "mark ends with whitespace: {node:?}"
                        );
                    }
                }
            }
            if let Some(children) = node.children() {
                check(children);
            }
        }
    }

    proptest!(|(doc in document())| {
        mdcore::document::visit::walk_inlines(&doc.blocks, &mut |node| {
            if let Some(children) = node.children() {
                check(children);
            }
            check(std::slice::from_ref(node));
        });
    });
}

/// A soft break does not survive a trip through HTML, and becomes a space.
///
/// This is the one documented lossy edge of the HTML path, excluded from
/// `markdown_survives_a_trip_through_teams_html` and asserted here instead so that it is a stated
/// behaviour rather than an untested gap. If this test starts failing, the HTML parser has begun
/// honouring source newlines — check what that does to real Teams HTML before accepting it.
#[test]
fn soft_breaks_become_spaces_through_html() {
    let profile = RenderProfile::teams();
    let md = "one
two";

    let html = mdcore::convert_with(md, Format::Markdown, Format::Html, &profile).unwrap();
    let back = mdcore::convert_with(&html, Format::Html, Format::Markdown, &profile).unwrap();

    assert_eq!(
        back, "one two",
        "a soft break should collapse to a space, not vanish"
    );

    // A hard break, by contrast, is representable and must survive intact.
    let src = "one\\\ntwo"; // a trailing backslash, then a newline
    let hard = mdcore::convert_with(src, Format::Markdown, Format::Html, &profile).unwrap();
    assert!(
        hard.contains("<br>"),
        "hard break should render as <br>, got: {hard}"
    );
    let hard_back = mdcore::convert_with(&hard, Format::Html, Format::Markdown, &profile).unwrap();
    assert_eq!(
        hard_back, src,
        "a hard break must survive an HTML round trip"
    );
}
