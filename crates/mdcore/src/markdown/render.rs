//! Document model -> Markdown.
//!
//! # Conventions this renderer commits to
//!
//! * **No trailing newline.** [`render`] returns exactly the document's content, so an empty
//!   document renders to `""` and the Markdown pane never shows a phantom blank last line.
//!   Callers that want a file-shaped string append `\n` themselves.
//! * **Exactly one blank line between blocks**, at every nesting level.
//! * **Bold is `**`, italic is `*`, strikethrough is `~~`.** Italic uses `*` rather than `_`
//!   because `_` carries an *extra* restriction that `*` does not: CommonMark refuses intraword
//!   `_` emphasis, so `Text("a") + Italic("b")` would render as `a_b_` and read back as literal
//!   text. `*` also keeps `Bold(Italic(x))` as `***x***`, whose delimiter run is followed by a
//!   word character and therefore flanks correctly — `**_x_**` does not, because a `**` run
//!   followed by punctuation only opens emphasis when it is itself preceded by whitespace or
//!   punctuation.
//! * **Hard breaks render as a trailing backslash**, not as two trailing spaces. Trailing spaces
//!   are invisible, get eaten by editors and diff tools, and cannot be told apart from an accident;
//!   a backslash survives a round trip through anything that preserves the bytes.
//! * **Soft breaks render as a bare newline**, and the text after one is escaped with
//!   [`escape_line_start`] because block markers can interrupt a paragraph on a continuation line.
//! * **Tables are padded to even columns**, with a break inside a cell collapsed to a space. The
//!   Markdown pane is read and edited by a person, and a table whose columns do not line up is not
//!   one; a row, meanwhile, is exactly one line, so a break has nowhere to go. `<br>` would keep
//!   the break but is raw HTML that the Markdown parser reads back as literal text and the next
//!   render escapes, so the table would visibly change on the second conversion.
//!
//! # Known lossy corners
//!
//! * A [`Block::Unsupported`] renders as a plain paragraph of its fallback text, so re-parsing
//!   gives a `Paragraph`, not the `Unsupported` back. That is the point of the node: degrade once,
//!   then stay put.
//! * Two adjacent lists of the same kind cannot be kept apart in Markdown; separated by the
//!   mandatory blank line they read back as one loose list. Parsers never produce that shape.
//! * A single-item list with a single paragraph is always tight in the output, because no blank
//!   line has anywhere to go. `tight: false` on such a list is unrepresentable.

use std::borrow::Cow;

use crate::document::model::{Alignment, Block, Document, Inline, List, Row, Table};
use crate::markdown::escape::{escape_inline, escape_line_start, escape_table_cell};

/// Render a document as CommonMark with GFM strikethrough.
///
/// The result has no trailing newline; blocks are separated by exactly one blank line.
///
/// ```
/// use mdcore::{Block, Document};
///
/// let doc = Document::from_blocks(vec![Block::para("hi"), Block::ThematicBreak]);
/// assert_eq!(mdcore::markdown::render(&doc), "hi\n\n---");
/// ```
pub fn render(doc: &Document) -> String {
    render_blocks(&doc.blocks)
}

/// Render a run of blocks, separated by exactly one blank line.
fn render_blocks(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(render_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_block(block: &Block) -> String {
    match block {
        // An unsupported block is emitted as its fallback text and nothing else; the `kind` is a
        // diagnostic, not output.
        Block::Paragraph(inlines)
        | Block::Unsupported {
            fallback: inlines, ..
        } => {
            // A break at the very start or end of a paragraph has nothing to separate, and no
            // parser can produce one, so it is dropped rather than emitted as a stray blank line.
            let (_, core, _) = split_off_whitespace(inlines);
            let mut out = String::new();
            let mut line_start = true;
            render_inlines(core, &mut out, &mut line_start, None);
            trim_trailing_spaces(&mut out);
            out
        }
        Block::Heading { level, content } => render_heading(*level, content),
        Block::List(list) => render_list(list),
        Block::BlockQuote(blocks) => prefix_lines(&render_blocks(blocks), "> ", "> "),
        Block::CodeBlock { lang, code } => render_code_block(lang.as_deref(), code),
        Block::Table(table) => render_table(table),
        // Safe next to anything because every block is preceded by a blank line, so this can never
        // be read as a setext underline for the paragraph above.
        Block::ThematicBreak => "---".to_string(),
    }
}

/// The narrowest a table column is allowed to be.
///
/// Three is the width of the widest delimiter cell that carries no padding (`:-:`), so every
/// column can express any alignment without the delimiter row being wider than the data.
const MIN_COLUMN_WIDTH: usize = 3;

/// Render a GFM table, padded so the columns line up in the source.
///
/// The Markdown pane is something a person reads and edits, not just a serialization format, and
/// an unpadded table is unreadable the moment two cells differ in length. The padding costs
/// nothing on the way back in: GFM trims each cell.
///
/// Ragged rows — which the model deliberately allows — are padded out to
/// [`Table::columns`] with empty cells, because a row shorter than the header row is the one
/// shape GFM genuinely cannot express.
fn render_table(table: &Table) -> String {
    let columns = table.columns();
    if columns == 0 {
        // No header and no rows. Nothing can be written that parses as a table.
        return String::new();
    }

    let head = render_row(&table.head, columns);
    let body: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|row| render_row(row, columns))
        .collect();

    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            std::iter::once(&head)
                .chain(body.iter())
                .map(|row| display_width(&row[i]))
                .max()
                .unwrap_or(0)
                .max(MIN_COLUMN_WIDTH)
        })
        .collect();

    // GFM has no headerless table, so a table that lost its header (an HTML one with no `<thead>`)
    // gets a row of empty cells rather than promoting its first body row and changing the meaning.
    let mut lines = Vec::with_capacity(body.len() + 2);
    lines.push(join_cells(
        head.iter()
            .enumerate()
            .map(|(i, cell)| pad_cell(cell, widths[i], table.alignment(i))),
    ));
    lines.push(join_cells(
        widths
            .iter()
            .enumerate()
            .map(|(i, &w)| delimiter_cell(w, table.alignment(i))),
    ));
    for row in &body {
        lines.push(join_cells(
            row.iter()
                .enumerate()
                .map(|(i, cell)| pad_cell(cell, widths[i], table.alignment(i))),
        ));
    }
    lines.join("\n")
}

/// Render one row's cells, padded out to `columns` with empty cells.
fn render_row(row: &Row, columns: usize) -> Vec<String> {
    (0..columns)
        .map(|i| row.get(i).map(|cell| render_cell(cell)).unwrap_or_default())
        .collect()
}

/// Render one cell's inline content onto a single line.
///
/// # Why a break becomes a space and not `<br>`
///
/// A GFM table row is one line, so an [`Inline::SoftBreak`] or [`Inline::HardBreak`] arriving from
/// an HTML `<br>` inside a `<td>` has to turn into something. `<br>` would keep the visual break,
/// but it is raw HTML, and [`crate::markdown::parse`] maps inline HTML back to *literal text* —
/// so the next conversion would escape it to `\<br\>` and the user would watch their table change
/// the moment they clicked into the other pane. A space converges on the first pass, and it is
/// already what [`render_heading`] does with a break for exactly the same reason: the construct is
/// single-line, so the break has nowhere to go.
///
/// A cell is never at the start of a line — the `|` is — so the inline escaping is used rather
/// than [`escape_line_start`]; `- x` or `# x` in a cell is text, not a block marker.
fn render_cell(cell: &[Inline]) -> String {
    let mut out = String::new();
    let mut line_start = false;
    // Every cell is emitted surrounded by padding spaces, so a space is what really follows.
    render_inlines(&flatten_breaks(cell), &mut out, &mut line_start, Some(' '));
    escape_table_cell(out.trim())
}

/// Wrap a row's already-padded cells in pipes.
fn join_cells<I: Iterator<Item = String>>(cells: I) -> String {
    let mut out = String::from("|");
    for cell in cells {
        out.push(' ');
        out.push_str(&cell);
        out.push_str(" |");
    }
    out
}

/// Pad a cell to `width`, placing the slack according to the column's alignment.
fn pad_cell(content: &str, width: usize, align: Alignment) -> String {
    let slack = width.saturating_sub(display_width(content));
    match align {
        Alignment::Right => format!("{}{content}", " ".repeat(slack)),
        Alignment::Center => {
            let left = slack / 2;
            format!("{}{content}{}", " ".repeat(left), " ".repeat(slack - left))
        }
        Alignment::None | Alignment::Left => format!("{content}{}", " ".repeat(slack)),
    }
}

/// The delimiter-row cell for a column: dashes, with colons marking the alignment.
///
/// `width` is at least [`MIN_COLUMN_WIDTH`], so every form keeps at least one dash.
fn delimiter_cell(width: usize, align: Alignment) -> String {
    match align {
        Alignment::None => "-".repeat(width),
        Alignment::Left => format!(":{}", "-".repeat(width - 1)),
        Alignment::Right => format!("{}:", "-".repeat(width - 1)),
        Alignment::Center => format!(":{}:", "-".repeat(width - 2)),
    }
}

/// How wide a rendered cell is, for padding purposes.
///
/// Counted in `char`s, which is exact for the Latin text that dominates and wrong for East Asian
/// double-width characters and combining marks. The cost of being wrong is a column of source that
/// looks slightly ragged; no parser cares. A correct answer would mean a Unicode width table, and
/// this crate is deliberately dependency-light.
fn display_width(s: &str) -> usize {
    s.chars().count()
}

/// Render an ATX heading.
///
/// Headings are single-line, so any break inside the content collapses to a space.
fn render_heading(level: u8, content: &[Inline]) -> String {
    let mut body = String::new();
    let mut line_start = false;
    render_inlines(&flatten_breaks(content), &mut body, &mut line_start, None);

    // A trailing run of `#` would be eaten as an ATX closing sequence. Escaping the *first* `#` of
    // the run stops the run being recognized as a closing sequence at all, and the whole run comes
    // back as literal text (CommonMark 0.31 example 43).
    if body.ends_with('#') {
        let at = body.trim_end_matches('#').len();
        body.insert(at, '\\');
    }

    let hashes = "#".repeat(usize::from(level.clamp(1, 3)));
    // Breaks collapsed to spaces can leave the body padded; trailing spaces on a heading line are
    // not content.
    format!("{hashes} {}", body.trim())
}

/// Replace breaks with spaces, recursively, for contexts that must stay on one line.
fn flatten_breaks(inlines: &[Inline]) -> Vec<Inline> {
    inlines
        .iter()
        .map(|node| match node {
            Inline::SoftBreak | Inline::HardBreak => Inline::Text(" ".to_string()),
            other => match other.children() {
                Some(children) => other.with_children(flatten_breaks(children)),
                None => other.clone(),
            },
        })
        .collect()
}

fn render_list(list: &List) -> String {
    let mut rendered = Vec::with_capacity(list.items.len());
    for (i, item) in list.items.iter().enumerate() {
        let marker = if list.ordered {
            format!("{}. ", list.start.saturating_add(i as u64))
        } else {
            "- ".to_string()
        };
        // Continuation lines align with the item's content column, which for `10. ` is one column
        // further than for `9. `.
        let indent = " ".repeat(marker.len());
        let body = render_item(&item.blocks, list.tight);
        rendered.push(prefix_lines(&body, &marker, &indent));
    }
    rendered.join(if list.tight { "\n" } else { "\n\n" })
}

/// Render one item's blocks.
///
/// In a tight list a nested list hangs directly off the item's paragraph with no blank line.
/// Anything else needs a blank line, which makes that item loose in the output whether the model
/// said so or not — Markdown has no way to express "tight item with two paragraphs".
fn render_item(blocks: &[Block], tight: bool) -> String {
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            let sep = if tight && matches!(block, Block::List(_)) {
                "\n"
            } else {
                "\n\n"
            };
            out.push_str(sep);
        }
        out.push_str(&render_block(block));
    }
    out
}

fn render_code_block(lang: Option<&str>, code: &str) -> String {
    let info = lang.unwrap_or("");
    // A backtick in the info string would close a backtick fence immediately, so switch to tildes.
    let fence_char = if info.contains('`') { '~' } else { '`' };
    let longest = longest_run(code, fence_char);
    let fence: String = std::iter::repeat_n(fence_char, (longest + 1).max(3)).collect();

    let mut out = String::with_capacity(code.len() + 2 * fence.len() + info.len() + 2);
    out.push_str(&fence);
    out.push_str(info);
    out.push('\n');
    out.push_str(code);
    out.push('\n');
    out.push_str(&fence);
    out
}

/// Drop spaces and tabs from the end of the buffer.
///
/// Markdown strips trailing whitespace from a line, so it can never be content — and *two* of them
/// before a newline is a hard break, which would silently invent a line break that the document
/// never had.
fn trim_trailing_spaces(out: &mut String) {
    while out.ends_with(' ') || out.ends_with('\t') {
        out.pop();
    }
}

/// The length of the longest run of `c` in `s`.
fn longest_run(s: &str, c: char) -> usize {
    let mut best = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch == c {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best
}

/// Prefix every line of `body`, using `first` for the first line and `rest` for the others.
///
/// Blank lines keep the prefix with its trailing space trimmed: a blockquote needs its `>` on
/// blank lines to stay one quote, but neither it nor a list indent should leave trailing
/// whitespace behind.
fn prefix_lines(body: &str, first: &str, rest: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let prefix = if i == 0 { first } else { rest };
        if line.is_empty() {
            out.push_str(prefix.trim_end());
        } else {
            out.push_str(prefix);
            out.push_str(line);
        }
    }
    out
}

/// Render inline content, tracking whether the cursor is at the start of a line.
///
/// `line_start` matters because ATX headings, list markers, blockquote markers and setext
/// underlines can all interrupt a paragraph on a continuation line, so text after a break needs
/// the stricter [`escape_line_start`] treatment.
///
/// `next_char` is the character that will follow this whole run — the closing `]` of a link, the
/// closing delimiter of an enclosing mark, or `None` at the end of a paragraph. Emphasis needs it
/// to decide whether its closing delimiter would actually flank (see [`render_mark`]).
fn render_inlines(
    inlines: &[Inline],
    out: &mut String,
    line_start: &mut bool,
    next_char: Option<char>,
) {
    for (i, node) in inlines.iter().enumerate() {
        let following = first_char_of(&inlines[i + 1..], next_char);
        match node {
            Inline::Text(t) => {
                if *line_start {
                    out.push_str(&escape_line_start(t));
                } else {
                    out.push_str(&escape_inline(t));
                }
                if !t.is_empty() {
                    *line_start = false;
                }
            }
            Inline::Bold(children) => {
                let children = flatten_nested(children, is_bold);
                render_mark(&children, "**", Inline::Bold, out, line_start, following);
            }
            Inline::Italic(children) => {
                let children = flatten_nested(children, is_italic);
                render_mark(&children, "*", Inline::Italic, out, line_start, following);
            }
            Inline::Strike(children) => {
                let children = flatten_nested(children, is_strike);
                render_mark(&children, "~~", Inline::Strike, out, line_start, following);
            }
            Inline::Code(code) => {
                out.push_str(&render_code_span(code));
                *line_start = false;
            }
            Inline::Link {
                href,
                title,
                content,
            } => {
                // CommonMark forbids a link inside a link, and the inner one would be unclickable
                // anyway; keep its text and drop the wrapper.
                let content = flatten_nested(content, is_link);
                out.push('[');
                *line_start = false;
                // A link's content is never at the start of a line: the `[` is.
                let mut inner_line_start = false;
                render_inlines(&content, out, &mut inner_line_start, Some(']'));
                out.push_str("](");
                out.push_str(&render_dest(href));
                if let Some(title) = title {
                    out.push_str(" \"");
                    out.push_str(&escape_title(title));
                    out.push('"');
                }
                out.push(')');
            }
            Inline::SoftBreak => {
                trim_trailing_spaces(out);
                out.push('\n');
                *line_start = true;
            }
            Inline::HardBreak => {
                trim_trailing_spaces(out);
                out.push_str("\\\n");
                *line_start = true;
            }
        }
    }
}

fn is_bold(node: &Inline) -> bool {
    matches!(node, Inline::Bold(_))
}

fn is_italic(node: &Inline) -> bool {
    matches!(node, Inline::Italic(_))
}

fn is_strike(node: &Inline) -> bool {
    matches!(node, Inline::Strike(_))
}

fn is_link(node: &Inline) -> bool {
    matches!(node, Inline::Link { .. })
}

/// Splice out any descendant mark matching `is_same`, keeping its children in place.
///
/// A mark nested inside the same mark cannot be written in Markdown and means nothing anyway:
/// `Bold([a, Bold([b])])` is exactly `Bold([a, b])`, and a link inside a link is illegal in both
/// CommonMark and HTML. [`crate::document::normalize`] collapses the case where the inner mark is
/// the *only* child; this catches the rest, because [`render`] accepts any `&Document` and must
/// never emit `**a**b****` and call it bold.
fn flatten_nested<'a>(children: &'a [Inline], is_same: fn(&Inline) -> bool) -> Cow<'a, [Inline]> {
    if has_nested(children, is_same) {
        Cow::Owned(splice_nested(children, is_same))
    } else {
        Cow::Borrowed(children)
    }
}

fn has_nested(children: &[Inline], is_same: fn(&Inline) -> bool) -> bool {
    children.iter().any(|node| {
        is_same(node)
            || node
                .children()
                .is_some_and(|inner| has_nested(inner, is_same))
    })
}

fn splice_nested(children: &[Inline], is_same: fn(&Inline) -> bool) -> Vec<Inline> {
    let mut out = Vec::with_capacity(children.len());
    for node in children {
        match node.children() {
            Some(inner) if is_same(node) => out.extend(splice_nested(inner, is_same)),
            Some(inner) => out.push(node.with_children(splice_nested(inner, is_same))),
            None => out.push(node.clone()),
        }
    }
    out
}

/// Render an emphasis mark, hoisting any leading or trailing whitespace outside the delimiters.
///
/// An emphasis delimiter must sit flush against non-whitespace or it is not emphasis at all:
/// `Bold([SoftBreak])` naively renders as `**\n**`, which reads back as four literal asterisks.
/// [`crate::document::normalize`] hoists this shape away, but [`render`] accepts any `&Document`,
/// so the renderer refuses to emit invalid emphasis rather than trusting its input.
/// When the delimiters would not flank, the mark is **dropped** and its content rendered bare.
/// That is deliberately lossy, and deliberately *deterministic*: the two editor panes rely on
/// `render(parse(render(doc))) == render(doc)`, and emitting delimiters that do not parse breaks
/// it on the very first pass — pass one writes `*`, pass two reads it as text and writes `\*`,
/// and the user's document visibly changes the moment they click into the other pane. Dropping
/// converges immediately, because the re-parsed document simply has no mark there.
///
/// The genuinely complete fix is an inline-HTML `<em>` fallback, which needs the parser to map
/// inline HTML back to marks. That is logged as Phase 6.
fn render_mark(
    children: &[Inline],
    delim: &str,
    wrap: fn(Vec<Inline>) -> Inline,
    out: &mut String,
    line_start: &mut bool,
    next_char: Option<char>,
) {
    let (lead, core, trail) = split_off_whitespace(children);
    render_inlines(lead, out, line_start, next_char);

    if !core.is_empty() {
        let after = first_char_of(trail, next_char);
        let before = out.chars().last();

        // Render the content first so the flanking test can look at the characters it actually
        // produced, then decide whether the delimiters may go around it.
        let start = out.len();
        let saved_line_start = *line_start;
        *line_start = false;
        render_inlines(core, out, line_start, delim.chars().next());

        // Delimiters of the same character that end up adjacent merge into a single run, so the
        // flanking neighbour is the first content character that is *not* this delimiter:
        // `Bold(Italic(x))` is the one run `***x***`, whose neighbours are the `x`, not the inner
        // `*`. Judging `**` and `*` separately would reject the most common nesting there is.
        let content = &out[start..];
        let delim_char = delim.chars().next().unwrap_or('*');
        let inner_open = content.trim_start_matches(delim_char).chars().next();
        let inner_close = content.trim_end_matches(delim_char).chars().last();

        let opens = is_left_flanking(before, inner_open);
        let closes = is_right_flanking(inner_close, after);

        // Either way the content has to be written again: it was rendered assuming a closing
        // delimiter would follow it, and without one the true following character changes its own
        // flanking decisions.
        if opens && closes {
            out.insert_str(start, delim);
            out.push_str(delim);
            *line_start = false;
        } else if core.len() > 1 {
            // Distribute rather than give up. A mark over a run means the same as the mark over
            // each part of the run — `Bold([a, b])` is `Bold([a]) Bold([b])` — and each part then
            // gets its own flanking decision, so the formatting that *can* be written survives.
            // The normalizer merges the parts back together on the way in.
            out.truncate(start);
            *line_start = saved_line_start;
            let parts: Vec<Inline> = core.iter().map(|c| wrap(vec![c.clone()])).collect();
            render_inlines(&parts, out, line_start, after);
        } else {
            // A single part that still cannot flank has nowhere left to go: drop the mark.
            out.truncate(start);
            *line_start = saved_line_start;
            render_inlines(core, out, line_start, after);
        }
    }

    render_inlines(trail, out, line_start, next_char);
}

/// Whether a delimiter run placed here could **open** emphasis (CommonMark "left-flanking").
///
/// A run is left-flanking when it is not followed by whitespace, and either is not followed by
/// punctuation, or is followed by punctuation *and* preceded by whitespace or punctuation. The
/// start or end of a line counts as whitespace.
///
/// This is why `plain*`code`plain*` is not emphasis: the opening `*` follows `n` (alphanumeric)
/// and precedes a backtick (punctuation), so neither branch holds.
fn is_left_flanking(before: Option<char>, after: Option<char>) -> bool {
    let Some(after) = after else {
        return false; // end of line
    };
    if after.is_whitespace() {
        return false;
    }
    if !is_punctuation(after) {
        return true;
    }
    match before {
        None => true, // start of line
        Some(c) => c.is_whitespace() || is_punctuation(c),
    }
}

/// Whether a delimiter run placed here could **close** emphasis (CommonMark "right-flanking").
///
/// The mirror of [`is_left_flanking`]: not preceded by whitespace, and either not preceded by
/// punctuation, or preceded by punctuation *and* followed by whitespace or punctuation.
fn is_right_flanking(before: Option<char>, after: Option<char>) -> bool {
    let Some(before) = before else {
        return false; // start of line
    };
    if before.is_whitespace() {
        return false;
    }
    if !is_punctuation(before) {
        return true;
    }
    match after {
        None => true, // end of line
        Some(c) => c.is_whitespace() || is_punctuation(c),
    }
}

/// CommonMark's "Unicode punctuation character", approximated without a Unicode tables crate.
///
/// ASCII punctuation is exact; beyond ASCII, anything that is neither alphanumeric nor whitespace
/// is treated as punctuation, which matches the Unicode P* and S* categories closely enough that
/// the only cost of a miss is a mark dropped that could have been kept.
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

/// Predict the first character `inlines` will emit, falling back to `fallback` when it is empty.
///
/// Only the character's *class* matters to the flanking rules, and escaping only ever inserts a
/// backslash before ASCII punctuation — punctuation either way — so the raw character is enough.
/// Where a nested mark's fate is not yet decided, the answer is biased towards the class that
/// makes flanking *harder*, so this never talks a caller into emitting a delimiter that will not
/// parse.
fn first_char_of(inlines: &[Inline], fallback: Option<char>) -> Option<char> {
    for node in inlines {
        match node {
            Inline::Text(t) => {
                if let Some(c) = t.chars().next() {
                    return Some(c);
                }
            }
            Inline::Bold(c) | Inline::Italic(c) | Inline::Strike(c) => {
                // A kept mark leads with its delimiter (punctuation, which helps the caller); a
                // dropped one leads with its content. Assume the unhelpful case when they differ.
                return match first_char_of(c, fallback) {
                    Some(inner) if !is_punctuation(inner) && !inner.is_whitespace() => Some(inner),
                    _ => Some('*'),
                };
            }
            Inline::Code(_) => return Some('`'),
            Inline::Link { .. } => return Some('['),
            Inline::SoftBreak => return Some('\n'),
            Inline::HardBreak => return Some('\\'),
        }
    }
    fallback
}

/// Split a mark's children into leading whitespace, emphasizable core, and trailing whitespace.
///
/// When every child is whitespace there is nothing to emphasize, so the core is empty and the
/// whole run is returned as the lead.
fn split_off_whitespace(children: &[Inline]) -> (&[Inline], &[Inline], &[Inline]) {
    let Some(first) = children.iter().position(|n| !is_blank(n)) else {
        return (children, &[], &[]);
    };
    let last = children.iter().rposition(|n| !is_blank(n)).unwrap_or(first);
    (
        &children[..first],
        &children[first..=last],
        &children[last + 1..],
    )
}

/// True for nodes that render as whitespace, and so cannot sit against an emphasis delimiter.
fn is_blank(node: &Inline) -> bool {
    match node {
        Inline::SoftBreak | Inline::HardBreak => true,
        Inline::Text(t) => t.chars().all(char::is_whitespace),
        _ => false,
    }
}

/// Render an inline code span, choosing a backtick fence long enough to contain the content.
///
/// CommonMark strips one leading and one trailing space when the content has both and is not all
/// spaces, and a fence directly against a backtick would be misread as a longer fence. Both cases
/// are handled by padding with a space on each side.
fn render_code_span(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    // A code span cannot contain a line ending; CommonMark turns one into a space, so do it here
    // rather than emitting something that reads back differently.
    let content = content.replace('\n', " ");

    let fence: String = std::iter::repeat_n('`', longest_run(&content, '`') + 1).collect();
    let all_spaces = content.chars().all(|c| c == ' ');
    let needs_padding = content.starts_with('`')
        || content.ends_with('`')
        || (content.starts_with(' ') && content.ends_with(' ') && !all_spaces);

    if needs_padding {
        format!("{fence} {content} {fence}")
    } else {
        format!("{fence}{content}{fence}")
    }
}

/// Render a link destination, using the `<...>` form when the URL contains whitespace.
fn render_dest(href: &str) -> String {
    if href.is_empty() {
        return "<>".to_string();
    }
    if href
        .chars()
        .any(|c| c.is_whitespace() || c == '<' || c == '>')
    {
        let inner = href
            .replace('\\', "\\\\")
            .replace('<', "\\<")
            .replace('>', "\\>");
        format!("<{inner}>")
    } else {
        href.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    }
}

fn escape_title(title: &str) -> String {
    title.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::model::{List, ListItem};
    use crate::markdown::parse::parse;

    fn doc(blocks: Vec<Block>) -> Document {
        Document::from_blocks(blocks)
    }

    fn md(blocks: Vec<Block>) -> String {
        render(&doc(blocks))
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    // ---- blocks -----------------------------------------------------------------------------

    #[test]
    fn an_empty_document_renders_to_nothing() {
        assert_eq!(render(&Document::new()), "");
    }

    #[test]
    fn blocks_are_separated_by_exactly_one_blank_line_and_there_is_no_trailing_newline() {
        let out = md(vec![Block::para("a"), Block::para("b"), Block::para("c")]);
        assert_eq!(out, "a\n\nb\n\nc");
        assert!(!out.ends_with('\n'));
    }

    #[test]
    fn headings() {
        assert_eq!(
            md(vec![
                Block::heading(1, vec![text("one")]),
                Block::heading(2, vec![text("two")]),
                Block::heading(3, vec![text("three")]),
            ]),
            "# one\n\n## two\n\n### three"
        );
    }

    #[test]
    fn a_heading_ending_in_a_hash_is_escaped_so_it_is_not_a_closing_sequence() {
        assert_eq!(md(vec![Block::heading(1, vec![text("C#")])]), r"# C\#");
    }

    #[test]
    fn thematic_break() {
        assert_eq!(md(vec![Block::para("a"), Block::ThematicBreak]), "a\n\n---");
    }

    #[test]
    fn code_block_with_a_language() {
        assert_eq!(
            md(vec![Block::CodeBlock {
                lang: Some("rust".into()),
                code: "let x = 1;".into(),
            }]),
            "```rust\nlet x = 1;\n```"
        );
    }

    #[test]
    fn code_block_fence_grows_past_any_backtick_run_inside() {
        assert_eq!(
            md(vec![Block::CodeBlock {
                lang: None,
                code: "a\n```\nb".into(),
            }]),
            "````\na\n```\nb\n````"
        );
    }

    #[test]
    fn blockquote_prefixes_every_line_including_the_blank_ones() {
        assert_eq!(
            md(vec![Block::BlockQuote(vec![
                Block::para("one"),
                Block::para("two"),
            ])]),
            "> one\n>\n> two"
        );
    }

    #[test]
    fn nested_blockquotes() {
        assert_eq!(
            md(vec![Block::BlockQuote(vec![Block::BlockQuote(vec![
                Block::para("deep"),
            ])])]),
            "> > deep"
        );
    }

    #[test]
    fn unsupported_blocks_render_their_fallback_text() {
        assert_eq!(
            md(vec![Block::Unsupported {
                kind: "table".into(),
                fallback: vec![text("a | b")],
            }]),
            "a | b"
        );
    }

    // ---- tables -----------------------------------------------------------------------------

    fn cell(s: &str) -> Vec<Inline> {
        vec![text(s)]
    }

    fn row(cells: &[&str]) -> Row {
        cells.iter().map(|c| cell(c)).collect()
    }

    /// Render a table straight from the model, without normalization, so shapes the parser cannot
    /// produce (ragged rows, a missing header) can be tested.
    fn raw_table(table: Table) -> String {
        render(&Document {
            blocks: vec![Block::Table(table)],
        })
    }

    #[test]
    fn a_table_is_padded_so_the_columns_line_up() {
        assert_eq!(
            raw_table(Table {
                head: row(&["name", "n"]),
                align: vec![],
                rows: vec![row(&["a much longer cell", "1"])],
            }),
            "\
| name               | n   |
| ------------------ | --- |
| a much longer cell | 1   |"
        );
    }

    #[test]
    fn a_narrow_column_still_gets_three_dashes() {
        // Anything narrower cannot carry `:-:`, and a delimiter row wider than its data reads as a
        // mistake.
        assert_eq!(
            raw_table(Table {
                head: row(&["a"]),
                align: vec![],
                rows: vec![row(&["b"])],
            }),
            "| a   |\n| --- |\n| b   |"
        );
    }

    #[test]
    fn alignment_becomes_colons_and_moves_the_padding() {
        assert_eq!(
            raw_table(Table {
                head: row(&["l", "c", "r", "n"]),
                align: vec![
                    Alignment::Left,
                    Alignment::Center,
                    Alignment::Right,
                    Alignment::None,
                ],
                rows: vec![row(&[
                    "wide left",
                    "wide centre",
                    "wide right",
                    "wide none"
                ])],
            }),
            "\
| l         |      c      |          r | n         |
| :-------- | :---------: | ---------: | --------- |
| wide left | wide centre | wide right | wide none |"
        );
    }

    #[test]
    fn alignment_round_trips_through_a_reparse() {
        let original = doc(vec![Block::Table(Table {
            head: row(&["l", "c", "r", "n"]),
            align: vec![
                Alignment::Left,
                Alignment::Center,
                Alignment::Right,
                Alignment::None,
            ],
            rows: vec![row(&["1", "2", "3", "4"])],
        })]);
        let once = render(&original);
        assert_eq!(parse(&once), original, "--- rendered ---\n{once}");
        assert_eq!(render(&parse(&once)), once);
    }

    #[test]
    fn a_column_with_no_alignment_recorded_defaults_to_none() {
        // `align` is allowed to be shorter than the widest row; the missing entries are `None`.
        let out = raw_table(Table {
            head: row(&["a", "b"]),
            align: vec![Alignment::Right],
            rows: vec![row(&["1", "2"])],
        });
        assert_eq!(out.lines().nth(1), Some("| --: | --- |"));
    }

    #[test]
    fn a_pipe_in_a_cell_is_escaped() {
        let out = raw_table(Table {
            head: row(&["a | b"]),
            align: vec![],
            rows: vec![row(&["c|d"])],
        });
        assert_eq!(out, "| a \\| b |\n| ------ |\n| c\\|d   |");
        // And it reads back as content, not as another column.
        let reparsed = parse(&out);
        let [Block::Table(t)] = &reparsed.blocks[..] else {
            panic!("expected a table, got {reparsed:?}");
        };
        assert_eq!(t.columns(), 1);
        assert_eq!(t.head, vec![cell("a | b")]);
        assert_eq!(t.rows[0], vec![cell("c|d")]);
    }

    #[test]
    fn a_pipe_inside_a_code_span_is_escaped_too() {
        // GFM splits the row on pipes before it looks for code spans, so the backticks do not
        // protect it.
        let out = raw_table(Table {
            head: vec![vec![Inline::Code("a|b".into())]],
            align: vec![],
            rows: vec![],
        });
        assert_eq!(out, "| `a\\|b` |\n| ------ |");
        let reparsed = parse(&out);
        let [Block::Table(t)] = &reparsed.blocks[..] else {
            panic!("expected a table, got {reparsed:?}");
        };
        assert_eq!(t.head, vec![vec![Inline::Code("a|b".into())]]);
    }

    #[test]
    fn ragged_rows_are_padded_out_to_the_widest_row() {
        // The model tolerates ragged rows because HTML tables in the wild are; GFM does not, so a
        // short row is filled with empty cells rather than shifting the columns.
        assert_eq!(
            raw_table(Table {
                head: row(&["a"]),
                align: vec![],
                rows: vec![row(&["1", "2", "3"]), row(&["x"])],
            }),
            "\
| a   |     |     |
| --- | --- | --- |
| 1   | 2   | 3   |
| x   |     |     |"
        );
    }

    #[test]
    fn an_empty_header_still_emits_a_header_row_so_the_output_parses() {
        // GFM has no headerless table. Promoting the first body row would change the document, so
        // an empty header row goes out instead.
        let out = raw_table(Table {
            head: vec![],
            align: vec![],
            rows: vec![row(&["a", "b"]), row(&["c", "d"])],
        });
        assert_eq!(
            out,
            "|     |     |\n| --- | --- |\n| a   | b   |\n| c   | d   |"
        );

        let reparsed = parse(&out);
        let [Block::Table(t)] = &reparsed.blocks[..] else {
            panic!("expected a table, got {reparsed:?}");
        };
        assert_eq!(t.rows.len(), 2, "no body row was eaten by the header");
        assert_eq!(t.rows[0], row(&["a", "b"]));
    }

    #[test]
    fn a_table_with_nothing_in_it_renders_to_nothing() {
        assert_eq!(raw_table(Table::default()), "");
    }

    #[test]
    fn cells_carry_inline_marks_a_link_and_code() {
        assert_eq!(
            raw_table(Table {
                head: vec![
                    vec![Inline::bold("bold")],
                    vec![Inline::italic("it")],
                    vec![Inline::Strike(vec![text("s")])],
                ],
                align: vec![],
                rows: vec![vec![
                    vec![Inline::Link {
                        href: "http://e.com".into(),
                        title: Some("t".into()),
                        content: vec![text("link")],
                    }],
                    vec![Inline::Code("x".into())],
                    vec![text("plain")],
                ]],
            }),
            "\
| **bold**                 | *it* | ~~s~~ |
| ------------------------ | ---- | ----- |
| [link](http://e.com \"t\") | `x`  | plain |"
        );
    }

    #[test]
    fn a_break_inside_a_cell_becomes_a_space() {
        // A row is one line. `<br>` would be raw HTML that the parser reads back as literal text
        // and the next render escapes, so the pane would change under the user; a space converges.
        assert_eq!(
            raw_table(Table {
                head: row(&["h"]),
                align: vec![],
                rows: vec![vec![vec![
                    text("a"),
                    Inline::HardBreak,
                    text("b"),
                    Inline::SoftBreak,
                    text("c"),
                ]]],
            }),
            "| h     |\n| ----- |\n| a b c |"
        );
    }

    #[test]
    fn a_newline_that_reaches_a_cell_anyway_becomes_a_space() {
        // The model says a `Text` never holds a newline, but `render` takes any `&Document`, and a
        // newline here would end the row and shear the table in half.
        assert_eq!(
            raw_table(Table {
                head: row(&["h"]),
                align: vec![],
                rows: vec![vec![vec![text("a\nb")]]],
            }),
            "| h   |\n| --- |\n| a b |"
        );
    }

    #[test]
    fn a_cell_beginning_with_a_block_marker_needs_no_escape() {
        // A cell is never the start of a line — the `|` is — so `#` and `-` are just text.
        let out = raw_table(Table {
            head: row(&["# not a heading", "- not a bullet"]),
            align: vec![],
            rows: vec![],
        });
        assert!(out.starts_with("| # not a heading | - not a bullet |"));
        assert_eq!(parse(&out).blocks.len(), 1);
    }

    #[test]
    fn a_table_inside_a_blockquote_keeps_its_quote_marker_on_every_line() {
        assert_eq!(
            md(vec![Block::BlockQuote(vec![Block::Table(Table {
                head: row(&["a", "b"]),
                align: vec![],
                rows: vec![row(&["1", "2"])],
            })])]),
            "> | a   | b   |\n> | --- | --- |\n> | 1   | 2   |"
        );
    }

    #[test]
    fn round_trip_a_table_from_markdown_and_back() {
        let src = "\
| Name | Qty | Price |
| :--- | --: | :---: |
| Bolt | 12 | 0.10 |
| Long widget name | 3 | 11.00 |";
        let once = render(&parse(src));
        assert_eq!(
            once,
            "\
| Name             | Qty | Price |
| :--------------- | --: | :---: |
| Bolt             |  12 | 0.10  |
| Long widget name |   3 | 11.00 |"
        );
        // Markdown -> AST -> Markdown -> AST is a fixpoint in both directions.
        assert_eq!(parse(&once), parse(src));
        assert_eq!(render(&parse(&once)), once);
    }

    // ---- lists ------------------------------------------------------------------------------

    #[test]
    fn tight_bullet_list() {
        assert_eq!(
            md(vec![Block::List(List::bulleted(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
            ]))]),
            "- a\n- b"
        );
    }

    #[test]
    fn loose_lists_get_a_blank_line_between_items() {
        let list = List {
            tight: false,
            ..List::bulleted(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
            ])
        };
        assert_eq!(md(vec![Block::List(list)]), "- a\n\n- b");
    }

    #[test]
    fn ordered_list_respects_start() {
        let list = List {
            start: 5,
            ..List::numbered(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
            ])
        };
        assert_eq!(md(vec![Block::List(list)]), "5. a\n6. b");
    }

    #[test]
    fn ordered_list_continuation_aligns_with_a_widening_marker() {
        let list = List {
            start: 9,
            tight: false,
            ..List::numbered(vec![
                ListItem {
                    blocks: vec![Block::para("a"), Block::para("still a")],
                },
                ListItem {
                    blocks: vec![Block::para("b"), Block::para("still b")],
                },
            ])
        };
        assert_eq!(
            md(vec![Block::List(list)]),
            "9. a\n\n   still a\n\n10. b\n\n    still b"
        );
    }

    #[test]
    fn nested_lists_two_deep_indent_to_the_parent_content_column() {
        let inner = Block::List(List::bulleted(vec![ListItem::of(vec![text("b")])]));
        let outer = Block::List(List::bulleted(vec![
            ListItem {
                blocks: vec![Block::Paragraph(vec![text("a")]), inner],
            },
            ListItem::of(vec![text("c")]),
        ]));
        assert_eq!(md(vec![outer]), "- a\n  - b\n- c");
    }

    // ---- inlines ----------------------------------------------------------------------------

    #[test]
    fn bold_italic_and_strike() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![
                Inline::bold("b"),
                text(" "),
                Inline::italic("i"),
                text(" "),
                Inline::Strike(vec![text("s")]),
            ])]),
            "**b** *i* ~~s~~"
        );
    }

    #[test]
    fn italic_uses_a_star_so_it_works_intraword() {
        // `pre_mid_post` is not emphasis in CommonMark; `pre*mid*post` is.
        assert_eq!(
            md(vec![Block::Paragraph(vec![
                text("pre"),
                Inline::italic("mid"),
                text("post"),
            ])]),
            "pre*mid*post"
        );
    }

    #[test]
    fn bold_around_italic_is_three_stars_so_the_run_flanks_correctly() {
        // `**_x_**` does not open emphasis: a `**` run followed by punctuation only opens when it
        // is itself preceded by whitespace or punctuation, and here it follows `n`.
        let doc = doc(vec![Block::Paragraph(vec![Inline::Strike(vec![
            text("plain"),
            Inline::Bold(vec![Inline::italic("plain")]),
        ])])]);
        let once = render(&doc);
        assert_eq!(once, "~~plain***plain***~~");
        // The property the coordinator asked for: rendering reaches a fixpoint in one pass.
        assert_eq!(render(&parse(&once)), once);
        assert_eq!(parse(&once), doc);
    }

    // ---- emphasis flanking --------------------------------------------------------------------

    #[test]
    fn emphasis_that_could_not_open_is_distributed_rather_than_emitted_unstably() {
        // `plain*`code`plain*` is not emphasis: the opening `*` follows `n` (alphanumeric) and
        // precedes a backtick (punctuation), so it is not left-flanking. Distributing the italic
        // over the two children keeps it on the half that can carry it.
        let doc = doc(vec![Block::Paragraph(vec![Inline::Link {
            href: "https://example.com/p".into(),
            title: None,
            content: vec![
                text("plain"),
                Inline::Italic(vec![Inline::Code("code".into()), text("plain")]),
            ],
        }])]);
        let once = render(&doc);
        assert_eq!(once, "[plain`code`*plain*](https://example.com/p)");
        // The invariant the editor panes depend on: one more pass changes nothing.
        assert_eq!(render(&parse(&once)), once);
    }

    #[test]
    fn emphasis_that_could_not_close_is_distributed_too() {
        // Mirror case: the closing `*` would follow a backtick (punctuation) and precede `p`, so
        // it is not right-flanking.
        let doc = doc(vec![Block::Paragraph(vec![
            Inline::Italic(vec![text("plain"), Inline::Code("code".into())]),
            text("plain"),
        ])]);
        let once = render(&doc);
        assert_eq!(once, "*plain*`code`plain");
        assert_eq!(render(&parse(&once)), once);
    }

    #[test]
    fn a_lone_child_that_cannot_flank_has_its_mark_dropped() {
        // Nothing left to distribute over, so the mark goes. Deterministic and lossy beats
        // unstable.
        let doc = doc(vec![Block::Paragraph(vec![
            text("plain"),
            Inline::Italic(vec![Inline::Code("code".into())]),
        ])]);
        let once = render(&doc);
        assert_eq!(once, "plain`code`");
        assert_eq!(render(&parse(&once)), once);
    }

    #[test]
    fn the_flanking_check_is_not_over_broad_and_normal_emphasis_still_works() {
        for blocks in [
            vec![Block::Paragraph(vec![Inline::italic("plain")])],
            vec![Block::Paragraph(vec![
                text("a "),
                Inline::bold("b"),
                text(" c"),
            ])],
            vec![Block::Paragraph(vec![
                text("pre"),
                Inline::italic("mid"),
                text("post"),
            ])],
            vec![Block::Paragraph(vec![Inline::Strike(vec![
                text("plain"),
                Inline::Bold(vec![Inline::italic("plain")]),
            ])])],
            vec![Block::Paragraph(vec![Inline::Italic(vec![Inline::Code(
                "code".into(),
            )])])],
        ] {
            let doc = doc(blocks);
            let once = render(&doc);
            assert!(
                once.contains('*') || once.contains('~'),
                "emphasis was dropped from {once:?}"
            );
            assert_eq!(parse(&once), doc, "round trip failed for {once:?}");
        }
    }

    #[test]
    fn a_mark_wrapping_only_whitespace_emits_no_delimiters() {
        // `**\n**` is not emphasis; emitting it would read back as four literal asterisks. The
        // normalizer removes this shape, but `render` takes any `&Document`.
        let raw = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Bold(vec![
                Inline::SoftBreak,
            ])])],
        };
        assert_eq!(render(&raw), "\n");
    }

    #[test]
    fn whitespace_is_hoisted_out_of_a_mark_at_render_time() {
        let raw = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Bold(vec![
                Inline::SoftBreak,
                text("x"),
                text(" "),
            ])])],
        };
        // The hoisted trailing space is then dropped as end-of-line whitespace.
        assert_eq!(render(&raw), "\n**x**");
    }

    #[test]
    fn trailing_spaces_before_a_break_are_dropped() {
        // Two trailing spaces before a newline *are* a hard break in CommonMark, so leaving them
        // would invent a line break the document never had.
        let raw = Document {
            blocks: vec![Block::Paragraph(vec![
                text("a  "),
                Inline::SoftBreak,
                text("b"),
            ])],
        };
        assert_eq!(render(&raw), "a\nb");
    }

    #[test]
    fn a_mark_nested_inside_the_same_mark_is_flattened() {
        // `**a**b****` is not bold-inside-bold, it is four stray asterisks.
        let raw = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Bold(vec![
                text("a"),
                Inline::Bold(vec![text("b")]),
            ])])],
        };
        assert_eq!(render(&raw), "**ab**");
    }

    #[test]
    fn a_link_inside_a_link_keeps_only_the_outer_one() {
        let raw = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Link {
                href: "outer".into(),
                title: None,
                content: vec![Inline::link("inner", "text")],
            }])],
        };
        assert_eq!(render(&raw), "[text](outer)");
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Code("x".into())])]),
            "`x`"
        );
    }

    #[test]
    fn inline_code_containing_backticks_gets_a_longer_fence() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Code("a`b".into())])]),
            "``a`b``"
        );
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Code("a``b".into())])]),
            "```a``b```"
        );
    }

    #[test]
    fn inline_code_starting_or_ending_with_a_backtick_is_padded() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Code("`".into())])]),
            "`` ` ``"
        );
    }

    #[test]
    fn inline_code_wrapped_in_spaces_is_padded_so_the_spaces_survive() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Code(" x ".into())])]),
            "`  x  `"
        );
    }

    #[test]
    fn links() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::link(
                "http://e.com",
                "text"
            )])]),
            "[text](http://e.com)"
        );
    }

    #[test]
    fn link_with_a_title() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::Link {
                href: "u".into(),
                title: Some("t".into()),
                content: vec![text("x")],
            }])]),
            r#"[x](u "t")"#
        );
    }

    #[test]
    fn link_text_containing_a_bracket_is_escaped() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::link("u", "a]b")])]),
            r"[a\]b](u)"
        );
    }

    #[test]
    fn link_destination_with_a_space_uses_angle_brackets() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![Inline::link("a b", "t")])]),
            "[t](<a b>)"
        );
    }

    #[test]
    fn hard_break_is_a_trailing_backslash() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![
                text("a"),
                Inline::HardBreak,
                text("b"),
            ])]),
            "a\\\nb"
        );
    }

    #[test]
    fn soft_break_is_a_bare_newline() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![
                text("a"),
                Inline::SoftBreak,
                text("b"),
            ])]),
            "a\nb"
        );
    }

    // ---- escaping in context ----------------------------------------------------------------

    #[test]
    fn a_paragraph_beginning_with_a_hash_is_not_a_heading() {
        assert_eq!(
            md(vec![Block::para("# not a heading")]),
            r"\# not a heading"
        );
    }

    #[test]
    fn snake_case_words_are_left_alone() {
        assert_eq!(
            md(vec![Block::para("call snake_case_word now")]),
            "call snake_case_word now"
        );
    }

    #[test]
    fn text_after_a_soft_break_gets_line_start_escaping() {
        assert_eq!(
            md(vec![Block::Paragraph(vec![
                text("a"),
                Inline::SoftBreak,
                text("- not a bullet"),
            ])]),
            "a\n\\- not a bullet"
        );
    }

    #[test]
    fn the_first_line_of_a_list_item_still_gets_line_start_escaping() {
        assert_eq!(
            md(vec![Block::List(List::bulleted(vec![ListItem::of(vec![
                text("> not a quote")
            ])]))]),
            r"- \> not a quote"
        );
    }

    // ---- round trips ------------------------------------------------------------------------

    fn round_trip(blocks: Vec<Block>) {
        let original = doc(blocks);
        let rendered = render(&original);
        let reparsed = parse(&rendered);
        assert_eq!(
            reparsed, original,
            "round trip diverged.\n--- rendered ---\n{rendered}\n--- end ---"
        );
    }

    #[test]
    fn round_trip_marks_and_links() {
        round_trip(vec![Block::Paragraph(vec![
            text("plain "),
            Inline::bold("b"),
            text(" "),
            Inline::italic("i"),
            text(" "),
            Inline::Strike(vec![text("s")]),
            text(" "),
            Inline::Code("co`de".into()),
            text(" "),
            Inline::Link {
                href: "http://e.com/x(y)".into(),
                title: Some("a title".into()),
                content: vec![text("link ]text")],
            },
        ])]);
    }

    #[test]
    fn round_trip_headings_and_paragraphs() {
        round_trip(vec![
            Block::heading(1, vec![text("Title")]),
            Block::para("Body text."),
            Block::heading(3, vec![text("Sub")]),
            Block::para("More."),
        ]);
    }

    #[test]
    fn round_trip_nested_tight_list() {
        let inner = Block::List(List::bulleted(vec![
            ListItem::of(vec![text("b")]),
            ListItem::of(vec![text("c")]),
        ]));
        round_trip(vec![Block::List(List::bulleted(vec![
            ListItem {
                blocks: vec![Block::Paragraph(vec![text("a")]), inner],
            },
            ListItem::of(vec![text("d")]),
        ]))]);
    }

    #[test]
    fn round_trip_ordered_list_starting_at_five() {
        round_trip(vec![Block::List(List {
            start: 5,
            ..List::numbered(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
                ListItem::of(vec![text("c")]),
            ])
        })]);
    }

    #[test]
    fn round_trip_loose_list() {
        round_trip(vec![Block::List(List {
            tight: false,
            ..List::bulleted(vec![
                ListItem::of(vec![text("a")]),
                ListItem::of(vec![text("b")]),
            ])
        })]);
    }

    #[test]
    fn round_trip_blockquote_and_code_block() {
        round_trip(vec![
            Block::BlockQuote(vec![Block::para("quoted one"), Block::para("quoted two")]),
            Block::CodeBlock {
                lang: Some("rust".into()),
                code: "fn main() {\n    let s = \"```\";\n}".into(),
            },
            Block::ThematicBreak,
        ]);
    }

    #[test]
    fn round_trip_awkward_text() {
        round_trip(vec![
            Block::para("# not a heading"),
            Block::para("1. not a list"),
            Block::para("snake_case_word and a*star and <html> and &amp; and ~~tilde~~"),
            Block::para("--- not a break"),
            Block::para(r"a backslash \ and a [bracket]"),
        ]);
    }

    #[test]
    fn round_trip_breaks_inside_a_paragraph() {
        round_trip(vec![Block::Paragraph(vec![
            text("one"),
            Inline::HardBreak,
            text("two"),
            Inline::SoftBreak,
            text("three"),
        ])]);
    }

    #[test]
    fn round_trip_a_whole_mixed_document() {
        round_trip(vec![
            Block::heading(1, vec![text("Richochet")]),
            Block::Paragraph(vec![
                text("Converts "),
                Inline::bold("rich text"),
                text(" to "),
                Inline::italic("Markdown"),
                text("."),
            ]),
            Block::List(List::bulleted(vec![
                ListItem::of(vec![text("one")]),
                ListItem {
                    blocks: vec![
                        Block::Paragraph(vec![text("two")]),
                        Block::List(List::bulleted(vec![ListItem::of(vec![Inline::Code(
                            "nested".into(),
                        )])])),
                    ],
                },
            ])),
            Block::BlockQuote(vec![Block::para("A quote.")]),
            Block::CodeBlock {
                lang: None,
                code: "plain code".into(),
            },
            Block::ThematicBreak,
            Block::Paragraph(vec![Inline::link("http://e.com", "http://e.com")]),
        ]);
    }

    #[test]
    fn rendering_is_idempotent_through_a_reparse() {
        let original = doc(vec![
            Block::heading(2, vec![text("H")]),
            Block::para("a_b and *c*"),
            Block::List(List::bulleted(vec![ListItem::of(vec![text("x")])])),
        ]);
        let once = render(&original);
        let twice = render(&parse(&once));
        assert_eq!(once, twice);
    }
}
