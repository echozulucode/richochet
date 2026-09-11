# Implementation Plan — Richochet

Derived from [`plan.md`](plan.md). That document is the *design rationale*; this one is the
*build order*. Where the two disagree, this document wins and `plan.md` should be updated.

**Audience:** the lead agent and a team of subagents. Every phase states a goal, machine-checkable
exit criteria, a task table (with the files each task owns, so tasks can be fanned out without
collisions), and the risks that phase exists to retire.

---

## 0. Ground rules for the team

### Contract-first fan-out

Subagents may only work in parallel on tasks that touch **disjoint file sets**. The shared
interfaces — the document AST, the Tauri command signatures, the fixture layout — are *frozen
first, by a single agent*, before any fan-out. Task tables mark each task `parallel` (safe to run
concurrently with its siblings) or `serial` (must complete before the rest of the phase).

### Definition of done (every task)

1. Code compiles: `just check` clean (rustc + clippy `-D warnings` + tsc).
2. Tests pass: `just test`.
3. Formatted: `just fmt`.
4. New behaviour has a test. A new Teams quirk gets a **fixture**, not a code branch.
5. Public Rust items have doc comments; non-obvious normalization rules cite the fixture that
   motivated them.

### Verification discipline

No agent reports a phase complete on the strength of "it should work". Exit criteria are written so
they can be *run*. If a criterion cannot be automated (e.g. "paste into Teams Desktop and the bold
survives"), it is a **manual checklist item** and the agent must say plainly that it needs the user
to run it.

### Naming

- Product name **Richochet** — "rich text" + *ricochet*, content bouncing between formats. The `h`
  is deliberate; it is not a typo and must never be "corrected" to `Ricochet` in code, config, docs
  or UI copy.
- Bundle id `com.echozed.richochet`. Repo dir stays `markdown-converter`. Window title `Richochet`.
- The left pane is called **Formatted**, never "Teams", per `plan.md`.

---

## Target repository layout

A Cargo **workspace**, so the conversion engine is a library that knows nothing about Tauri. This is
the most important structural decision here: it keeps the engine unit-testable in milliseconds,
reusable from a CLI, and portable to WASM later.

```text
markdown-converter/
├── justfile                     # the only entry point anyone should need
├── Cargo.toml                   # [workspace] members = crates/*, src-tauri
├── rust-toolchain.toml
├── package.json                 # frontend only
├── vite.config.ts
├── AGENTS.md
├── docs/
│   ├── plan.md                  # design rationale (input)
│   ├── implementation-plan.md   # this file
│   ├── clipboard-findings.md    # WRITTEN BY PHASE 1 — the empirical truth
│   └── adr/                     # one file per irreversible decision
├── crates/
│   ├── mdcore/                  # the conversion engine. No Tauri, no I/O.
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── document/
│   │   │   │   ├── model.rs     # the AST (frozen contract)
│   │   │   │   ├── normalize.rs # cleanup passes over the AST
│   │   │   │   └── visit.rs
│   │   │   ├── markdown/
│   │   │   │   ├── parse.rs     # md  -> AST   (pulldown-cmark)
│   │   │   │   ├── render.rs    # AST -> md
│   │   │   │   └── escape.rs    # contextual Markdown escaping
│   │   │   ├── html/
│   │   │   │   ├── parse.rs     # html -> AST  (html5ever)
│   │   │   │   ├── styles.rs    # inline-style -> semantic mark inference
│   │   │   │   └── render.rs    # AST -> html, driven by a RenderProfile
│   │   │   ├── text/            # plain text <-> AST
│   │   │   └── profile.rs       # RenderProfile: the Teams dialect, as DATA
│   │   └── tests/
│   │       ├── corpus.rs        # data-driven runner over tests/fixtures/
│   │       └── roundtrip.rs     # proptest invariants
│   └── mdcli/                   # debug CLI: mdcli md2teams, mdcli dump-clipboard
├── src-tauri/
│   ├── src/
│   │   ├── main.rs              # thin passthrough -> app_lib::run()
│   │   ├── lib.rs               # builder, state, generate_handler!
│   │   ├── commands.rs          # #[tauri::command] surface (frozen contract)
│   │   └── clipboard/
│   │       ├── mod.rs
│   │       ├── read.rs          # enumerate + rank available formats
│   │       ├── write.rs         # multi-format atomic write
│   │       └── cf_html.rs       # Windows CF_HTML header encode/decode
│   ├── capabilities/default.json
│   └── tauri.conf.json
├── src/                         # React + TS frontend
│   ├── components/
│   │   ├── RichEditor/          # TipTap
│   │   ├── MarkdownEditor/      # CodeMirror 6
│   │   ├── CopyActions/
│   │   ├── SplitPane/
│   │   └── StatusToast/
│   ├── stores/                  # Zustand
│   ├── lib/                     # invoke wrappers, debounce, seq guard
│   └── App.tsx
└── tests/
    └── fixtures/                # THE GOLDEN CORPUS — shared by Rust tests + CLI
        └── <case-name>/
            ├── source.html      # captured from Teams (optional)
            ├── source.md        # (optional)
            ├── expected.md
            ├── expected.teams.html
            ├── expected.txt
            └── notes.md         # where it came from, what it proves
```

---

## Pinned versions

Verified against the registries on 2026-09-10. Pin these; do not float.

| Rust | Version | Role |
|---|---|---|
| `tauri` / `tauri-build` | 2 | shell |
| `tauri-plugin-clipboard` | 2.1.11 | **community** plugin — text/html/rtf/image/files, read + write |
| `tauri-plugin-store` | 2.4.4 | preferences only |
| `pulldown-cmark` | 0.13 | Markdown -> events |
| `html5ever` + `markup5ever_rcdom` | 0.39 | browser-grade HTML parse |
| `ammonia` | 4.1 | sanitize untrusted clipboard HTML |
| `clipboard-win` | 5.4 | raw Windows format enumeration (spike + fallback) |
| `thiserror` | 2.0 | error types |
| `insta` | 1.48 | snapshot review |
| `proptest` | 1.11 | round-trip invariants |

| Frontend | Version |
|---|---|
| `@tauri-apps/api` / `@tauri-apps/cli` | 2.11 |
| `react` | 19.3 |
| `vite` | 8.3 |
| `tailwindcss` + `@tailwindcss/vite` | 4.3 |
| `@tiptap/react` | 3.31 |
| `codemirror` + `@codemirror/lang-markdown` | 6.x |
| `zustand` | 5.0 |
| `vitest` | 5.0 |
| `@playwright/test` | 1.63 |

> **Tailwind 4 note:** CSS-first config. `@import "tailwindcss";` in `src/styles.css` plus the
> `@tailwindcss/vite` plugin. There is **no** `tailwind.config.js` and no PostCSS step. An agent
> that scaffolds the v3 way produces a build that silently ships no styles.

> **`pulldown-cmark-to-cmark` is deliberately NOT used.** We own our own AST, not a pulldown event
> stream, so we write our own serializer and keep full control of escaping.

---

## Phase 0 — Scaffold and toolchain

**Goal:** a repo where `just dev` opens a window and `just test` is green, with nothing in it yet.

**Exit criteria**

- [ ] `just dev` launches the Tauri window showing a placeholder, with HMR working.
- [ ] `just build` produces an installer under `src-tauri/target/release/bundle/`.
- [ ] `just check` and `just test` exit 0 on the empty project.
- [ ] Initial commit exists; `.gitignore` covers `target/`, `node_modules/`, `dist/`.
- [ ] CI runs `just ci` on push.

| # | Task | Owns | Mode |
|---|---|---|---|
| 0.1 | Cargo workspace, `rust-toolchain.toml`, skeletons for `mdcore`/`mdcli`, `src-tauri` (scaffold with `create-tauri-app`, then restructure) | `Cargo.toml`, `crates/**`, `src-tauri/**` | serial |
| 0.2 | Frontend: Vite 8 + React 19 + TS strict + Tailwind 4 + Zustand; placeholder two-pane shell | `package.json`, `vite.config.ts`, `src/**` | parallel |
| 0.3 | The `justfile` (below) | `justfile` | parallel |
| 0.4 | `.github/workflows/ci.yml` running `just ci` on `windows-latest` | `.github/**` | parallel |
| 0.5 | `AGENTS.md` | `AGENTS.md` | parallel |

### Identity (task 0.1) — copy these values verbatim

```json
{
  "productName": "Richochet",
  "identifier": "com.echozed.richochet",
  "app": { "windows": [{ "label": "main", "title": "Richochet" }] }
}
```

Spelling check for every agent: **Richochet**, `rich` + `ochet`. The blend of "rich text" and
"ricochet" is the whole point of the name. Spell-checkers, autocomplete and well-meaning agents will
all try to write `Ricochet`; do not let them. Add `Richochet` to `cspell.json` / the editor
dictionary in this task so the suggestion stops appearing.

### The justfile

`just` is the single entry point. No agent or human should need to remember a raw `cargo`/`pnpm`
incantation, and `AGENTS.md` points here rather than restating commands.

```just
set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

# List available recipes
default:
    @just --list

# One-time setup: install JS deps and cargo tooling
setup:
    pnpm install
    cargo install cargo-insta tauri-cli --locked

# Run the desktop app with hot reload
dev:
    pnpm tauri dev

# Build the release bundle
build:
    pnpm tauri build

# Everything CI runs
ci: check test

# Compile + lint, no tests
check: check-rust check-ui

check-rust:
    cargo clippy --workspace --all-targets -- -D warnings

check-ui:
    pnpm tsc --noEmit
    pnpm eslint .

# Full test suite
test: test-core test-ui

# Engine tests only — fast, no Tauri. Run these constantly.
test-core:
    cargo test -p mdcore

test-tauri:
    cargo test -p app

test-ui:
    pnpm vitest run

# Review pending snapshot changes interactively
review:
    cargo insta review

# Format everything
fmt:
    cargo fmt --all
    pnpm prettier --write .

# Convert on the command line (debug aid)
# usage: just conv md2teams path/to/file.md
conv *ARGS:
    cargo run -q -p mdcli -- {{ARGS}}

# Dump every format currently on the clipboard (Phase 1 workhorse)
clipdump:
    cargo run -q -p mdcli -- dump-clipboard

# Capture the current clipboard into tests/fixtures/<NAME>/
capture NAME:
    cargo run -q -p mdcli -- capture --name {{NAME}}

clean:
    cargo clean
    pnpm exec rimraf dist node_modules/.vite
```

---

## Phase 1 — Clipboard spike (GATE)

**This phase blocks Phases 3 and 4.** Its entire purpose is to replace assumptions about Teams with
measurements. Phase 2 may run concurrently, because the AST does not depend on the answer.

**Goal:** know exactly what Teams puts on the clipboard, and exactly what it accepts back.

**Deliverables**

1. `just clipdump` — prints every clipboard format id, name, byte length and a decoded preview.
2. `just capture <name>` — writes the current clipboard into a new fixture directory.
3. `docs/clipboard-findings.md` — the empirical record (template below).
4. A populated `tests/fixtures/` corpus from real Teams copies.
5. `docs/adr/0001-clipboard-strategy.md` — **`tauri-plugin-clipboard`** vs. hand-rolled
   **`clipboard-win`**, with the reason.

### Why this is not trivial

Tauri's official `clipboard-manager` exposes `readText`, images and `writeHtml` — there is **no
rich-HTML read**, so it cannot do this app's primary job.

**Resolved (ADR 0001): use `clipboard-win` 5.4 directly.** Reading its source settled it — it
already provides `raw::get_html` (header stripped), `raw::set_html` (header written correctly),
`raw::EnumFormats` + `format_name_big` for the spike's format enumeration, and
`raw::set_without_clear`, which is what lets "Copy for Teams" write HTML *and* a plain-text fallback
in a single open/empty/write session. Writing the two representations in separate open/close cycles
would leave only the last on the clipboard — the exact bug this app cannot afford.

That removes most of the cost originally budgeted for this phase. What it does **not** remove is
the empirical work: tasks 1.4 and 1.5 still require a human with Teams open.

On Windows, HTML lives on the clipboard under the registered `HTML Format`, which is **not** raw
HTML. It carries a CF_HTML header:

```text
Version:0.9
StartHTML:0000000105
EndHTML:0000000501
StartFragment:0000000141
EndFragment:0000000465
<html><body><!--StartFragment-->…<!--EndFragment--></body></html>
```

Those numbers are **byte offsets from the start of the CF_HTML block**, zero-padded to a fixed
width, so they can only be computed once the header's own length is fixed. Getting this wrong is the
most common cause of "the paste is blank in one app and fine in another". `cf_html.rs` therefore
gets its own unit tests with byte-exact expected output and an `encode(decode(x)) == x` round trip.

### Task table

| # | Task | Owns | Mode |
|---|---|---|---|
| 1.1 | `cf_html.rs`: encode/decode + byte-exact unit tests | `src-tauri/src/clipboard/cf_html.rs` | parallel |
| 1.2 | `mdcli dump-clipboard` + `capture`, using `clipboard-win` raw enumeration | `crates/mdcli/**` | parallel |
| 1.3 | ~~Evaluate `tauri-plugin-clipboard`~~ — **done**, see ADR 0001: use `clipboard-win` directly | `docs/adr/0001-*.md` | ✅ |
| 1.4 | **Capture matrix** from Teams **Desktop** and Teams **Web** | `tests/fixtures/**` | serial, needs 1.2 |
| 1.5 | **Paste-back matrix**: put candidate HTML on the clipboard, paste into Teams, record what survives | `docs/clipboard-findings.md` | serial, needs 1.1 |
| 1.6 | Derive initial `RenderProfile` defaults from 1.5 | `crates/mdcore/src/profile.rs` | serial |

### Capture matrix (run for Desktop **and** Web)

Bold · italic · strikethrough · inline code · link · plain paragraph · two paragraphs · hard line
break (Shift+Enter) · H1/H2/H3 · bulleted list · numbered list · **nested list, two levels** ·
blockquote · fenced code block with a language · mixed inline marks on one line · a mark spanning a
link · an emoji · an @mention (expected to degrade).

For each: record which formats appeared, and save the HTML verbatim as a fixture.

### Paste-back matrix

For each construct, try the candidates and record **exactly one winner**:

| Construct | Candidates to try |
|---|---|
| Bold | `<strong>` · `<b>` · `<span style="font-weight:bold">` |
| Heading | `<h2>` · `<p><strong>` fallback |
| Nested list | native `<ul><li><ul>` · `style="margin-left"` |
| Code block | `<pre><code>` · `<div style="font-family:monospace">` |
| Blockquote | `<blockquote>` · `> ` prefixed text |
| Link | `<a href>` · bare URL |

### `docs/clipboard-findings.md` template

```markdown
# Clipboard findings
Teams Desktop version: … | Teams Web (browser + version): … | Windows build: … | Date: …

## Formats observed on copy
| Format | Present (Desktop) | Present (Web) | Notes |

## Per-construct capture results
| Construct | HTML Teams emits | Lossless? | Fixture |

## Paste-back results
| Construct | Encoding that survives | Encodings that fail | Notes |

## Consequences for RenderProfile
…

## Open questions / Desktop-vs-Web divergence
…
```

**Exit criteria**

- [ ] `just clipdump` prints a full format dump after copying from Teams.
- [ ] At least 18 fixtures captured, with Desktop and Web both represented.
- [ ] `clipboard-findings.md` has a filled-in winner for every row of the paste-back matrix.
- [ ] ADR 0001 decides the clipboard implementation, with a stated reason.
- [ ] `cf_html.rs` round-trip tests pass.

---

## Phase 2 — Core conversion engine (`mdcore`)

**Goal:** a pure Rust library converting between Markdown, HTML, plain text and the AST, with no
knowledge of Tauri or the clipboard. The bulk of the work, and the most parallelizable phase.

### 2.1 (serial) — freeze the AST

One agent writes `document/model.rs` and nothing else proceeds until it lands. Starting shape:

```rust
pub struct Document { pub blocks: Vec<Block> }

pub enum Block {
    Paragraph(Vec<Inline>),
    Heading { level: u8, content: Vec<Inline> },       // 1..=3, enforced on construction
    List(List),
    BlockQuote(Vec<Block>),
    CodeBlock { lang: Option<String>, code: String },
    ThematicBreak,
    /// Recognized but not representable. Degrades gracefully; every renderer
    /// emits `fallback` instead.
    Unsupported { kind: String, fallback: Vec<Inline> },
}

pub struct List { pub ordered: bool, pub start: u64, pub tight: bool, pub items: Vec<ListItem> }
pub struct ListItem { pub blocks: Vec<Block> }

pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Strike(Vec<Inline>),
    Code(String),
    Link { href: String, title: Option<String>, content: Vec<Inline> },
    SoftBreak,
    HardBreak,
}
```

**Invariants the normalizer must guarantee** (each gets a test):

1. No empty `Text("")`, and no mark wrapping an empty run.
2. Adjacent identical marks merge: `Bold(a), Bold(b)` -> `Bold(ab)`.
3. Marks are not redundantly nested: `Bold(Bold(x))` -> `Bold(x)`.
4. **Whitespace is hoisted out of marks**: `Bold(" x ")` -> `Text(" "), Bold("x"), Text(" ")`.
   Without this the Markdown renderer emits `** x **`, which is not bold in any Markdown parser.
   This is the single highest-value normalization rule in the codebase.
5. Mark nesting order is canonical (Bold outside Italic outside Strike) so round trips are stable.
6. `Heading.level` is clamped to 1..=3.

### 2.2 (serial) — freeze the Tauri command surface

Written once in `src-tauri/src/commands.rs`, as signatures with `todo!()` bodies, so the frontend
can be built against it immediately:

```rust
convert(input: String, from: Format, to: Format) -> Result<String, ConvertError>
read_clipboard() -> Result<ClipboardPayload, ClipError>            // { kind, html?, rtf?, text }
write_clipboard(payload: OutboundPayload) -> Result<(), ClipError> // html + text together
```

`Format` is `{ Markdown, Html, Text }`. `ClipboardPayload.kind` drives the "Pasted rich text" /
"Pasted plain text" toast. Every command returns `Result`; every error type implements `Serialize`;
every command is registered in `generate_handler![]` — an unregistered command fails *silently* at
runtime and costs an hour to diagnose.

### 2.3 (parallel) — workstreams

| # | Task | Owns |
|---|---|---|
| 2.3a | `markdown/parse.rs` — pulldown-cmark (GFM strikethrough on) -> AST | `markdown/parse.rs` |
| 2.3b | `markdown/render.rs` + `escape.rs` — AST -> Markdown, contextual escaping | `markdown/render.rs`, `escape.rs` |
| 2.3c | `html/parse.rs` + `styles.rs` — html5ever -> AST, style inference | `html/parse.rs`, `styles.rs` |
| 2.3d | `html/render.rs` + `profile.rs` — AST -> HTML under a `RenderProfile` | `html/render.rs`, `profile.rs` |
| 2.3e | `text/` — plain text <-> AST | `text/**` |
| 2.3f | `document/normalize.rs` — the invariant passes above | `normalize.rs` |
| 2.3g | Test harness: `tests/corpus.rs` runner + `tests/roundtrip.rs` proptests | `crates/mdcore/tests/**` |

Build **2.3g first**. The corpus runner is what lets every other workstream verify itself.

### Notes the implementing agents need

**Markdown escaping (2.3b)** is its own problem, not an afterthought. Escape ``\ ` * _ [ ] ( ) # + -
. ! > ~ |`` **contextually**: `_` only when intraword-ambiguous, `#`/`>`/`-`/`1.` only at line
start, `|` only inside tables. Over-escaping is ugly but correct; under-escaping is silently wrong.
Prefer over-escaping when unsure, and add a fixture.

**Style inference (2.3c)** is where Teams markup is normalized to intent, per `plan.md`:

| Signal | Becomes |
|---|---|
| `<b>`, `<strong>`, `font-weight` >= 600 or `bold`/`bolder` | `Bold` |
| `<i>`, `<em>`, `font-style: italic\|oblique` | `Italic` |
| `<s>`, `<del>`, `<strike>`, `text-decoration` containing `line-through` | `Strike` |
| `<code>`, `<tt>`, monospace `font-family` | `Code` |
| `<div>`, `<p>` | `Paragraph` |
| `<br>` | `HardBreak` |
| `font-weight: normal` on a descendant of a bold run | **removes** the mark |

Collapse HTML whitespace per the HTML rules (runs of whitespace -> one space; leading/trailing in a
block dropped) **except** inside `<pre>`. Run `ammonia` before parsing — clipboard HTML is untrusted
input from another process.

**`RenderProfile` (2.3d)** is the mechanism that makes Phase 4 cheap. The Teams dialect is expressed
as *data*, so tuning fidelity means editing a struct literal, not rewriting a renderer:

```rust
pub struct RenderProfile {
    pub bold: MarkStyle,               // Tag("strong") | Tag("b") | Style("font-weight:bold")
    pub headings: HeadingStrategy,     // Native | BoldParagraph
    pub code_block: CodeBlockStrategy, // PreCode | MonospaceDiv
    pub nested_lists: NestingStrategy, // Native | MarginIndent
    pub blockquote: QuoteStrategy,
    pub hard_break: BreakStrategy,
}

impl RenderProfile {
    pub fn teams() -> Self;     // tuned in Phase 4
    pub fn standard() -> Self;  // plain semantic HTML
}
```

### Fixture format (2.3g) — as built

One directory per case under `tests/fixtures/`, holding a `source.md`, `source.html` and/or
`source.txt` plus a mandatory `notes.md` recording where the case came from and what it proves. The
runner enumerates directories, so **adding a case requires no code change** — which is exactly what
makes Phase 4 tractable. A test asserts every fixture has notes; an undocumented fixture is a test
nobody can safely change later.

Expected output lives in **`insta` snapshots**, not hand-written `expected.*` files. Each source is
rendered to all three output formats in one reviewable snapshot, so an engine change surfaces as a
diff to accept or reject (`just review`) rather than as a literal someone has to guess at. Each
case is also asserted **idempotent**: converting a format to itself twice must equal converting it
once, which is the property that stops the two editor panes drifting as the user types.

Property tests: `parse_md(render_md(ast)) == ast` for arbitrary ASTs, and `render_md(parse_md(s))`
reaching a fixpoint in one pass.

**Exit criteria**

- [ ] `just test-core` green, with every Phase 1 fixture passing.
- [ ] Round-trip proptests pass at 1000+ cases.
- [ ] `just conv md2teams docs/plan.md` produces sane HTML from the command line.
- [ ] `cargo doc -p mdcore` has no missing-docs warnings on public items.
- [ ] `mdcore` has **zero** dependency on `tauri` — assert it: `cargo tree -p mdcore` contains no
      `tauri` node.

---

## Phase 3 — Minimal desktop UI

**Goal:** the one-screen two-pane app from `plan.md`, with live conversion and the three copy
actions. Depends on Phase 2's command surface (2.2); can start against the `todo!()` stubs.

| # | Task | Owns | Mode |
|---|---|---|---|
| 3.1 | Zustand store + the sync engine (below) | `src/stores/**`, `src/lib/**` | serial |
| 3.2 | `MarkdownEditor` — CodeMirror 6, markdown mode, monospace, no line numbers | `components/MarkdownEditor/**` | parallel |
| 3.3 | `RichEditor` — TipTap 3, marks restricted to the AST's inline set | `components/RichEditor/**` | parallel |
| 3.4 | `CopyActions` — Copy for Teams · Copy Markdown · Copy Text | `components/CopyActions/**` | parallel |
| 3.5 | `StatusToast` — transient "Pasted rich text" / "Copied" | `components/StatusToast/**` | parallel |
| 3.6 | Paste interception on the Formatted pane -> `read_clipboard` -> convert | `components/RichEditor/paste.ts` | serial, needs 3.1 |
| 3.7 | Wire the real clipboard commands per ADR 0001; permissions in `capabilities/default.json` | `src-tauri/**` | parallel |
| 3.8 | Playwright functional suite (below) | `playwright.config.ts`, `e2e/**` | serial, needs 3.1–3.6 |

### Functional tests (3.8) — how Playwright reaches a Tauri app

Playwright drives a browser, not a Tauri webview, and a browser has no Rust engine behind it. Rather
than reimplement conversion in JavaScript — which would test nothing — the frontend's conversion
backend is an interface with two implementations, chosen by feature-detecting Tauri at startup and
overridable with `?backend=mock`:

- **`tauri`** — `invoke('convert' | 'read_clipboard' | 'write_clipboard')`.
- **`mock`** — replays `src/test-support/oracle.json`, a table of **real engine output** keyed
  `"<from>:<to>:<input>"`, generated from the fixture corpus by
  `cargo run -p mdcli -- export-fixtures` (`just oracle`, which `just test-e2e` depends on, and
  which CI regenerates so the table can never drift from the engine).

So the E2E suite exercises the real UI against real conversions, while conversion *correctness*
stays covered exhaustively by the Rust corpus. What Playwright is actually there to catch is the
class of bug unit tests cannot reach: caret theft, sync loops, stale async responses landing out of
order, copy actions wiring the wrong payload, and layout breaking at small window sizes.

### The sync engine (3.1) — the one genuinely hard piece of the frontend

Two editors that each regenerate the other will loop, fight over the caret, and corrupt input. Three
rules prevent all of it:

1. **Single authority.** The focused pane owns the document. The other pane is *derived* and is
   never converted back while unfocused. Track `owner: 'markdown' | 'rich'`, set on focus.
2. **Suppressed echo.** When applying a derived update to a pane, set a flag so that pane's
   `onChange` does not re-enter the conversion pipeline.
3. **Sequence guard.** Conversion is async over IPC. Tag each request with an incrementing seq and
   drop any response whose seq is lower than the last applied. Without this, fast typing lands
   responses out of order and the pane flickers between stale states.

Debounce conversion at ~120 ms. Keep the canonical text in the store, not in editor state, so
undo/redo and pane swapping stay coherent.

**Exit criteria**

- [ ] Typing Markdown updates the Formatted pane live; editing Formatted updates Markdown live.
- [ ] Neither pane steals the caret or reorders text during fast typing (test by holding a key).
- [ ] Ctrl+V of Teams content into the Formatted pane yields correct Markdown plus a toast.
- [ ] All three copy buttons work; Copy for Teams places **HTML and plain text together**.
- [ ] The window resizes to 800×600 without layout breakage.

---

## Phase 4 — Teams fidelity

**Goal:** close the loop against real Teams. Requires Phase 1's findings and Phase 3's UI.

This phase is deliberately a **loop, not a list**:

1. Pick a construct from the matrix.
2. Round-trip it: Teams -> app -> Markdown -> app -> Copy for Teams -> Teams.
3. If it degrades, capture the failure as a **fixture**, then adjust `RenderProfile` or the
   normalizer until the fixture passes.
4. Record the finding in `clipboard-findings.md`.

Priority order, hardest first: nested lists -> code blocks -> blockquotes -> headings -> links inside
marks -> hard breaks -> mixed inline marks.

**Exit criteria**

- [ ] Every construct in the capture matrix survives a full round trip in Teams **Desktop**.
- [ ] The same, verified in Teams **Web**, with any divergence documented.
- [ ] Every fix has a fixture; `just test-core` green.
- [ ] `clipboard-findings.md` updated with the final profile rationale.

*Manual verification required. An agent cannot confirm this phase alone and must say so.*

---

## Phase 5 — Polish

| # | Task | Owns |
|---|---|---|
| 5.1 | Light / dark / system themes via CSS custom properties; follow the OS by default | `src/styles.css`, `stores/theme.ts` |
| 5.2 | Keyboard shortcuts: Ctrl+1/2 focus pane, Ctrl+Shift+C copy for Teams, Ctrl+B/I/K in the rich pane | `src/lib/shortcuts.ts` |
| 5.3 | Draggable pane divider with a persisted ratio | `components/SplitPane/**` |
| 5.4 | Floating selection toolbar (B I S `</>` Link) — **no permanent toolbar** | `components/RichEditor/BubbleMenu.tsx` |
| 5.5 | Responsive single-pane mode with a toggle below ~700 px | `App.tsx` |
| 5.6 | Preferences via `tauri-plugin-store`: theme, split ratio, window size | `src-tauri/**`, `stores/prefs.ts` |
| 5.7 | Undo/redo coherent across both panes | `stores/**` |
| 5.8 | Empty state, app icon, About | assets |

**Exit criteria:** all of the above work; `just build` ships an installer that runs on a clean
machine; no console errors in the release build.

---

## Phase 6 — Advanced (explicitly out of MVP)

Only after Phase 4 is reliable. Each is independently scoped; none blocks a release.

RTF import · tables · Teams @mentions · images · emoji shortcodes · a published `mdcli` binary ·
a WASM build of `mdcore` for a browser version · auto-update.

---

## Risk register

| Risk | Phase | Mitigation |
|---|---|---|
| Teams emits poor HTML, or accepts almost nothing on paste | 1 | Exactly why Phase 1 is a gate. If it fails, the fallback is plain-Markdown-on-clipboard and the app becomes Teams->Markdown only — still useful, and the architecture is unchanged. |
| `tauri-plugin-clipboard` cannot read HTML faithfully | 1 | The `clipboard-win` fallback is already the spike tool; ADR 0001 records the call. |
| CF_HTML offsets wrong -> blank pastes | 1 | Byte-exact round-trip unit tests before anything depends on it. |
| Two-editor sync loop / caret theft | 3 | Authority + echo suppression + seq guard, specified in 3.1. |
| Markdown escaping bugs producing silently wrong output | 2 | Dedicated `escape.rs`, proptest round trips, over-escape-when-unsure rule. |
| Teams Desktop and Web diverge | 4 | The capture matrix runs against both; `RenderProfile` can carry a per-target variant. |
| Scope creep into Teams APIs / message sending | all | `plan.md` excludes it explicitly; Phase 6 is the holding pen. |

## Suggested sequencing

```text
Phase 0 ──┬── Phase 1 (spike, GATE) ──┬── Phase 4 ── Phase 5 ── Phase 6
          └── Phase 2 (engine) ───────┴── Phase 3
```

Phases 1 and 2 run concurrently: the AST does not depend on the spike's answer, only the
`RenderProfile` defaults do. Phase 3 needs Phase 2's command surface but not its implementations.
