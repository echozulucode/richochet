/**
 * Wire types mirrored from the frozen IPC contract in `src-tauri/src/commands.rs`.
 *
 * These names are load-bearing: `WireFormat` is serialized straight into
 * `invoke('convert', { from, to })` and must match the serde `rename_all = "camelCase"`
 * variants of the Rust enum.
 */

/** A conversion endpoint understood by the Rust engine. */
export type WireFormat = 'markdown' | 'html' | 'text';

/** What `read_clipboard` found. Mirrors `commands::ClipboardPayload`. */
export interface ClipboardPayload {
  /** `'rich'` when HTML was available, `'text'` when only plain text was. */
  kind: string;
  /** HTML fragment with any CF_HTML header already stripped. */
  html: string | null;
  /** RTF representation, if present. Unused until Phase 6. */
  rtf: string | null;
  /** Plain-text fallback. Always present. */
  text: string;
}

/** What `write_clipboard` accepts. Mirrors `commands::OutboundPayload`. */
export interface OutboundPayload {
  /** HTML representation; written alongside `text` when present. */
  html: string | null;
  /** Plain-text fallback. Always written. */
  text: string;
}

/** Which implementation of the conversion surface is live. */
export type BackendName = 'tauri' | 'mock';

/**
 * The whole of the app's contact with the outside world.
 *
 * Two implementations sit behind this: the real Tauri IPC backend, and a mock backed by a
 * precomputed table of real engine output so Playwright can drive the UI in a plain browser.
 */
export interface ConversionBackend {
  readonly name: BackendName;
  convert(input: string, from: WireFormat, to: WireFormat): Promise<string>;
  readClipboard(): Promise<ClipboardPayload>;
  writeClipboard(payload: OutboundPayload): Promise<void>;
}
