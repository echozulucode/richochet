# ADR 0001 — Read the platform clipboard directly, via `clipboard-win`

- **Status:** accepted
- **Date:** 2026-09-10
- **Phase:** 1

## Context

Richochet's whole value depends on getting rich content off the clipboard and putting rich content
back. Tauri's official `clipboard-manager` plugin exposes `readText()`, images and `writeHtml()`,
but **no rich-HTML read**, so it cannot do the primary job. Three options were on the table:

1. **`tauri-plugin-clipboard` 2.1.11** (community, CrossCopy) — advertises text, HTML, RTF, image
   and file read _and_ write.
2. **`arboard` 3.6** — cross-platform, can `set_html`, but reads only text and images. Same gap as
   the official plugin.
3. **`clipboard-win` 5.4 directly** — raw Win32 clipboard access.

## Decision

Use **`clipboard-win` 5.4 directly** on Windows, with a `#[cfg(not(windows))]` arm that returns a
clear "unsupported platform" error.

## Rationale

Reading the `clipboard-win` source settled it. It already provides exactly what this app needs,
including the part that was expected to be the most error-prone:

- `raw::get_html` **already strips the CF_HTML header** and returns the fragment.
- `raw::set_html` **already writes a correct header** with valid byte offsets.
- `raw::EnumFormats` + `raw::format_name_big` give full format enumeration, which the Phase 1 spike
  needs regardless of which library serves the app itself.
- `raw::set_without_clear` allows writing HTML and a plain-text fallback in a **single
  open/empty/write session**, which is what makes "Copy for Teams" atomic. Writing the two
  representations in separate open/close cycles would leave only the last one on the clipboard —
  the exact bug this app cannot afford.

Given that a direct dependency covers the whole surface, adding a community Tauri plugin on top
would buy cross-platform reach we do not currently need (Teams-on-Windows is the target) at the
cost of an extra abstraction layer, an extra permission surface, and a third-party release cadence
between us and a Win32 API we are already calling correctly.

## Consequences

- **Windows-only clipboard for now.** macOS and Linux support means implementing `read`/`write`
  against `NSPasteboard` / X11+Wayland, or adopting `arboard` for text and a platform-specific HTML
  path. The `clipboard::{read, write}` signatures are platform-neutral, so this is an additive
  change behind an existing seam, not a redesign.
- We own a `cf_html` module anyway, as a **pure** encoder/decoder. It is not on the hot clipboard
  path — `clipboard-win` handles that — but it is needed to decode captured fixture bytes, to
  verify byte-exactly what Teams emits, and to keep the CF_HTML rules documented and tested in one
  place.
- The Phase 1 spike tool (`just clipdump`, `just capture`) and the app share one clipboard library,
  so what the spike observes is what the app will see.

## Revisit if

- Richochet needs to ship on macOS or Linux.
- Teams starts publishing a format `clipboard-win` cannot express.
- `raw::set_without_clear` turns out not to preserve the first format in practice — this is the one
  assumption here that has not yet been verified against a running Teams, and it is load-bearing
  for "Copy for Teams".
