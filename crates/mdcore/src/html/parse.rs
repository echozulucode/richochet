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
            let fallback = table_fallback(node, ctx.distributing(declared));
            out.push_block(Block::Unsupported {
                kind: "table".to_string(),
                fallback,
            });
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
fn table_fallback(node: &Handle, ctx: Ctx) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for row in descendants_named(node, &["tr"]) {
        let mut cells: Vec<Vec<Inline>> = Vec::new();
        for cell in child_elements(&row) {
            if matches!(tag_name(&cell).as_deref(), Some("td") | Some("th")) {
                let content = inline_subtree(&cell, ctx);
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

    #[test]
    fn a_table_degrades_to_unsupported_with_its_cell_text() {
        let got =
            blocks("<table><tr><th>h1</th><th>h2</th></tr><tr><td>a</td><td>b</td></tr></table>");
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
