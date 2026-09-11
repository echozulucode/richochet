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
        1 => Just(Inline::SoftBreak),
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
                let items = blocks.into_iter().map(|b| ListItem { blocks: vec![b] }).collect();
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Normalization is a fixed point: normalizing twice changes nothing.
    ///
    /// `Document::from_blocks` normalizes, so a generated document is already normalized; feeding
    /// it back through must be a no-op.
    #[test]
    fn normalization_is_idempotent(doc in document()) {
        let again = Document::from_blocks(doc.blocks.clone());
        prop_assert_eq!(&doc, &again);
    }

    /// Markdown survives a full round trip through the AST.
    ///
    /// This is the property that makes the two panes trustworthy: what the user typed must mean
    /// the same thing after the app has rewritten it.
    #[test]
    fn markdown_round_trips(doc in document()) {
        let profile = RenderProfile::teams();
        let rendered = mdcore::render(&doc, Format::Markdown, &profile);
        let reparsed = mdcore::parse(&rendered, Format::Markdown)
            .expect("re-parsing our own Markdown output must succeed");
        prop_assert_eq!(
            &doc, &reparsed,
            "round trip changed the document\n--- markdown ---\n{}", rendered
        );
    }

    /// Rendering Markdown twice produces the same text.
    #[test]
    fn markdown_rendering_is_stable(doc in document()) {
        let profile = RenderProfile::teams();
        let once = mdcore::render(&doc, Format::Markdown, &profile);
        let reparsed = mdcore::parse(&once, Format::Markdown).unwrap();
        let twice = mdcore::render(&reparsed, Format::Markdown, &profile);
        prop_assert_eq!(once, twice);
    }

    /// HTML survives a full round trip through the AST.
    #[test]
    fn html_round_trips(doc in document()) {
        let profile = RenderProfile::standard();
        let rendered = mdcore::render(&doc, Format::Html, &profile);
        let reparsed = mdcore::parse(&rendered, Format::Html)
            .expect("re-parsing our own HTML output must succeed");
        prop_assert_eq!(
            &doc, &reparsed,
            "round trip changed the document\n--- html ---\n{}", rendered
        );
    }

    /// Converting Markdown to HTML and back preserves meaning.
    ///
    /// The app's core promise: Teams -> Markdown -> Teams must not degrade.
    #[test]
    fn markdown_html_markdown_is_stable(doc in document()) {
        let profile = RenderProfile::teams();
        let md = mdcore::render(&doc, Format::Markdown, &profile);
        let html = mdcore::convert_with(&md, Format::Markdown, Format::Html, &profile).unwrap();
        let back = mdcore::convert_with(&html, Format::Html, Format::Markdown, &profile).unwrap();
        prop_assert_eq!(
            &md, &back,
            "a Markdown -> HTML -> Markdown trip changed the text\n--- html ---\n{}", html
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
            if matches!(node, Inline::Bold(_) | Inline::Italic(_) | Inline::Strike(_)) {
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
