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

`ADR 0001` depends on writing HTML and a plain-text fallback in one clipboard session. This is the
one load-bearing assumption in that ADR that has not been verified against a running Teams.

**Answer:**

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

## 5. Desktop vs Web divergence

Anything that behaves differently between the two clients. If this section is non-empty,
`RenderProfile` may need a per-target variant.

## 6. Open questions
