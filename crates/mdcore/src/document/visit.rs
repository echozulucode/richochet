//! Shared traversal helpers over the document model.

use crate::document::model::{Block, Inline};

/// Call `f` on every inline node in `blocks`, depth first, marks before their children.
pub fn walk_inlines<F: FnMut(&Inline)>(blocks: &[Block], f: &mut F) {
    for block in blocks {
        match block {
            Block::Paragraph(c) | Block::Unsupported { fallback: c, .. } => walk_inline_slice(c, f),
            Block::Heading { content, .. } => walk_inline_slice(content, f),
            Block::BlockQuote(b) => walk_inlines(b, f),
            Block::List(l) => {
                for item in &l.items {
                    walk_inlines(&item.blocks, f);
                }
            }
            Block::CodeBlock { .. } | Block::ThematicBreak => {}
        }
    }
}

fn walk_inline_slice<F: FnMut(&Inline)>(inlines: &[Inline], f: &mut F) {
    for node in inlines {
        f(node);
        if let Some(children) = node.children() {
            walk_inline_slice(children, f);
        }
    }
}

/// Collect the plain text of an inline run, ignoring all formatting.
pub fn inline_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    push_inline_text(inlines, &mut out);
    out
}

fn push_inline_text(inlines: &[Inline], out: &mut String) {
    for node in inlines {
        match node {
            Inline::Text(t) => out.push_str(t),
            Inline::Code(c) => out.push_str(c),
            Inline::SoftBreak => out.push(' '),
            Inline::HardBreak => out.push('\n'),
            other => {
                if let Some(children) = other.children() {
                    push_inline_text(children, out);
                }
            }
        }
    }
}
