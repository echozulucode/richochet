# teams-table-degrades

**Source:** hand-written (`source.html`), 2026-09-10. **Not yet captured from real Teams** — replace with a real capture via `just capture teams-table-degrades` during the Phase 1 spike.

**What this proves:** A pasted table survives as a table. The simplest possible shape — bare `<tr>`/`<td>`, no `<thead>`, no alignment — becomes `Block::Table` and renders back as real `<table>` markup in HTML, a GFM pipe table in Markdown, and padded columns in plain text.

**History.** This fixture was originally the proof that tables were *out of scope* and had to degrade gracefully: the parser turned `<table>` into `Block::Unsupported { kind: "table" }` with cells joined by `|`, and the snapshot recorded `a | b`. That output was the worst of both worlds — it read as a broken Markdown table rather than as either a real table or an honest paragraph. The model now has `Block::Table`, so the fixture's job is inverted: it no longer proves graceful degradation, it proves survival. The name is kept because the snapshot file is keyed on it and the history is worth being able to find.

Note the empty header row in the Markdown output. GFM has no way to write a table without a header, so a table that genuinely has none gets a blank one; that is the Markdown renderer's decision, not a parse failure. That Markdown is itself stable — it round trips through HTML and back to the same three lines — but the intermediate HTML has picked up a `<thead>` of blank `<th>`s along the way, because reading the blank header row back gives a header of empty cells rather than no header at all. Going HTML → HTML, which is what the app actually does, keeps the table headerless.
