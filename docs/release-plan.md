# Release Plan — going public, installers, auto-updates

**Goal:** Richochet is a public MIT repo on GitHub that hands a Windows user a double-clickable
installer, and that installer quietly keeps itself current.

The shape is lifted from `echozulucode/hyperspanner`, which already runs this flow: a tag push
builds the Tauri bundle in Actions, attaches the installer plus a minisign-signed `latest.json` to a
**draft** GitHub Release, and the running app pings `releases/latest/download/latest.json` on
launch. Publishing the draft is the moment clients see the update. This plan adapts that to
Richochet's single-package layout, Windows-only scope, and much smaller UI surface.

Auto-update currently sits in Phase 6 of `implementation-plan.md` ("explicitly out of MVP"). This
plan pulls it, and packaging, forward into a **Phase 7 — Release engineering** that runs after
Phase 5 polish.

## Decisions taken

| Decision                    | Choice                                                      | Rationale                                                                                                                                                                                                          |
| --------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| License                     | MIT, `LICENSE` at repo root                                 | `Cargo.toml` already declares `license = "MIT"` but no file exists — a public repo with a dangling license claim is worse than no claim                                                                            |
| Platforms in the release    | **Windows only**                                            | The clipboard layer is Win32 by design (ADR 0001). A Linux bundle would build and then fail at the one thing the app does. Additive later, not now                                                                 |
| Windows bundle target       | **NSIS only** — drop `msi`                                  | The updater can only hand off to NSIS; MSI needs elevation on every update. Shipping both means shipping an installer whose users can never auto-update. Revisit if Intune/enterprise deployment is ever asked for |
| Install mode                | `currentUser` → `%LOCALAPPDATA%\Richochet`                  | No UAC prompt on install _or_ update. The VS Code user-installer pattern                                                                                                                                           |
| Code signing (Authenticode) | Skip for 0.x                                                | SmartScreen warns on first run; documented in the release notes. Revisit before non-technical users — see "Code signing, later"                                                                                    |
| Update endpoint             | `releases/latest/download/latest.json` on the public repo   | Zero hosting. `latest` resolves to the newest non-draft, non-prerelease release, so a draft is invisible to clients until published                                                                                |
| Update check cadence        | On launch, once                                             | No background timer to own. Offline is a silent no-op                                                                                                                                                              |
| Update UX                   | A section in the existing gear menu, plus a dot on the gear | This app has one settings affordance and no room for a banner. Don't grow a chrome surface for this                                                                                                                |
| Release trigger             | Push of a `v*` tag                                          | Plus a `workflow_dispatch` dry run, so the first real tag isn't also the first time the workflow has ever run                                                                                                      |
| Release visibility          | Created as a **draft**                                      | Notes get edited and the installer gets smoke-tested before any client is told a new version exists                                                                                                                |

## Phase 7.1 — Make the repo fit to be public (LANDED 2026-09-12, except 7.1.7 re-run)

|   #   | Task                                                                                                          | Files                                 |
| :---: | ------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| 7.1.1 | Add `LICENSE` — MIT, `Copyright (c) 2026 Eric Zimmerman`                                                      | `LICENSE`                             |
| 7.1.2 | Fill in package metadata: `license`, `author`, `homepage`, `repository`, `bugs`                               | `package.json`                        |
| 7.1.3 | Add `authors`, `repository`, `homepage` to the workspace package block                                        | `Cargo.toml`                          |
| 7.1.4 | Add `"packageManager": "pnpm@10.x"`, then drop the `version:` input from `pnpm/action-setup` everywhere       | `package.json`, `.github/workflows/*` |
| 7.1.5 | README: install section pointing at Releases, the SmartScreen note, a CI badge, "Windows only" above the fold | `README.md`                           |
| 7.1.6 | `SECURITY.md` — where to report, and the honest "hobby project, best effort" scope                            | `SECURITY.md`                         |
| 7.1.7 | Audit history for anything not meant to be public                                                             | —                                     |

7.1.7 is the one with teeth. `tests/fixtures/` is hand-written today (per `status.md`), but Phase 1
replaces those with **real Teams captures** — check that no real message content, names, or internal
URLs ride along. Do this before the repo flips, not after; public git history is not retractable.

**Exit:** repo is public, `LICENSE` renders on GitHub, CI badge green.

## Phase 7.2 — One version, three files (LANDED 2026-09-12)

The version lives in three places and they must agree, or the updater compares the wrong numbers and
either nags forever or never fires:

1. `package.json` → `version`
2. `Cargo.toml` → `[workspace.package] version` (inherited by `mdcore`, `mdcli`, `src-tauri`)
3. `src-tauri/tauri.conf.json` → `version` — **this is what `getVersion()` and the updater compare**

Hyperspanner mirrors five files with `scripts/bump-version.mjs`; the same script adapted to three is
the right answer here, with one change: the Cargo regex must anchor on `[workspace.package]`, not
`[package]`, and must not reach the `[workspace.dependencies]` version strings below it.

|   #   | Task                                                                                                     | Notes                                                   |
| :---: | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| 7.2.1 | `tools/bump-version.mjs` — takes `patch\|minor\|major\|<semver>`; `--tag` commits and tags, never pushes | Push stays a hand-step so a typo can't launch a release |
| 7.2.2 | `just bump <arg>` recipe                                                                                 | `just` is the only entry point (AGENTS.md)              |
| 7.2.3 | A `verify-version` job in `release.yml` that fails if the tag (minus `v`) doesn't match all three        | The cheapest guard against the worst-feeling bug here   |

Do **not** try to have `tauri.conf.json` inherit the version from Cargo: `src-tauri` uses
`version.workspace = true`, and Tauri's config reader does not resolve workspace inheritance. An
explicit, script-maintained, CI-verified number is the reliable option.

**Exit:** `just bump patch --tag` produces one commit touching exactly three files, plus a tag.

## Phase 7.3 — The installer (CONFIG LANDED 2026-09-12; steps 2-4 need a human)

All config, no code. `tauri.conf.json` currently bundles `["msi", "nsis"]` with no metadata and no
updater artifacts.

```jsonc
"bundle": {
  "active": true,
  "targets": ["nsis"],
  "publisher": "Eric Zimmerman",
  "copyright": "Copyright (c) 2026 Eric Zimmerman",
  "category": "Productivity",
  "shortDescription": "Teams rich text and Markdown, bouncing between formats.",
  "longDescription": "...",
  "homepage": "https://github.com/echozulucode/richochet",
  "licenseFile": "../LICENSE",             // NSIS shows the MIT text
  "icon": [ /* unchanged */ ],
  "windows": {
    "nsis": {
      "installMode": "currentUser",
      "displayLanguageSelector": false,
      "languages": ["English"]
    }
  }
}
```

`webviewInstallMode` stays at the default `downloadBootstrapper`: a smaller installer, and on Win11
WebView2 is already present so nothing downloads. `offlineInstaller` is the switch to flip if
air-gapped installs ever come up — it adds ~130 MB.

`createUpdaterArtifacts: true` belongs to **7.4**, not here. It is what emits the `.sig` the release
manifest needs, but with no signing key configured it breaks `just build` for anyone who clones the
repo — so it lands in the same commit as the key and the plugin, where it is first meaningful.

**Verification — needs a human, and ideally a clean VM:**

1. `just build`; confirm `target/release/bundle/nsis/Richochet_0.1.0_x64-setup.exe` (the target dir
   is at the workspace root, not under `src-tauri/`)
2. Run it: no UAC prompt, lands in `%LOCALAPPDATA%\Richochet`, Start-menu entry appears
3. Launch it; paste from real Teams; copy back. The **installed** build, not `just dev` — release
   mode has `panic = "abort"`, `strip = true` and a real CSP, and this is the first time all three
   are exercised together
4. Uninstall via Settings → Apps; confirm the install dir and Start-menu entry are gone

**Exit:** an installer that runs on a machine which has never had Rust or Node on it.

## Phase 7.4 — Signing keys and the updater plumbing

One-time, hands-on, and **not** something an agent should do — it involves a private key that must
never touch the repo or a tool transcript.

```sh
pnpm tauri signer generate -w $HOME/.tauri/richochet-updater.key
```

Set a passphrase. Then:

|   #   | Task                                                                                                          | Files                                 |
| :---: | ------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| 7.4.0 | Set `bundle.createUpdaterArtifacts: true` — deferred from 7.3, where it fails the build without a key         | `src-tauri/tauri.conf.json`           |
| 7.4.1 | Paste the `.pub` contents into `plugins.updater.pubkey`; set `endpoints` and `windows.installMode: "passive"` | `src-tauri/tauri.conf.json`           |
| 7.4.2 | Add `tauri-plugin-updater = "2"`, `tauri-plugin-process = "2"`                                                | `src-tauri/Cargo.toml`                |
| 7.4.3 | Register both plugins **before** `invoke_handler`                                                             | `src-tauri/src/lib.rs`                |
| 7.4.4 | Add `"updater:default"`, `"process:allow-restart"`                                                            | `src-tauri/capabilities/default.json` |
| 7.4.5 | `pnpm add @tauri-apps/plugin-updater @tauri-apps/plugin-process`, pinned exactly                              | `package.json`                        |
| 7.4.6 | Store `TAURI_SIGNING_PRIVATE_KEY` (file contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as Actions secrets | GitHub repo settings                  |

```jsonc
"plugins": {
  "updater": {
    "endpoints": ["https://github.com/echozulucode/richochet/releases/latest/download/latest.json"],
    "pubkey": "<contents of richochet-updater.key.pub>",
    "windows": { "installMode": "passive" }
  }
}
```

The private key is the whole security model: anyone holding it can ship a signed update to every
install. Back it up where you'd back up a password — losing it strands every existing install on its
current version permanently, with no recovery short of telling users to reinstall by hand.

Per AGENTS.md, a missing capability entry fails **silently at runtime**. If the update check does
nothing and logs nothing, 7.4.4 is the first thing to check.

**Exit:** `just check` clean with the plugins registered; a dev build's check returns "up to date"
rather than erroring.

## Phase 7.5 — The update UX

Richochet has one settings affordance: the gear in `TitleBar.tsx` opening a small menu with an
Appearance group. Updates become a second group in that same menu — no banner, no modal, no new
chrome.

```text
┌─ gear menu ──────────────┐
│ APPEARANCE               │
│   System          ✓      │
│   Light                  │
│   Dark                   │
│ ──────────────────────── │
│ UPDATES                  │
│   Richochet 0.1.0        │  ← up to date: quiet, no action
│   ▸ 0.1.1 available      │  ← available: click downloads
│   Downloading… 47%       │
│   ▸ Restart to update    │  ← calls relaunch()
└──────────────────────────┘
```

A small dot on the gear when an update is available is the only thing that draws the eye — the same
badge idea as hyperspanner's SETTINGS pill, scaled to a 28 px button.

|   #   | Task                                                                                                                                                             | Files                                  |
| :---: | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| 7.5.1 | `stores/updateStore.ts` — zustand, matching the `themeStore`/`toastStore` shape. `idle → checking → up-to-date \| available → downloading → ready`, plus `error` | `src/stores/updateStore.ts`            |
| 7.5.2 | An injectable client seam (`__setUpdaterClientForTests`) wrapping the two plugins behind dynamic `import()`                                                      | same                                   |
| 7.5.3 | Fire the launch check once; failures silent (`console.warn` only)                                                                                                | `src/App.tsx`                          |
| 7.5.4 | Updates group + gear dot                                                                                                                                         | `src/components/TitleBar/TitleBar.tsx` |
| 7.5.5 | Unit tests over the state machine with a fake client — every transition, error included                                                                          | `updateStore.test.ts`                  |
| 7.5.6 | E2E: the group renders and is inert under `?backend=mock`                                                                                                        | `e2e/*.spec.ts`                        |

The dynamic-import seam is load-bearing for the test layers: Playwright runs in a browser and Vitest
in jsdom, and neither has a Tauri runtime. A static import of `@tauri-apps/plugin-updater` would drag
the plugin into those bundles and throw on load.

**Offline tolerance is a requirement, not a nicety.** No network, GitHub down, a corporate proxy
eating the request — the app opens and works, logs a warning, re-checks next launch. The `error`
state renders no UI at all.

## Phase 7.6 — GitHub Actions

`ci.yml` stays as it is (windows-latest, `just check` + `just test`). Two new workflows:

### `release.yml` — tag push

```yaml
name: Release
on:
  push:
    tags: ['v*']
permissions:
  contents: write # tauri-action creates the release + uploads assets
jobs:
  verify-version: # tag must match all three version files
  build:
    needs: verify-version
    runs-on: windows-latest
    steps:
      - checkout / pnpm / node 24 / rust stable / Swatinem cache
      - pnpm install --frozen-lockfile
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: 'Richochet ${{ github.ref_name }}'
          releaseDraft: true
          prerelease: false
          updaterJsonPreferNsis: true
```

Differences from hyperspanner's workflow, all of them from layout:

- **No `projectPath`** — `src-tauri` is at the repo root, so the default is right. Hyperspanner needs
  `apps/desktop` because it's a pnpm workspace.
- **No matrix, no Linux dep install** — one runner, Windows.
- **`workspaces: src-tauri`** for `Swatinem/rust-cache` (hyperspanner points at
  `apps/desktop/src-tauri`). Worth checking whether caching the workspace root is better here, since
  `mdcore` is the bulk of the compile.
- **`version:` on `pnpm/action-setup`** — drop it once 7.1.4 adds `packageManager`; passing both
  makes the action refuse to start.
- **`updaterJsonPreferNsis: true`** is belt-and-braces with MSI gone, but harmless and correct if
  MSI ever returns.

The release body should carry, every time: the download name, the SmartScreen "More info → Run
anyway" instruction, and Windows-only scope.

### `release-dry-run.yml` — `workflow_dispatch`

The same build with no signing secrets and no release step, uploading the installer as a workflow
artifact. This exists so the first `v0.1.0` tag isn't also the first execution of a never-run
workflow. Run it, download the artifact, install it, _then_ tag.

**Exit:** a dispatched dry run produces an installable `.exe` artifact.

## The release ritual, once this lands

```sh
just ci                          # green locally first
just bump patch --tag            # three files, one commit, one tag
git push && git push origin v0.1.1
# Actions builds → draft release appears
# Download the .exe from the draft; install it over an existing install; smoke-test
# Edit the notes; Publish the draft   ← the moment clients see it
```

Everything before "Publish" is reversible. Deleting a published tag is not: any client that already
pinged `latest.json` has the version number, and a re-published tag with different bytes fails
signature verification on some installs and not others. **Bump forward, never re-tag.**

## End-to-end verification — needs two versions and a human

The updater cannot be tested from source; it needs two real signed releases.

1. Publish `v0.1.0`. Install on a clean machine. Gear menu shows `0.1.0`, no dot
2. `just bump patch --tag`, push, publish `v0.1.1`
3. Relaunch the installed `0.1.0`. Expect: dot on the gear, `0.1.1 available`
4. Click through download → `Restart to update` → app reopens reporting `0.1.1`
5. Confirm no UAC prompt at any point, and that window geometry and the stored prefs (theme, split
   ratio) survive the update
6. Disconnect the network and relaunch: app opens normally, no error UI

Step 5 matters more than it looks — `tauri-plugin-store` writes under the app-data identifier, and an
updated install that silently loses the user's theme is a real regression.

## Code signing, later

For 0.x, SmartScreen warns and the release notes explain it. The options when that stops being
acceptable:

| Option                | Cost         | Effect                                                                             |
| --------------------- | ------------ | ---------------------------------------------------------------------------------- |
| Nothing (current)     | $0           | "Unrecognized publisher" warning on first run, forever                             |
| Azure Trusted Signing | ~$120/yr     | Real Authenticode, no hardware token, works in CI. Needs a verifiable org identity |
| OV certificate        | ~$200–400/yr | Signs, but SmartScreen reputation still has to accumulate                          |
| EV certificate        | ~$400–700/yr | Immediate SmartScreen reputation; hardware token, awkward in CI                    |

Azure Trusted Signing is the one to reach for, via Tauri's custom sign command. It's an additive
change to `tauri.conf.json` plus two secrets — the rest of this plan doesn't move.

## Risks

| Risk                                                                | Mitigation                                                      |
| ------------------------------------------------------------------- | --------------------------------------------------------------- |
| Lost signing key strands every install                              | Back up the key + passphrase like a password, off-machine       |
| Three versions drift; updater compares wrong numbers                | `bump-version` script + the `verify-version` CI job             |
| Missing capability entry → update check silently does nothing       | Explicit in 7.4.4; first thing to check when the check is inert |
| Static plugin import breaks Vitest/Playwright                       | Dynamic-import client seam (7.5.2)                              |
| The first tag reveals a broken workflow                             | `release-dry-run.yml` before the first real tag                 |
| Real Teams captures leak into public history                        | Audit before flipping the repo public (7.1.7)                   |
| An update lands but prefs / window state reset                      | Explicit step 5 in the E2E verification                         |
| MSI shipped alongside NSIS strands users on a non-updatable install | Drop `msi` now, before anyone has installed it                  |

## Sequencing

```text
7.1 public-ready ──► 7.2 versioning ──► 7.3 installer ──┐
                                                        ├──► 7.6 Actions ──► v0.1.0
                     7.4 keys + plumbing ──► 7.5 UX ────┘
```

7.3 and 7.4/7.5 are independent — installer config and updater plumbing touch different parts of
`tauri.conf.json` and different code. 7.6 needs all of them. 7.1 comes first because the updater
endpoint URL only resolves once the repo is public.
