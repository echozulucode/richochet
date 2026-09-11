//! HTML -> document model, via html5ever.
//!
//! Input is untrusted: it arrives on the clipboard from another process. Sanitize before parsing.
//!
//! The job of this module is the principle in `docs/plan.md`: **preserve intent, not Teams
//! markup**. Nothing Teams-specific survives past here. A `<span style="font-weight:600">` is not
//! recorded as a span with a style; it is recorded as [`Inline::Bold`], and the difference between
//! the two is the difference between a maintainable converter and a pile of special cases.
//!
//! The pipeline is:
//!
//! 1. [`sanitize`] with `ammonia` — scripts, iframes and event handlers never reach the parser.
//! 2. Parse the result as an HTML *fragment* (clipboard HTML is a fragment, not a document).
//! 3. Walk the tree, accumulating [`MarkSet`]s down the branches and collapsing whitespace per
//!    the HTML rules on the way.
//! 4. Hand the blocks to [`Document::from_blocks`], which normalizes them.

use std::collections::{HashMap, HashSet};

use html5ever::tendril::TendrilSink;
use html5ever::{local_name, namespace_url, ns, parse_fragment, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::model::{Block, Document, Inline, List, ListItem};
use crate::html::styles::{marks_for, MarkSet};

/// Parse an HTML fragment into the document model, normalizing markup to intent.
pub fn parse(input: &str) -> Document {
    let sanitized = sanitize(input);
    let dom = parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("body")),
        Vec::new(),
        false,
    )
    .one(sanitized);

    // Fragment parsing wraps the result in a synthetic context element; its children are the
    // fragment proper.
    let root = dom.document.children.borrow().first().cloned();
    let mut builder = BlockBuilder::new();
    if let Some(root) = root {
        walk_children(&root, MarkSet::none(), &mut builder);
    }
    Document::from_blocks(builder.finish())
}

/// Strip everything dangerous from untrusted clipboard HTML, keeping only what the walker reads.
///
/// This runs *before* html5ever rather than filtering during the walk, because the walk has to be
/// permissive — unknown elements are traversed into, not dropped — and a permissive walk over
/// unsanitized input is exactly how a script tag's text content ends up in a document.
pub fn sanitize(input: &str) -> String {
    // Everything the walker understands, plus the containers producers wrap content in.
    let tags: HashSet<&str> = HashSet::from([
        "a", "abbr", "address", "article", "aside", "b", "big", "blockquote", "br", "caption",
        "center", "cite", "code", "col", "colgroup", "dd", "del", "details", "dfn", "div", "dl",
        "dt", "em", "figcaption", "figure", "footer", "h1", "h2", "h3", "h4", "h5", "h6", "header",
        "hgroup", "hr", "i", "img", "ins", "kbd", "li", "main", "mark", "nav", "ol", "p", "pre",
        "q", "s", "samp", "section", "small", "span", "strike", "strong", "sub", "summary", "sup",
        "table", "tbody", "td", "tfoot", "th", "thead", "tr", "tt", "u", "ul", "var", "wbr",
    ]);
    // Attributes allowed anywhere. `style` is the whole point of `styles.rs`; `class` carries the
    // `language-*` hint on code blocks.
    let generic_attributes: HashSet<&str> = HashSet::from(["style", "class", "title", "lang"]);
    let tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::from([
        ("a", HashSet::from(["href"])),
        ("ol", HashSet::from(["start"])),
        ("img", HashSet::from(["alt", "src"])),
        ("td", HashSet::from(["colspan", "rowspan"])),
        ("th", HashSet::from(["colspan", "rowspan", "scope"])),
    ]);
    // Blacklisted *with* their contents: dropping the tag but keeping the text would paste a
    // script body into the document as prose.
    let clean_content_tags: HashSet<&str> = HashSet::from([
        "script", "style", "iframe", "object", "embed", "noscript", "template", "form", "input",
        "button", "select", "textarea", "svg", "math", "head", "title", "meta", "link", "base",
    ]);

    ammonia::Builder::default()
        .tags(tags)
        .generic_attributes(generic_attributes)
        .tag_attributes(tag_attributes)
        .clean_content_tags(clean_content_tags)
        // We never re-serialize this HTML to a browser, and `rel` would only be noise the walker
        // has to ignore.
        .link_rel(None)
        .strip_comments(true)
        .clean(input)
        .to_string()
}

// ---------------------------------------------------------------------------------------------
// Inline accumulation
// ---------------------------------------------------------------------------------------------

/// Accumulates the inline run of the block currently being built, applying the HTML whitespace
/// rules as it goes.
///
/// Whitespace is handled by *deferring* it: a collapsible run of whitespace sets `space_pending`
/// rather than emitting anything, and the space is only materialized once something follows it in
/// the same block. That drops leading and trailing block whitespace for free, and it is the
/// reason `<div><span style="font-weight:600"> Important </span></div>` comes out as a bold
/// "Important" with no stray spaces at all.
///
/// `frames` is a stack so that a `<a>` can collect its own content without losing continuity of
/// the whitespace state with the text around it.
struct InlineBuilder {
    frames: Vec<Vec<Inline>>,
    /// A collapsible whitespace run has been seen and not yet emitted.
    space_pending: bool,
    /// Something has been emitted since the start of the block (or since the last `<br>`), so a
    /// pending space is now meaningful rather than leading whitespace.
    emitted: bool,
}

impl InlineBuilder {
    fn new() -> Self {
        InlineBuilder {
            frames: vec![Vec::new()],
            space_pending: false,
            emitted: false,
        }
    }

    fn current(&mut self) -> &mut Vec<Inline> {
        self.frames
            .last_mut()
            .expect("the base frame is never popped")
    }

    fn is_empty(&self) -> bool {
        self.frames.iter().all(Vec::is_empty)
    }

    /// Emit a deferred space, if one is owed and would not be leading whitespace.
    fn flush_space(&mut self) {
        if self.space_pending && self.emitted {
            self.current().push(Inline::Text(" ".to_string()));
        }
        self.space_pending = false;
    }

    /// Add text, collapsing whitespace per the HTML rules.
    fn push_text(&mut self, raw: &str, marks: MarkSet) {
        let collapsed = collapse_whitespace(raw);
        if collapsed.is_empty() {
            return;
        }
        if collapsed == " " {
            self.space_pending = true;
            return;
        }
        let leading = collapsed.starts_with(' ');
        let trailing = collapsed.ends_with(' ');
        let core = collapsed.trim_matches(' ');

        if leading {
            self.space_pending = true;
        }
        self.flush_space();
        let node = wrap_marks(core.to_string(), marks);
        self.current().push(node);
        self.emitted = true;
        self.space_pending = trailing;
    }

    /// Add a ready-made inline node (a link, an image's alt text, ...).
    fn push_inline(&mut self, node: Inline) {
        self.flush_space();
        self.current().push(node);
        self.emitted = true;
    }

    /// Add an explicit line break.
    ///
    /// A break at the very start of a block is dropped: Teams writes `<div><br></div>` for a blank
    /// line between paragraphs, and that is paragraph separation, not content.
    fn push_break(&mut self) {
        self.space_pending = false;
        if self.is_empty() {
            return;
        }
        self.current().push(Inline::HardBreak);
        // Whitespace after a break is leading whitespace again.
        self.emitted = false;
    }

    fn push_frame(&mut self) {
        self.frames.push(Vec::new());
    }

    fn pop_frame(&mut self) -> Vec<Inline> {
        self.frames.pop().unwrap_or_default()
    }

    /// Take the accumulated run and reset for the next block.
    fn take(&mut self) -> Vec<Inline> {
        let out = std::mem::take(self.current());
        self.space_pending = false;
        self.emitted = false;
        out
    }
}

/// Collapse runs of collapsible whitespace into single spaces.
///
/// Deliberately uses ASCII whitespace rather than `char::is_whitespace`: U+00A0 NO-BREAK SPACE is
/// Unicode whitespace but is *not* collapsible in HTML, and Teams emits a lot of `&nbsp;`.
fn collapse_whitespace(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_space = false;
    for ch in raw.chars() {
        if ch.is_ascii_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
}

/// Wrap text in whichever marks are active where it appears.
///
/// The order here is the model's canonical nesting order (bold outside italic outside strike), so
/// the normalizer has nothing to reorder.
fn wrap_marks(text: String, marks: MarkSet) -> Inline {
    let mut node = if marks.is_code() {
        Inline::Code(text)
    } else {
        Inline::Text(text)
    };
    if marks.is_strike() {
        node = Inline::Strike(vec![node]);
    }
    if marks.is_italic() {
        node = Inline::Italic(vec![node]);
    }
    if marks.is_bold() {
        node = Inline::Bold(vec![node]);
    }
    node
}

// ---------------------------------------------------------------------------------------------
// Block accumulation
// ---------------------------------------------------------------------------------------------

/// Accumulates the blocks of one block container (the document, a `<li>`, a `<blockquote>`, ...).
struct BlockBuilder {
    blocks: Vec<Block>,
    inlines: InlineBuilder,
    /// A `<p>` was seen as content of this container. Used only to decide list tightness.
    saw_paragraph: bool,
}

impl BlockBuilder {
    fn new() -> Self {
        BlockBuilder {
            blocks: Vec::new(),
            inlines: InlineBuilder::new(),
            saw_paragraph: false,
        }
    }

    /// Close the inline run in progress, if any, as a paragraph.
    fn flush(&mut self) {
        if self.inlines.is_empty() {
            self.inlines.take();
            return;
        }
        let content = self.inlines.take();
        self.blocks.push(Block::Paragraph(content));
    }

    fn push_block(&mut self, block: Block) {
        self.flush();
        self.blocks.push(block);
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }
}

// ---------------------------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------------------------

fn walk_children(node: &Handle, marks: MarkSet, out: &mut BlockBuilder) {
    // Collect first: the recursive walk must not hold a borrow on the children list.
    let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
    for child in &children {
        walk_node(child, marks, out);
    }
}

fn walk_node(node: &Handle, marks: MarkSet, out: &mut BlockBuilder) {
    match &node.data {
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            out.inlines.push_text(&text, marks);
        }
        NodeData::Element { .. } => walk_element(node, marks, out),
        // Documents, doctypes, comments and processing instructions carry nothing we want.
        _ => {}
    }
}

fn walk_element(node: &Handle, inherited: MarkSet, out: &mut BlockBuilder) {
    let Some(tag) = tag_name(node) else {
        return;
    };
    // Every element, block or inline, can contribute marks to its descendants.
    let marks = inherited.merge(marks_for(&tag, attr(node, "style").as_deref()));

    match tag.as_str() {
        "p" => {
            out.saw_paragraph = true;
            push_container(node, marks, out);
        }
        "div" | "section" | "article" | "header" | "footer" | "main" | "aside" | "figure"
        | "figcaption" | "address" | "center" | "dl" | "dt" | "dd" | "li" | "details"
        | "summary" | "caption" => push_container(node, marks, out),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<u8>().unwrap_or(1);
            let content = inline_subtree(node, marks);
            out.push_block(Block::heading(level, content));
        }
        "ul" | "ol" => {
            let list = build_list(node, &tag, marks);
            out.push_block(Block::List(list));
        }
        "blockquote" => {
            let mut sub = BlockBuilder::new();
            walk_children(node, marks, &mut sub);
            out.push_block(Block::BlockQuote(sub.finish()));
        }
        "pre" => {
            let code = code_text(node);
            out.push_block(Block::CodeBlock {
                lang: code_language(node),
                code,
            });
        }
        "hr" => out.push_block(Block::ThematicBreak),
        "table" => {
            let fallback = table_fallback(node, marks);
            out.push_block(Block::Unsupported {
                kind: "table".to_string(),
                fallback,
            });
        }
        "br" => out.inlines.push_break(),
        "a" => walk_anchor(node, marks, out),
        "img" => {
            // An image cannot be represented, but its alt text is the author's own description of
            // it and is worth more than nothing.
            if let Some(alt) = attr(node, "alt") {
                out.inlines.push_text(&alt, marks);
            }
        }
        // `<span>`, `<b>`, `<code>` and everything unrecognized: the element itself contributes
        // marks (already merged above) and is otherwise transparent.
        _ => walk_children(node, marks, out),
    }
}

/// Walk a block container: its content becomes blocks of its own, never merged with the inline
/// run that surrounded it.
fn push_container(node: &Handle, marks: MarkSet, out: &mut BlockBuilder) {
    let mut sub = BlockBuilder::new();
    walk_children(node, marks, &mut sub);
    let blocks = sub.finish();
    if blocks.is_empty() {
        return;
    }
    out.flush();
    out.blocks.extend(blocks);
}

fn walk_anchor(node: &Handle, marks: MarkSet, out: &mut BlockBuilder) {
    let Some(href) = attr(node, "href") else {
        // An anchor with no target is just a styled span (Teams uses them as mention wrappers).
        walk_children(node, marks, out);
        return;
    };

    out.inlines.push_frame();
    walk_children(node, marks, out);
    let mut content = out.inlines.pop_frame();

    // Whitespace between the previous text and the link belongs outside the link: the clickable
    // region should not start with a space. (The normalizer deliberately does not hoist
    // whitespace out of links, because for links it would change what is clickable.)
    let leading_space = matches!(content.first(), Some(Inline::Text(t)) if t == " ");
    if leading_space {
        content.remove(0);
        out.inlines.space_pending = true;
    }

    out.inlines.push_inline(Inline::Link {
        href,
        title: attr(node, "title"),
        content,
    });
}

fn build_list(node: &Handle, tag: &str, marks: MarkSet) -> List {
    let ordered = tag == "ol";
    let start = attr(node, "start")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1);

    let mut items: Vec<ListItem> = Vec::new();
    let mut tight = true;
    let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
    for child in &children {
        let Some(child_tag) = tag_name(child) else {
            continue;
        };
        match child_tag.as_str() {
            "li" => {
                let item_marks = marks.merge(marks_for("li", attr(child, "style").as_deref()));
                let mut sub = BlockBuilder::new();
                walk_children(child, item_marks, &mut sub);
                if sub.saw_paragraph {
                    // A `<li>` whose content is wrapped in `<p>` is a loose list item; that is
                    // how CommonMark distinguishes the two and how our own renderer writes them.
                    tight = false;
                }
                let blocks = sub.finish();
                if !blocks.is_empty() {
                    items.push(ListItem { blocks });
                }
            }
            // A list nested directly inside a list, with no `<li>` wrapper, is invalid but
            // common. Attach it to the preceding item rather than losing it.
            "ul" | "ol" => {
                let nested = Block::List(build_list(child, &child_tag, marks));
                match items.last_mut() {
                    Some(item) => item.blocks.push(nested),
                    None => items.push(ListItem {
                        blocks: vec![nested],
                    }),
                }
            }
            _ => {}
        }
    }

    List {
        ordered,
        start,
        tight,
        items,
    }
}

/// Collect a subtree as a single inline run, flattening any block structure inside it.
///
/// Used where the model has no room for blocks: heading content and table cells.
fn inline_subtree(node: &Handle, marks: MarkSet) -> Vec<Inline> {
    let mut sub = BlockBuilder::new();
    walk_children(node, marks, &mut sub);
    flatten_blocks(sub.finish())
}

/// Reduce blocks to one inline run, separating what were block boundaries with hard breaks.
fn flatten_blocks(blocks: Vec<Block>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for block in blocks {
        let part = match block {
            Block::Paragraph(c) | Block::Unsupported { fallback: c, .. } => c,
            Block::Heading { content, .. } => content,
            Block::BlockQuote(b) => flatten_blocks(b),
            Block::List(l) => {
                let mut items: Vec<Inline> = Vec::new();
                for item in l.items {
                    if !items.is_empty() {
                        items.push(Inline::HardBreak);
                    }
                    items.extend(flatten_blocks(item.blocks));
                }
                items
            }
            // Inline code carries no newlines, so a flattened code block becomes one line.
            Block::CodeBlock { code, .. } => vec![Inline::Code(code.replace('\n', " "))],
            Block::ThematicBreak => Vec::new(),
        };
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Inline::HardBreak);
        }
        out.extend(part);
    }
    out
}

/// The fallback content of a table: the cell text, rows separated by hard breaks and cells by a
/// pipe, so that a pasted table still reads as a table after conversion.
fn table_fallback(node: &Handle, marks: MarkSet) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for row in descendants_named(node, &["tr"]) {
        let mut cells: Vec<Vec<Inline>> = Vec::new();
        for cell in child_elements(&row) {
            if matches!(tag_name(&cell).as_deref(), Some("td") | Some("th")) {
                let content = inline_subtree(&cell, marks);
                if !content.is_empty() {
                    cells.push(content);
                }
            }
        }
        if cells.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Inline::HardBreak);
        }
        for (index, cell) in cells.into_iter().enumerate() {
            if index > 0 {
                out.push(Inline::Text(" | ".to_string()));
            }
            out.extend(cell);
        }
    }
    out
}

/// The verbatim text of a `<pre>`: no whitespace collapsing, `<br>` counted as a newline.
fn code_text(node: &Handle) -> String {
    let mut out = String::new();
    collect_verbatim(node, &mut out);
    // The HTML parser already drops a newline immediately after `<pre>`; do it again in case the
    // producer wrote one that survived, and drop the trailing newline the model does not keep.
    let trimmed = out.strip_prefix('\n').unwrap_or(&out);
    trimmed.trim_end_matches(['\n', '\r']).to_string()
}

fn collect_verbatim(node: &Handle, out: &mut String) {
    match &node.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { name, .. } => {
            if &*name.local == "br" {
                out.push('\n');
                return;
            }
            let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
            for child in &children {
                collect_verbatim(child, out);
            }
        }
        _ => {}
    }
}

/// Read a language hint from `class="language-rust"` (or `lang-rust`) on the `<pre>` itself or on
/// a `<code>` inside it — the convention every Markdown-to-HTML renderer emits.
fn code_language(pre: &Handle) -> Option<String> {
    let mut candidates = vec![pre.clone()];
    candidates.extend(descendants_named(pre, &["code"]));
    for node in candidates {
        let Some(class) = attr(&node, "class") else {
            continue;
        };
        for token in class.split_whitespace() {
            let lower = token.to_ascii_lowercase();
            for prefix in ["language-", "lang-"] {
                if let Some(rest) = lower.strip_prefix(prefix) {
                    if !rest.is_empty() {
                        return Some(rest.to_string());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// DOM helpers
// ---------------------------------------------------------------------------------------------

/// The lowercase local name of an element node, or `None` for anything else.
fn tag_name(node: &Handle) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.to_ascii_lowercase()),
        _ => None,
    }
}

/// An element's attribute value, matched case-insensitively.
fn attr(node: &Handle, wanted: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|a| a.name.local.eq_ignore_ascii_case(wanted))
        .map(|a| a.value.to_string())
}

fn child_elements(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
        .collect()
}

/// Every descendant element whose tag is one of `names`, in document order.
fn descendants_named(node: &Handle, names: &[&str]) -> Vec<Handle> {
    let mut out = Vec::new();
    let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
    for child in &children {
        if let Some(tag) = tag_name(child) {
            if names.contains(&tag.as_str()) {
                out.push(child.clone());
            }
        }
        out.extend(descendants_named(child, names));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(html: &str) -> Vec<Block> {
        parse(html).blocks
    }

    fn para(html: &str) -> Vec<Inline> {
        match blocks(html).into_iter().next() {
            Some(Block::Paragraph(c)) => c,
            other => panic!("expected a paragraph, got {other:?}"),
        }
    }

    #[test]
    fn the_teams_bold_span_from_the_plan() {
        // docs/plan.md: this exact input must become Paragraph > Bold > "Important", with the
        // whitespace hoisted out of the mark and then dropped at the block boundary.
        let got = blocks(r#"<div><span style="font-weight:600"> Important </span></div>"#);
        assert_eq!(
            got,
            vec![Block::Paragraph(vec![Inline::Bold(vec![Inline::Text(
                "Important".to_string()
            )])])]
        );
    }

    #[test]
    fn whitespace_is_hoisted_out_of_a_mark_but_kept_between_words() {
        let got = para(r#"before<b> bold </b>after"#);
        assert_eq!(
            got,
            vec![
                Inline::Text("before ".to_string()),
                Inline::Bold(vec![Inline::Text("bold".to_string())]),
                Inline::Text(" after".to_string()),
            ]
        );
    }

    #[test]
    fn font_weight_normal_cancels_an_enclosing_bold() {
        let got = para(r#"<b>bold<span style="font-weight:normal">plain</span></b>"#);
        assert_eq!(
            got,
            vec![
                Inline::Bold(vec![Inline::Text("bold".to_string())]),
                Inline::Text("plain".to_string()),
            ]
        );
    }

    #[test]
    fn nested_marks_come_out_in_canonical_order() {
        let got = para(r#"<em><strong>both</strong></em>"#);
        assert_eq!(
            got,
            vec![Inline::Bold(vec![Inline::Italic(vec![Inline::Text(
                "both".to_string()
            )])])]
        );
    }

    #[test]
    fn presentational_spans_become_semantic_marks() {
        let got = para(
            r#"<span style="font-style:italic">i</span><span style="text-decoration:line-through">s</span><span style="font-family:Consolas">c</span>"#,
        );
        assert_eq!(
            got,
            vec![
                Inline::Italic(vec![Inline::Text("i".to_string())]),
                Inline::Strike(vec![Inline::Text("s".to_string())]),
                Inline::Code("c".to_string()),
            ]
        );
    }

    #[test]
    fn divs_and_paragraphs_both_become_paragraphs() {
        assert_eq!(
            blocks("<div>one</div><p>two</p>"),
            vec![Block::para("one"), Block::para("two")]
        );
    }

    #[test]
    fn a_blank_line_div_does_not_become_a_paragraph() {
        // Teams writes `<div><br></div>` between paragraphs.
        assert_eq!(
            blocks("<div>one</div><div><br></div><div>two</div>"),
            vec![Block::para("one"), Block::para("two")]
        );
    }

    #[test]
    fn br_becomes_a_hard_break() {
        assert_eq!(
            para("a<br>b"),
            vec![
                Inline::Text("a".to_string()),
                Inline::HardBreak,
                Inline::Text("b".to_string()),
            ]
        );
    }

    #[test]
    fn headings_are_clamped_to_the_model_range() {
        assert_eq!(
            blocks("<h1>one</h1><h6>six</h6>"),
            vec![
                Block::heading(1, vec![Inline::text("one")]),
                Block::heading(3, vec![Inline::text("six")]),
            ]
        );
    }

    #[test]
    fn nested_lists_two_levels_deep() {
        let got = blocks("<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li><li>d</li></ul>");
        let inner = List::bulleted(vec![ListItem::of(vec![Inline::text("c")])]);
        let middle = List {
            items: vec![ListItem {
                blocks: vec![Block::para("b"), Block::List(inner)],
            }],
            ..List::bulleted(vec![])
        };
        let outer = List {
            items: vec![
                ListItem {
                    blocks: vec![Block::para("a"), Block::List(middle)],
                },
                ListItem::of(vec![Inline::text("d")]),
            ],
            ..List::bulleted(vec![])
        };
        assert_eq!(got, vec![Block::List(outer)]);
    }

    #[test]
    fn an_ordered_list_reads_its_start() {
        let Some(Block::List(list)) = blocks(r#"<ol start="3"><li>x</li></ol>"#).into_iter().next()
        else {
            panic!("expected a list");
        };
        assert!(list.ordered);
        assert_eq!(list.start, 3);
        assert!(list.tight);
    }

    #[test]
    fn paragraph_wrapped_items_make_a_loose_list() {
        let Some(Block::List(list)) = blocks("<ul><li><p>x</p></li></ul>").into_iter().next() else {
            panic!("expected a list");
        };
        assert!(!list.tight);
    }

    #[test]
    fn a_list_nested_without_an_li_wrapper_is_attached_to_the_previous_item() {
        let got = blocks("<ul><li>a</li><ul><li>b</li></ul></ul>");
        let Some(Block::List(list)) = got.into_iter().next() else {
            panic!("expected a list");
        };
        assert_eq!(list.items.len(), 1);
        assert_eq!(
            list.items[0].blocks,
            vec![
                Block::para("a"),
                Block::List(List::bulleted(vec![ListItem::of(vec![Inline::text("b")])])),
            ]
        );
    }

    #[test]
    fn blockquotes_nest() {
        assert_eq!(
            blocks("<blockquote><p>outer</p><blockquote>inner</blockquote></blockquote>"),
            vec![Block::BlockQuote(vec![
                Block::para("outer"),
                Block::BlockQuote(vec![Block::para("inner")]),
            ])]
        );
    }

    #[test]
    fn pre_is_verbatim_while_its_neighbours_collapse() {
        let got = blocks(
            "<p>a    b</p><pre><code class=\"language-rust\">fn main() {\n    let x = 1;\n}\n</code></pre><p>c\n\nd</p>",
        );
        assert_eq!(
            got,
            vec![
                Block::para("a b"),
                Block::CodeBlock {
                    lang: Some("rust".to_string()),
                    code: "fn main() {\n    let x = 1;\n}".to_string(),
                },
                Block::para("c d"),
            ]
        );
    }

    #[test]
    fn pre_without_a_language_class_has_no_lang() {
        assert_eq!(
            blocks("<pre>plain  text</pre>"),
            vec![Block::CodeBlock {
                lang: None,
                code: "plain  text".to_string(),
            }]
        );
    }

    #[test]
    fn br_inside_pre_is_a_newline() {
        assert_eq!(
            blocks("<pre>one<br>two</pre>"),
            vec![Block::CodeBlock {
                lang: None,
                code: "one\ntwo".to_string(),
            }]
        );
    }

    #[test]
    fn links_keep_their_target_and_title() {
        assert_eq!(
            para(r#"see <a href="https://example.com/a?b=1&amp;c=2" title="t">the docs</a>."#),
            vec![
                Inline::Text("see ".to_string()),
                Inline::Link {
                    href: "https://example.com/a?b=1&c=2".to_string(),
                    title: Some("t".to_string()),
                    content: vec![Inline::Text("the docs".to_string())],
                },
                Inline::Text(".".to_string()),
            ]
        );
    }

    #[test]
    fn whitespace_inside_a_link_is_not_clickable() {
        let got = para(r#"a<a href="https://example.com"> b</a>"#);
        assert_eq!(
            got,
            vec![
                Inline::Text("a ".to_string()),
                Inline::link("https://example.com", "b"),
            ]
        );
    }

    #[test]
    fn a_link_split_across_anchors_is_rejoined() {
        // Teams does this constantly. The normalizer merges them; this proves the parser feeds it
        // the shape it needs.
        let got = para(
            r#"<a href="https://example.com">exa</a><a href="https://example.com">mple</a>"#,
        );
        assert_eq!(got, vec![Inline::link("https://example.com", "example")]);
    }

    #[test]
    fn an_anchor_without_a_target_is_transparent() {
        assert_eq!(para("<a>plain</a>"), vec![Inline::text("plain")]);
    }

    #[test]
    fn thematic_breaks_survive() {
        assert_eq!(
            blocks("<p>a</p><hr><p>b</p>"),
            vec![Block::para("a"), Block::ThematicBreak, Block::para("b")]
        );
    }

    #[test]
    fn a_table_degrades_to_unsupported_with_its_cell_text() {
        let got = blocks("<table><tr><th>h1</th><th>h2</th></tr><tr><td>a</td><td>b</td></tr></table>");
        assert_eq!(
            got,
            vec![Block::Unsupported {
                kind: "table".to_string(),
                fallback: vec![
                    Inline::Text("h1 | h2".to_string()),
                    Inline::HardBreak,
                    Inline::Text("a | b".to_string()),
                ],
            }]
        );
    }

    #[test]
    fn images_degrade_to_their_alt_text() {
        assert_eq!(
            para(r#"<p>before <img src="x.png" alt="a cat"> after</p>"#),
            vec![Inline::text("before a cat after")]
        );
    }

    #[test]
    fn scripts_are_stripped_with_their_contents() {
        let got = blocks(r#"<div>safe<script>alert("xss")</script></div>"#);
        assert_eq!(got, vec![Block::para("safe")]);
        assert!(!sanitize(r#"<script>alert(1)</script>"#).contains("alert"));
    }

    #[test]
    fn iframes_and_event_handlers_are_stripped() {
        let cleaned = sanitize(r#"<div onclick="steal()"><iframe src="evil"></iframe>text</div>"#);
        assert!(!cleaned.contains("onclick"), "{cleaned}");
        assert!(!cleaned.contains("iframe"), "{cleaned}");
        assert_eq!(blocks(r#"<div onclick="steal()">text</div>"#), vec![Block::para("text")]);
    }

    #[test]
    fn the_style_attribute_survives_sanitizing() {
        // If ammonia ever starts stripping `style`, every Teams bold span silently becomes plain
        // text. That failure is invisible without this test.
        let cleaned = sanitize(r#"<span style="font-weight:600">x</span>"#);
        assert!(cleaned.contains("font-weight"), "{cleaned}");
    }

    #[test]
    fn non_breaking_spaces_are_not_collapsed() {
        assert_eq!(para("a\u{a0}\u{a0}b   c"), vec![Inline::text("a\u{a0}\u{a0}b c")]);
    }

    #[test]
    fn entities_are_decoded_not_doubled() {
        assert_eq!(para("&amp;lt; &lt; &amp;"), vec![Inline::text("&lt; < &")]);
    }

    #[test]
    fn a_bare_text_fragment_becomes_a_paragraph() {
        assert_eq!(blocks("  just text  "), vec![Block::para("just text")]);
    }

    #[test]
    fn empty_input_is_an_empty_document() {
        assert!(parse("").is_empty());
        assert!(parse("   \n  ").is_empty());
        assert!(parse("<div></div>").is_empty());
    }

    #[test]
    fn marks_survive_across_block_boundaries_inside_a_styled_container() {
        let got = blocks(r#"<div style="font-weight:600"><div>a</div><div>b</div></div>"#);
        assert_eq!(
            got,
            vec![
                Block::Paragraph(vec![Inline::bold("a")]),
                Block::Paragraph(vec![Inline::bold("b")]),
            ]
        );
    }
}
