//! Document model -> HTML, under a [`RenderProfile`].
//!
//! Every dialect decision this module makes is read out of the profile rather than written into
//! the code. That is the whole point of [`RenderProfile`]: when the clipboard spike discovers that
//! Teams drops `<blockquote>` or flattens nested `<ul>`, the fix is a different struct literal and
//! a fixture, not a rewrite of this file.

use crate::document::model::{Alignment, Block, Document, Inline, List, Row, Table};
use crate::profile::{
    CodeBlockStrategy, HeadingStrategy, MarkStyle, NestingStrategy, QuoteStrategy, RenderProfile,
    TableStyle,
};

/// Indentation step, in pixels, for [`NestingStrategy::MarginIndent`].
const INDENT_PX: usize = 24;

/// Render a document as HTML in the given dialect.
pub fn render(doc: &Document, profile: &RenderProfile) -> String {
    let mut renderer = Renderer {
        out: String::new(),
        profile,
    };
    renderer.blocks(&doc.blocks, Context::default());
    let mut out = renderer.out;
    // `pretty` puts a newline *between* blocks; the last one is an artifact of emitting them
    // uniformly.
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// What the current block inherits from the blocks enclosing it.
#[derive(Debug, Clone, Copy, Default)]
struct Context {
    /// How many `<blockquote>`s deep we are, when quoting is done with a text prefix.
    quote_depth: usize,
    /// How many lists deep we are, when nesting is done with margins.
    list_depth: usize,
}

struct Renderer<'a> {
    out: String,
    profile: &'a RenderProfile,
}

impl Renderer<'_> {
    fn push(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// End a block-level element. In compact mode this is nothing at all: a clipboard payload
    /// wants no stray whitespace, because every byte of it is significant to the receiver.
    fn line_break(&mut self) {
        if self.profile.pretty {
            self.out.push('\n');
        }
    }

    // -----------------------------------------------------------------------------------------
    // Blocks
    // -----------------------------------------------------------------------------------------

    fn blocks(&mut self, blocks: &[Block], ctx: Context) {
        for block in blocks {
            self.block(block, ctx);
        }
    }

    fn block(&mut self, block: &Block, ctx: Context) {
        match block {
            Block::Paragraph(content) => self.paragraph(content, ctx),
            Block::Heading { level, content } => self.heading(*level, content, ctx),
            Block::List(list) => self.list(list, ctx),
            Block::BlockQuote(inner) => self.block_quote(inner, ctx),
            Block::CodeBlock { lang, code } => self.code_block(lang.as_deref(), code),
            Block::Table(table) => self.table(table),
            Block::ThematicBreak => {
                self.push("<hr>");
                self.line_break();
            }
            // The model's graceful degradation: emit the fallback, never nothing.
            Block::Unsupported { fallback, .. } => self.paragraph(fallback, ctx),
        }
    }

    fn paragraph(&mut self, content: &[Inline], ctx: Context) {
        self.push("<p>");
        self.quote_prefix(ctx);
        self.inlines(content);
        self.push("</p>");
        self.line_break();
    }

    fn heading(&mut self, level: u8, content: &[Inline], ctx: Context) {
        match self.profile.headings {
            HeadingStrategy::Native => {
                let tag = format!("h{level}");
                self.push(&format!("<{tag}>"));
                self.quote_prefix(ctx);
                self.inlines(content);
                self.push(&format!("</{tag}>"));
                self.line_break();
            }
            HeadingStrategy::BoldParagraph => {
                self.push("<p>");
                self.quote_prefix(ctx);
                let bold = self.profile.bold.clone();
                self.open_mark(&bold);
                self.inlines(content);
                self.close_mark(&bold);
                self.push("</p>");
                self.line_break();
            }
        }
    }

    fn block_quote(&mut self, inner: &[Block], ctx: Context) {
        match self.profile.blockquote {
            QuoteStrategy::Blockquote => {
                self.push("<blockquote>");
                self.line_break();
                self.blocks(inner, ctx);
                self.push("</blockquote>");
                self.line_break();
            }
            // No quote element at all: the quotation survives as text, the way it would in a
            // plain-text client.
            QuoteStrategy::TextPrefix => self.blocks(
                inner,
                Context {
                    quote_depth: ctx.quote_depth + 1,
                    ..ctx
                },
            ),
        }
    }

    /// The `&gt; ` markers [`QuoteStrategy::TextPrefix`] uses in place of a `<blockquote>`.
    fn quote_prefix(&mut self, ctx: Context) {
        for _ in 0..ctx.quote_depth {
            self.push("&gt; ");
        }
    }

    fn code_block(&mut self, lang: Option<&str>, code: &str) {
        match self.profile.code_block {
            CodeBlockStrategy::PreCode => {
                self.push("<pre><code");
                if let Some(lang) = lang {
                    self.push(&format!(" class=\"language-{}\"", escape_attr(lang)));
                }
                self.push(">");
                self.push(&escape_text(code));
                self.push("</code></pre>");
            }
            CodeBlockStrategy::MonospaceDiv => {
                // `white-space: pre` keeps the indentation if the style survives; the explicit
                // `<br>`s keep the line structure if it does not.
                self.push("<div style=\"font-family:Consolas,monospace;white-space:pre\">");
                let mut first = true;
                for line in code.split('\n') {
                    if !first {
                        self.push("<br>");
                    }
                    self.push(&escape_text(line));
                    first = false;
                }
                self.push("</div>");
            }
        }
        self.line_break();
    }

    // -----------------------------------------------------------------------------------------
    // Tables
    // -----------------------------------------------------------------------------------------

    /// Render a table as real `<table>` markup.
    ///
    /// Alignment is written as an inline `style="text-align:..."` rather than as a class. A
    /// clipboard payload travels without a stylesheet, so a class is a promise the receiving
    /// application cannot keep; `text-align` is understood as-is by mail clients and chat
    /// composers, which is the only place this output is ever read.
    ///
    /// Deliberately carries no quote prefix: under [`QuoteStrategy::TextPrefix`] a `&gt; ` in
    /// every cell would be noise rather than quotation, the same reason a code block skips it.
    fn table(&mut self, table: &Table) {
        let columns = table.columns();
        if columns == 0 {
            return;
        }

        self.push(&format!("<table{}>", self.table_attrs()));
        self.line_break();
        // An empty header is not written out at all: `<thead>` with blank cells reads as a real
        // but nameless header row, where no `<thead>` reads as a table that has none.
        if !table.head.is_empty() {
            self.push("<thead>");
            self.line_break();
            self.table_row(&table.head, "th", table, columns);
            self.push("</thead>");
            self.line_break();
        }
        if !table.rows.is_empty() {
            self.push("<tbody>");
            self.line_break();
            for row in &table.rows {
                self.table_row(row, "td", table, columns);
            }
            self.push("</tbody>");
            self.line_break();
        }
        self.push("</table>");
        self.line_break();
    }

    /// The `<table>` element's own attributes under the current profile.
    fn table_attrs(&self) -> &'static str {
        match self.profile.tables {
            TableStyle::Plain => "",
            // `border-collapse` has to be on the table itself; without it every cell draws its own
            // box and the rules come out doubled.
            TableStyle::Ruled => " style=\"border-collapse:collapse\"",
        }
    }

    /// The per-cell style rules under the current profile, if any.
    fn cell_style(&self) -> Option<&'static str> {
        match self.profile.tables {
            TableStyle::Plain => None,
            // A mid grey so the rules read against a light *or* a dark background - Teams has both,
            // and the usual mail-client `#ccc` disappears entirely on dark.
            TableStyle::Ruled => Some("border:1px solid #9aa0a6;padding:6px 10px"),
        }
    }

    /// One `<tr>`, padded out to `columns` so that a ragged row still renders rectangular.
    ///
    /// The model tolerates ragged rows because HTML in the wild is ragged; HTML rendering does
    /// not, because a short row would silently borrow the next column's alignment and pull the
    /// table's shape apart.
    fn table_row(&mut self, row: &Row, tag: &str, table: &Table, columns: usize) {
        self.push("<tr>");
        for column in 0..columns {
            self.push(&format!("<{tag}"));
            let mut css = String::new();
            if let Some(rules) = self.cell_style() {
                css.push_str(rules);
            }
            if let Some(align) = align_style(table.alignment(column)) {
                if !css.is_empty() {
                    css.push(';');
                }
                css.push_str("text-align:");
                css.push_str(align);
            }
            if !css.is_empty() {
                self.push(&format!(" style=\"{css}\""));
            }
            self.push(">");
            if let Some(cell) = row.get(column) {
                self.inlines(cell);
            }
            self.push(&format!("</{tag}>"));
        }
        self.push("</tr>");
        self.line_break();
    }

    // -----------------------------------------------------------------------------------------
    // Lists
    // -----------------------------------------------------------------------------------------

    fn list(&mut self, list: &List, ctx: Context) {
        match self.profile.nested_lists {
            NestingStrategy::Native => self.list_native(list, ctx),
            NestingStrategy::MarginIndent => self.list_flat(list, ctx),
        }
    }

    fn list_native(&mut self, list: &List, ctx: Context) {
        self.open_list(list, 0);
        self.line_break();
        for item in &list.items {
            self.push("<li>");
            self.item_blocks(&item.blocks, list.tight, ctx);
            self.push("</li>");
            self.line_break();
        }
        self.close_list(list);
        self.line_break();
    }

    /// Nesting expressed as indentation on sibling lists, for targets that flatten nested lists.
    ///
    /// Lossy on purpose: a nested list becomes a sibling, so an ordered list interrupted by a
    /// nested one restarts its numbering. Targets that need this strategy have already thrown the
    /// nesting away themselves.
    fn list_flat(&mut self, list: &List, ctx: Context) {
        let depth = ctx.list_depth;
        let mut open = false;
        for item in &list.items {
            let content: Vec<Block> = item
                .blocks
                .iter()
                .filter(|b| !matches!(b, Block::List(_)))
                .cloned()
                .collect();
            if !content.is_empty() {
                if !open {
                    self.open_list(list, depth);
                    self.line_break();
                    open = true;
                }
                self.push("<li>");
                self.item_blocks(&content, list.tight, ctx);
                self.push("</li>");
                self.line_break();
            }
            for block in &item.blocks {
                let Block::List(nested) = block else {
                    continue;
                };
                if open {
                    self.close_list(list);
                    self.line_break();
                    open = false;
                }
                self.list_flat(
                    nested,
                    Context {
                        list_depth: depth + 1,
                        ..ctx
                    },
                );
            }
        }
        if open {
            self.close_list(list);
            self.line_break();
        }
    }

    fn open_list(&mut self, list: &List, depth: usize) {
        self.push(if list.ordered { "<ol" } else { "<ul" });
        if list.ordered && list.start != 1 {
            self.push(&format!(" start=\"{}\"", list.start));
        }
        if depth > 0 {
            self.push(&format!(" style=\"margin-left:{}px\"", depth * INDENT_PX));
        }
        self.push(">");
    }

    fn close_list(&mut self, list: &List) {
        self.push(if list.ordered { "</ol>" } else { "</ul>" });
    }

    /// The content of one `<li>`.
    ///
    /// A tight item's paragraphs are written bare, a loose item's keep their `<p>` — which is both
    /// what CommonMark's own HTML output does and the signal [`crate::html::parse`] reads back to
    /// recover `tight`.
    fn item_blocks(&mut self, blocks: &[Block], tight: bool, ctx: Context) {
        let mut bare_paragraphs = 0;
        for block in blocks {
            match block {
                Block::Paragraph(content) if tight => {
                    if bare_paragraphs > 0 {
                        self.push("<br>");
                    }
                    self.quote_prefix(ctx);
                    self.inlines(content);
                    bare_paragraphs += 1;
                }
                Block::List(nested) => {
                    if self.profile.pretty {
                        self.line_break();
                    }
                    // A nested list inside its parent `<li>` is the native encoding; the flat
                    // strategy never reaches here, because `list_flat` pulls nested lists out.
                    self.list_native(nested, ctx);
                }
                other => self.block(other, ctx),
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Inlines
    // -----------------------------------------------------------------------------------------

    fn inlines(&mut self, inlines: &[Inline]) {
        for node in inlines {
            self.inline(node);
        }
    }

    fn inline(&mut self, node: &Inline) {
        match node {
            Inline::Text(text) => {
                let escaped = escape_text(text);
                self.push(&escaped);
            }
            Inline::Bold(children) => {
                let style = self.profile.bold.clone();
                self.marked(&style, children);
            }
            Inline::Italic(children) => {
                let style = self.profile.italic.clone();
                self.marked(&style, children);
            }
            Inline::Strike(children) => {
                let style = self.profile.strike.clone();
                self.marked(&style, children);
            }
            Inline::Code(code) => {
                let style = self.profile.code.clone();
                self.open_mark(&style);
                let escaped = escape_text(code);
                self.push(&escaped);
                self.close_mark(&style);
            }
            Inline::Link {
                href,
                title,
                content,
            } => {
                self.push(&format!("<a href=\"{}\"", escape_href(href)));
                if let Some(title) = title {
                    self.push(&format!(" title=\"{}\"", escape_attr(title)));
                }
                self.push(">");
                self.inlines(content);
                self.push("</a>");
            }
            // A soft break is a space in HTML either way; in pretty mode spend it on a newline.
            Inline::SoftBreak => self.push(if self.profile.pretty { "\n" } else { " " }),
            Inline::HardBreak => self.push("<br>"),
        }
    }

    fn marked(&mut self, style: &MarkStyle, children: &[Inline]) {
        self.open_mark(style);
        self.inlines(children);
        self.close_mark(style);
    }

    fn open_mark(&mut self, style: &MarkStyle) {
        match style {
            MarkStyle::Tag(tag) => self.push(&format!("<{tag}>")),
            MarkStyle::Style(css) => self.push(&format!("<span style=\"{}\">", escape_attr(css))),
        }
    }

    fn close_mark(&mut self, style: &MarkStyle) {
        match style {
            MarkStyle::Tag(tag) => self.push(&format!("</{tag}>")),
            MarkStyle::Style(_) => self.push("</span>"),
        }
    }
}

/// The `text-align` keyword for an alignment, or `None` when the model has no opinion and the
/// cell should carry no `style` at all.
fn align_style(alignment: Alignment) -> Option<&'static str> {
    match alignment {
        Alignment::None => None,
        Alignment::Left => Some("left"),
        Alignment::Center => Some("center"),
        Alignment::Right => Some("right"),
    }
}

// -------------------------------------------------------------------------------------------
// Escaping
// -------------------------------------------------------------------------------------------

/// Escape text content: `&`, `<` and `>`.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape an attribute value: as text, plus the quote that delimits it.
fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a URL for an `href`.
///
/// Two different escapings apply and both are needed: characters that cannot appear literally in
/// a URL are percent-encoded, and then the result is escaped as an HTML attribute so that a query
/// string's `&` does not read as an entity.
fn escape_href(href: &str) -> String {
    let mut encoded = String::with_capacity(href.len());
    for ch in href.chars() {
        match ch {
            ' ' => encoded.push_str("%20"),
            '"' => encoded.push_str("%22"),
            '<' => encoded.push_str("%3C"),
            '>' => encoded.push_str("%3E"),
            '`' => encoded.push_str("%60"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let mut buf = [0u8; 4];
                for byte in c.encode_utf8(&mut buf).as_bytes() {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
            c => encoded.push(c),
        }
    }
    escape_attr(&encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::model::ListItem;
    use crate::html::parse;

    fn doc(blocks: Vec<Block>) -> Document {
        Document::from_blocks(blocks)
    }

    fn std_render(blocks: Vec<Block>) -> String {
        render(&doc(blocks), &RenderProfile::standard())
    }

    // ---------------------------------------------------------------------------------------
    // Baseline dialect
    // ---------------------------------------------------------------------------------------

    #[test]
    fn standard_is_plain_semantic_html() {
        let got = std_render(vec![
            Block::Heading {
                level: 2,
                content: vec![Inline::text("Title")],
            },
            Block::Paragraph(vec![
                Inline::text("a "),
                Inline::bold("b"),
                Inline::text(" "),
                Inline::italic("c"),
                Inline::text(" "),
                Inline::Strike(vec![Inline::text("d")]),
                Inline::text(" "),
                Inline::Code("e".to_string()),
            ]),
            Block::ThematicBreak,
        ]);
        assert_eq!(
            got,
            "<h2>Title</h2><p>a <strong>b</strong> <em>c</em> <s>d</s> <code>e</code></p><hr>"
        );
    }

    #[test]
    fn compact_output_has_no_stray_whitespace() {
        let got = std_render(vec![Block::para("a"), Block::para("b")]);
        assert_eq!(got, "<p>a</p><p>b</p>");
        assert!(!got.contains('\n'));
    }

    #[test]
    fn pretty_puts_newlines_between_blocks_only() {
        let profile = RenderProfile::standard().pretty();
        let got = render(&doc(vec![Block::para("a"), Block::para("b")]), &profile);
        assert_eq!(got, "<p>a</p>\n<p>b</p>");
    }

    #[test]
    fn text_is_escaped() {
        let got = std_render(vec![Block::para("a & b < c > d \"e\"")]);
        assert_eq!(got, "<p>a &amp; b &lt; c &gt; d \"e\"</p>");
    }

    #[test]
    fn attribute_values_escape_their_delimiter() {
        let got = std_render(vec![Block::Paragraph(vec![Inline::Link {
            href: "https://example.com/?a=1&b=2".to_string(),
            title: Some("a \"quoted\" title".to_string()),
            content: vec![Inline::text("x")],
        }])]);
        assert_eq!(
            got,
            "<p><a href=\"https://example.com/?a=1&amp;b=2\" title=\"a &quot;quoted&quot; title\">x</a></p>"
        );
    }

    #[test]
    fn hrefs_percent_encode_what_cannot_be_literal() {
        let got = std_render(vec![Block::Paragraph(vec![Inline::link(
            "https://example.com/a b\"c",
            "x",
        )])]);
        assert_eq!(
            got,
            "<p><a href=\"https://example.com/a%20b%22c\">x</a></p>"
        );
    }

    #[test]
    fn hard_and_soft_breaks() {
        let blocks = vec![Block::Paragraph(vec![
            Inline::text("a"),
            Inline::HardBreak,
            Inline::text("b"),
            Inline::SoftBreak,
            Inline::text("c"),
        ])];
        assert_eq!(std_render(blocks.clone()), "<p>a<br>b c</p>");
        let pretty = render(&doc(blocks), &RenderProfile::standard().pretty());
        assert_eq!(pretty, "<p>a<br>b\nc</p>");
    }

    #[test]
    fn an_empty_document_renders_to_nothing() {
        assert_eq!(std_render(vec![]), "");
    }

    #[test]
    fn unsupported_blocks_render_their_fallback() {
        // Not a table: tables are represented now. This is the shape a Loop component or an
        // embedded card still arrives in.
        let got = std_render(vec![Block::Unsupported {
            kind: "loop-component".to_string(),
            fallback: vec![Inline::text("a shared list")],
        }]);
        assert_eq!(got, "<p>a shared list</p>");
    }

    // ---------------------------------------------------------------------------------------
    // Lists, quotes, code
    // ---------------------------------------------------------------------------------------

    #[test]
    fn tight_and_loose_items_differ_by_their_paragraph_tags() {
        let tight = std_render(vec![Block::List(List::bulleted(vec![ListItem::of(vec![
            Inline::text("a"),
        ])]))]);
        assert_eq!(tight, "<ul><li>a</li></ul>");

        let loose = std_render(vec![Block::List(List {
            tight: false,
            ..List::bulleted(vec![ListItem::of(vec![Inline::text("a")])])
        })]);
        assert_eq!(loose, "<ul><li><p>a</p></li></ul>");
    }

    #[test]
    fn an_ordered_list_emits_start_only_when_it_is_not_one() {
        let list = |start: u64| {
            std_render(vec![Block::List(List {
                start,
                ..List::numbered(vec![ListItem::of(vec![Inline::text("a")])])
            })])
        };
        assert_eq!(list(1), "<ol><li>a</li></ol>");
        assert_eq!(list(4), "<ol start=\"4\"><li>a</li></ol>");
    }

    fn nested_list_blocks() -> Vec<Block> {
        vec![Block::List(List::bulleted(vec![
            ListItem {
                blocks: vec![
                    Block::para("a"),
                    Block::List(List::bulleted(vec![ListItem::of(vec![Inline::text("b")])])),
                ],
            },
            ListItem::of(vec![Inline::text("c")]),
        ]))]
    }

    #[test]
    fn native_nesting_puts_the_child_list_inside_the_li() {
        assert_eq!(
            std_render(nested_list_blocks()),
            "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>"
        );
    }

    #[test]
    fn margin_indent_nesting_flattens_to_indented_siblings() {
        let profile = RenderProfile {
            nested_lists: NestingStrategy::MarginIndent,
            ..RenderProfile::standard()
        };
        assert_eq!(
            render(&doc(nested_list_blocks()), &profile),
            "<ul><li>a</li></ul><ul style=\"margin-left:24px\"><li>b</li></ul><ul><li>c</li></ul>"
        );
    }

    #[test]
    fn blockquote_strategies() {
        let blocks = vec![Block::BlockQuote(vec![
            Block::para("a"),
            Block::BlockQuote(vec![Block::para("b")]),
        ])];
        assert_eq!(
            std_render(blocks.clone()),
            "<blockquote><p>a</p><blockquote><p>b</p></blockquote></blockquote>"
        );

        let profile = RenderProfile {
            blockquote: QuoteStrategy::TextPrefix,
            ..RenderProfile::standard()
        };
        assert_eq!(
            render(&doc(blocks), &profile),
            "<p>&gt; a</p><p>&gt; &gt; b</p>"
        );
    }

    #[test]
    fn code_block_strategies() {
        let blocks = vec![Block::CodeBlock {
            lang: Some("rust".to_string()),
            code: "let x = 1 < 2;\nlet y = &x;".to_string(),
        }];
        assert_eq!(
            std_render(blocks.clone()),
            "<pre><code class=\"language-rust\">let x = 1 &lt; 2;\nlet y = &amp;x;</code></pre>"
        );

        let profile = RenderProfile {
            code_block: CodeBlockStrategy::MonospaceDiv,
            ..RenderProfile::standard()
        };
        assert_eq!(
            render(&doc(blocks), &profile),
            "<div style=\"font-family:Consolas,monospace;white-space:pre\">let x = 1 &lt; 2;<br>let y = &amp;x;</div>"
        );
    }

    #[test]
    fn a_code_block_without_a_language_has_no_class() {
        assert_eq!(
            std_render(vec![Block::CodeBlock {
                lang: None,
                code: "x".to_string(),
            }]),
            "<pre><code>x</code></pre>"
        );
    }

    // ---------------------------------------------------------------------------------------
    // Tables
    // ---------------------------------------------------------------------------------------

    fn cell(text: &str) -> Vec<Inline> {
        vec![Inline::text(text)]
    }

    #[test]
    fn a_table_with_a_header_emits_thead_and_tbody() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![cell("H1"), cell("H2")],
            align: vec![],
            rows: vec![vec![cell("a"), cell("b")]],
        })]);
        assert_eq!(
            got,
            concat!(
                "<table><thead><tr><th>H1</th><th>H2</th></tr></thead>",
                "<tbody><tr><td>a</td><td>b</td></tr></tbody></table>"
            )
        );
    }

    #[test]
    fn a_table_without_a_header_emits_no_thead() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![],
            align: vec![],
            rows: vec![vec![cell("a")], vec![cell("b")]],
        })]);
        assert_eq!(
            got,
            "<table><tbody><tr><td>a</td></tr><tr><td>b</td></tr></tbody></table>"
        );
    }

    #[test]
    fn alignment_is_an_inline_style_on_every_cell_of_the_column() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![cell("l"), cell("c"), cell("r"), cell("n")],
            align: vec![
                Alignment::Left,
                Alignment::Center,
                Alignment::Right,
                Alignment::None,
            ],
            rows: vec![vec![cell("1"), cell("2"), cell("3"), cell("4")]],
        })]);
        assert_eq!(
            got,
            concat!(
                "<table><thead><tr>",
                "<th style=\"text-align:left\">l</th>",
                "<th style=\"text-align:center\">c</th>",
                "<th style=\"text-align:right\">r</th>",
                // `Alignment::None` emits nothing at all.
                "<th>n</th>",
                "</tr></thead><tbody><tr>",
                "<td style=\"text-align:left\">1</td>",
                "<td style=\"text-align:center\">2</td>",
                "<td style=\"text-align:right\">3</td>",
                "<td>4</td>",
                "</tr></tbody></table>"
            )
        );
    }

    #[test]
    fn ragged_rows_are_padded_to_the_widest_one() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![cell("a")],
            align: vec![],
            rows: vec![vec![cell("b"), cell("c"), cell("d")], vec![cell("e")]],
        })]);
        assert_eq!(
            got,
            concat!(
                "<table><thead><tr><th>a</th><th></th><th></th></tr></thead>",
                "<tbody><tr><td>b</td><td>c</td><td>d</td></tr>",
                "<tr><td>e</td><td></td><td></td></tr></tbody></table>"
            )
        );
    }

    #[test]
    fn cell_content_is_ordinary_inline_content() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![],
            align: vec![],
            rows: vec![vec![vec![
                Inline::bold("b"),
                Inline::text(" & "),
                Inline::link("https://example.com", "x"),
            ]]],
        })]);
        assert_eq!(
            got,
            concat!(
                "<table><tbody><tr><td><strong>b</strong> &amp; ",
                "<a href=\"https://example.com\">x</a></td></tr></tbody></table>"
            )
        );
    }

    #[test]
    fn pretty_breaks_a_table_across_lines() {
        let profile = RenderProfile::standard().pretty();
        let got = render(
            &doc(vec![Block::Table(Table {
                head: vec![cell("h")],
                align: vec![],
                rows: vec![vec![cell("a")]],
            })]),
            &profile,
        );
        assert_eq!(
            got,
            concat!(
                "<table>\n<thead>\n<tr><th>h</th></tr>\n</thead>\n",
                "<tbody>\n<tr><td>a</td></tr>\n</tbody>\n</table>"
            )
        );
    }

    // ---------------------------------------------------------------------------------------
    // Every profile variant
    // ---------------------------------------------------------------------------------------

    #[test]
    fn heading_strategies() {
        let blocks = vec![Block::Heading {
            level: 3,
            content: vec![Inline::text("T")],
        }];
        assert_eq!(std_render(blocks.clone()), "<h3>T</h3>");

        let profile = RenderProfile {
            headings: HeadingStrategy::BoldParagraph,
            ..RenderProfile::standard()
        };
        assert_eq!(render(&doc(blocks), &profile), "<p><strong>T</strong></p>");
    }

    #[test]
    fn mark_styles_can_be_tags_or_spans() {
        let blocks = vec![Block::Paragraph(vec![
            Inline::bold("b"),
            Inline::italic("i"),
            Inline::Strike(vec![Inline::text("s")]),
            Inline::Code("c".to_string()),
        ])];
        let profile = RenderProfile {
            bold: MarkStyle::Style("font-weight:bold"),
            italic: MarkStyle::Style("font-style:italic"),
            strike: MarkStyle::Style("text-decoration:line-through"),
            code: MarkStyle::Style("font-family:Consolas,monospace"),
            ..RenderProfile::standard()
        };
        assert_eq!(
            render(&doc(blocks.clone()), &profile),
            concat!(
                "<p><span style=\"font-weight:bold\">b</span>",
                "<span style=\"font-style:italic\">i</span>",
                "<span style=\"text-decoration:line-through\">s</span>",
                "<span style=\"font-family:Consolas,monospace\">c</span></p>"
            )
        );

        // And the tag form, with a different tag than the default, to prove nothing is hard-coded.
        let tags = RenderProfile {
            bold: MarkStyle::Tag("b"),
            italic: MarkStyle::Tag("i"),
            strike: MarkStyle::Tag("del"),
            code: MarkStyle::Tag("tt"),
            ..RenderProfile::standard()
        };
        assert_eq!(
            render(&doc(blocks), &tags),
            "<p><b>b</b><i>i</i><del>s</del><tt>c</tt></p>"
        );
    }

    #[test]
    fn the_standard_profile_leaves_tables_unstyled() {
        let got = std_render(vec![Block::Table(Table {
            head: vec![vec![Inline::text("H")]],
            align: vec![],
            rows: vec![vec![vec![Inline::text("a")]]],
        })]);
        assert!(
            !got.contains("border"),
            "standard profile should not style tables: {got}"
        );
        assert!(
            !got.contains("padding"),
            "standard profile should not style tables: {got}"
        );
    }

    #[test]
    fn the_teams_profile_rules_its_tables() {
        // Clipboard HTML travels without a stylesheet, so a table that carries no styling of its
        // own pastes as a borderless grid of text.
        let doc = doc(vec![Block::Table(Table {
            head: vec![vec![Inline::text("H")]],
            align: vec![Alignment::Center],
            rows: vec![vec![vec![Inline::text("a")]]],
        })]);
        let got = render(&doc, &RenderProfile::teams());

        assert!(got.contains("border-collapse:collapse"), "{got}");
        assert!(got.contains("border:1px solid"), "{got}");
        assert!(got.contains("padding:"), "{got}");
        // Alignment still rides along in the same style attribute rather than being displaced.
        assert!(got.contains("text-align:center"), "{got}");
        // No header fill: Teams has a dark theme, and a pale one would look wrong there.
        assert!(!got.contains("background"), "{got}");
    }

    #[test]
    fn ruled_tables_still_parse_back_to_the_same_table() {
        let doc = doc(vec![Block::Table(Table {
            head: vec![vec![Inline::text("H")], vec![Inline::text("I")]],
            align: vec![Alignment::None, Alignment::Right],
            rows: vec![vec![vec![Inline::text("a")], vec![Inline::bold("b")]]],
        })]);
        let html = render(&doc, &RenderProfile::teams());
        // The styling must not confuse our own parser on the way back in.
        assert_eq!(crate::html::parse(&html), doc);
    }

    /// Exercise every strategy variant at once, and confirm the parser still recognizes the
    /// intent of the resulting markup even in the deviant dialects.
    #[test]
    fn the_deviant_profile_still_parses_back_to_recognizable_intent() {
        let profile = RenderProfile {
            bold: MarkStyle::Style("font-weight:600"),
            italic: MarkStyle::Style("font-style:italic"),
            strike: MarkStyle::Style("text-decoration:line-through"),
            code: MarkStyle::Style("font-family:Consolas,monospace"),
            headings: HeadingStrategy::BoldParagraph,
            code_block: CodeBlockStrategy::MonospaceDiv,
            nested_lists: NestingStrategy::MarginIndent,
            blockquote: QuoteStrategy::TextPrefix,
            tables: TableStyle::Ruled,
            pretty: false,
        };
        let source = doc(vec![Block::Paragraph(vec![
            Inline::bold("b"),
            Inline::text(" "),
            Inline::italic("i"),
            Inline::text(" "),
            Inline::Strike(vec![Inline::text("s")]),
        ])]);
        let html = render(&source, &profile);
        assert_eq!(parse(&html), source);
    }

    // ---------------------------------------------------------------------------------------
    // Round trips
    // ---------------------------------------------------------------------------------------

    /// Documents that survive HTML in both directions.
    ///
    /// `SoftBreak` is deliberately absent: HTML has no way to say "a newline that renders as a
    /// space", so a soft break renders as a space and parses back as one. `Unsupported` is absent
    /// for the same reason — its fallback is, by definition, a lossy encoding.
    fn round_trip_documents() -> Vec<Document> {
        vec![
            doc(vec![Block::para("just a paragraph")]),
            doc(vec![Block::Paragraph(vec![
                Inline::text("a "),
                Inline::bold("bold"),
                Inline::text(" and "),
                Inline::italic("italic"),
                Inline::text(" and "),
                Inline::Strike(vec![Inline::text("struck")]),
                Inline::text(" and "),
                Inline::Code("code()".to_string()),
                Inline::text(" & <entities>"),
            ])]),
            doc(vec![
                Block::Heading {
                    level: 1,
                    content: vec![Inline::text("One")],
                },
                Block::Heading {
                    level: 3,
                    content: vec![Inline::Bold(vec![Inline::text("Three")])],
                },
                Block::ThematicBreak,
            ]),
            doc(vec![Block::List(List::bulleted(vec![
                ListItem {
                    blocks: vec![
                        Block::para("a"),
                        Block::List(List {
                            start: 2,
                            ..List::numbered(vec![ListItem::of(vec![Inline::text("b")])])
                        }),
                    ],
                },
                ListItem::of(vec![Inline::text("c")]),
            ]))]),
            doc(vec![Block::List(List {
                tight: false,
                ..List::bulleted(vec![ListItem {
                    blocks: vec![Block::para("first"), Block::para("second")],
                }])
            })]),
            doc(vec![Block::BlockQuote(vec![
                Block::para("quoted"),
                Block::BlockQuote(vec![Block::para("deeper")]),
            ])]),
            doc(vec![Block::CodeBlock {
                lang: Some("rust".to_string()),
                code: "fn main() {\n    println!(\"1 < 2 && 3 > 2\");\n}".to_string(),
            }]),
            doc(vec![Block::Paragraph(vec![
                Inline::text("see "),
                Inline::Link {
                    href: "https://example.com/a?b=1&c=2".to_string(),
                    title: Some("the title".to_string()),
                    content: vec![Inline::Bold(vec![Inline::text("docs")])],
                },
                Inline::HardBreak,
                Inline::text("next line"),
            ])]),
            // Tables round trip only when they are already rectangular: the renderer pads a
            // ragged row out to `columns()`, and the parser has no way to know the padding was
            // not in the source. Alignment must likewise be canonical — the parser drops trailing
            // `Alignment::None`s, because `Table::alignment` defaults for a short `align`.
            doc(vec![Block::Table(Table {
                head: vec![cell("Name"), cell("Qty"), cell("Notes")],
                align: vec![Alignment::None, Alignment::Right],
                rows: vec![
                    vec![cell("widget"), cell("3"), cell("in stock")],
                    vec![
                        vec![Inline::bold("gadget")],
                        cell("1"),
                        vec![Inline::link("https://example.com/g", "spec")],
                    ],
                ],
            })]),
            doc(vec![Block::Table(Table {
                head: vec![],
                align: vec![Alignment::Center, Alignment::Left],
                rows: vec![vec![cell("a"), cell("b")], vec![cell("c"), cell("d")]],
            })]),
            doc(vec![Block::Table(Table {
                head: vec![cell("one")],
                align: vec![],
                rows: vec![vec![vec![Inline::Code("x < y".to_string())]]],
            })]),
        ]
    }

    #[test]
    fn parse_of_render_is_the_identity_under_the_standard_profile() {
        for source in round_trip_documents() {
            let html = render(&source, &RenderProfile::standard());
            assert_eq!(parse(&html), source, "round trip failed for {html}");
        }
    }

    #[test]
    fn the_pretty_profile_round_trips_too() {
        for source in round_trip_documents() {
            let html = render(&source, &RenderProfile::standard().pretty());
            assert_eq!(parse(&html), source, "round trip failed for {html}");
        }
    }

    #[test]
    fn rendering_is_stable_across_a_second_trip() {
        for source in round_trip_documents() {
            let once = render(&source, &RenderProfile::teams());
            let twice = render(&parse(&once), &RenderProfile::teams());
            assert_eq!(once, twice);
        }
    }
}
