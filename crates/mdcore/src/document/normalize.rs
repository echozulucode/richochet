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

use crate::document::model::{Block, Document, Inline, List, ListItem, Row, Table, HEADING_MAX};

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
        Block::Paragraph(c) => Block::Paragraph(trim_edges(normalize_inlines(c))),
        Block::Heading { level, content } => Block::Heading {
            level: level.clamp(1, HEADING_MAX),
            content: trim_edges(normalize_inlines(breaks_to_spaces(content))),
        },
        Block::List(l) => {
            let items: Vec<ListItem> = l
                .items
                .into_iter()
                .map(|i| ListItem {
                    blocks: normalize_blocks(i.blocks),
                })
                .filter(|i| !i.blocks.is_empty())
                .collect();
            Block::List(List {
                tight: l.tight && items.iter().all(can_be_tight),
                items,
                ..l
            })
        }
        Block::BlockQuote(b) => Block::BlockQuote(normalize_blocks(b)),
        Block::CodeBlock { lang, code } => Block::CodeBlock {
            // An empty info string is the same as none.
            lang: lang.filter(|l| !l.trim().is_empty()),
            code,
        },
        Block::ThematicBreak => Block::ThematicBreak,
        Block::Table(t) => Block::Table(Table {
            head: normalize_row(t.head),
            align: t.align,
            rows: t.rows.into_iter().map(normalize_row).collect(),
        }),
        Block::Unsupported { kind, fallback } => Block::Unsupported {
            kind,
            fallback: trim_edges(normalize_inlines(fallback)),
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
    out = flatten_links(out);
    out = unwrap_void_links(out);
    out = label_bare_links(out);
    out = dedupe_nested_marks(out);
    out = factor_marks(out);
    out = collapse_around_breaks(out);
    out = drop_empty(out);
    out = merge_adjacent(out);
    out = collapse_spaces(out);
    out
}

/// Collapse runs of spaces and tabs inside text to a single space.
///
/// Both output formats already do this — HTML by its whitespace rules, Markdown by rendering — so
/// a document holding `"a  b"` renders as `a b`, reads back as one space, and has changed. Only
/// [`Inline::Text`] is touched; [`Inline::Code`] and [`Block::CodeBlock`] are verbatim by
/// definition and never pass through here.
///
/// Runs after [`merge_adjacent`] so that a space ending one text node and a space beginning the
/// next — the shape [`hoist_whitespace`] produces from two neighbouring marks — is collapsed too.
fn collapse_spaces(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| match node {
            Inline::Text(t) if t.contains("  ") || t.contains('\t') => {
                let mut out = String::with_capacity(t.len());
                let mut prev_space = false;
                for c in t.chars() {
                    let is_space = c == ' ' || c == '\t';
                    if !is_space {
                        out.push(c);
                    } else if !prev_space {
                        out.push(' ');
                    }
                    prev_space = is_space;
                }
                Inline::Text(out)
            }
            other => other,
        })
        .collect()
}

/// A link may not contain another link.
///
/// Both HTML and Markdown forbid it — an HTML parser drops the inner `<a>`, and Markdown has no
/// syntax for it at all, so a nested link renders as literal brackets and changes meaning on the
/// next round trip. The outer link wins, which is what a browser does.
fn flatten_links(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| match node {
            Inline::Link {
                href,
                title,
                content,
            } => Inline::Link {
                href,
                title,
                // Whitespace at the edges of a link label is not part of the label. Unlike the
                // emphasis marks it is dropped rather than hoisted, because moving it outside
                // would change which characters are clickable.
                content: trim_edges(strip_links(content)),
            },
            other => other,
        })
        .collect()
}

fn strip_links(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .flat_map(|node| match node {
            Inline::Link { content, .. } => strip_links(content),
            other => match other.children() {
                Some(children) => vec![other.with_children(strip_links(children.to_vec()))],
                None => vec![other],
            },
        })
        .collect()
}

/// Remove a mark from anywhere inside the same mark.
///
/// `Strike([a, Strike([b])])` says nothing `Strike([a, b])` does not. [`unwrap_redundant`] only
/// catches the single-child case; this catches it at any depth, which is the shape Teams produces
/// when nested `<span>`s each re-state a style their parent already set.
fn dedupe_nested_marks(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| match kind_of(&node) {
            Some(kind) => {
                let children = node.children().map(<[Inline]>::to_vec).unwrap_or_default();
                let cleaned = children.into_iter().flat_map(|c| strip(c, kind)).collect();
                node.with_children(cleaned)
            }
            None => node,
        })
        .collect()
}

/// Replace a link that has no destination with its own text.
///
/// `[text]()` is a valid CommonMark link with an empty destination, and `<a href="">text</a>` is
/// what Word and Outlook emit for bookmark anchors. There is nothing to link *to*, but the words
/// are real content: dropping the node used to take them with it, which is silent content loss —
/// the worst kind of conversion bug, because nothing tells the user it happened.
///
/// Found by CommonMark examples 200, 485, 486 and 567 (`tests/commonmark.rs`).
fn unwrap_void_links(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .flat_map(|node| match node {
            Inline::Link { ref href, .. } if href.is_empty() => {
                node.children().map(<[Inline]>::to_vec).unwrap_or_default()
            }
            other => vec![other],
        })
        .collect()
}

/// Give a link with no visible text the URL as its text.
///
/// `[](url)` and `[ ](url)` have nothing to click. Teams produces them when a link's label was an
/// element we dropped, and Markdown cannot render them usefully — so fall back to showing the URL,
/// which is what a bare link looks like anyway. A link with no href at all is dropped instead, by
/// [`drop_empty`].
fn label_bare_links(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| match &node {
            Inline::Link {
                href,
                title,
                content,
            } if !href.is_empty() && content.iter().all(is_whitespace_like) => Inline::Link {
                href: href.clone(),
                title: title.clone(),
                content: vec![Inline::Text(href.clone())],
            },
            _ => node,
        })
        .collect()
}

/// Drop whitespace that sits against a line break.
///
/// Markdown discards it on the next read: a space before a newline is insignificant (and *two*
/// spaces are a hard break, so keeping them would invent one), and leading whitespace on a
/// continuation line is stripped. Leaving it in place means the document changes every round trip.
fn collapse_around_breaks(mut inlines: Vec<Inline>) -> Vec<Inline> {
    for i in 0..inlines.len() {
        if !matches!(inlines[i], Inline::SoftBreak | Inline::HardBreak) {
            continue;
        }
        if i > 0 {
            if let Inline::Text(t) = &mut inlines[i - 1] {
                *t = t.trim_end().to_string();
            }
        }
        if let Some(Inline::Text(t)) = inlines.get_mut(i + 1) {
            *t = t.trim_start().to_string();
        }
    }
    inlines.retain(|n| !matches!(n, Inline::Text(t) if t.is_empty()));
    inlines
}

/// The three emphasis marks, as a value so they can be iterated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkKind {
    Bold,
    Italic,
    Strike,
}

fn kind_of(node: &Inline) -> Option<MarkKind> {
    match node {
        Inline::Bold(_) => Some(MarkKind::Bold),
        Inline::Italic(_) => Some(MarkKind::Italic),
        Inline::Strike(_) => Some(MarkKind::Strike),
        _ => None,
    }
}

fn wrap(kind: MarkKind, children: Vec<Inline>) -> Inline {
    match kind {
        MarkKind::Bold => Inline::Bold(children),
        MarkKind::Italic => Inline::Italic(children),
        MarkKind::Strike => Inline::Strike(children),
    }
}

/// Does this node carry `kind` anywhere in its chain of single-child marks?
///
/// A link counts when *all* of its content carries the mark: `[**a**](u)` and `**[a](u)**` mean
/// the same thing, so an emphasis shared across a link and its neighbours can still be factored
/// out around the whole run.
fn carries(node: &Inline, kind: MarkKind) -> bool {
    if let Inline::Link { content, .. } = node {
        return !content.is_empty() && content.iter().all(|c| carries(c, kind));
    }
    match kind_of(node) {
        Some(k) if k == kind => true,
        Some(_) => node
            .children()
            .is_some_and(|c| c.len() == 1 && carries(&c[0], kind)),
        None => false,
    }
}

/// Remove `kind` from a node's mark chain, splicing its children into its place.
fn strip(node: Inline, kind: MarkKind) -> Vec<Inline> {
    if matches!(node, Inline::Link { .. }) {
        let children = node.children().map(<[Inline]>::to_vec).unwrap_or_default();
        let stripped = children.into_iter().flat_map(|c| strip(c, kind)).collect();
        return vec![node.with_children(stripped)];
    }
    match kind_of(&node) {
        Some(k) if k == kind => node.children().map(<[Inline]>::to_vec).unwrap_or_default(),
        Some(_) => {
            let children = node.children().map(<[Inline]>::to_vec).unwrap_or_default();
            let stripped = children.into_iter().flat_map(|c| strip(c, kind)).collect();
            vec![node.with_children(stripped)]
        }
        None => vec![node],
    }
}

/// Pull a mark shared by adjacent siblings out into a single wrapper around all of them.
///
/// **This is what makes HTML and Markdown agree.** The HTML parser distributes marks down to the
/// leaves, because Teams expresses formatting with nested `<span style>` elements and each run of
/// text has to carry the full set that applies to it. The Markdown parser does the opposite and
/// keeps the tree factored. Without this pass the same document has two shapes depending on which
/// side it came from, round trips never converge, and — worse for the user — Teams HTML converts
/// to `~~plain~~**~~plain~~**` where it should read `~~plain**plain**~~`.
///
/// Marks are factored outermost-rank first so the result is also canonically ordered.
fn factor_marks(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = inlines;
    for kind in [MarkKind::Bold, MarkKind::Italic, MarkKind::Strike] {
        out = factor_one(out, kind);
    }
    out
}

fn factor_one(inlines: Vec<Inline>, kind: MarkKind) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    let mut i = 0;
    while i < inlines.len() {
        if !carries(&inlines[i], kind) {
            out.push(inlines[i].clone());
            i += 1;
            continue;
        }

        // Extend the run over carriers, stepping across whitespace between them. Whitespace is
        // neutral: `hoist_whitespace` lifts the space out of `~~plain ~~` and it then sits between
        // the two marks, which would otherwise break the run and leave `~~plain~~ ~~`code`~~`
        // where `~~plain `code`~~` was meant. The run must still *begin* and *end* on a carrier,
        // so trailing whitespace is never pulled inside.
        let start = i;
        let mut last_carrier = i;
        let mut scan = i;
        while scan < inlines.len() {
            if carries(&inlines[scan], kind) {
                last_carrier = scan;
            } else if !is_whitespace_like(&inlines[scan]) {
                break;
            }
            scan += 1;
        }

        let end = last_carrier + 1;
        let run = &inlines[start..end];
        if run.iter().filter(|n| carries(n, kind)).count() >= 2 {
            let inner = run
                .iter()
                .cloned()
                .flat_map(|n| strip(n, kind))
                .collect::<Vec<_>>();
            out.push(wrap(kind, inner));
        } else {
            out.extend_from_slice(run);
        }
        i = end;
    }
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
        if children.iter().all(is_whitespace_like) {
            out.extend(children);
            continue;
        }

        let mut inner = children;
        let mut lead: Vec<Inline> = Vec::new();
        let mut trail: Vec<Inline> = Vec::new();

        loop {
            match inner.first() {
                // A line break is whitespace too: `Bold([SoftBreak, "x"])` would render as
                // `**\nx**`, and a mark holding only a break renders as `**\n**`, which is not
                // emphasis in any parser.
                Some(Inline::SoftBreak | Inline::HardBreak) => lead.push(inner.remove(0)),
                Some(Inline::Text(t)) => {
                    let trimmed = t.trim_start().to_string();
                    if trimmed.len() == t.len() {
                        break;
                    }
                    lead.push(Inline::Text(t[..t.len() - trimmed.len()].to_string()));
                    if trimmed.is_empty() {
                        inner.remove(0);
                    } else {
                        inner[0] = Inline::Text(trimmed);
                        break;
                    }
                }
                _ => break,
            }
        }
        loop {
            match inner.last() {
                Some(Inline::SoftBreak | Inline::HardBreak) => {
                    let node = inner.pop().expect("last() matched");
                    trail.insert(0, node);
                }
                Some(Inline::Text(t)) => {
                    let trimmed = t.trim_end().to_string();
                    if trimmed.len() == t.len() {
                        break;
                    }
                    trail.insert(0, Inline::Text(t[trimmed.len()..].to_string()));
                    if trimmed.is_empty() {
                        inner.pop();
                    } else {
                        let last = inner.len() - 1;
                        inner[last] = Inline::Text(trimmed);
                        break;
                    }
                }
                _ => break,
            }
        }

        out.extend(lead);
        if !inner.is_empty() {
            out.push(node.with_children(inner));
        }
        out.extend(trail);
    }
    out
}

/// Normalize every cell in a table row.
///
/// A cell is an inline context like a paragraph, so it gets the same edge trimming: a cell that
/// begins or ends with whitespace would widen the rendered column for nothing.
fn normalize_row(row: Row) -> Row {
    row.into_iter()
        .map(|cell| trim_edges(normalize_inlines(cell)))
        .collect()
}

/// Can this item sit in a tight list?
///
/// CommonMark infers tightness from blank lines, so the flag is not free to disagree with the
/// text: an item whose blocks force a blank line between them makes the whole list loose when it
/// is read back, and a list that claims to be tight would then flip to loose on every round trip.
/// A single block never needs a blank line, and neither does a paragraph followed by nested lists
/// — that is the ordinary `- outer` / `  - inner` shape. Anything else is conservatively loose.
fn can_be_tight(item: &ListItem) -> bool {
    match item.blocks.as_slice() {
        [] | [_] => true,
        [Block::Paragraph(_), rest @ ..] => rest.iter().all(|b| matches!(b, Block::List(_))),
        _ => false,
    }
}

/// Replace line breaks with spaces.
///
/// A heading is a single line in both Markdown (`# ...` ends at the newline) and HTML rendering, so
/// a break inside one cannot survive being written out and read back. Turning it into a space
/// preserves the word gap instead of silently joining two words together.
fn breaks_to_spaces(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|node| match node {
            Inline::SoftBreak | Inline::HardBreak => Inline::Text(" ".to_string()),
            other => match other.children() {
                Some(children) => other.with_children(breaks_to_spaces(children.to_vec())),
                None => other,
            },
        })
        .collect()
}

/// Drop whitespace at the two ends of a block's inline content.
///
/// A break or a run of spaces at the edge of a paragraph or heading renders to nothing and is
/// dropped when the output is re-parsed, so leaving it in place means the document changes shape
/// every time it round-trips. Trimming here also makes a block whose *only* content was whitespace
/// become genuinely empty, so the empty-block filter can collect it — `Paragraph([SoftBreak])`
/// would otherwise render as a blank line that re-parses to nothing at all.
///
/// Only the outer edges are touched: whitespace between two pieces of content is meaningful, and
/// whitespace inside a mark is [`hoist_whitespace`]'s job.
fn trim_edges(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while inlines.first().is_some_and(is_whitespace_like) {
        inlines.remove(0);
    }
    while inlines.last().is_some_and(is_whitespace_like) {
        inlines.pop();
    }
    if let Some(Inline::Text(t)) = inlines.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Inline::Text(t)) = inlines.last_mut() {
        *t = t.trim_end().to_string();
    }
    inlines.retain(|n| !n.is_empty());
    inlines
}

/// Whitespace for the purposes of mark hoisting: blank text, or either kind of line break.
fn is_whitespace_like(node: &Inline) -> bool {
    match node {
        Inline::Text(t) => t.chars().all(char::is_whitespace),
        Inline::SoftBreak | Inline::HardBreak => true,
        _ => false,
    }
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
            // Two code spans with nothing between them cannot be written as two code spans:
            // `` `a``b` `` re-parses as one span containing backticks. They are indistinguishable
            // from a single span, so make them one.
            (Some(Inline::Code(prev)), Inline::Code(next)) => {
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
        // The run of spaces also collapses to one: see `collapse_spaces`.
        assert_eq!(
            norm(vec![Inline::bold("   ")]),
            vec![Inline::Text(" ".into())]
        );
    }

    #[test]
    fn runs_of_spaces_collapse() {
        assert_eq!(
            norm(vec![Inline::text("a  \t b")]),
            vec![Inline::Text("a b".into())]
        );
    }

    #[test]
    fn two_marks_separated_only_by_hoisted_space_become_one() {
        // `<em>a </em><em> b</em>` displays exactly as `<em>a b</em>`, so say it once. The space
        // that `hoist_whitespace` lifts out does not break the run — see `factor_one`.
        let got = norm(vec![Inline::italic("a "), Inline::italic(" b")]);
        assert_eq!(got, vec![Inline::Italic(vec![Inline::Text("a b".into())])]);
    }

    #[test]
    fn a_real_word_between_two_marks_does_break_the_run() {
        // Only whitespace is neutral. Factoring across `and` would italicize it too.
        let got = norm(vec![
            Inline::italic("a"),
            Inline::text(" and "),
            Inline::italic("b"),
        ]);
        assert_eq!(
            got,
            vec![
                Inline::Italic(vec![Inline::Text("a".into())]),
                Inline::Text(" and ".into()),
                Inline::Italic(vec![Inline::Text("b".into())]),
            ]
        );
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
    fn a_link_with_no_destination_keeps_its_text() {
        // `<a href="">text</a>` is what Word and Outlook emit for bookmark anchors, and `[x]()` is
        // a valid CommonMark link. Dropping the node used to delete the words inside it — silent
        // content loss, found by CommonMark examples 200/485/486/567.
        let got = norm(vec![Inline::Link {
            href: String::new(),
            title: None,
            content: vec![Inline::text("the docs")],
        }]);
        assert_eq!(got, vec![Inline::Text("the docs".into())]);
    }

    #[test]
    fn a_link_with_neither_destination_nor_text_is_dropped() {
        let got = norm(vec![Inline::Link {
            href: String::new(),
            title: None,
            content: vec![],
        }]);
        assert_eq!(got, vec![]);
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
            vec![Inline::Bold(vec![Inline::Italic(vec![Inline::Text(
                "x".into()
            )])])]
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
        assert_eq!(doc.blocks, vec![Block::ThematicBreak, Block::para("kept")]);
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
