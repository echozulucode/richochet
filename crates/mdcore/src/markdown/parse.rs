//! Markdown -> document model, via pulldown-cmark.
//!
//! The event stream is folded onto a stack of open containers. Anything the
//! [frozen AST](crate::document::model) cannot represent — tables, footnote definitions, raw HTML
//! blocks — becomes a [`Block::Unsupported`] carrying the plain text of what was dropped, so
//! content is never silently lost.
//!
//! Three deliberately lossy mappings, all of which converge on a second pass:
//!
//! * **Images become links.** The model has no image node, and an image sitting inside a paragraph
//!   cannot be a block-level `Unsupported`, so `![alt](url)` becomes `[alt](url)`. That keeps both
//!   the alt text and the URL, and renders back to something stable.
//! * **Inline HTML becomes literal text.** `<br>` inside a paragraph arrives as the text `<br>`,
//!   which the renderer escapes to `\<br\>`; re-parsing yields the same text rather than
//!   accumulating markup.
//! * **Footnote references become literal text.** `[^1]` becomes the text `[^1]`.
//!
//! Tables and footnotes are *enabled* on purpose. Leaving them off would not make them go away; it
//! would shred a table into paragraphs of pipes. Enabled, they arrive as recognizable containers
//! that can be captured wholesale as `Unsupported`.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::document::model::{Block, Document, Inline, List, ListItem};

/// Parse CommonMark (with GFM strikethrough) into the document model.
///
/// The result is always normalized — this returns [`Document::from_blocks`], so every invariant
/// in [`crate::document::normalize`] holds of the output.
///
/// ```
/// use mdcore::{Block, Inline};
///
/// let doc = mdcore::markdown::parse("**bold**");
/// assert_eq!(doc.blocks, vec![Block::Paragraph(vec![Inline::bold("bold")])]);
/// ```
pub fn parse(input: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);

    let mut builder = Builder::new();
    for event in Parser::new_ext(input, options) {
        builder.event(event);
    }
    Document::from_blocks(builder.finish())
}

/// What kind of container a [`Frame`] is.
enum Open {
    /// The document itself; always the bottom of the stack.
    Root,
    Paragraph,
    Heading(u8),
    BlockQuote,
    Item,
    List {
        ordered: bool,
        start: u64,
        /// Starts optimistic: any `Start(Paragraph)` directly inside an item clears it, which is
        /// exactly how pulldown-cmark signals a loose list.
        tight: bool,
        items: Vec<ListItem>,
    },
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    /// A construct the model cannot represent; swallows its whole subtree as plain text.
    Unsupported {
        kind: &'static str,
        text: String,
        /// Nesting depth of unclosed tags, so we know which `End` closes the collector.
        depth: usize,
    },
    Bold,
    Italic,
    Strike,
    Link {
        href: String,
        title: Option<String>,
    },
    /// Collects an image's alt text before it is turned into a link.
    Image {
        href: String,
        title: Option<String>,
    },
}

/// One open container, plus the children gathered into it so far.
struct Frame {
    open: Open,
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
}

impl Frame {
    fn new(open: Open) -> Self {
        Frame {
            open,
            blocks: Vec::new(),
            inlines: Vec::new(),
        }
    }
}

/// Folds the event stream onto a stack of open containers.
struct Builder {
    stack: Vec<Frame>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            stack: vec![Frame::new(Open::Root)],
        }
    }

    /// The innermost open container. The root frame is never popped, so this cannot fail.
    fn top(&mut self) -> &mut Frame {
        self.stack
            .last_mut()
            .expect("the root frame is never popped")
    }

    /// Turn inlines that arrived directly in a block container into a paragraph.
    ///
    /// Tight list items emit their text with no `Paragraph` tag around it, so a frame that holds
    /// blocks can still accumulate loose inlines. They become a paragraph the moment a real block
    /// arrives or the container closes.
    fn flush_pending(&mut self) {
        let frame = self.top();
        if frame.inlines.is_empty() {
            return;
        }
        if matches!(frame.open, Open::Root | Open::BlockQuote | Open::Item) {
            let inlines = std::mem::take(&mut frame.inlines);
            frame.blocks.push(Block::Paragraph(inlines));
        }
    }

    fn add_block(&mut self, block: Block) {
        self.flush_pending();
        self.top().blocks.push(block);
    }

    fn push_inline(&mut self, inline: Inline) {
        self.top().inlines.push(inline);
    }

    fn push_frame(&mut self, open: Open) {
        self.stack.push(Frame::new(open));
    }

    /// Close the innermost frame, flushing any inlines it was still holding.
    fn pop_frame(&mut self) -> Frame {
        self.flush_pending();
        self.stack
            .pop()
            .expect("pulldown-cmark balances start and end events")
    }

    /// pulldown-cmark signals a loose list by wrapping item content in a paragraph.
    fn mark_enclosing_list_loose(&mut self) {
        if !matches!(self.stack.last().map(|f| &f.open), Some(Open::Item)) {
            return;
        }
        let Some(idx) = self.stack.len().checked_sub(2) else {
            return;
        };
        if let Open::List { tight, .. } = &mut self.stack[idx].open {
            *tight = false;
        }
    }

    fn in_unsupported(&self) -> bool {
        matches!(
            self.stack.last().map(|f| &f.open),
            Some(Open::Unsupported { .. })
        )
    }

    fn event(&mut self, event: Event<'_>) {
        if self.in_unsupported() {
            self.collect_unsupported(event);
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Open::CodeBlock { code, .. } = &mut self.top().open {
                    code.push_str(&t);
                } else {
                    self.push_inline(Inline::Text(t.into_string()));
                }
            }
            Event::Code(c) => self.push_inline(Inline::Code(c.into_string())),
            // Math is never enabled; if it somehow appears, keep the source text.
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.push_inline(Inline::Text(t.into_string()));
            }
            // A bare `Html` event outside an `HtmlBlock` is raw inline markup.
            Event::Html(h) | Event::InlineHtml(h) => {
                self.push_inline(Inline::Text(h.into_string()));
            }
            Event::FootnoteReference(label) => {
                self.push_inline(Inline::Text(format!("[^{label}]")));
            }
            Event::SoftBreak => self.push_inline(Inline::SoftBreak),
            Event::HardBreak => self.push_inline(Inline::HardBreak),
            Event::Rule => self.add_block(Block::ThematicBreak),
            Event::TaskListMarker(done) => {
                let marker = if done { "[x] " } else { "[ ] " };
                self.push_inline(Inline::Text(marker.to_string()));
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.mark_enclosing_list_loose();
                self.flush_pending();
                self.push_frame(Open::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush_pending();
                self.push_frame(Open::Heading(level as u8));
            }
            Tag::BlockQuote(_) => {
                self.flush_pending();
                self.push_frame(Open::BlockQuote);
            }
            Tag::CodeBlock(kind) => {
                self.flush_pending();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        // The info string's first word is the language; the rest is metadata the
                        // model has nowhere to put.
                        let first = info.split_whitespace().next().unwrap_or("").to_string();
                        (!first.is_empty()).then_some(first)
                    }
                    CodeBlockKind::Indented => None,
                };
                self.push_frame(Open::CodeBlock {
                    lang,
                    code: String::new(),
                });
            }
            Tag::List(start) => {
                self.flush_pending();
                self.push_frame(Open::List {
                    ordered: start.is_some(),
                    start: start.unwrap_or(1),
                    tight: true,
                    items: Vec::new(),
                });
            }
            Tag::Item => self.push_frame(Open::Item),
            Tag::Emphasis => self.push_frame(Open::Italic),
            Tag::Strong => self.push_frame(Open::Bold),
            Tag::Strikethrough => self.push_frame(Open::Strike),
            Tag::Link {
                dest_url, title, ..
            } => self.push_frame(Open::Link {
                href: dest_url.into_string(),
                title: (!title.is_empty()).then(|| title.into_string()),
            }),
            Tag::Image {
                dest_url, title, ..
            } => self.push_frame(Open::Image {
                href: dest_url.into_string(),
                title: (!title.is_empty()).then(|| title.into_string()),
            }),
            Tag::Table(_) => self.open_unsupported("table"),
            Tag::FootnoteDefinition(label) => {
                self.flush_pending();
                self.push_frame(Open::Unsupported {
                    kind: "footnote",
                    text: format!("[^{label}]: "),
                    depth: 1,
                });
            }
            Tag::HtmlBlock => self.open_unsupported("html"),
            // Definition lists, metadata blocks, sub/superscript and stray table parts are all
            // behind options we do not enable; capture rather than assume they cannot happen.
            _ => self.open_unsupported("unsupported"),
        }
    }

    fn open_unsupported(&mut self, kind: &'static str) {
        self.flush_pending();
        self.push_frame(Open::Unsupported {
            kind,
            text: String::new(),
            depth: 1,
        });
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let frame = self.pop_frame();
                self.add_block(Block::Paragraph(frame.inlines));
            }
            TagEnd::Heading(_) => {
                let frame = self.pop_frame();
                let level = match frame.open {
                    Open::Heading(l) => l,
                    _ => 1,
                };
                self.add_block(Block::heading(level, frame.inlines));
            }
            TagEnd::BlockQuote(_) => {
                let frame = self.pop_frame();
                self.add_block(Block::BlockQuote(frame.blocks));
            }
            TagEnd::CodeBlock => {
                let frame = self.pop_frame();
                if let Open::CodeBlock { lang, code } = frame.open {
                    // pulldown-cmark includes the final newline; the model does not.
                    let code = code.strip_suffix('\n').unwrap_or(&code).to_string();
                    self.add_block(Block::CodeBlock { lang, code });
                }
            }
            TagEnd::List(_) => {
                let frame = self.pop_frame();
                if let Open::List {
                    ordered,
                    start,
                    tight,
                    items,
                } = frame.open
                {
                    self.add_block(Block::List(List {
                        ordered,
                        start,
                        tight,
                        items,
                    }));
                }
            }
            TagEnd::Item => {
                let frame = self.pop_frame();
                let item = ListItem {
                    blocks: frame.blocks,
                };
                if let Some(Frame {
                    open: Open::List { items, .. },
                    ..
                }) = self.stack.last_mut()
                {
                    items.push(item);
                }
            }
            TagEnd::Emphasis => {
                let frame = self.pop_frame();
                self.push_inline(Inline::Italic(frame.inlines));
            }
            TagEnd::Strong => {
                let frame = self.pop_frame();
                self.push_inline(Inline::Bold(frame.inlines));
            }
            TagEnd::Strikethrough => {
                let frame = self.pop_frame();
                self.push_inline(Inline::Strike(frame.inlines));
            }
            TagEnd::Link => {
                let frame = self.pop_frame();
                if let Open::Link { href, title } = frame.open {
                    self.push_inline(Inline::Link {
                        href,
                        title,
                        content: frame.inlines,
                    });
                }
            }
            TagEnd::Image => {
                let frame = self.pop_frame();
                if let Open::Image { href, title } = frame.open {
                    // An image with no alt text would render as an invisible link, so fall back to
                    // showing the URL.
                    let content = if frame.inlines.is_empty() {
                        vec![Inline::Text(href.clone())]
                    } else {
                        frame.inlines
                    };
                    self.push_inline(Inline::Link {
                        href,
                        title,
                        content,
                    });
                }
            }
            _ => {
                // Every remaining `TagEnd` belongs to a construct opened as `Unsupported`, whose
                // events never reach this method.
            }
        }
    }

    /// Swallow one event into the open [`Open::Unsupported`] collector.
    fn collect_unsupported(&mut self, event: Event<'_>) {
        let finished = {
            let Some(Frame {
                open: Open::Unsupported { text, depth, .. },
                ..
            }) = self.stack.last_mut()
            else {
                return;
            };
            match event {
                Event::Start(_) => {
                    *depth += 1;
                    false
                }
                Event::End(TagEnd::TableCell) => {
                    text.push_str(" | ");
                    *depth -= 1;
                    *depth == 0
                }
                Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                    while text.ends_with(' ') || text.ends_with('|') {
                        text.pop();
                    }
                    text.push('\n');
                    *depth -= 1;
                    *depth == 0
                }
                Event::End(_) => {
                    *depth -= 1;
                    *depth == 0
                }
                Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                    text.push_str(&t);
                    false
                }
                Event::InlineMath(t) | Event::DisplayMath(t) => {
                    text.push_str(&t);
                    false
                }
                Event::FootnoteReference(label) => {
                    text.push_str(&format!("[^{label}]"));
                    false
                }
                Event::SoftBreak => {
                    text.push(' ');
                    false
                }
                Event::HardBreak | Event::Rule => {
                    text.push('\n');
                    false
                }
                Event::TaskListMarker(_) => false,
            }
        };

        if finished {
            let frame = self
                .stack
                .pop()
                .expect("the collector frame was just inspected");
            if let Open::Unsupported { kind, text, .. } = frame.open {
                let text = text.trim().to_string();
                self.add_block(Block::Unsupported {
                    kind: kind.to_string(),
                    fallback: vec![Inline::Text(text)],
                });
            }
        }
    }

    /// Close any frames the input left open and return the document's blocks.
    fn finish(mut self) -> Vec<Block> {
        while self.stack.len() > 1 {
            let frame = self.pop_frame();
            // Salvage whatever the unbalanced frame held rather than dropping it.
            let mut salvaged = frame.blocks;
            if !frame.inlines.is_empty() {
                salvaged.push(Block::Paragraph(frame.inlines));
            }
            self.top().blocks.extend(salvaged);
        }
        self.flush_pending();
        self.stack
            .pop()
            .expect("the root frame is never popped")
            .blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(input: &str) -> Vec<Block> {
        parse(input).blocks
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn empty_input_is_an_empty_document() {
        assert!(parse("").is_empty());
        assert_eq!(blocks("   \n\n  "), vec![]);
    }

    #[test]
    fn plain_paragraph() {
        assert_eq!(blocks("hello world"), vec![Block::para("hello world")]);
    }

    #[test]
    fn two_paragraphs() {
        assert_eq!(
            blocks("one\n\ntwo"),
            vec![Block::para("one"), Block::para("two")]
        );
    }

    #[test]
    fn headings_of_every_level_clamp_to_three() {
        assert_eq!(
            blocks("# a\n\n## b\n\n### c\n\n#### d\n\n###### f"),
            vec![
                Block::heading(1, vec![text("a")]),
                Block::heading(2, vec![text("b")]),
                Block::heading(3, vec![text("c")]),
                Block::heading(3, vec![text("d")]),
                Block::heading(3, vec![text("f")]),
            ]
        );
    }

    #[test]
    fn setext_headings_work_too() {
        assert_eq!(
            blocks("Title\n====="),
            vec![Block::heading(1, vec![text("Title")])]
        );
    }

    #[test]
    fn emphasis_marks() {
        assert_eq!(
            blocks("**b** _i_ ~~s~~"),
            vec![Block::Paragraph(vec![
                Inline::bold("b"),
                text(" "),
                Inline::italic("i"),
                text(" "),
                Inline::Strike(vec![text("s")]),
            ])]
        );
    }

    #[test]
    fn strikethrough_is_enabled() {
        assert_eq!(
            blocks("~~gone~~"),
            vec![Block::Paragraph(vec![Inline::Strike(vec![text("gone")])])]
        );
    }

    #[test]
    fn nested_marks_are_normalized_to_canonical_order() {
        // `***x***` parses as em(strong(x)); the normalizer flips it to Bold outside Italic.
        assert_eq!(
            blocks("***x***"),
            vec![Block::Paragraph(vec![Inline::Bold(vec![Inline::Italic(
                vec![text("x")]
            )])])]
        );
    }

    #[test]
    fn inline_code_is_verbatim() {
        assert_eq!(
            blocks("a `x*y_z` b"),
            vec![Block::Paragraph(vec![
                text("a "),
                Inline::Code("x*y_z".into()),
                text(" b"),
            ])]
        );
    }

    #[test]
    fn inline_code_containing_backticks() {
        assert_eq!(
            blocks("`` a`b ``"),
            vec![Block::Paragraph(vec![Inline::Code("a`b".into())])]
        );
    }

    #[test]
    fn links_keep_href_and_title() {
        assert_eq!(
            blocks(r#"[t](http://e.com "the title")"#),
            vec![Block::Paragraph(vec![Inline::Link {
                href: "http://e.com".into(),
                title: Some("the title".into()),
                content: vec![text("t")],
            }])]
        );
    }

    #[test]
    fn link_without_a_title_has_none() {
        assert_eq!(
            blocks("[t](u)"),
            vec![Block::Paragraph(vec![Inline::link("u", "t")])]
        );
    }

    #[test]
    fn autolinks_become_links() {
        assert_eq!(
            blocks("<http://e.com>"),
            vec![Block::Paragraph(vec![Inline::link(
                "http://e.com",
                "http://e.com"
            )])]
        );
    }

    #[test]
    fn images_degrade_to_links_rather_than_vanishing() {
        assert_eq!(
            blocks("![alt](pic.png)"),
            vec![Block::Paragraph(vec![Inline::link("pic.png", "alt")])]
        );
    }

    #[test]
    fn an_image_without_alt_text_shows_its_url() {
        assert_eq!(
            blocks("![](pic.png)"),
            vec![Block::Paragraph(vec![Inline::link("pic.png", "pic.png")])]
        );
    }

    #[test]
    fn soft_and_hard_breaks() {
        assert_eq!(
            blocks("a\nb"),
            vec![Block::Paragraph(vec![
                text("a"),
                Inline::SoftBreak,
                text("b")
            ])]
        );
        assert_eq!(
            blocks("a\\\nb"),
            vec![Block::Paragraph(vec![
                text("a"),
                Inline::HardBreak,
                text("b")
            ])]
        );
        assert_eq!(
            blocks("a  \nb"),
            vec![Block::Paragraph(vec![
                text("a"),
                Inline::HardBreak,
                text("b")
            ])]
        );
    }

    #[test]
    fn thematic_break() {
        assert_eq!(
            blocks("a\n\n---\n\nb"),
            vec![Block::para("a"), Block::ThematicBreak, Block::para("b"),]
        );
    }

    #[test]
    fn fenced_code_block_with_a_language() {
        assert_eq!(
            blocks("```rust\nfn main() {}\n```"),
            vec![Block::CodeBlock {
                lang: Some("rust".into()),
                code: "fn main() {}".into(),
            }]
        );
    }

    #[test]
    fn fenced_code_block_without_a_language() {
        assert_eq!(
            blocks("```\nplain\n```"),
            vec![Block::CodeBlock {
                lang: None,
                code: "plain".into(),
            }]
        );
    }

    #[test]
    fn indented_code_block() {
        assert_eq!(
            blocks("    indented\n    two"),
            vec![Block::CodeBlock {
                lang: None,
                code: "indented\ntwo".into(),
            }]
        );
    }

    #[test]
    fn code_block_keeps_interior_blank_lines() {
        assert_eq!(
            blocks("```\na\n\nb\n```"),
            vec![Block::CodeBlock {
                lang: None,
                code: "a\n\nb".into(),
            }]
        );
    }

    #[test]
    fn bullet_list_is_tight() {
        assert_eq!(
            blocks("- a\n- b"),
            vec![Block::List(List::bulleted(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
            ]))]
        );
    }

    #[test]
    fn blank_lines_between_items_make_a_loose_list() {
        let Block::List(list) = &blocks("- a\n\n- b")[0] else {
            panic!("expected a list");
        };
        assert!(!list.tight);
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn ordered_list_respects_its_start() {
        let Block::List(list) = &blocks("5. a\n6. b")[0] else {
            panic!("expected a list");
        };
        assert!(list.ordered);
        assert_eq!(list.start, 5);
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn nested_lists_two_deep() {
        let expected = Block::List(List::bulleted(vec![ListItem {
            blocks: vec![
                Block::Paragraph(vec![text("a")]),
                Block::List(List::bulleted(vec![ListItem::of(vec![text("b")])])),
            ],
        }]));
        assert_eq!(blocks("- a\n  - b"), vec![expected]);
    }

    #[test]
    fn blockquote_contains_blocks() {
        assert_eq!(
            blocks("> quoted\n>\n> # h"),
            vec![Block::BlockQuote(vec![
                Block::para("quoted"),
                Block::heading(1, vec![text("h")]),
            ])]
        );
    }

    #[test]
    fn nested_blockquotes() {
        assert_eq!(
            blocks("> > deep"),
            vec![Block::BlockQuote(vec![Block::BlockQuote(vec![
                Block::para("deep")
            ])])]
        );
    }

    #[test]
    fn a_table_becomes_unsupported_with_its_text_as_fallback() {
        let got = blocks("| a | b |\n| - | - |\n| 1 | 2 |");
        let [Block::Unsupported { kind, fallback }] = &got[..] else {
            panic!("expected one unsupported block, got {got:?}");
        };
        assert_eq!(kind, "table");
        // The cells survive; the layout does not.
        let flat = format!("{fallback:?}");
        for cell in ["a", "b", "1", "2"] {
            assert!(flat.contains(cell), "cell {cell} missing from {flat}");
        }
    }

    #[test]
    fn an_html_block_becomes_unsupported() {
        let got = blocks("<div>\nhi\n</div>");
        let [Block::Unsupported { kind, fallback }] = &got[..] else {
            panic!("expected one unsupported block, got {got:?}");
        };
        assert_eq!(kind, "html");
        assert!(format!("{fallback:?}").contains("hi"));
    }

    #[test]
    fn a_footnote_definition_becomes_unsupported_and_keeps_its_label() {
        let got = blocks("text[^1]\n\n[^1]: the note");
        let unsupported: Vec<_> = got
            .iter()
            .filter_map(|b| match b {
                Block::Unsupported { kind, fallback } => {
                    Some((kind.clone(), format!("{fallback:?}")))
                }
                _ => None,
            })
            .collect();
        assert_eq!(unsupported.len(), 1, "got {got:?}");
        assert_eq!(unsupported[0].0, "footnote");
        assert!(unsupported[0].1.contains("the note"));
        assert!(unsupported[0].1.contains("[^1]"));
    }

    #[test]
    fn a_footnote_reference_survives_as_text() {
        let got = blocks("see[^a]");
        assert_eq!(got, vec![Block::para("see[^a]")]);
    }

    #[test]
    fn inline_html_survives_as_literal_text() {
        assert_eq!(blocks("a <b>c"), vec![Block::para("a <b>c")]);
    }

    #[test]
    fn backslash_escapes_are_resolved_not_preserved() {
        assert_eq!(
            blocks(r"\# not a heading"),
            vec![Block::para("# not a heading")]
        );
        assert_eq!(blocks(r"snake\_case"), vec![Block::para("snake_case")]);
    }

    #[test]
    fn output_is_normalized_whitespace_is_hoisted_out_of_marks() {
        // `** x **` is not emphasis at all, but `**x ** y` style input from other producers is;
        // whatever survives must obey the invariant that marks never wrap leading whitespace.
        let doc = parse("a **b** c");
        for block in &doc.blocks {
            if let Block::Paragraph(inlines) = block {
                for inline in inlines {
                    if let Inline::Bold(children) = inline {
                        let Some(Inline::Text(t)) = children.first() else {
                            continue;
                        };
                        assert!(!t.starts_with(' '));
                    }
                }
            }
        }
    }

    #[test]
    fn a_mixed_document_parses_end_to_end() {
        let src = "\
# Title

Some **bold** and a [link](http://e.com).

- one
- two
  - nested

> quoted

```rust
let x = 1;
```

---
";
        let got = blocks(src);
        assert_eq!(got.len(), 6, "got {got:?}");
        assert!(matches!(got[0], Block::Heading { level: 1, .. }));
        assert!(matches!(got[1], Block::Paragraph(_)));
        assert!(matches!(got[2], Block::List(_)));
        assert!(matches!(got[3], Block::BlockQuote(_)));
        assert!(matches!(got[4], Block::CodeBlock { .. }));
        assert!(matches!(got[5], Block::ThematicBreak));
    }
}
