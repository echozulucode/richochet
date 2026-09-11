# AGENTS.md

Working agreement for agents on **Richochet** — a Tauri 2 desktop app that converts between
Microsoft Teams rich clipboard content and Markdown.

> **The name is `Richochet`** — "rich text" + *ricochet*, content bouncing between formats. The `h`
> after `Ric` is deliberate. It is **not** a typo: never write `Ricochet` in code, config, commits,
> docs or UI copy, and never "fix" it if you see it. Bundle id `com.echozed.richochet`.

- **Design rationale:** `docs/plan.md`
- **Build order, phases, task tables:** `docs/implementation-plan.md` ← read this before starting work
- **Empirical Teams clipboard behaviour:** `docs/clipboard-findings.md` (produced by Phase 1)
- **Irreversible decisions:** `docs/adr/`

## Commands — use `just`, always

`just` is the only entry point. Do not invoke `cargo`/`pnpm` directly in a report, a script, or CI;
if a command you need is missing, add a recipe to the `justfile` rather than working around it.

| Command | What it does |
|---|---|
| `just` | list all recipes |
| `just setup` | one-time install of JS deps + cargo tooling |
| `just dev` | run the app with hot reload |
| `just build` | release bundle |
| `just check` | clippy `-D warnings` + `tsc --noEmit` + eslint |
| `just test` | full suite |
| `just test-core` | engine tests only — fast, no Tauri. **Your inner loop.** |
| `just review` | review pending `insta` snapshot changes |
| `just fmt` | format Rust + TS |
| `just ci` | exactly what CI runs (`check` + `test`) |
| `just conv <mode> <file>` | convert on the command line, e.g. `just conv md2teams notes.md` |
| `just clipdump` | dump every format on the clipboard |
| `just capture <name>` | save the clipboard into `tests/fixtures/<name>/` |

## Architecture in one paragraph

Everything normalizes into a small **document AST** (`crates/mdcore/src/document/model.rs`).
Markdown, HTML and plain text each have a parser into it and a renderer out of it. Nothing else is
canonical — never treat HTML as the source of truth, and never let Teams-specific markup leak past
`html/parse.rs` into the rest of the codebase. `mdcore` is a pure library with **no Tauri
dependency**; `src-tauri` is a thin shell that owns the clipboard and the command surface.

## Rules

### Do

- **Preserve intent, not markup.** `<span style="font-weight:600">` becomes `Bold`, not a styled
  span. Normalize at the parse boundary.
- **New Teams quirk = new fixture**, not a new code branch. Drop a directory into
  `tests/fixtures/` with a `notes.md` saying where it came from; the corpus runner picks it up with
  no code change.
- **Freeze shared contracts before fanning out.** The AST, the Tauri command signatures and the
  fixture layout are written by one agent first. Parallel tasks must own disjoint files.
- Register every command in `tauri::generate_handler![]`, and add the matching permission to
  `src-tauri/capabilities/default.json`. Missing either fails **silently at runtime**.
- Return `Result<T, E>` from every command; error types must implement `Serialize`.
- Use owned types (`String`, not `&str`) in async commands.
- Keep `src-tauri/src/main.rs` a thin passthrough to `lib.rs::run()`.
- State plainly when a check needs a human — anything requiring a real paste into Teams cannot be
  verified by an agent.

### Do not

- Do not add a dependency on `tauri` to `mdcore`. It is load-bearing: it keeps engine tests fast and
  the engine reusable.
- Do not scaffold Tailwind the v3 way. This is **Tailwind 4**: `@import "tailwindcss";` plus the
  `@tailwindcss/vite` plugin. No `tailwind.config.js`, no PostCSS. The v3 layout builds cleanly and
  ships zero styles.
- Do not use `@tauri-apps/api/tauri` — that is v1. It is `@tauri-apps/api/core`.
- Do not float dependency versions. The pinned table is in `docs/implementation-plan.md`.
- Do not add a permanent formatting toolbar. Selection-triggered floating controls only.
- Do not widen scope into Teams APIs, auth, sending messages, attachments, mentions or Loop
  components. `docs/plan.md` excludes them; Phase 6 is the holding pen.

## Traps that have a real cost here

1. **Whitespace inside marks.** `Bold(" x ")` renders as `** x **`, which no Markdown parser treats
   as bold. The normalizer must hoist whitespace out of marks. Highest-value rule in the codebase.
2. **CF_HTML offsets.** Windows clipboard HTML is prefixed by a header whose `StartHTML`/`EndHTML`/
   `StartFragment`/`EndFragment` fields are **byte offsets into the block itself**, zero-padded to a
   fixed width. Wrong offsets produce pastes that are blank in one app and fine in another. Test
   byte-exactly.
3. **Two-editor sync loops.** The focused pane is the sole authority; the other is derived and never
   converted back. Suppress the echo when applying a derived update. Tag async conversions with an
   incrementing seq and drop stale responses. See §3.1 of the implementation plan.
4. **Markdown escaping.** Escape contextually, and prefer over-escaping when unsure — it is ugly but
   correct; under-escaping is silently wrong.
5. **Clipboard HTML is untrusted input** from another process. Run `ammonia` before parsing.

## Test architecture

Four layers, each catching what the others can't. Put a test at the lowest layer that can catch the
bug.

| Layer | Where | Catches |
|---|---|---|
| Unit | inline `#[cfg(test)]` / `*.test.ts` | logic inside one module |
| Corpus | `tests/fixtures/` + `crates/mdcore/tests/corpus.rs` | conversion output, per construct |
| Property | `crates/mdcore/tests/roundtrip.rs` | the *rules* — round trips, idempotence, escaping |
| Functional | `e2e/*.spec.ts` (Playwright) | caret theft, sync loops, stale responses, copy wiring, layout |

**Adding a fixture** — create `tests/fixtures/<name>/` with a `source.md`, `source.html` and/or
`source.txt`, plus a `notes.md` saying where it came from and what it proves (a test enforces the
notes). No code change needed; the runner enumerates directories. Expected output lives in `insta`
snapshots — run `just test-core`, then `just review` to accept the diff.

**Playwright** drives a real browser, which has no Rust engine behind it. The frontend's conversion
backend is an interface: `tauri` (IPC) in the app, `mock` in tests. The mock replays
`src/test-support/oracle.json` — **real engine output** precomputed from the corpus by `just
oracle` — so E2E exercises real conversions without reimplementing anything in JS. Never "fix" an
E2E failure by editing the oracle; it is generated, gitignored, and regenerated by CI.

Query params the app honours for tests: `?backend=mock`, `?latency=<ms>`. Test hooks live on
`window.__richochet_test` (mock backend only). Elements carry stable `data-testid`s — use those in
specs, not CSS classes or text content.

## Code conventions

**Rust** — edition 2021, `rustfmt` defaults, clippy clean at `-D warnings`. `thiserror` for error
types. Doc comments on public items. A non-obvious normalization rule cites the fixture that
motivated it:

```rust
// Teams Web wraps list text in a redundant <span>; see tests/fixtures/web-nested-list/
```

**TypeScript** — strict mode, no `any`. Components are function components with named exports.
Zustand stores live in `src/stores/` and hold the canonical text; editors hold view state only.

## Definition of done

1. `just check` clean.
2. `just test` green.
3. `just fmt` applied.
4. New behaviour has a test; a new Teams quirk has a fixture.
5. You have actually run the commands — never report a phase complete on the strength of "it should
   work".

## Commits

Conventional commits, scoped to the crate or area: `feat(mdcore): hoist whitespace out of marks`,
`fix(clipboard): correct CF_HTML fragment offsets`, `test(fixtures): add Teams Web nested list`.
End commit messages with:

```
Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
```
