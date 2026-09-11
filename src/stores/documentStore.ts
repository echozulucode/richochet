import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand/vanilla';

import { getBackend } from '../lib/converter';
import { bumpPending, updateTestHook } from '../lib/testHook';
import type { ConversionBackend, WireFormat } from '../lib/types';

/** The two panes. The focused one owns the document; the other is derived. */
export type PaneId = 'markdown' | 'rich';

/** How long typing must pause before a conversion is issued. */
export const DEBOUNCE_MS = 120;

/** Cancels a scheduled run. */
type Cancel = () => void;

/** Injection seam so tests can supply a deterministic converter and scheduler. */
export interface SyncDeps {
  /** Resolved lazily so backend selection can happen after this module is imported. */
  converter: () => ConversionBackend;
  debounceMs: number;
  /** Defaults to a debounceMs timer. Tests pass an immediate scheduler. */
  schedule: (run: () => void) => Cancel;
}

export interface DocumentState {
  /** Canonical Markdown. Authoritative while owner is 'markdown'. */
  markdown: string;
  /** Canonical HTML. Authoritative while owner is 'rich'. */
  html: string;
  /** The pane that owns the document. Set on focus. Rule 1: single authority. */
  owner: PaneId;
  /**
   * Bumped whenever a pane's text is replaced by something other than that pane's own editing.
   * Editors watch this to know when to push new text into their view; it is view bookkeeping,
   * not part of the three sync rules.
   */
  markdownRevision: number;
  htmlRevision: number;
  /** Rule 2: suppressed echo. Set before a derived update; consumed by that pane's onChange. */
  suppressed: Record<PaneId, boolean>;
  /** Rule 3: sequence guard. The last seq issued, and the last one actually applied. */
  issuedSeq: number;
  appliedSeq: number;
  /** True while at least one conversion is in flight. */
  converting: boolean;
  /** Last conversion error, or null. */
  error: string | null;
}

export interface DocumentActions {
  /** Take ownership of the document. Clears both echo flags. */
  focusPane: (pane: PaneId) => void;
  /** Report an edit made inside a pane's editor. */
  editPane: (pane: PaneId, text: string) => void;
  /** Replace the whole document with Markdown (the paste path) and re-derive the rich pane. */
  replaceDocument: (markdown: string) => void;
  /** The owning pane's text and its format - what every copy action converts from. */
  canonical: () => { input: string; from: WireFormat };
  /** Clear the conversion error. */
  clearError: () => void;
  /** Reset to an empty document. */
  reset: () => void;
}

export type DocumentStore = DocumentState & DocumentActions;

const initialState: DocumentState = {
  markdown: '',
  html: '',
  owner: 'markdown',
  markdownRevision: 0,
  htmlRevision: 0,
  suppressed: { markdown: false, rich: false },
  issuedSeq: 0,
  appliedSeq: 0,
  converting: false,
  error: null,
};

function defaultSchedule(debounceMs: number): (run: () => void) => Cancel {
  return (run) => {
    const handle = setTimeout(run, debounceMs);
    return () => {
      clearTimeout(handle);
    };
  };
}

function formatOf(pane: PaneId): WireFormat {
  return pane === 'markdown' ? 'markdown' : 'html';
}

function otherPane(pane: PaneId): PaneId {
  return pane === 'markdown' ? 'rich' : 'markdown';
}

function messageOf(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Build a document store.
 *
 * The three rules from section 3.1 of the implementation plan live here and nowhere else:
 *
 * 1. Single authority - editPane drops any edit from a pane that is not the owner, so the
 *    derived pane is never converted back while unfocused.
 * 2. Suppressed echo - applying a derived update sets suppressed[pane], which that pane's next
 *    editPane call consumes without re-entering the pipeline.
 * 3. Sequence guard - every request carries an incrementing seq; a response whose seq is not
 *    newer than appliedSeq is dropped, so a slow early response can never overwrite a fast
 *    later one.
 */
export function createDocumentStore(overrides: Partial<SyncDeps> = {}): StoreApi<DocumentStore> {
  const debounceMs = overrides.debounceMs ?? DEBOUNCE_MS;
  const deps: SyncDeps = {
    converter: overrides.converter ?? getBackend,
    debounceMs,
    schedule: overrides.schedule ?? defaultSchedule(debounceMs),
  };

  let cancelPending: Cancel | null = null;

  return createStore<DocumentStore>()((set, get) => {
    /** Apply a conversion response, or drop it if a newer one already landed. */
    function applyResponse(seq: number, target: PaneId, output: string): void {
      const state = get();
      if (seq <= state.appliedSeq) return; // Rule 3: stale response, drop it.
      const patch: Partial<DocumentState> = {
        appliedSeq: seq,
        converting: seq < state.issuedSeq,
        error: null,
        // Rule 2: the derived pane must not echo this back into the pipeline.
        suppressed: { ...state.suppressed, [target]: true },
      };
      if (target === 'markdown') {
        patch.markdown = output;
        patch.markdownRevision = state.markdownRevision + 1;
      } else {
        patch.html = output;
        patch.htmlRevision = state.htmlRevision + 1;
      }
      set(patch);
      updateTestHook({ lastAppliedSeq: seq });
    }

    function applyFailure(seq: number, error: unknown): void {
      const state = get();
      // Still advance appliedSeq: an older in-flight response must not overwrite this outcome.
      if (seq <= state.appliedSeq) return;
      set({ appliedSeq: seq, converting: seq < state.issuedSeq, error: messageOf(error) });
      updateTestHook({ lastAppliedSeq: seq });
    }

    function convert(input: string, from: WireFormat, to: WireFormat, target: PaneId): void {
      const seq = get().issuedSeq + 1;
      set({ issuedSeq: seq, converting: true });
      updateTestHook({ lastSeq: seq });
      bumpPending(1);
      void deps
        .converter()
        .convert(input, from, to)
        .then(
          (output) => {
            applyResponse(seq, target, output);
          },
          (error: unknown) => {
            applyFailure(seq, error);
          },
        )
        .finally(() => {
          bumpPending(-1);
        });
    }

    /** Drop a debounced run that has not fired yet. */
    function clearScheduled(): void {
      if (!cancelPending) return;
      cancelPending();
      cancelPending = null;
      bumpPending(-1);
    }

    /**
     * Debounced: convert the owner's text into the other pane.
     *
     * A scheduled-but-not-yet-issued conversion counts as pending, so an E2E test waiting for
     * "nothing in flight" cannot slip through the debounce window and assert on stale text.
     */
    function scheduleSync(): void {
      clearScheduled();
      bumpPending(1);
      let fired = false;
      const cancel = deps.schedule(() => {
        fired = true;
        cancelPending = null;
        bumpPending(-1);
        const { owner, markdown, html } = get();
        const target = otherPane(owner);
        const input = owner === 'markdown' ? markdown : html;
        convert(input, formatOf(owner), formatOf(target), target);
      });
      // A synchronous scheduler (used by the unit tests) has already run by this point; only
      // store the canceller when there is still something to cancel.
      if (!fired) cancelPending = cancel;
    }

    return {
      ...initialState,

      focusPane(pane) {
        if (get().owner === pane) return;
        set({ owner: pane, suppressed: { markdown: false, rich: false } });
      },

      editPane(pane, text) {
        const state = get();
        if (state.suppressed[pane]) {
          // Rule 2: this is our own derived update coming back. Consume the flag and stop.
          set({ suppressed: { ...state.suppressed, [pane]: false } });
          return;
        }
        // Rule 1: only the owner may change the document.
        if (state.owner !== pane) return;
        if (pane === 'markdown') {
          if (state.markdown === text) return;
          set({ markdown: text });
        } else {
          if (state.html === text) return;
          set({ html: text });
        }
        scheduleSync();
      },

      replaceDocument(markdown) {
        clearScheduled();
        const state = get();
        set({
          markdown,
          markdownRevision: state.markdownRevision + 1,
          suppressed: { ...state.suppressed, markdown: true },
          error: null,
        });
        // The rich pane is derived from the pasted Markdown regardless of who holds focus.
        convert(markdown, 'markdown', 'html', 'rich');
      },

      canonical() {
        const { owner, markdown, html } = get();
        return owner === 'markdown'
          ? { input: markdown, from: 'markdown' }
          : { input: html, from: 'html' };
      },

      clearError() {
        set({ error: null });
      },

      reset() {
        clearScheduled();
        set({ ...initialState, suppressed: { markdown: false, rich: false } });
      },
    };
  });
}

/** The app's store. */
export const documentStore = createDocumentStore();

/** React binding for the app store. */
export function useDocumentStore<T>(selector: (state: DocumentStore) => T): T {
  return useStore(documentStore, selector);
}
