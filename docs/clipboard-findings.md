# Clipboard findings

**Status: NOT YET MEASURED.** Every row below is empty on purpose. This document is the empirical
record of what Microsoft Teams actually does with the clipboard, and nothing in it may be filled in
from documentation, reasoning or memory — only from running the tools against a real Teams client.
Until it is filled in, `RenderProfile::teams()` is a _hypothesis_ (currently: "Teams accepts
well-formed semantic HTML"), not a finding.

Filling this in requires a human with Teams open. See `docs/implementation-plan.md` Phase 1.

## Environment

|                               |     |
| ----------------------------- | --- |
| Teams Desktop version         |     |
| Teams Web (browser + version) |     |
| Windows build                 |     |
| Date measured                 |     |
| Measured by                   |     |

## How to measure

```sh
just clipdump            # after copying from Teams: dumps every clipboard format
just capture <name>      # saves the current clipboard as tests/fixtures/<name>/
```

For the paste-back direction, put candidate HTML on the clipboard and paste into a Teams compose
box:

```sh
just conv md2teams some-file.md      # see what we would send
```

## 1. Formats observed on copy

Run `just clipdump` after copying a formatted message out of Teams.

| Format id | Name | Desktop | Web | Notes |
| --------- | ---- | ------- | --- | ----- |
|           |      |         |     |       |

## 2. Per-construct capture results

For each construct: copy it from Teams, run `just capture <name>`, then record what Teams emitted.
"Lossless?" means the HTML carried enough information to reconstruct the construct exactly.

| Construct                      | HTML Teams emits | Lossless? | Fixture                |
| ------------------------------ | ---------------- | --------- | ---------------------- |
| Bold                           |                  |           | `teams-styled-bold`    |
| Italic                         |                  |           |                        |
| Strikethrough                  |                  |           |                        |
| Inline code                    |                  |           | `teams-monospace-span` |
| Link                           |                  |           | `teams-split-anchor`   |
| Plain paragraph                |                  |           | `teams-div-paragraphs` |
| Two paragraphs                 |                  |           |                        |
| Hard break (Shift+Enter)       |                  |           | `teams-br`             |
| H1 / H2 / H3                   |                  |           | `teams-headings`       |
| Bulleted list                  |                  |           |                        |
| Numbered list                  |                  |           |                        |
| **Nested list, two levels**    |                  |           | `teams-nested-list`    |
| Blockquote                     |                  |           | `teams-blockquote`     |
| Fenced code block + language   |                  |           | `teams-pre-code`       |
| Mixed inline marks on one line |                  |           |                        |
| A mark spanning a link         |                  |           |                        |
| Emoji                          |                  |           |                        |
| @mention (expected to degrade) |                  |           |                        |

## 3. Paste-back results

For each construct, put each candidate encoding on the clipboard, paste into Teams, and record
which one survives. **Exactly one winner per row.**

| Construct     | Candidates tried                                       | Winner | What failed, and how |
| ------------- | ------------------------------------------------------ | ------ | -------------------- |
| Bold          | `<strong>` · `<b>` · `<span style="font-weight:bold">` |        |                      |
| Italic        | `<em>` · `<i>` · inline style                          |        |                      |
| Strikethrough | `<s>` · `<del>` · inline style                         |        |                      |
| Inline code   | `<code>` · monospace span                              |        |                      |
| Heading       | `<h2>` · `<p><strong>`                                 |        |                      |
| Nested list   | native `<ul><li><ul>` · `margin-left`                  |        |                      |
| Code block    | `<pre><code>` · monospace `<div>`                      |        |                      |
| Blockquote    | `<blockquote>` · `> ` prefix                           |        |                      |
| Link          | `<a href>` · bare URL                                  |        |                      |
| Hard break    | `<br>` · two `<div>`s                                  |        |                      |

### Does Teams accept HTML at all?

The single question this phase exists to answer. If the answer is no, the fallback is
plain-Markdown-on-clipboard and the app becomes Teams → Markdown only — still useful, and the
architecture is unchanged.

**Answer:**

### Does `raw::set_without_clear` actually preserve both formats?

`ADR 0001` depends on writing HTML and a plain-text fallback in one clipboard session.

**Answer: yes — verified 2026-09-10**, against the built app rather than in theory. Typing
`# Heading` / `Some **bold** and *italic* text` into Richochet and clicking **Copy for Teams**
leaves five formats on the clipboard, including both of the ones that matter:

```text
─── CF_UNICODETEXT ─── 70 bytes
Heading

Some bold and italic text

─── HTML Format ─── 251 bytes
Version:0.9
StartHTML:0000000105
EndHTML:0000000251
StartFragment:0000000141
EndFragment:0000000215
<html>
<body>
<!--StartFragment--><h1>Heading</h1><p>Some <strong>bold</strong> and <em>italic</em> text</p><!--EndFragment-->
</body>
</html>
```

All four CF_HTML offsets are byte-exact: the header is 105 bytes, `<html>
<body>
<!--StartFragment-->`
adds 36 to reach 141, the 74-byte fragment ends at 215, and the closing 36 bytes reach 251.

This says nothing about whether **Teams accepts** that HTML — only that we emit it correctly. The
accepting half is task 1.5 below and still needs a human.

## 4. Consequences for `RenderProfile`

What `RenderProfile::teams()` must be changed to, and why. Each change needs a fixture.

| Field          | Hypothesis      | Measured | Fixture proving it |
| -------------- | --------------- | -------- | ------------------ |
| `bold`         | `Tag("strong")` |          |                    |
| `italic`       | `Tag("em")`     |          |                    |
| `strike`       | `Tag("s")`      |          |                    |
| `code`         | `Tag("code")`   |          |                    |
| `headings`     | `Native`        |          |                    |
| `code_block`   | `PreCode`       |          |                    |
| `nested_lists` | `Native`        |          |                    |
| `blockquote`   | `Blockquote`    |          |                    |

## 4b. Chromium proxy observations (not Teams)

Teams Desktop is a WebView2 app, so its clipboard HTML is Chromium's. While Teams is unavailable,
`just capture-web <name>` copies a sample page out of a real headed Chromium and captures the
genuine bytes. **This is a proxy for what Teams emits and says nothing about what Teams accepts.**
Everything here must be re-confirmed against a real Teams window before it is treated as a finding.

Captured 2026-09-11 (`tests/fixtures/chromium-rich-message`), 9 clipboard formats:

| Format                               | Bytes | Note                                                               |
| ------------------------------------ | ----- | ------------------------------------------------------------------ |
| `HTML Format`                        | 7223  | the fragment itself is ~1.2 kB; the rest is inlined computed style |
| `CF_UNICODETEXT`                     | 1254  | plain-text fallback                                                |
| `CF_TEXT` / `CF_OEMTEXT`             | 627   | ANSI fallbacks                                                     |
| `Chromium internal source URL`       | 75    | Chromium-private                                                   |
| `Chromium internal source RFH token` | 24    | Chromium-private                                                   |
| `CanIncludeInClipboardHistory`       | 4     | Windows clipboard-history opt-in                                   |
| `CanUploadToCloudClipboard`          | 4     | Windows cloud-clipboard opt-in                                     |
| `CF_LOCALE`                          | 4     |                                                                    |

What it showed:

- The CF_HTML header carries an extra **`SourceURL:`** field. Our decoder tolerates it; a stricter
  one would not.
- Chromium inlines the **entire computed style** onto every element — `font-style: normal`,
  `text-decoration-thickness: initial`, `color`, `font-family`, `letter-spacing` and more. Style
  inference has to ignore all of that noise without accidentally cancelling a real mark. It does:
  a `font-weight: normal` span nested inside bold still correctly splits the bold around it.
- Marks, links, hard breaks, two-level nested lists, an ordered list with `start="3"`, blockquotes,
  code blocks, and a table with per-column alignment and a bold cell all converted correctly.
- **One gap.** A heading written as a styled `<div>` (`font-size: 20px; font-weight: 600`) converts
  to **bold text, not a heading** — the parser infers marks from inline style but never infers
  block level from font size. Whether that matters depends entirely on how Teams marks up its
  headings, which is question 2 in the capture matrix above. Do not "fix" this before measuring:
  inferring headings from font size would misread every merely-large piece of text as a heading.

## 5. Desktop vs Web divergence

Anything that behaves differently between the two clients. If this section is non-empty,
`RenderProfile` may need a per-target variant.

## 6. Open questions
