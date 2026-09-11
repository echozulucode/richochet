//! The normalized document model.
//!
//! This is the **frozen contract** at the centre of Richochet. Markdown, HTML and plain text each
//! have a parser into this model and a renderer out of it. Nothing else is canonical: in
//! particular, HTML is never the source of truth, and Teams-specific markup must never survive
//! past [`crate::html::parse`].
//!
//! See `docs/implementation-plan.md` §2.1 for the invariants the normalizer guarantees.

/// A whole document: an ordered list of block-level nodes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Document {
    /// The document's top-level blocks, in order.
    pub blocks: Vec<Block>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a document from blocks, normalizing it in the process.
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        crate::document::normalize::normalize(Document { blocks })
    }

    /// True when the document has no blocks, or only blocks that render to nothing.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(Block::is_empty)
    }
}

/// A block-level node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "camelCase"))]
pub enum Block {
    /// A run of inline content terminated by a blank line.
    Paragraph(Vec<Inline>),
    /// A heading. `level` is always in `1..=3`; see [`HEADING_MAX`].
    Heading {
        /// Heading depth, clamped to `1..=3`.
        level: u8,
        /// The heading text.
        content: Vec<Inline>,
    },
    /// An ordered or unordered list.
    List(List),
    /// A block quotation, which may contain any blocks including nested quotes.
    BlockQuote(Vec<Block>),
    /// A fenced or indented code block. Content is verbatim, never escaped or normalized.
    CodeBlock {
        /// The info string / language hint, if one was present.
        lang: Option<String>,
        /// Verbatim code, newline-separated, without a trailing newline.
        code: String,
    },
    /// A horizontal rule.
    ThematicBreak,
    /// A table.
    ///
    /// Teams messages carry these routinely — pasted out of Excel or Word — and losing them is the
    /// most visible way a conversion can disappoint. GFM can express them, so the model does too.
    Table(Table),
    /// Something we recognized but cannot represent.
    ///
    /// This is how the model degrades gracefully instead of losing content (tables, mentions,
    /// images, Loop components). Every renderer emits `fallback`; `kind` exists for diagnostics
    /// and for future support.
    Unsupported {
        /// A short identifier for what was dropped, e.g. `"table"` or `"mention"`.
        kind: String,
        /// What renderers emit in its place.
        fallback: Vec<Inline>,
    },
}

/// The deepest heading level the model represents.
///
/// Teams only renders three heading sizes, so deeper headings are clamped rather than dropped.
pub const HEADING_MAX: u8 = 3;

impl Block {
    /// A paragraph of plain text.
    pub fn para<S: Into<String>>(text: S) -> Self {
        Block::Paragraph(vec![Inline::text(text)])
    }

    /// A heading, with `level` clamped into `1..=HEADING_MAX`.
    pub fn heading(level: u8, content: Vec<Inline>) -> Self {
        Block::Heading {
            level: level.clamp(1, HEADING_MAX),
            content,
        }
    }

    /// True when this block would render to nothing at all.
    pub fn is_empty(&self) -> bool {
        match self {
            Block::Paragraph(c) | Block::Unsupported { fallback: c, .. } => {
                c.iter().all(Inline::is_empty)
            }
            Block::Heading { content, .. } => content.iter().all(Inline::is_empty),
            Block::List(l) => l.items.is_empty(),
            Block::BlockQuote(b) => b.iter().all(Block::is_empty),
            Block::CodeBlock { code, .. } => code.is_empty(),
            Block::ThematicBreak => false,
            // A table with no body and no header text renders to nothing worth keeping.
            Block::Table(t) => t
                .rows
                .iter()
                .chain(std::iter::once(&t.head))
                .all(|row| row.iter().all(|cell| cell.iter().all(Inline::is_empty))),
        }
    }
}

/// One table cell's inline content.
pub type Cell = Vec<Inline>;

/// One table row.
pub type Row = Vec<Cell>;

/// How a table column is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum Alignment {
    /// No explicit alignment; the renderer decides.
    #[default]
    None,
    /// Left aligned (`:---`).
    Left,
    /// Centred (`:---:`).
    Center,
    /// Right aligned (`---:`).
    Right,
}

/// A table.
///
/// Rows are not required to be rectangular: HTML tables in the wild routinely are not, and a
/// renderer padding short rows is kinder than a parser rejecting them. `align` is per column and
/// may be shorter than the widest row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Table {
    /// The header row. GFM requires one; an HTML table without a `<thead>` gets an empty header,
    /// and the Markdown renderer emits a blank header row so the result still parses.
    pub head: Row,
    /// Per-column alignment.
    pub align: Vec<Alignment>,
    /// The body rows.
    pub rows: Vec<Row>,
}

impl Table {
    /// The widest row, which is how many columns the table really has.
    pub fn columns(&self) -> usize {
        std::iter::once(self.head.len())
            .chain(self.rows.iter().map(Vec::len))
            .max()
            .unwrap_or(0)
    }

    /// Alignment for a column, defaulting when `align` is short.
    pub fn alignment(&self, column: usize) -> Alignment {
        self.align.get(column).copied().unwrap_or_default()
    }
}

/// An ordered or unordered list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct List {
    /// Whether the list is numbered.
    pub ordered: bool,
    /// The first number of an ordered list. Ignored when `ordered` is false.
    pub start: u64,
    /// Tight lists render without blank lines between items.
    pub tight: bool,
    /// The list's items.
    pub items: Vec<ListItem>,
}

impl List {
    /// An unordered list of the given items.
    pub fn bulleted(items: Vec<ListItem>) -> Self {
        List {
            ordered: false,
            start: 1,
            tight: true,
            items,
        }
    }

    /// An ordered list starting at 1.
    pub fn numbered(items: Vec<ListItem>) -> Self {
        List {
            ordered: true,
            start: 1,
            tight: true,
            items,
        }
    }
}

/// A single list item, which may contain nested blocks (including nested lists).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ListItem {
    /// The item's block content.
    pub blocks: Vec<Block>,
}

impl ListItem {
    /// A list item holding a single paragraph of inline content.
    pub fn of(content: Vec<Inline>) -> Self {
        ListItem {
            blocks: vec![Block::Paragraph(content)],
        }
    }
}

/// An inline node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "camelCase"))]
pub enum Inline {
    /// Literal text. Never contains a newline; use [`Inline::SoftBreak`] or
    /// [`Inline::HardBreak`] instead.
    Text(String),
    /// Strong emphasis.
    Bold(Vec<Inline>),
    /// Emphasis.
    Italic(Vec<Inline>),
    /// Struck-through text.
    Strike(Vec<Inline>),
    /// Inline code. Content is verbatim and carries no nested marks.
    Code(String),
    /// A hyperlink.
    Link {
        /// The target URL.
        href: String,
        /// An optional title attribute.
        title: Option<String>,
        /// The link's visible content.
        content: Vec<Inline>,
    },
    /// A newline in the source that renders as a space.
    SoftBreak,
    /// An explicit line break within a paragraph (Shift+Enter in Teams, `<br>` in HTML).
    HardBreak,
}

impl Inline {
    /// Literal text.
    pub fn text<S: Into<String>>(s: S) -> Self {
        Inline::Text(s.into())
    }

    /// Bold text.
    pub fn bold<S: Into<String>>(s: S) -> Self {
        Inline::Bold(vec![Inline::text(s)])
    }

    /// Italic text.
    pub fn italic<S: Into<String>>(s: S) -> Self {
        Inline::Italic(vec![Inline::text(s)])
    }

    /// A link with plain-text content.
    pub fn link<H: Into<String>, S: Into<String>>(href: H, text: S) -> Self {
        Inline::Link {
            href: href.into(),
            title: None,
            content: vec![Inline::text(text)],
        }
    }

    /// True when this node would render to nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Inline::Text(t) => t.is_empty(),
            Inline::Code(c) => c.is_empty(),
            Inline::Bold(c) | Inline::Italic(c) | Inline::Strike(c) => {
                c.iter().all(Inline::is_empty)
            }
            // A link is empty only when it has neither a destination nor anything to show. An
            // empty href alone must NOT make it empty: `<a href="">text</a>` is what Word and
            // Outlook emit for bookmark anchors, and treating it as empty deleted the words
            // inside it. Found by CommonMark examples 200/485/486/567.
            Inline::Link { href, content, .. } => {
                href.is_empty() && content.iter().all(Inline::is_empty)
            }
            Inline::SoftBreak | Inline::HardBreak => false,
        }
    }

    /// The children of a mark node, if this is one.
    pub fn children(&self) -> Option<&[Inline]> {
        match self {
            Inline::Bold(c) | Inline::Italic(c) | Inline::Strike(c) => Some(c),
            Inline::Link { content, .. } => Some(content),
            _ => None,
        }
    }

    /// Rebuild this node with different children. Non-mark nodes are returned unchanged.
    pub fn with_children(&self, children: Vec<Inline>) -> Inline {
        match self {
            Inline::Bold(_) => Inline::Bold(children),
            Inline::Italic(_) => Inline::Italic(children),
            Inline::Strike(_) => Inline::Strike(children),
            Inline::Link { href, title, .. } => Inline::Link {
                href: href.clone(),
                title: title.clone(),
                content: children,
            },
            other => other.clone(),
        }
    }

    /// The canonical nesting rank of a mark, outermost first.
    ///
    /// Used by the normalizer to put marks in a stable order so that round trips converge.
    /// `Bold` outside `Italic` outside `Strike`; links always sit outside the marks they carry.
    pub fn mark_rank(&self) -> Option<u8> {
        match self {
            Inline::Link { .. } => Some(0),
            Inline::Bold(_) => Some(1),
            Inline::Italic(_) => Some(2),
            Inline::Strike(_) => Some(3),
            _ => None,
        }
    }
}
