import type { ClipboardPayload } from './types';

/**
 * A tiny window-level probe for the Playwright suite.
 *
 * Only installed when the mock backend is live, so the shipped desktop app never grows a
 * test surface. `lastSeq` is the most recently *issued* conversion sequence number and
 * `lastAppliedSeq` the most recently applied one; when they differ, a response is in flight
 * or a stale response was dropped.
 */
export interface TestHook {
  /** Conversion requests issued but not yet resolved. */
  pendingConversions: number;
  /** The most recently issued conversion sequence number. */
  lastSeq: number;
  /** The most recently applied conversion sequence number. */
  lastAppliedSeq: number;
  /** Which backend is live. */
  backend: string;
  /** Every payload handed to the mock `write_clipboard`, oldest first. */
  clipboardWrites: Array<{ html: string | null; text: string }>;
  /** How many times the mock `read_clipboard` has been called. */
  clipboardReads: number;
  /** The scroll-sync outline in use: 1-based source line per top-level block. */
  outlineLines: number[];
  /** How many outline responses have been applied. Lets a spec wait for a fresh one. */
  outlineRevision: number;
  /** Stage what the next `read_clipboard` should return. */
  stageClipboard(payload: StagedClipboard): void;
}

/** What a test stages for the next clipboard read. */
export interface StagedClipboard {
  kind: 'rich' | 'text';
  html?: string;
  text: string;
}

declare global {
  interface Window {
    __richochet_test?: TestHook;
  }
}

let hook: TestHook | null = null;
let staged: ClipboardPayload | null = null;

/** Install the probe on `window`. Idempotent. */
export function installTestHook(backend: string): TestHook {
  hook = {
    pendingConversions: 0,
    lastSeq: 0,
    lastAppliedSeq: 0,
    backend,
    clipboardWrites: [],
    clipboardReads: 0,
    outlineLines: [],
    outlineRevision: 0,
    stageClipboard(payload: StagedClipboard) {
      staged = {
        kind: payload.kind,
        html: payload.html ?? null,
        rtf: null,
        text: payload.text,
      };
    },
  };
  if (typeof window !== 'undefined') {
    window.__richochet_test = hook;
  }
  return hook;
}

/** Mutate the probe if it is installed; a no-op in the desktop app. */
export function updateTestHook(patch: Partial<Omit<TestHook, 'stageClipboard'>>): void {
  if (!hook) return;
  Object.assign(hook, patch);
}

/** Add `delta` to the in-flight conversion count. */
export function bumpPending(delta: number): void {
  if (!hook) return;
  hook.pendingConversions = Math.max(0, hook.pendingConversions + delta);
}

/** Record a clipboard write made by the mock backend. */
export function recordClipboardWrite(payload: { html: string | null; text: string }): void {
  hook?.clipboardWrites.push(payload);
}

/** Take the payload a test staged for `read_clipboard`, if any. Consumed once. */
export function takeStagedClipboard(): ClipboardPayload | null {
  if (hook) hook.clipboardReads += 1;
  const value = staged;
  staged = null;
  return value;
}

/** Read the probe, for tests. */
export function peekTestHook(): TestHook | null {
  return hook;
}

/** Drop the probe. Used by unit tests to keep instances isolated. */
export function resetTestHook(): void {
  hook = null;
  staged = null;
  if (typeof window !== 'undefined') {
    delete window.__richochet_test;
  }
}
