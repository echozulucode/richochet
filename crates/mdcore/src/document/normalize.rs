//! Canonicalizing passes over the document model.
//!
//! Every parser runs its output through [`normalize`] so that all downstream renderers can assume
//! the invariants hold. The passes are applied repeatedly until the document stops changing,
//! because one pass can expose work for another: hoisting whitespace out of a mark can leave an
//! empty mark, which removal then collects.
//!
//! The invariants, from `docs/implementation-plan.md` §2.1:
//!
//! 1. No empty text, and no mark wrapping an empty run.
//! 2. Adjacent identical marks are merged.
//! 3. Marks are not redundantly nested.
//! 4. Whitespace is hoisted out of marks.
//! 5. Mark nesting order is canonical.
//! 6. Heading levels are clamped to `1..=3`.

use crate::document::model::{Block, Document, Inline, List, ListItem, HEADING_MAX};

/// How many times to re-run the passes before giving up on reaching a fixed point.
///
/// The passes are contracting, so two or three rounds is typical; the cap only exists so a bug
/// cannot hang the UI thread.
const MAX_ROUNDS: usize = 8;

/// Apply every canonicalizing pass, to a fixed point.
pub fn normalize(doc: Document) -> Document {
    let mut blocks = doc.blocks;
    for _ in 0..MAX_ROUNDS {
        let next = normalize_blocks(blocks.clone());
        if next == blocks {
            break;
        }
        blocks = next;
    }
    Document { blocks }
}

fn normalize_blocks(blocks: Vec<Block>) -> Vec<Block> {
    blocks
        .into_iter()
        .map(normalize_block)
        // Drop blocks that would render to nothing. A thematic break renders even though it has
        // no content, so it is never dropped.
        .filter(|b| matches!(b, Block::ThematicBreak) || !b.is_empty())
        .collect()
}

fn normalize_block(block: Block) -> Block {
    match block {
        Block::Paragraph(c) => Block::Paragraph(normalize_inlines(c)),
        Block::Heading { level, content } => Block::Heading {
            level: level.clamp(1, HEADING_MAX),
            content: normalize_inlines(content),
        },
        Block::List(l) => Block::List(List {
            items: l
                .items
                .into_iter()
                .map(|i| ListItem {
                    blocks: normalize_blocks(i.blocks),
                })
                .filter(|i| !i.blocks.is_empty())
                .collect(),
            ..l
        }),
        Block::BlockQuote(b) => Block::BlockQuote(normalize_blocks(b)),
        Block::CodeBlock { lang, code } => Block::CodeBlock {
            // An empty info string is the same as none.
            lang: lang.filter(|l| !l.trim().is_empty()),
            code,
        },
        Block::ThematicBreak => Block::ThematicBreak,
        Block::Unsupported { kind, fallback } => Block::Unsupported {
            kind,
            fallback: normalize_inlines(fallback),
        },
    }
}

/// Run the inline passes in order, then recurse into whatever marks survive.
pub(crate) fn normalize_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = inlines;
    for _ in 0..MAX_ROUNDS {
        let next = inline_round(out.clone());
        if next == out {
            break;
        }
        out = next;
    }
    out
}

fn inline_round(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = split_newlines(inlines);
    out = out.into_iter().flat_map(recurse_children).collect();
    out = hoist_whitespace(out);
    out = canonical_order(out);
    out = unwrap_redundant(out);
    out = drop_empty(out);
    out = merge_adjacent(out);
    out
}

/// Recurse into a mark's children, normalizing them first.
fn recurse_children(node: Inline) -> Vec<Inline> {
    match node.children() {
        Some(children) => {
            let normalized = normalize_inlines(children.to_vec());
            vec![node.with_children(normalized)]
        }
        None => vec![node],
    }
}

/// Text nodes never contain newlines; a literal newline becomes a soft break.
fn split_newlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for node in inlines {
        match node {
            Inline::Text(t) if t.contains('\n') => {
                let mut first = true;
                for segment in t.split('\n') {
                    if !first {
                        out.push(Inline::SoftBreak);
                    }
                    if !segment.is_empty() {
                        out.push(Inline::Text(segment.to_string()));
                    }
                    first = false;
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Move leading and trailing whitespace outside of marks.
///
/// **The highest-value rule in the codebase.** `Bold(" x ")` renders as `** x **`, which no
/// Markdown parser treats as bold — the delimiters must sit flush against non-whitespace.
fn hoist_whitespace(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for node in inlines {
        // Links are excluded: their delimiters are not whitespace-sensitive, and hoisting would
        // change which characters are clickable.
        let is_hoistable = matches!(
            node,
            Inline::Bold(_) | Inline::Italic(_) | Inline::Strike(_)
        );
        if !is_hoistable {
            out.push(node);
            continue;
        }
        let Some(children) = node.children().map(<[Inline]>::to_vec) else {
            out.push(node);
            continue;
        };

        // A mark containing nothing but whitespace is not a mark at all.
        if children.iter().all(is_whitespace_text) {
            out.extend(children);
            continue;
        }

        let mut inner = children;
        let mut lead = String::new();
        let mut trail = String::new();

        while let Some(Inline::Text(t)) = inner.first() {
            let trimmed = t.trim_start();
            if trimmed.len() == t.len() {
                break;
            }
            lead.push_str(&t[..t.len() - trimmed.len()]);
            if trimmed.is_empty() {
                inner.remove(0);
            } else {
                inner[0] = Inline::Text(trimmed.to_string());
                break;
            }
        }
        while let Some(Inline::Text(t)) = inner.last() {
            let trimmed = t.trim_end();
            if trimmed.len() == t.len() {
                break;
            }
            let cut = t[trimmed.len()..].to_string();
            trail.insert_str(0, &cut);
            if trimmed.is_empty() {
                inner.pop();
            } else {
                let last = inner.len() - 1;
                inner[last] = Inline::Text(trimmed.to_string());
                break;
            }
        }

        if !lead.is_empty() {
            out.push(Inline::Text(lead));
        }
        if !inner.is_empty() {
            out.push(node.with_children(inner));
        }
        if !trail.is_empty() {
            out.push(Inline::Text(trail));
        }
    }
    out
}

fn is_whitespace_text(node: &Inline) -> bool {
    matches!(node, Inline::Text(t) if t.chars().all(char::is_whitespace))
}

/// Put nested marks in a stable order so that round trips converge.
///
/// Only applies when the inner mark is the sole child; otherwise reordering would change which
/// text each mark covers.
fn canonical_order(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| {
            let (Some(outer_rank), Some(children)) = (node.mark_rank(), node.children()) else {
                return node;
            };
            if children.len() != 1 {
                return node;
            }
            let inner = &children[0];
            let Some(inner_rank) = inner.mark_rank() else {
                return node;
            };
            if inner_rank < outer_rank {
                // The inner mark belongs on the outside: swap them.
                let grandchildren = inner.children().map(<[Inline]>::to_vec).unwrap_or_default();
                inner.with_children(vec![node.with_children(grandchildren)])
            } else {
                node
            }
        })
        .collect()
}

/// Collapse a mark wrapping the same mark: `Bold(Bold(x))` becomes `Bold(x)`.
fn unwrap_redundant(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| {
            let Some(children) = node.children() else {
                return node;
            };
            if children.len() != 1 {
                return node;
            }
            if same_mark(&node, &children[0]) {
                let grandchildren = children[0]
                    .children()
                    .map(<[Inline]>::to_vec)
                    .unwrap_or_default();
                node.with_children(grandchildren)
            } else {
                node
            }
        })
        .collect()
}

fn same_mark(a: &Inline, b: &Inline) -> bool {
    matches!(
        (a, b),
        (Inline::Bold(_), Inline::Bold(_))
            | (Inline::Italic(_), Inline::Italic(_))
            | (Inline::Strike(_), Inline::Strike(_))
    )
}

fn drop_empty(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines.into_iter().filter(|n| !n.is_empty()).collect()
}

/// Merge adjacent text nodes, and adjacent marks of the same kind.
fn merge_adjacent(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for node in inlines {
        match (out.last_mut(), &node) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => {
                prev.push_str(next);
            }
            (Some(prev), next) if same_mark(prev, next) => {
                let mut merged = prev.children().map(<[Inline]>::to_vec).unwrap_or_default();
                merged.extend(next.children().map(<[Inline]>::to_vec).unwrap_or_default());
                *prev = prev.with_children(normalize_inlines(merged));
            }
            (
                Some(Inline::Link {
                    href: ha,
                    content: ca,
                    ..
                }),
                Inline::Link {
                    href: hb,
                    content: cb,
                    ..
                },
            ) if ha == hb => {
                // Teams frequently splits one link across several <a> elements.
                let mut merged = ca.clone();
                merged.extend(cb.clone());
                *ca = normalize_inlines(merged);
            }
            _ => out.push(node),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(inlines: Vec<Inline>) -> Vec<Inline> {
        normalize_inlines(inlines)
    }

    #[test]
    fn hoists_leading_and_trailing_whitespace_out_of_marks() {
        // `Bold(" x ")` must not render as `** x **`.
        let got = norm(vec![Inline::bold(" x ")]);
        assert_eq!(
            got,
            vec![
                Inline::Text(" ".into()),
                Inline::Bold(vec![Inline::Text("x".into())]),
                Inline::Text(" ".into()),
            ]
        );
    }

    #[test]
    fn a_mark_containing_only_whitespace_is_not_a_mark() {
        assert_eq!(norm(vec![Inline::bold("   ")]), vec![Inline::Text("   ".into())]);
    }

    #[test]
    fn drops_empty_marks_and_text() {
        assert_eq!(norm(vec![Inline::bold(""), Inline::text("")]), vec![]);
    }

    #[test]
    fn merges_adjacent_text() {
        assert_eq!(
            norm(vec![Inline::text("a"), Inline::text("b")]),
            vec![Inline::Text("ab".into())]
        );
    }

    #[test]
    fn merges_adjacent_identical_marks() {
        assert_eq!(
            norm(vec![Inline::bold("a"), Inline::bold("b")]),
            vec![Inline::Bold(vec![Inline::Text("ab".into())])]
        );
    }

    #[test]
    fn merges_a_link_teams_split_across_two_anchors() {
        let got = norm(vec![
            Inline::link("https://example.com", "exa"),
            Inline::link("https://example.com", "mple"),
        ]);
        assert_eq!(got, vec![Inline::link("https://example.com", "example")]);
    }

    #[test]
    fn unwraps_redundant_nesting() {
        let got = norm(vec![Inline::Bold(vec![Inline::bold("x")])]);
        assert_eq!(got, vec![Inline::Bold(vec![Inline::Text("x".into())])]);
    }

    #[test]
    fn orders_nested_marks_canonically() {
        // Italic(Bold(x)) and Bold(Italic(x)) mean the same thing; pick one.
        let a = norm(vec![Inline::Italic(vec![Inline::bold("x")])]);
        let b = norm(vec![Inline::Bold(vec![Inline::italic("x")])]);
        assert_eq!(a, b);
        assert_eq!(
            a,
            vec![Inline::Bold(vec![Inline::Italic(vec![Inline::Text("x".into())])])]
        );
    }

    #[test]
    fn hoists_links_outside_marks() {
        let got = norm(vec![Inline::Bold(vec![Inline::link("u", "t")])]);
        assert_eq!(
            got,
            vec![Inline::Link {
                href: "u".into(),
                title: None,
                content: vec![Inline::Bold(vec![Inline::Text("t".into())])],
            }]
        );
    }

    #[test]
    fn newlines_in_text_become_soft_breaks() {
        assert_eq!(
            norm(vec![Inline::text("a\nb")]),
            vec![
                Inline::Text("a".into()),
                Inline::SoftBreak,
                Inline::Text("b".into())
            ]
        );
    }

    #[test]
    fn clamps_heading_levels() {
        let doc = normalize(Document {
            blocks: vec![Block::Heading {
                level: 6,
                content: vec![Inline::text("deep")],
            }],
        });
        assert_eq!(
            doc.blocks,
            vec![Block::Heading {
                level: 3,
                content: vec![Inline::Text("deep".into())]
            }]
        );
    }

    #[test]
    fn drops_empty_blocks_but_keeps_thematic_breaks() {
        let doc = normalize(Document {
            blocks: vec![
                Block::Paragraph(vec![]),
                Block::ThematicBreak,
                Block::para("kept"),
            ],
        });
        assert_eq!(
            doc.blocks,
            vec![Block::ThematicBreak, Block::para("kept")]
        );
    }

    #[test]
    fn is_idempotent() {
        let doc = Document {
            blocks: vec![Block::Paragraph(vec![
                Inline::bold(" a "),
                Inline::bold("b"),
                Inline::text(""),
            ])],
        };
        let once = normalize(doc);
        let twice = normalize(once.clone());
        assert_eq!(once, twice);
    }
}
