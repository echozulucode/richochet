set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

# List available recipes
default:
    @just --list

# One-time setup: install JS deps and cargo tooling
setup:
    pnpm install
    cargo install cargo-insta --locked

# Run the desktop app with hot reload
dev:
    pnpm tauri dev

# Run just the frontend in a browser against the mock conversion backend
dev-web:
    pnpm vite dev

# The bundler signs the installer for the auto-updater, so it needs the private key. Without this
# check it compiles for minutes, writes a complete-looking installer, and only then fails — leaving
# an .exe with no .sig that can never be offered as an update. Checking first fails in a second.

# Build the signed release installer (needs TAURI_SIGNING_PRIVATE_KEY; see README)
build:
    @if (-not $env:TAURI_SIGNING_PRIVATE_KEY) { Write-Host 'TAURI_SIGNING_PRIVATE_KEY is not set, so the installer cannot be signed for the auto-updater.' -ForegroundColor Red; Write-Host 'Set it (see README > Build it yourself), or run `just build-unsigned` for a local test build.' -ForegroundColor Red; exit 1 }
    pnpm tauri build

# Build an installer WITHOUT updater signing — for local testing only, never for a release
build-unsigned:
    pnpm tauri build --config src-tauri/tauri.unsigned.conf.json

# Bump the version in all three files that carry it, e.g. `just bump patch --tag`
bump *ARGS:
    node tools/bump-version.mjs {{ARGS}}

# Verify all three version files agree with a version or tag — what the release workflow checks
check-version VERSION:
    node tools/bump-version.mjs --check {{VERSION}}

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
test: test-core test-tauri test-ui test-e2e

# Engine tests only — fast, no Tauri. The inner loop.
test-core:
    cargo test -p mdcore

test-tauri:
    cargo test -p app --lib

test-ui:
    pnpm vitest run

# Playwright functional tests against the real UI in a browser
test-e2e: oracle
    pnpm playwright test

# Source art must be a square PNG with transparency, 1024x1024 or larger. It is kept in the repo
# as src-tauri/icons/source.png so the set can always be regenerated from the original.

# Regenerate every app icon from the source art
icon FILE="src-tauri/icons/source.png":
    pnpm tauri icon {{FILE}}
    @pwsh -NoProfile -Command "Remove-Item -Recurse -Force src-tauri/icons/android, src-tauri/icons/ios -ErrorAction SilentlyContinue"
    @echo "Regenerated src-tauri/icons. Mobile icon sets removed - Richochet is a desktop app."

# Playwright runs in a browser, where the Rust engine is unavailable, so the mock backend replays
# real engine output rather than reimplementing conversion in JS. `just test-e2e` depends on this.

# Regenerate the E2E conversion oracle from the fixture corpus
oracle:
    cargo run -q -p mdcli -- export-fixtures --out src/test-support/oracle.json

# Review pending snapshot changes interactively
review:
    cargo insta review

# Format everything
fmt:
    cargo fmt --all
    pnpm prettier --write .

# Convert on the command line, e.g. `just conv md2teams notes.md`
conv *ARGS:
    cargo run -q -p mdcli -- {{ARGS}}

# Dump every format currently on the clipboard (Phase 1 workhorse)
clipdump:
    cargo run -q -p mdcli -- dump-clipboard

# Capture the current clipboard into tests/fixtures/<NAME>/
capture NAME:
    cargo run -q -p mdcli -- capture --name {{NAME}}

# Teams Desktop is a WebView2 app, so HTML copied out of Chromium is the closest proxy for what
# Teams emits. A proxy only: it says nothing about what Teams *accepts*, which needs Phase 1.

# Copy a sample page out of a real browser and capture it as a fixture
capture-web NAME FILE="tools/web-samples/rich-message.html":
    node tools/capture-web.mjs {{FILE}}
    cargo run -q -p mdcli -- capture --name {{NAME}}

clean:
    cargo clean
    pnpm exec rimraf dist node_modules/.vite test-results playwright-report
