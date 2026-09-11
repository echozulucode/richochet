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
//! 3. Walk the tree, wrapping each element's parsed children in the marks that element turns on
//!    and collapsing whitespace per the HTML rules on the way.
//! 4. Hand the blocks to [`Document::from_blocks`], which normalizes them.
//!
//! Step 3 wraps rather than distributes, and that distinction is load-bearing. Stamping the full
//! set of active marks onto every text run would turn `<em>a</em><s><em>b</em>c</s>` into three
//! independent leaves and lose the grouping the source had; re-factoring those leaves afterwards
//! is not a unique inverse, so the tree comes back a different — if equivalent — shape and a
//! round trip through Markdown stops converging. Wrapping the *result of parsing an element's
//! children* keeps the shape the author wrote.

use std::collections::{HashMap, HashSet};

use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns, parse_fragment, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::model::{
    Alignment, Block, Cell, Document, Inline, List, ListItem, Row, Table,
};
use crate::html::styles::{declarations, marks_for, MarkSet};

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
        walk_children(&root, Ctx::default(), &mut builder);
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
        "a",
        "abbr",
        "address",
        "article",
        "aside",
        "b",
        "big",
        "blockquote",
        "br",
        "caption",
        "center",
        "cite",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "details",
        "dfn",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hgroup",
        "hr",
        "i",
        "img",
        "ins",
        "kbd",
        "li",
        "main",
        "mark",
        "nav",
        "ol",
        "p",
        "pre",
        "q",
        "s",
        "samp",
        "section",
        "small",
        "span",
        "strike",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "tt",
        "u",
        "ul",
        "var",
        "wbr",
    ]);
    // Attributes allowed anywhere. `style` is the whole point of `styles.rs`; `class` carries the
    // `language-*` hint on code blocks.
    let generic_attributes: HashSet<&str> = HashSet::from(["style", "class", "title", "lang"]);
    // Table attributes are listed on every element that can carry them: `align` is the legacy
    // spelling of `text-align` and Word writes it far more often than the CSS form, and a `colspan`
    // that ammonia strips would silently shift every following cell one column to the left.
    let tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::from([
        ("a", HashSet::from(["href"])),
        ("ol", HashSet::from(["start"])),
        ("img", HashSet::from(["alt", "src"])),
        ("table", HashSet::from(["align"])),
        ("thead", HashSet::from(["align"])),
        ("tbody", HashSet::from(["align"])),
        ("tfoot", HashSet::from(["align"])),
        ("tr", HashSet::from(["align"])),
        ("colgroup", HashSet::from(["align", "span"])),
        ("col", HashSet::from(["align", "span"])),
        ("td", HashSet::from(["colspan", "rowspan", "align"])),
        (
            "th",
            HashSet::from(["colspan", "rowspan", "scope", "align"]),
        ),
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
// Marks
// ---------------------------------------------------------------------------------------------

/// A mark expressed by *wrapping* a run of inline nodes.
///
/// Code is absent on purpose: [`Inline::Code`] is a leaf holding a string rather than a wrapper
/// around other nodes, so code is the one mark that has to be applied run by run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Bold,
    Italic,
    Strike,
}

/// Innermost first. Wrapping in this order leaves bold outermost, which is the model's canonical
/// nesting order, so the normalizer has nothing to reorder.
const WRAP_ORDER: [Mark; 3] = [Mark::Strike, Mark::Italic, Mark::Bold];

/// A set of wrapper marks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MarkFlags {
    bold: bool,
    italic: bool,
    strike: bool,
}

impl MarkFlags {
    fn get(self, mark: Mark) -> bool {
        match mark {
            Mark::Bold => self.bold,
            Mark::Italic => self.italic,
            Mark::Strike => self.strike,
        }
    }

    fn set(&mut self, mark: Mark, value: bool) {
        match mark {
            Mark::Bold => self.bold = value,
            Mark::Italic => self.italic = value,
            Mark::Strike => self.strike = value,
        }
    }

    fn any(self) -> bool {
        self.bold || self.italic || self.strike
    }

    fn union(self, other: Self) -> Self {
        MarkFlags {
            bold: self.bold || other.bold,
            italic: self.italic || other.italic,
            strike: self.strike || other.strike,
        }
    }
}

/// Which marks are in force where the walk currently stands.
#[derive(Debug, Clone, Copy, Default)]
struct Ctx {
    /// Every mark in force, however it got there. This is what decides whether an element's
    /// `font-weight: normal` has anything to cancel.
    active: MarkSet,
    /// The subset of `active` that no enclosing inline element wraps. A
    /// `<div style="font-weight:600">` leaves nothing to wrap — its children are blocks, and a
    /// mark cannot span a block boundary — so its mark has to ride down to the text runs instead.
    distribute: MarkSet,
}

impl Ctx {
    /// Enter an element whose marks have nothing to wrap, so they are carried to the leaves.
    fn distributing(self, declared: MarkSet) -> Ctx {
        Ctx {
            active: self.active.merge(declared),
            distribute: self.distribute.merge(declared),
        }
    }

    /// Enter an inline element. The marks in `add` are wrapped by the caller, so they leave
    /// `distribute`: carrying them further would bold each text run a second time.
    fn wrapping(self, declared: MarkSet, add: &[Mark]) -> Ctx {
        let mut distribute = self.distribute.merge(declared);
        for mark in add {
            match mark {
                Mark::Bold => distribute.bold = None,
                Mark::Italic => distribute.italic = None,
                Mark::Strike => distribute.strike = None,
            }
        }
        Ctx {
            active: self.active.merge(declared),
            distribute,
        }
    }
}

/// What an element does to the marks in force: which it turns on, for the caller to wrap, and
/// which it turns off, for the caller to carve out of the wrap an ancestor will apply.
fn mark_delta(active: MarkSet, declared: MarkSet) -> (Vec<Mark>, MarkFlags) {
    let mut add = Vec::new();
    let mut cancel = MarkFlags::default();
    for mark in WRAP_ORDER {
        match opinion(declared, mark) {
            // A mark already in force is not wrapped again: it would only give the normalizer
            // work to undo.
            Some(true) if opinion(active, mark) != Some(true) => add.push(mark),
            Some(false) if opinion(active, mark) == Some(true) => cancel.set(mark, true),
            _ => {}
        }
    }
    (add, cancel)
}

fn opinion(marks: MarkSet, mark: Mark) -> Option<bool> {
    match mark {
        Mark::Bold => marks.bold,
        Mark::Italic => marks.italic,
        Mark::Strike => marks.strike,
    }
}

fn wrap(mark: Mark, nodes: Vec<Inline>) -> Inline {
    match mark {
        Mark::Bold => Inline::Bold(nodes),
        Mark::Italic => Inline::Italic(nodes),
        Mark::Strike => Inline::Strike(nodes),
    }
}

// ---------------------------------------------------------------------------------------------
// Inline accumulation
// ---------------------------------------------------------------------------------------------

/// A run of inline nodes together with the marks it *refuses*.
///
/// A run refuses a mark when something inside it turned that mark off — Teams nests a
/// `font-weight: normal` span inside a bold run, and the text in it must come out unbolded. The
/// enclosing element that turns the mark on skips such a run when it wraps, splitting the bold
/// around the exempt text instead of swallowing it.
struct Segment {
    cancels: MarkFlags,
    nodes: Vec<Inline>,
}

/// The inline nodes collected for one element, cut into segments wherever a descendant refused a
/// mark. Content that refuses nothing — the overwhelming majority — accumulates in `current` and
/// never becomes a segment boundary at all.
#[derive(Default)]
struct Frame {
    done: Vec<Segment>,
    current: Vec<Inline>,
}

impl Frame {
    fn is_empty(&self) -> bool {
        self.done.is_empty() && self.current.is_empty()
    }

    fn push_node(&mut self, node: Inline) {
        self.current.push(node);
    }

    fn close_current(&mut self) {
        if !self.current.is_empty() {
            self.done.push(Segment {
                cancels: MarkFlags::default(),
                nodes: std::mem::take(&mut self.current),
            });
        }
    }

    fn push_segment(&mut self, segment: Segment) {
        if segment.cancels.any() {
            self.close_current();
            self.done.push(segment);
        } else {
            self.current.extend(segment.nodes);
        }
    }

    fn finish(mut self) -> Vec<Segment> {
        self.close_current();
        self.done
    }
}

/// Wrap a frame's segments in `marks`, leaving out the runs that refuse each one.
fn apply_marks(segments: Vec<Segment>, marks: &[Mark]) -> Vec<Segment> {
    let mut out = segments;
    for &mark in marks {
        out = apply_mark(out, mark);
    }
    out
}

fn apply_mark(segments: Vec<Segment>, mark: Mark) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    let mut segments = segments.into_iter().peekable();
    while let Some(first) = segments.next() {
        if first.cancels.get(mark) {
            // This run opted out. Pass it through unwrapped; the refusal has now been honoured
            // and must not travel any further up.
            let mut passed = first;
            passed.cancels.set(mark, false);
            out.push(passed);
            continue;
        }
        // Take in the following runs that refuse exactly the same marks, so that one wrapper
        // covers as much as it can. Runs refusing something different cannot be merged without
        // changing what they refuse.
        let cancels = first.cancels;
        let mut nodes = first.nodes;
        while segments.peek().is_some_and(|next| next.cancels == cancels) {
            nodes.extend(segments.next().expect("just peeked").nodes);
        }
        out.push(Segment {
            cancels,
            nodes: vec![wrap(mark, nodes)],
        });
    }
    out
}

fn flatten_segments(segments: Vec<Segment>) -> Vec<Inline> {
    segments.into_iter().flat_map(|s| s.nodes).collect()
}

/// Accumulates the inline run of the block currently being built, applying the HTML whitespace
/// rules as it goes.
///
/// Whitespace is handled by *deferring* it: a collapsible run of whitespace sets `space_pending`
/// rather than emitting anything, and the space is only materialized once something follows it in
/// the same block. That drops leading and trailing block whitespace for free, and it is the
/// reason `<div><span style="font-weight:600"> Important </span></div>` comes out as a bold
/// "Important" with no stray spaces at all.
///
/// `frames` is a stack: an element that contributes marks — and every `<a>` — collects its own
/// children in a frame of its own, without losing continuity of the whitespace state with the
/// text around it.
struct InlineBuilder {
    frames: Vec<Frame>,
    /// A collapsible whitespace run has been seen and not yet emitted.
    space_pending: bool,
    /// Something has been emitted since the start of the block (or since the last `<br>`), so a
    /// pending space is now meaningful rather than leading whitespace.
    emitted: bool,
}

impl InlineBuilder {
    fn new() -> Self {
        InlineBuilder {
            frames: vec![Frame::default()],
            space_pending: false,
            emitted: false,
        }
    }

    fn current(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("the base frame is never popped")
    }

    fn is_empty(&self) -> bool {
        self.frames.iter().all(Frame::is_empty)
    }

    /// Emit a deferred space, if one is owed and would not be leading whitespace.
    fn flush_space(&mut self) {
        if self.space_pending && self.emitted {
            self.current().push_node(Inline::Text(" ".to_string()));
        }
        self.space_pending = false;
    }

    /// Add text, collapsing whitespace per the HTML rules.
    fn push_text(&mut self, raw: &str, ctx: Ctx) {
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
        let node = leaf(core.to_string(), ctx.distribute);
        self.current().push_node(node);
        self.emitted = true;
        self.space_pending = trailing;
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
        self.current().push_node(Inline::HardBreak);
        // Whitespace after a break is leading whitespace again.
        self.emitted = false;
    }

    fn push_frame(&mut self) {
        // Settle any owed space against the *parent* frame, so that it does not end up inside the
        // mark the child is about to be wrapped in.
        self.flush_space();
        self.frames.push(Frame::default());
    }

    fn pop_frame(&mut self) -> Vec<Segment> {
        self.frames.pop().unwrap_or_default().finish()
    }

    /// Fold a child element's finished segments into the frame it sits in.
    ///
    /// Deliberately does not settle a pending space: by the time a child's segments come back,
    /// an owed space is the child's own *trailing* whitespace, which belongs after this content
    /// and not before it. Whitespace owed from *before* the child was already settled by
    /// [`InlineBuilder::push_frame`].
    fn push_segments(&mut self, segments: Vec<Segment>) {
        if segments.iter().all(|s| s.nodes.is_empty()) {
            return;
        }
        for segment in segments {
            if !segment.nodes.is_empty() {
                self.current().push_segment(segment);
            }
        }
        self.emitted = true;
    }

    /// Take the accumulated run and reset for the next block.
    ///
    /// Any refusals still outstanding are discharged here: they could only be honoured by a mark
    /// an ancestor wraps, and no mark crosses a block boundary.
    fn take(&mut self) -> Vec<Inline> {
        let frame = std::mem::take(self.current());
        self.space_pending = false;
        self.emitted = false;
        flatten_segments(frame.finish())
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

/// Build a text leaf, carrying the marks that have no enclosing element to wrap them.
///
/// Code is always applied here, because [`Inline::Code`] is a leaf and there is nothing else for
/// it to wrap.
fn leaf(text: String, distribute: MarkSet) -> Inline {
    let mut node = if distribute.is_code() {
        Inline::Code(text)
    } else {
        Inline::Text(text)
    };
    if distribute.is_strike() {
        node = Inline::Strike(vec![node]);
    }
    if distribute.is_italic() {
        node = Inline::Italic(vec![node]);
    }
    if distribute.is_bold() {
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

fn walk_children(node: &Handle, ctx: Ctx, out: &mut BlockBuilder) {
    // Collect first: the recursive walk must not hold a borrow on the children list.
    let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
    for child in &children {
        walk_node(child, ctx, out);
    }
}

fn walk_node(node: &Handle, ctx: Ctx, out: &mut BlockBuilder) {
    match &node.data {
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            out.inlines.push_text(&text, ctx);
        }
        NodeData::Element { .. } => walk_element(node, ctx, out),
        // Documents, doctypes, comments and processing instructions carry nothing we want.
        _ => {}
    }
}

fn walk_element(node: &Handle, ctx: Ctx, out: &mut BlockBuilder) {
    let Some(tag) = tag_name(node) else {
        return;
    };
    // Every element, block or inline, can contribute marks to its descendants.
    let declared = marks_for(&tag, attr(node, "style").as_deref());

    match tag.as_str() {
        "p" => {
            out.saw_paragraph = true;
            push_container(node, ctx.distributing(declared), out);
        }
        "div" | "section" | "article" | "header" | "footer" | "main" | "aside" | "figure"
        | "figcaption" | "address" | "center" | "dl" | "dt" | "dd" | "li" | "details"
        | "summary" | "caption" => push_container(node, ctx.distributing(declared), out),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<u8>().unwrap_or(1);
            let content = inline_subtree(node, ctx.distributing(declared));
            out.push_block(Block::heading(level, content));
        }
        "ul" | "ol" => {
            let list = build_list(node, &tag, ctx.distributing(declared));
            out.push_block(Block::List(list));
        }
        "blockquote" => {
            let mut sub = BlockBuilder::new();
            walk_children(node, ctx.distributing(declared), &mut sub);
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
            let inner = ctx.distributing(declared);
            // A caption has nowhere to live in the model, so it is emitted as a paragraph
            // immediately before the table. See [`caption_paragraph`].
            if let Some(caption) = caption_paragraph(node, inner) {
                out.push_block(Block::Paragraph(caption));
            }
            out.push_block(Block::Table(build_table(node, inner)));
        }
        "br" => out.inlines.push_break(),
        "a" => walk_anchor(node, ctx, declared, out),
        "img" => {
            // An image cannot be represented, but its alt text is the author's own description of
            // it and is worth more than nothing.
            if let Some(alt) = attr(node, "alt") {
                out.inlines.push_text(&alt, ctx.distributing(declared));
            }
        }
        // `<span>`, `<b>`, `<code>` and everything unrecognized.
        _ => walk_inline(node, ctx, declared, out),
    }
}

/// Walk an inline element: whatever marks it turns on wrap the *result* of parsing its children,
/// and whatever it turns off is carved out of the wrap an ancestor will apply.
///
/// This is the heart of preserving shape rather than merely preserving meaning.
fn walk_inline(node: &Handle, ctx: Ctx, declared: MarkSet, out: &mut BlockBuilder) {
    let (add, cancel) = mark_delta(ctx.active, declared);
    let inner = ctx.wrapping(declared, &add);

    // Nothing to wrap and nothing to carve out: the element is purely transparent, and a frame
    // would only be overhead.
    if add.is_empty() && !cancel.any() {
        walk_children(node, inner, out);
        return;
    }

    out.inlines.push_frame();
    walk_children(node, inner, out);
    let mut segments = apply_marks(out.inlines.pop_frame(), &add);
    for segment in &mut segments {
        segment.cancels = segment.cancels.union(cancel);
    }
    out.inlines.push_segments(segments);
}

/// Walk a block container: its content becomes blocks of its own, never merged with the inline
/// run that surrounded it.
fn push_container(node: &Handle, ctx: Ctx, out: &mut BlockBuilder) {
    let mut sub = BlockBuilder::new();
    walk_children(node, ctx, &mut sub);
    let blocks = sub.finish();
    if blocks.is_empty() {
        return;
    }
    out.flush();
    out.blocks.extend(blocks);
}

fn walk_anchor(node: &Handle, ctx: Ctx, declared: MarkSet, out: &mut BlockBuilder) {
    let Some(href) = attr(node, "href") else {
        // An anchor with no target is just a styled span (Teams uses them as mention wrappers).
        walk_inline(node, ctx, declared, out);
        return;
    };
    let (add, cancel) = mark_delta(ctx.active, declared);

    out.inlines.push_frame();
    walk_children(node, ctx.wrapping(declared, &add), out);
    // A link is a single node, so it cannot be split around a refusal the way a mark can: any
    // refusal from inside the label is discharged here.
    let mut content = flatten_segments(apply_marks(out.inlines.pop_frame(), &add));

    // Whitespace between the previous text and the link belongs outside the link: the clickable
    // region should not start with a space. (The normalizer deliberately does not hoist
    // whitespace out of links, because for links it would change what is clickable.)
    let leading_space = matches!(content.first(), Some(Inline::Text(t)) if t == " ");
    if leading_space {
        content.remove(0);
    }

    // Any space owed at this point is the label's own trailing whitespace, which belongs *after*
    // the link. Hold it while the leading space is settled in front.
    let trailing_space = out.inlines.space_pending;
    out.inlines.space_pending = leading_space;
    out.inlines.flush_space();
    out.inlines.push_segments(vec![Segment {
        cancels: cancel,
        nodes: vec![Inline::Link {
            href,
            title: attr(node, "title"),
            content,
        }],
    }]);
    out.inlines.space_pending = trailing_space;
}

fn build_list(node: &Handle, tag: &str, ctx: Ctx) -> List {
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
                let item_ctx = ctx.distributing(marks_for("li", attr(child, "style").as_deref()));
                let mut sub = BlockBuilder::new();
                walk_children(child, item_ctx, &mut sub);
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
                let nested = Block::List(build_list(child, &child_tag, ctx));
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
fn inline_subtree(node: &Handle, ctx: Ctx) -> Vec<Inline> {
    let mut sub = BlockBuilder::new();
    walk_children(node, ctx, &mut sub);
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
            Block::Table(table) => flatten_table(table),
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

/// Reduce a table to a single inline run, for the places the model has no room for one.
///
/// Reached only by [`flatten_blocks`] — that is, by a table nested inside a heading or inside
/// another table's cell. Supporting nested tables properly would mean a cell type that can hold
/// blocks, which is a large change to the frozen model for a shape almost nothing produces, so an
/// inner table degrades to its cell text instead: cells separated by a pipe, rows by a break.
fn flatten_table(table: Table) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for row in std::iter::once(table.head).chain(table.rows) {
        let cells: Vec<Cell> = row.into_iter().filter(|cell| !cell.is_empty()).collect();
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

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

/// The largest `colspan`, `rowspan` or `<col span>` this module will honour.
///
/// Spans arrive from another process on the clipboard and are never validated by anything, so
/// `colspan="100000000"` is a plausible input. Expanding one into empty cells buys nothing and
/// costs a hang, so spans are clamped instead.
const MAX_SPAN: usize = 1000;

/// A `<td>`/`<th>` as read from the DOM, before its spans are laid out on the grid.
struct RawCell {
    content: Vec<Inline>,
    colspan: usize,
    rowspan: usize,
    align: Alignment,
}

/// A `<tr>` as read from the DOM.
struct RawRow {
    cells: Vec<RawCell>,
    /// The row came out of a `<thead>`.
    in_head: bool,
    /// The row has cells and every one of them is a `<th>`.
    all_header: bool,
}

/// Build a [`Table`] from a `<table>` element.
fn build_table(node: &Handle, ctx: Ctx) -> Table {
    let raw = collect_rows(node, ctx);
    let head_index = header_index(&raw);
    let column_align = column_alignments(node);
    let mut placed = place_grid(raw);

    // `place_grid` keeps head and body on one grid, so a `rowspan` reaching out of the `<thead>`
    // still shifts the body rows it covers. The header row is lifted out afterwards.
    let (head, head_align) = match head_index {
        Some(index) if index < placed.len() => placed.remove(index),
        _ => (Row::new(), Vec::new()),
    };
    let body_align = placed.first().map(|(_, a)| a.clone()).unwrap_or_default();
    let rows: Vec<Row> = placed.into_iter().map(|(row, _)| row).collect();

    // Alignment comes from the header row when there is one, because that is the row an author
    // styles; otherwise from the first body row. `<col>` fills in whatever neither declared.
    let cell_align = if head_index.is_some() {
        head_align
    } else {
        body_align
    };

    Table {
        head,
        align: merge_alignments(&cell_align, &column_align),
        rows,
    }
}

/// Read every `<tr>` under a `<table>`, in document order.
///
/// `<tbody>`, `<tfoot>` and a bare `<tr>` all contribute body rows; only `<thead>` is special. A
/// `<tfoot>` keeps its document position rather than being moved to the end: the model has no
/// footer, and reordering content is a bigger surprise than leaving it where the author put it.
fn collect_rows(table: &Handle, ctx: Ctx) -> Vec<RawRow> {
    let mut rows = Vec::new();
    for child in child_elements(table) {
        let in_head = match tag_name(&child).as_deref() {
            Some("thead") => true,
            Some("tbody") | Some("tfoot") => false,
            // html5ever normally moves a bare `<tr>` into an implied `<tbody>`, but a fragment
            // parse does not always, so handle it here too.
            Some("tr") => {
                rows.push(read_row(&child, false, ctx));
                continue;
            }
            _ => continue,
        };
        for tr in child_elements(&child) {
            if tag_name(&tr).as_deref() == Some("tr") {
                rows.push(read_row(&tr, in_head, ctx));
            }
        }
    }
    rows
}

/// Read one `<tr>` and its cells.
fn read_row(tr: &Handle, in_head: bool, ctx: Ctx) -> RawRow {
    let row_ctx = ctx.distributing(marks_for("tr", attr(tr, "style").as_deref()));
    let row_align = align_of(tr);

    let mut cells = Vec::new();
    let mut all_header = true;
    for cell in child_elements(tr) {
        let tag = tag_name(&cell).unwrap_or_default();
        match tag.as_str() {
            "th" => {}
            "td" => all_header = false,
            _ => continue,
        }
        let cell_ctx = row_ctx.distributing(marks_for(&tag, attr(&cell, "style").as_deref()));
        cells.push(RawCell {
            // Cell content is inline content, run through the same machinery as everything else,
            // so marks, links and code inside a cell survive.
            content: inline_subtree(&cell, cell_ctx),
            colspan: span_attr(&cell, "colspan"),
            rowspan: span_attr(&cell, "rowspan"),
            align: first_align(align_of(&cell), row_align),
        });
    }

    RawRow {
        all_header: all_header && !cells.is_empty(),
        cells,
        in_head,
    }
}

/// Which row, if any, is the header.
///
/// A `<thead>` says so outright. Failing that, a first row made up entirely of `<th>` is the
/// convention every producer uses; anything else leaves the header empty and puts every row in the
/// body, because guessing wrong promotes real data into a header where it cannot be read back.
fn header_index(rows: &[RawRow]) -> Option<usize> {
    if let Some(index) = rows.iter().position(|row| row.in_head) {
        return Some(index);
    }
    rows.first().filter(|row| row.all_header).map(|_| 0)
}

/// Lay the rows out on a grid, expanding `colspan` and `rowspan` into the cells they cover.
///
/// **Neither span can be represented.** A [`Row`] is a flat list of cells with no notion of one
/// cell covering several, and widening the model to carry spans would change a contract four other
/// modules depend on for a feature GFM cannot express either. Dropping the content would be worse
/// than changing its shape, so a spanning cell keeps its content in the first grid slot it covers
/// and every further slot it covers becomes an empty cell. A merged table therefore comes out
/// rectangular, with the merge showing as blanks rather than as missing text — and, importantly,
/// with the cells *after* a span still in their own columns instead of shifted left.
///
/// Returns each row together with the per-column alignment its cells declared.
fn place_grid(rows: Vec<RawRow>) -> Vec<(Row, Vec<Alignment>)> {
    let mut out = Vec::with_capacity(rows.len());
    // For each column, how many rows are still covered by a `rowspan` opened above.
    let mut blocked: Vec<usize> = Vec::new();

    for raw in rows {
        let mut cells = Row::new();
        let mut aligns: Vec<Alignment> = Vec::new();
        let mut column = 0usize;

        for cell in raw.cells {
            // Step over the columns a `rowspan` from an earlier row already owns.
            while blocked.get(column).copied().unwrap_or(0) > 0 {
                cells.push(Cell::new());
                aligns.push(Alignment::None);
                column += 1;
            }
            let mut content = Some(cell.content);
            for _ in 0..cell.colspan {
                if blocked.len() <= column {
                    blocked.resize(column + 1, 0);
                }
                // This row plus the `rowspan - 1` rows below it; the decrement at the end of the
                // row settles the count.
                blocked[column] = cell.rowspan;
                cells.push(content.take().unwrap_or_default());
                aligns.push(cell.align);
                column += 1;
            }
        }

        for count in blocked.iter_mut() {
            *count = count.saturating_sub(1);
        }
        out.push((cells, aligns));
    }
    out
}

/// Per-column alignment declared by `<colgroup>` / `<col>`.
fn column_alignments(table: &Handle) -> Vec<Alignment> {
    let mut out = Vec::new();
    for child in child_elements(table) {
        match tag_name(&child).as_deref() {
            Some("colgroup") => {
                let group = align_of(&child);
                let cols: Vec<Handle> = child_elements(&child)
                    .into_iter()
                    .filter(|c| tag_name(c).as_deref() == Some("col"))
                    .collect();
                if cols.is_empty() {
                    // A `<colgroup span="3">` with no children spans that many columns itself.
                    out.extend(std::iter::repeat_n(group, span_attr(&child, "span")));
                }
                for col in cols {
                    let align = first_align(align_of(&col), group);
                    out.extend(std::iter::repeat_n(align, span_attr(&col, "span")));
                }
            }
            Some("col") => {
                let align = align_of(&child);
                out.extend(std::iter::repeat_n(align, span_attr(&child, "span")));
            }
            _ => {}
        }
    }
    out
}

/// Let the cells' alignment win where they have one, falling back to `<col>`.
///
/// Trailing [`Alignment::None`]s are dropped: [`Table::alignment`] defaults when `align` is short,
/// so keeping them would only make two equal tables compare unequal.
fn merge_alignments(cells: &[Alignment], columns: &[Alignment]) -> Vec<Alignment> {
    let width = cells.len().max(columns.len());
    let mut out: Vec<Alignment> = (0..width)
        .map(|i| {
            first_align(
                cells.get(i).copied().unwrap_or_default(),
                columns.get(i).copied().unwrap_or_default(),
            )
        })
        .collect();
    while out.last() == Some(&Alignment::None) {
        out.pop();
    }
    out
}

/// The alignment an element declares, from `style="text-align:..."` or the legacy `align="..."`.
///
/// CSS wins over the presentational attribute, which is what a browser does.
fn align_of(node: &Handle) -> Alignment {
    if let Some(style) = attr(node, "style") {
        for (property, value) in declarations(&style) {
            if property == "text-align" {
                if let Some(align) = parse_align(&value) {
                    return align;
                }
            }
        }
    }
    attr(node, "align")
        .and_then(|value| parse_align(&value.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn parse_align(value: &str) -> Option<Alignment> {
    match value.trim() {
        "left" | "start" => Some(Alignment::Left),
        "center" | "centre" => Some(Alignment::Center),
        "right" | "end" => Some(Alignment::Right),
        // `justify`, `inherit` and anything malformed are no opinion at all.
        _ => None,
    }
}

/// `a` unless it has no opinion, in which case `b`.
fn first_align(a: Alignment, b: Alignment) -> Alignment {
    if a == Alignment::None {
        b
    } else {
        a
    }
}

/// A `colspan` / `rowspan` / `span` attribute, defaulting to 1 and clamped to [`MAX_SPAN`].
fn span_attr(node: &Handle, name: &str) -> usize {
    attr(node, name)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_SPAN)
}

/// A table's `<caption>`, as the inline content of a paragraph.
///
/// The model has no field for a caption and will not grow one for a single string, so the choice
/// is where to put the text rather than whether to keep it. A paragraph immediately *before* the
/// table is where a reader expects a caption, is what HTML itself renders by default, and is the
/// only placement that survives Markdown — folding it into a cell would put prose in the data.
fn caption_paragraph(table: &Handle, ctx: Ctx) -> Option<Vec<Inline>> {
    for child in child_elements(table) {
        if tag_name(&child).as_deref() != Some("caption") {
            continue;
        }
        let caption_ctx = ctx.distributing(marks_for("caption", attr(&child, "style").as_deref()));
        let content = inline_subtree(&child, caption_ctx);
        if !content.is_empty() {
            return Some(content);
        }
    }
    None
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
        NodeData::Element { name, .. } => Some((*name.local).to_ascii_lowercase()),
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
        .find(|a| (*a.name.local).eq_ignore_ascii_case(wanted))
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
    fn sibling_marks_keep_the_grouping_the_source_had() {
        // Found by the round-trip property test. Distributing marks to the leaves would give
        // `Italic(a), Italic(Strike(b)), Strike(c)`, which re-factors into an equivalent but
        // differently shaped tree — and the shape is what the Markdown renderer writes out, so
        // the round trip stopped converging. Wrapping keeps the author's grouping.
        let got = para("<em>plain</em><s><em>plain</em>plain</s>");
        assert_eq!(
            got,
            vec![
                Inline::Italic(vec![Inline::Text("plain".to_string())]),
                Inline::Strike(vec![
                    Inline::Italic(vec![Inline::Text("plain".to_string())]),
                    Inline::Text("plain".to_string()),
                ]),
            ]
        );
    }

    #[test]
    fn the_reported_round_trip_repro_survives_intact() {
        // `> - # *plain*~~*plain*plain~~`, the exact case from the property-test failure.
        let html = concat!(
            "<blockquote><ul><li><h1><em>plain</em>",
            "<s><em>plain</em>plain</s></h1></li></ul></blockquote>"
        );
        let markdown = crate::markdown::render(&parse(html));
        assert_eq!(markdown.trim_end(), "> - # *plain*~~*plain*plain~~");
    }

    #[test]
    fn a_cancelled_mark_splits_the_run_it_sits_in() {
        let got = para(r#"<b>a<span style="font-weight:normal">b</span>c</b>"#);
        assert_eq!(
            got,
            vec![
                Inline::bold("a"),
                Inline::Text("b".to_string()),
                Inline::bold("c"),
            ]
        );
    }

    #[test]
    fn a_cancellation_lifts_only_the_mark_it_names() {
        // The italic still covers both runs; only the bold is carved out. The parser emits
        // `Bold(Italic(x)), Italic(y)` and the normalizer then factors the shared italic out
        // across the two runs, which is the same document either way.
        let got = para(r#"<b><em>x<span style="font-weight:normal">y</span></em></b>"#);
        assert_eq!(
            got,
            vec![Inline::Italic(vec![
                Inline::Bold(vec![Inline::Text("x".to_string())]),
                Inline::Text("y".to_string()),
            ])]
        );
    }

    #[test]
    fn one_span_declaring_several_marks_nests_them_canonically() {
        let got = para(r#"<span style="font-weight:600;font-style:italic">x</span>"#);
        assert_eq!(
            got,
            vec![Inline::Bold(vec![Inline::Italic(vec![Inline::Text(
                "x".to_string()
            )])])]
        );
    }

    #[test]
    fn a_block_container_carrying_a_mark_still_reaches_every_run() {
        // Nothing for the div to wrap — its children are blocks — so the mark rides down to the
        // text runs instead, and `font-weight:normal` inside still cancels it.
        let got = blocks(concat!(
            r#"<div style="font-weight:600">a<em>b</em>"#,
            r#"<span style="font-weight:normal">c</span></div>"#
        ));
        assert_eq!(
            got,
            vec![Block::Paragraph(vec![
                Inline::Bold(vec![
                    Inline::Text("a".to_string()),
                    Inline::Italic(vec![Inline::Text("b".to_string())]),
                ]),
                Inline::Text("c".to_string()),
            ])]
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
        let Some(Block::List(list)) = blocks(r#"<ol start="3"><li>x</li></ol>"#)
            .into_iter()
            .next()
        else {
            panic!("expected a list");
        };
        assert!(list.ordered);
        assert_eq!(list.start, 3);
        assert!(list.tight);
    }

    #[test]
    fn paragraph_wrapped_items_make_a_loose_list() {
        let Some(Block::List(list)) = blocks("<ul><li><p>x</p></li></ul>").into_iter().next()
        else {
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
        let got =
            para(r#"<a href="https://example.com">exa</a><a href="https://example.com">mple</a>"#);
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

    // ---------------------------------------------------------------------------------------
    // Tables
    // ---------------------------------------------------------------------------------------

    /// The single table a fixture parses to, for the many cases that produce exactly one.
    fn table(html: &str) -> Table {
        match blocks(html).into_iter().next() {
            Some(Block::Table(t)) => t,
            other => panic!("expected a table, got {other:?}"),
        }
    }

    fn cell(text: &str) -> Cell {
        vec![Inline::text(text)]
    }

    #[test]
    fn a_thead_is_the_header_row() {
        let got = table(concat!(
            "<table><thead><tr><th>H1</th><th>H2</th></tr></thead>",
            "<tbody><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></tbody></table>"
        ));
        assert_eq!(got.head, vec![cell("H1"), cell("H2")]);
        assert_eq!(
            got.rows,
            vec![vec![cell("a"), cell("b")], vec![cell("c"), cell("d")]]
        );
        assert_eq!(got.align, vec![]);
        assert_eq!(got.columns(), 2);
    }

    #[test]
    fn without_a_thead_an_all_th_first_row_is_the_header() {
        let got =
            table("<table><tr><th>H1</th><th>H2</th></tr><tr><td>a</td><td>b</td></tr></table>");
        assert_eq!(got.head, vec![cell("H1"), cell("H2")]);
        assert_eq!(got.rows, vec![vec![cell("a"), cell("b")]]);
    }

    #[test]
    fn a_table_with_no_header_puts_everything_in_the_body() {
        let got =
            table("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>");
        assert!(got.head.is_empty());
        assert_eq!(
            got.rows,
            vec![vec![cell("a"), cell("b")], vec![cell("c"), cell("d")]]
        );
    }

    #[test]
    fn a_mixed_first_row_is_data_not_a_header() {
        // Promoting it would move real data somewhere it can never be read back from.
        let got = table("<table><tr><th>label</th><td>value</td></tr></table>");
        assert!(got.head.is_empty());
        assert_eq!(got.rows, vec![vec![cell("label"), cell("value")]]);
    }

    #[test]
    fn tbody_tfoot_and_bare_rows_all_contribute_body_rows() {
        let got = table(concat!(
            "<table><thead><tr><th>h</th></tr></thead>",
            "<tbody><tr><td>body</td></tr></tbody>",
            "<tfoot><tr><td>foot</td></tr></tfoot></table>"
        ));
        assert_eq!(got.head, vec![cell("h")]);
        assert_eq!(got.rows, vec![vec![cell("body")], vec![cell("foot")]]);
    }

    #[test]
    fn extra_thead_rows_stay_in_the_body() {
        // The model holds one header row; the rest are content and keep their order.
        let got = table(concat!(
            "<table><thead><tr><th>H</th></tr><tr><th>sub</th></tr></thead>",
            "<tbody><tr><td>a</td></tr></tbody></table>"
        ));
        assert_eq!(got.head, vec![cell("H")]);
        assert_eq!(got.rows, vec![vec![cell("sub")], vec![cell("a")]]);
    }

    #[test]
    fn ragged_rows_are_kept_ragged() {
        // The model tolerates this on purpose; padding is the renderer's job, not the parser's.
        let got = table(concat!(
            "<table><tr><td>a</td><td>b</td><td>c</td></tr>",
            "<tr><td>d</td></tr></table>"
        ));
        assert_eq!(
            got.rows,
            vec![vec![cell("a"), cell("b"), cell("c")], vec![cell("d")]]
        );
        assert_eq!(got.columns(), 3);
    }

    #[test]
    fn alignment_comes_from_the_header_row_either_spelling() {
        let got = table(concat!(
            r#"<table><thead><tr><th align="RIGHT">a</th>"#,
            r#"<th style="text-align: center">b</th><th>c</th></tr></thead>"#,
            "<tbody><tr><td>1</td><td>2</td><td>3</td></tr></tbody></table>"
        ));
        // The trailing "no opinion" is dropped: `Table::alignment` defaults when `align` is short.
        assert_eq!(got.align, vec![Alignment::Right, Alignment::Center]);
        assert_eq!(got.alignment(2), Alignment::None);
    }

    #[test]
    fn alignment_falls_back_to_the_first_body_row() {
        let got = table(concat!(
            r#"<table><tr><td style="text-align:left">a</td>"#,
            r#"<td align="right">b</td></tr>"#,
            r#"<tr><td align="center">c</td><td>d</td></tr></table>"#
        ));
        assert_eq!(got.align, vec![Alignment::Left, Alignment::Right]);
    }

    #[test]
    fn col_elements_supply_alignment_the_cells_do_not() {
        let got = table(concat!(
            r#"<table><colgroup><col align="center"><col align="right"></colgroup>"#,
            r#"<tr><td style="text-align:left">a</td><td>b</td></tr></table>"#
        ));
        // The cell wins where it has an opinion; `<col>` fills in the rest.
        assert_eq!(got.align, vec![Alignment::Left, Alignment::Right]);
    }

    #[test]
    fn a_cell_keeps_its_marks_and_links() {
        let got = table(concat!(
            "<table><tr><td><b>bold</b> and ",
            r#"<a href="https://example.com">a <code>link</code></a></td></tr></table>"#
        ));
        assert_eq!(
            got.rows,
            vec![vec![vec![
                Inline::bold("bold"),
                Inline::Text(" and ".to_string()),
                Inline::Link {
                    href: "https://example.com".to_string(),
                    title: None,
                    content: vec![
                        Inline::Text("a ".to_string()),
                        Inline::Code("link".to_string()),
                    ],
                },
            ]]]
        );
    }

    #[test]
    fn a_caption_becomes_a_paragraph_before_the_table() {
        let got = blocks("<table><caption>Q3 revenue</caption><tr><td>a</td></tr></table>");
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], Block::para("Q3 revenue"));
        assert!(matches!(got[1], Block::Table(_)), "{got:?}");
    }

    #[test]
    fn a_colspan_leaves_the_columns_it_covers_empty() {
        let got = table(concat!(
            r#"<table><tr><td colspan="2">wide</td><td>c</td></tr>"#,
            "<tr><td>a</td><td>b</td><td>c</td></tr></table>"
        ));
        assert_eq!(
            got.rows,
            vec![
                vec![cell("wide"), Cell::new(), cell("c")],
                vec![cell("a"), cell("b"), cell("c")],
            ]
        );
    }

    #[test]
    fn a_rowspan_keeps_the_following_rows_in_their_own_columns() {
        let got = table(concat!(
            r#"<table><tr><td rowspan="2">tall</td><td>b</td></tr>"#,
            "<tr><td>c</td></tr></table>"
        ));
        assert_eq!(
            got.rows,
            vec![
                vec![cell("tall"), cell("b")],
                // Without grid placement `c` would land in column 0, under "tall".
                vec![Cell::new(), cell("c")],
            ]
        );
    }

    #[test]
    fn an_absurd_span_is_clamped_rather_than_expanded() {
        let got = table(r#"<table><tr><td colspan="100000000">x</td></tr></table>"#);
        assert_eq!(got.columns(), MAX_SPAN);
    }

    #[test]
    fn a_nested_table_degrades_to_its_cell_text() {
        let got = table(concat!(
            "<table><tr><td><table><tr><td>x</td><td>y</td></tr></table></td>",
            "<td>b</td></tr></table>"
        ));
        assert_eq!(got.rows, vec![vec![cell("x | y"), cell("b")]]);
    }

    #[test]
    fn an_empty_table_disappears() {
        assert!(parse("<table></table>").is_empty());
        assert!(parse("<table><tr><td></td></tr></table>").is_empty());
    }

    #[test]
    fn table_markup_survives_sanitizing() {
        // The analogue of `the_style_attribute_survives_sanitizing`, and a worse failure: if
        // ammonia drops these tags every pasted table silently becomes an empty document, and if
        // it drops `colspan` every cell after a merge shifts one column to the left.
        let cleaned = sanitize(concat!(
            r#"<table><caption>c</caption><colgroup><col span="2" align="right"></colgroup>"#,
            r#"<thead><tr><th align="center" colspan="2">h</th></tr></thead>"#,
            r#"<tbody><tr><td rowspan="2" style="text-align:left">a</td></tr></tbody>"#,
            "<tfoot><tr><td>f</td></tr></tfoot></table>"
        ));
        for needle in [
            "<table",
            "<caption",
            "<colgroup",
            "<col ",
            "<thead",
            "<tbody",
            "<tfoot",
            "<tr",
            "<th",
            "<td",
            "colspan",
            "rowspan",
            "align=",
            "text-align",
            "span=",
        ] {
            assert!(
                cleaned.contains(needle),
                "ammonia stripped {needle}: {cleaned}"
            );
        }
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
        assert_eq!(
            blocks(r#"<div onclick="steal()">text</div>"#),
            vec![Block::para("text")]
        );
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
        assert_eq!(
            para("a\u{a0}\u{a0}b   c"),
            vec![Inline::text("a\u{a0}\u{a0}b c")]
        );
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
