I’d build this as a **clipboard-aware rich-text ↔ Markdown converter**, rather than treating it as a simple text transformer. The important architectural choice is to normalize everything into a small internal document model and generate Markdown, preview HTML, and Teams-compatible clipboard content from that same representation.

Microsoft Teams does support a Markdown-style syntax for bold, italics, lists, links, code, headings, blockquotes, etc., but Microsoft explicitly notes that it is not identical to standard Markdown. ([Microsoft Support][1]) That makes a normalized intermediate representation especially useful.

## High-level product concept

A small Tauri desktop utility with an Apple-like minimalist interface:

```text
┌─────────────────────────────────────────────────────────────┐
│  Teams ↔ Markdown                                      ⚙   │
├────────────────────────────┬────────────────────────────────┤
│ Teams / Rich Text          │ Markdown                       │
│                            │                                │
│ Paste Teams content here   │ ## Example                     │
│                            │                                │
│ Hello Eric                 │ **Hello Eric**                 │
│                            │                                │
│ • Item one                 │ - Item one                     │
│ • Item two                 │ - Item two                     │
│                            │                                │
│                            │                                │
├────────────────────────────┴────────────────────────────────┤
│     Copy for Teams     Copy Markdown     Copy Plain Text    │
└─────────────────────────────────────────────────────────────┘
```

Both sides should be editable.

Changing the Markdown immediately updates the rich preview. Pasting rich Teams content immediately generates Markdown.

## Recommended architecture

```text
                       ┌─────────────────────┐
Teams Clipboard ──────►│ Clipboard Importer  │
 HTML / RTF / Text     └─────────┬───────────┘
                                 │
                                 ▼
                      ┌───────────────────────┐
                      │ Normalized Document   │
                      │         Model         │
                      │                       │
                      │ Document              │
                      │ ├─ Paragraph          │
                      │ ├─ Heading            │
                      │ ├─ List               │
                      │ ├─ Blockquote         │
                      │ ├─ CodeBlock          │
                      │ └─ Inline nodes       │
                      │    ├─ Bold            │
                      │    ├─ Italic          │
                      │    ├─ Code            │
                      │    └─ Link            │
                      └───┬─────────┬─────────┘
                          │         │
                  ┌───────▼──┐   ┌──▼──────────────┐
                  │ Markdown │   │ Teams Clipboard │
                  │ Renderer │   │ Renderer        │
                  └──────────┘   └─────────────────┘
```

I would **not** make HTML the canonical representation. Instead, use a relatively tiny AST/document model representing the semantic constructs you care about. This prevents Teams-specific HTML quirks from contaminating the Markdown conversion logic.

## Clipboard handling is the key technical area

When something is copied from Teams, the clipboard can potentially contain multiple representations of the same selection:

```text
text/html
text/plain
RTF
possibly application-specific clipboard formats
```

You want to inspect all available formats and choose the richest representation you understand.

One complication is that Tauri's official clipboard-manager currently exposes `readText()`, images, and `writeHtml()`, but does **not expose a separate rich-HTML read API**. It can, however, place HTML plus a plain-text fallback onto the clipboard. ([Tauri][2])

Because of that, I would plan for the Rust layer to eventually handle native clipboard interrogation rather than limiting the design to the basic JavaScript-facing Tauri clipboard API. There are also Tauri ecosystem clipboard plugins supporting text/image/HTML/RTF that may be worth evaluating before writing the native code yourself. ([Tauri][3])

For output, the ideal operation becomes:

```text
Copy for Teams
      │
      ├── HTML representation
      └── Plain-text fallback
```

Teams can then consume the rich clipboard representation when the user presses Ctrl+V.

That is preferable to simply putting Markdown syntax on the clipboard and hoping Teams interprets everything correctly.

## Conversion strategy

I would support three paths:

| Input                | Internal conversion | Output                          |
| -------------------- | ------------------- | ------------------------------- |
| Teams rich clipboard | HTML/RTF → AST      | Markdown                        |
| Markdown             | Markdown → AST      | Teams-compatible rich clipboard |
| Plain text           | Text → AST          | Markdown / Teams / text         |

The internal model only needs to support the practical intersection initially:

**Inline:** bold, italic, strikethrough, inline code, links, line breaks.

**Blocks:** paragraphs, H1-H3, ordered lists, unordered lists, nested lists, blockquotes and fenced code blocks.

That closely aligns with the formatting Teams itself documents as supporting through its Markdown-style interface. ([Microsoft Support][1])

Tables, mentions, emojis, attachments, Loop components and other Teams-specific constructs should initially degrade gracefully instead of becoming MVP requirements.

## UX design

I would make the application essentially **one screen**.

The left side is a rich editor/preview called something like **Formatted**, rather than specifically "Teams", because internally it represents portable rich text.

The right side is **Markdown** using a clean monospace editor.

At the bottom or upper-right are just three prominent actions:

**Copy for Teams · Copy Markdown · Copy Text**

When the user pastes into the formatted side, show a very small transient indication of what was detected:

```text
Pasted rich text
```

or

```text
Pasted plain text
```

Nothing more intrusive.

For your Apple-like direction, I would avoid permanent toolbars full of formatting controls. Editing can support keyboard shortcuts and a context-sensitive floating toolbar for:

```text
B   I   S   </>   Link
```

Otherwise the content remains the focus.

## Suggested implementation stack

Given your Rust/Tauri preference, I would use:

```text
Desktop
  Tauri 2

Backend
  Rust
  ├─ clipboard abstraction
  ├─ HTML sanitization/parser
  ├─ normalized document model
  ├─ Markdown parser/serializer
  └─ Teams clipboard serializer

Frontend
  React + TypeScript
  Tailwind CSS
  Radix/shadcn primitives where useful

Markdown editor
  CodeMirror 6

Rich editor
  TipTap / ProseMirror
       OR
  a deliberately small contenteditable implementation

State
  Zustand

Persistence
  Tauri Store
  only for preferences
```

I particularly like **CodeMirror + TipTap** here because the two editors correspond naturally to the two representations.

The Rust core should ideally own conversion rules so the conversion engine could eventually be reused from a CLI or another tool.

## Important design principle: preserve intent, not Teams markup

For example, if Teams gives you something ugly like:

```html
<div>
  <span style="font-weight:600"> Important </span>
</div>
```

normalize it immediately to:

```text
Paragraph
 └── Bold
      └── "Important"
```

Then:

```text
Markdown:
**Important**

Teams HTML:
<strong>Important</strong>

Plain text:
Important
```

This will make the converter much more maintainable.

## Phased plan

1. **Clipboard investigation / spike.** Build a tiny Tauri utility that dumps every clipboard representation produced by Teams Desktop and Teams Web. Test paragraphs, bold/italic, links, nested lists, headings, quotes, inline code and code blocks. Also determine exactly which generated HTML formats Teams accepts when pasted back. Microsoft documents normal desktop copy/paste, but the exact rich clipboard representation isn't part of the user-facing contract, so empirical testing is important. ([Microsoft Support][4])

2. **Core conversion engine.** Define the Rust document AST, then implement Markdown → AST → Markdown first. Add HTML → AST and AST → sanitized HTML afterward. Create fixtures so every discovered Teams clipboard example becomes a regression test.

3. **Minimal desktop UI.** Implement the two-pane Markdown/rich-text interface, live conversion, paste detection, Copy Markdown, Copy Teams and Copy Plain Text.

4. **Teams fidelity.** Tune generated HTML against Teams Desktop and Web. Add list indentation, code blocks, links, headings and blockquotes. Teams has special handling for code blocks, including triple-backtick creation in its composer, so these deserve explicit tests. ([Microsoft Support][5])

5. **Polish.** Add light/dark/system themes, keyboard shortcuts, subtle paste/copy feedback, draggable pane divider, responsive single-pane mode for small windows, history/undo and settings.

6. **Advanced formats.** Only after the core is reliable, investigate RTF import, Teams mentions, tables, images, message metadata and other proprietary structures.

## MVP boundary I would choose

The first genuinely useful release should do only this:

**Teams → Markdown**

Copy ordinary formatted content from Teams, switch to the app, paste, and immediately get high-quality Markdown.

**Markdown → Teams**

Paste or type Markdown, see an accurate rich preview, click **Copy for Teams**, switch to Teams, Ctrl+V, and have the formatting survive.

**Supporting**

Copy Markdown, Copy Teams, Copy Plain Text, live synchronization, light/dark/system, undo/redo and keyboard shortcuts.

I would deliberately leave message history, Teams APIs, authentication, sending messages directly to Teams, attachments, mentions and Loop components out of the initial application.

### The architecture I would target

```text
teams-markdown/
├── src-tauri/
│   └── src/
│       ├── clipboard/
│       │   ├── reader.rs
│       │   ├── writer.rs
│       │   └── formats.rs
│       ├── document/
│       │   ├── model.rs
│       │   └── normalize.rs
│       ├── markdown/
│       │   ├── parse.rs
│       │   └── render.rs
│       ├── html/
│       │   ├── parse.rs
│       │   ├── sanitize.rs
│       │   └── teams.rs
│       └── commands.rs
│
└── src/
    ├── components/
    │   ├── RichEditor/
    │   ├── MarkdownEditor/
    │   ├── CopyActions/
    │   └── StatusToast/
    ├── stores/
    └── App.tsx
```

The **clipboard spike should be the very first implementation task**. If Teams provides good HTML on copy and accepts well-formed HTML on paste, this application will be quite straightforward. If it doesn't, you'll discover that before investing in the editor or conversion architecture. The rest of the design can remain essentially the same either way.

[1]: https://support.microsoft.com/en-us/teams/chat/use-markdown-formatting-in-microsoft-teams?utm_source=chatgpt.com 'Use Markdown formatting in Microsoft Teams | Microsoft Support'
[2]: https://tauri.app/reference/javascript/clipboard-manager/?utm_source=chatgpt.com '@tauri-apps/plugin-clipboard-manager | Tauri'
[3]: https://v2.tauri.app/plugin/?utm_source=chatgpt.com 'Features & Recipes | Tauri'
[4]: https://support.microsoft.com/en-us/teams/chat/copy-and-paste-text-in-microsoft-teams?utm_source=chatgpt.com 'Copy and paste text in Microsoft Teams | Microsoft Support'
[5]: https://support.microsoft.com/en-us/teams/chat/use-code-blocks-in-microsoft-teams?utm_source=chatgpt.com 'Use code blocks in Microsoft Teams | Microsoft Support'
