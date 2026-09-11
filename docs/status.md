# Status — 2026-09-10

Where the build actually is, and what needs a human next. Phase numbers refer to
[`implementation-plan.md`](implementation-plan.md).

## What works

Run it: `just setup` once, then `just dev`.

- **Phase 0 — done.** Cargo workspace, justfile as the single entry point, CI, icons, ADR 0001.
- **Phase 1 — tooling done, measurement outstanding.** See "What needs you" below.
- **Phase 2 — done.** The engine converts Markdown ↔ document model ↔ HTML ↔ plain text.
- **Phase 3 — done.** Two panes over one document, live conversion, three copy actions, paste
  detection, themes, draggable divider, single-pane mode.
- **Verified in the built app**, not only in tests: it launches, both panes render, typing Markdown
  live-updates the Formatted pane through real Tauri IPC, and **Copy for Teams** puts correct
  `HTML Format` _and_ `CF_UNICODETEXT` on the real Windows clipboard together.
- **Phases 4–6 — not started.** Phase 4 is blocked on Phase 1.

## Test coverage

| Layer                 | Count | What it covers                                            |
| --------------------- | ----- | --------------------------------------------------------- |
| Engine unit           | 197   | parsing, rendering, escaping, normalization               |
| Clipboard unit        | 17    | CF_HTML byte offsets, multibyte, malformed input          |
| Frontend unit         | 40    | sync engine — authority, echo suppression, sequence guard |
| Corpus snapshots      | 47    | every construct, in all three output formats              |
| Property              | 8     | convergence, idempotence, escaping, never-panic           |
| Playwright functional | 18    | caret, sync loops, stale responses, copy wiring, layout   |

`just test` runs all of it. `just test-core` is the fast inner loop.

## What needs you

**The Phase 1 clipboard spike.** It cannot be done without a real Teams window, and until it is
done `RenderProfile::teams()` is a _hypothesis_ — currently "Teams accepts well-formed semantic
HTML" — rather than a finding. Everything else is built around being able to change that cheaply.

The tooling is working and verified against a live clipboard:

```sh
just clipdump              # copy from Teams first; prints every format, header stripped
just capture nested-list   # saves it as tests/fixtures/nested-list/
```

Then fill in [`clipboard-findings.md`](clipboard-findings.md) — every table in it is deliberately
empty, and none of it may be filled from documentation or memory.

ADR 0001's load-bearing assumption — that one clipboard session really does leave _both_ the HTML
and the plain-text representation on — **is now verified** against the built app, with byte-exact
CF_HTML offsets. What is still unknown is the other half: whether Teams _accepts_ that HTML on
paste. That is task 1.5 and only you can run it.

## Known limitations, each pinned by a test

- **CommonMark cannot express emphasis that starts or ends with punctuation directly against a word
  character.** No delimiter satisfies the flanking rules. The renderer detects it and degrades
  deterministically rather than emitting something that would read back as literal asterisks. The
  complete fix is an inline-HTML `<em>` fallback, which needs the Markdown _parser_ to map inline
  HTML back to marks — logged for Phase 6.
- **`SoftBreak` does not survive a trip through HTML** and becomes a space. HTML has no way to say
  "a newline that renders as a space", and honouring source newlines would wrap Markdown at
  arbitrary points wherever Teams pretty-prints its HTML. Pinned by
  `soft_breaks_become_spaces_through_html`.
- **Tables, images, footnotes and mentions degrade** to `Block::Unsupported` or plain links rather
  than being dropped. Out of MVP scope by design.
- **Windows only.** The clipboard layer is Win32 (ADR 0001); the `read`/`write` seam is
  platform-neutral, so other platforms are additive.

## Notes for the next session

- The corpus is 47 hand-written fixtures. **They are not real Teams captures** — each `notes.md`
  says so. Replacing them with real ones is Phase 1 task 1.4.
- `src/test-support/oracle.json` is generated and gitignored. Never edit it to fix a failing E2E
  test; regenerate it with `just oracle`.
- The E2E specs' input strings are listed in `crates/mdcli/src/fixtures.rs` (`E2E_INPUTS`). A string
  typed in a spec but missing there falls back to a passthrough, which would make the assertion
  meaningless rather than failing loudly.
