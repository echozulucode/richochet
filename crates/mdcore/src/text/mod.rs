//! Plain text <-> document model.
//!
//! Plain text is lossy by definition: it carries no marks. Parsing recovers only block structure,
//! and rendering strips formatting while keeping the shape of the document readable — which is
//! what someone pasting into a plain-text field actually wants.

use crate::document::model::{Block, Document, Inline, List, ListItem, Table};
use crate::document::visit::inline_text;

/// Parse plain text into the document model.
///
/// Blank lines separate paragraphs. A single newline inside a paragraph becomes a
/// [`Inline::HardBreak`], because in plain text a line break is deliberate and should survive the
/// round trip rather than being reflowed away.
pub fn parse(input: &str) -> Document {
    let mut blocks = Vec::new();
    let mut current: Vec<Inline> = Vec::new();

    for line in input.replace("\r\n", "\n").split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut current)));
            }
            continue;
        }
        if !current.is_empty() {
            current.push(Inline::HardBreak);
        }
        current.push(Inline::Text(line.to_string()));
    }
    if !current.is_empty() {
        blocks.push(Block::Paragraph(current));
    }

    Document::from_blocks(blocks)
}

/// Render a document as unformatted plain text.
pub fn render(doc: &Document) -> String {
    let mut out = String::new();
    render_blocks(&doc.blocks, 0, &mut out);
    // Collapse the trailing blank line left by the last block.
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_blocks(blocks: &[Block], depth: usize, out: &mut String) {
    for block in blocks {
        match block {
            Block::Paragraph(c) | Block::Unsupported { fallback: c, .. } => {
                push_indented(&inline_text(c), depth, out);
                out.push('\n');
            }
            Block::Heading { content, .. } => {
                push_indented(&inline_text(content), depth, out);
                out.push('\n');
            }
            Block::List(list) => {
                render_list(list, depth, out);
                out.push('\n');
            }
            Block::BlockQuote(inner) => {
                let mut quoted = String::new();
                render_blocks(inner, 0, &mut quoted);
                for line in quoted.trim_end().split('\n') {
                    push_indented(&format!("> {line}"), depth, out);
                }
                out.push('\n');
            }
            Block::CodeBlock { code, .. } => {
                for line in code.split('\n') {
                    push_indented(line, depth, out);
                }
                out.push('\n');
            }
            Block::ThematicBreak => {
                push_indented("---", depth, out);
                out.push('\n');
            }
            Block::Table(table) => {
                render_table(table, depth, out);
                out.push('\n');
            }
        }
    }
}

/// Lay a table out as padded columns.
///
/// Plain text has no table syntax, so the only thing that keeps a table readable is alignment.
/// Pipes would just be Markdown leaking into output whose whole purpose is to have no markup.
fn render_table(table: &Table, depth: usize, out: &mut String) {
    let columns = table.columns();
    if columns == 0 {
        return;
    }

    let rows: Vec<Vec<String>> = std::iter::once(&table.head)
        .chain(table.rows.iter())
        .filter(|row| !row.is_empty())
        .map(|row| {
            (0..columns)
                .map(|i| row.get(i).map(|c| inline_text(c)).unwrap_or_default())
                .collect()
        })
        .collect();

    let widths: Vec<usize> = (0..columns)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();

    for row in &rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            line.push_str(cell);
            // No trailing padding on the last column: it would be invisible whitespace.
            if i + 1 < columns {
                for _ in 0..widths[i].saturating_sub(cell.chars().count()) {
                    line.push(' ');
                }
            }
        }
        push_indented(line.trim_end(), depth, out);
    }
}

fn render_list(list: &List, depth: usize, out: &mut String) {
    for (i, item) in list.items.iter().enumerate() {
        let marker = if list.ordered {
            format!("{}. ", list.start + i as u64)
        } else {
            "• ".to_string()
        };
        render_item(item, &marker, depth, list.tight, out);
    }
}

fn render_item(item: &ListItem, marker: &str, depth: usize, tight: bool, out: &mut String) {
    let mut body = String::new();
    render_blocks(&item.blocks, 0, &mut body);
    let body = body.trim_end_matches('\n');

    // A tight list has no blank lines between an item's blocks; a loose one keeps them. Either
    // way a blank line is never indented — trailing whitespace on an empty line is just noise.
    let lines = body.split('\n').filter(|l| !(tight && l.trim().is_empty()));

    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            out.push('\n');
        } else if i == 0 {
            push_indented(&format!("{marker}{line}"), depth, out);
        } else {
            // Continuation lines align under the first line's text.
            push_indented(
                &format!("{}{}", " ".repeat(marker.chars().count()), line),
                depth,
                out,
            );
        }
    }
}

fn push_indented(text: &str, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str(text);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_lines_separate_paragraphs() {
        let doc = parse("one\n\ntwo");
        assert_eq!(doc.blocks, vec![Block::para("one"), Block::para("two")]);
    }

    #[test]
    fn a_single_newline_becomes_a_hard_break() {
        let doc = parse("one\ntwo");
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph(vec![
                Inline::Text("one".into()),
                Inline::HardBreak,
                Inline::Text("two".into()),
            ])]
        );
    }

    #[test]
    fn handles_crlf() {
        assert_eq!(parse("a\r\n\r\nb"), parse("a\n\nb"));
    }

    #[test]
    fn rendering_strips_marks() {
        // The example from docs/plan.md: **Important** becomes Important.
        let doc = Document {
            blocks: vec![Block::Paragraph(vec![Inline::bold("Important")])],
        };
        assert_eq!(render(&doc), "Important");
    }

    #[test]
    fn renders_bullets_and_numbers() {
        let doc = Document {
            blocks: vec![
                Block::List(List::bulleted(vec![
                    ListItem::of(vec![Inline::text("one")]),
                    ListItem::of(vec![Inline::text("two")]),
                ])),
                Block::List(List {
                    start: 3,
                    ..List::numbered(vec![ListItem::of(vec![Inline::text("third")])])
                }),
            ],
        };
        assert_eq!(render(&doc), "• one\n• two\n\n3. third");
    }

    #[test]
    fn renders_nested_lists_indented() {
        let doc = Document {
            blocks: vec![Block::List(List::bulleted(vec![ListItem {
                blocks: vec![
                    Block::para("outer"),
                    Block::List(List::bulleted(vec![ListItem::of(vec![Inline::text(
                        "inner",
                    )])])),
                ],
            }]))],
        };
        assert_eq!(render(&doc), "• outer\n  • inner");
    }

    #[test]
    fn renders_blockquotes_with_a_prefix() {
        let doc = Document {
            blocks: vec![Block::BlockQuote(vec![Block::para("quoted")])],
        };
        assert_eq!(render(&doc), "> quoted");
    }

    #[test]
    fn code_blocks_render_verbatim() {
        let doc = Document {
            blocks: vec![Block::CodeBlock {
                lang: Some("rust".into()),
                code: "fn main() {}".into(),
            }],
        };
        assert_eq!(render(&doc), "fn main() {}");
    }

    #[test]
    fn round_trips_plain_paragraphs() {
        let src = "one\n\ntwo\nthree";
        assert_eq!(render(&parse(src)), src);
    }
}
