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
