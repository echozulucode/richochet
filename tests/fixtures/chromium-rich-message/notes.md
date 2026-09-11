# chromium-rich-message

**Source:** captured from the clipboard on 2026-09-11.

**Copied from:** a real headed Chromium, via `just capture-web chromium-rich-message`
(`tools/web-samples/rich-message.html`).

**This is a proxy, not Teams.** Teams Desktop is a WebView2 app, so its clipboard HTML is
Chromium's and has this same shape. It does **not** substitute for Phase 1: only a real Teams
window can say what Teams emits, and nothing here says anything about what Teams _accepts_ on
paste.

**What this proves:** the parser survives genuine browser clipboard bytes rather than the tidy
HTML a human would write. Chromium inlines the _entire computed style_ onto every element, so this
fixture carries `font-style: normal`, `text-decoration-thickness: initial`, explicit `color` and
`font-family` on nearly every node — exactly the noise that can accidentally cancel a mark. It also
carries a `SourceURL:` line in the CF_HTML header, which the decoder has to tolerate.

Verified by hand at capture time: marks, links, a hard break, `font-weight: normal` cancelling an
enclosing bold, two-level nested lists, an ordered list starting at 3, a blockquote, a code block,
and a table with per-column alignment and a bold cell all convert correctly.

**Known gap it exposes:** the sample's heading is a `<div>` styled `font-size: 20px;
font-weight: 600`, and it converts to **bold text, not a heading** — the parser infers marks from
style but not block level from font size. Whether that matters depends on what Teams actually
emits for its headings, which is a Phase 1 question.

## Formats present at capture time

- `HTML Format` (id 49465, 7223 bytes)
- `CF_UNICODETEXT` (id 13, 1254 bytes)
- `Chromium internal source RFH token` (id 50018, 24 bytes)
- `Chromium internal source URL` (id 49433, 75 bytes)
- `CanIncludeInClipboardHistory` (id 49696, 4 bytes)
- `CanUploadToCloudClipboard` (id 49714, 4 bytes)
- `CF_LOCALE` (id 16, 4 bytes)
- `CF_TEXT` (id 1, 627 bytes)
- `CF_OEMTEXT` (id 7, 627 bytes)
