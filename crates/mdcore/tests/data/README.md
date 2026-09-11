# Vendored specification corpora

Input data for `crates/mdcore/tests/commonmark.rs`. Everything here is **committed on purpose**: the
test suite must run offline and produce the same result on every machine, so nothing is fetched at
test time.

| File                         | Source                                                                    | Version                     | Vendored   | Licence                       |
| ---------------------------- | ------------------------------------------------------------------------- | --------------------------- | ---------- | ----------------------------- |
| `commonmark-0.31.2.spec.txt` | <https://spec.commonmark.org/0.31.2/spec.txt>                             | CommonMark 0.31.2           | 2026-09-10 | CC-BY-SA 4.0, John MacFarlane |
| `gfm-0.29.spec.txt`          | <https://raw.githubusercontent.com/github/cmark-gfm/master/test/spec.txt> | GFM 0.29 (dated 2019-04-06) | 2026-09-10 | CC-BY-SA 4.0, GitHub Inc.     |
| `known-divergences.txt`      | ours                                                                      | —                           | —          | —                             |

Both files are verbatim downloads. They are the machine-readable source the published specs are
generated from, so the example numbers here are the same ones
<https://spec.commonmark.org/0.31.2/#example-1> and <https://github.github.com/gfm/#example-1> use.

## The example format

Each example is a fence of 32 backticks followed by `example` (GFM extensions add a tag, e.g.
`example table`), the Markdown input, a line containing only `.`, the reference HTML, then a closing
fence. Tabs are written as `→` (U+2192) so they survive editing; the test decodes them back. Examples
are numbered sequentially in order of appearance.

## What we take from each

**CommonMark: all 652 examples.**

**GFM: only the 24 examples in the five `(extension)` sections** — Tables, Task list items,
Strikethrough, Autolinks, Disallowed Raw HTML. The rest of the GFM spec is a fork of CommonMark
0.29, so running it whole would re-run an older copy of everything the CommonMark file already
covers, under a second set of numbers, and would double the length of the divergence list with
duplicate entries.

The `spec.json` build of the CommonMark spec is deliberately **not** used. It carries the same
examples, and using the `.txt` form means the same ~20-line scanner reads both specs and the test
needs no JSON dependency.

## What the tests do with it

They use the examples as **input**, and never compare our HTML to the reference HTML. Richochet is
not a CommonMark renderer: it renders through a Teams-shaped `RenderProfile`, its model is
deliberately small, and `Block::Unsupported` degrades content on purpose. Asserting equality with
the reference output would be asserting we are something we explicitly do not want to be. See the
module docs in `commonmark.rs` for the four invariants that are asserted instead.

## `known-divergences.txt`

The examples that legitimately do not round-trip, one per line with a reason. The tests assert the
observed divergences are **exactly** this list, so a new one fails the build and a fixed one fails
too until its line is deleted. When a test fails it prints the lines to add or remove in this
file's format.

## Re-vendoring

Download the new spec over the old file, then run `just test-core`. Expect
`corpus_is_intact` to fail first (its example counts are pinned); update those, then re-curate
`known-divergences.txt` from the diff the other tests print. Note that example **numbers shift
between spec versions**, so every id in `known-divergences.txt` has to be re-checked, not carried
over.
