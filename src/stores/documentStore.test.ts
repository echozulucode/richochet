import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StoreApi } from 'zustand/vanilla';

import { installTestHook, resetTestHook } from '../lib/testHook';
import type { ConversionBackend, WireFormat } from '../lib/types';
import { createDocumentStore } from './documentStore';
import type { DocumentStore } from './documentStore';

interface PendingCall {
  input: string;
  from: WireFormat;
  to: WireFormat;
  resolve: (output: string) => void;
  reject: (error: unknown) => void;
}

/** A converter whose responses are resolved by hand, so ordering is fully deterministic. */
function deferredBackend(): { backend: ConversionBackend; calls: PendingCall[] } {
  const calls: PendingCall[] = [];
  const backend: ConversionBackend = {
    name: 'mock',
    convert(input, from, to) {
      return new Promise<string>((resolve, reject) => {
        calls.push({ input, from, to, resolve, reject });
      });
    },
    readClipboard() {
      return Promise.resolve({ kind: 'text', html: null, rtf: null, text: '' });
    },
    writeClipboard() {
      return Promise.resolve();
    },
  };
  return { backend, calls };
}

/** Let queued promise callbacks run. */
const settle = (): Promise<void> => Promise.resolve().then(() => undefined);

describe('documentStore sync engine', () => {
  let calls: PendingCall[];
  let store: StoreApi<DocumentStore>;

  beforeEach(() => {
    const deferred = deferredBackend();
    calls = deferred.calls;
    store = createDocumentStore({
      converter: () => deferred.backend,
      // Immediate scheduler: the debounce itself is covered separately.
      schedule: (run) => {
        run();
        return () => undefined;
      },
    });
  });

  describe('rule 1 - single authority', () => {
    it('converts the focused pane into the other one', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', '# Hi');

      expect(calls).toHaveLength(1);
      expect(calls[0]).toMatchObject({ input: '# Hi', from: 'markdown', to: 'html' });

      calls[0]?.resolve('<h1>Hi</h1>');
      await settle();

      expect(store.getState().html).toBe('<h1>Hi</h1>');
      expect(store.getState().markdown).toBe('# Hi');
    });

    it('ignores edits from the pane that does not own the document', () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('rich', '<p>typed into the derived pane</p>');

      expect(calls).toHaveLength(0);
      expect(store.getState().html).toBe('');
    });

    it('hands authority over on focus, and converts the other way', async () => {
      store.getState().focusPane('rich');
      store.getState().editPane('rich', '<p><strong>Bold</strong></p>');

      expect(store.getState().owner).toBe('rich');
      expect(calls[0]).toMatchObject({ from: 'html', to: 'markdown' });

      calls[0]?.resolve('**Bold**');
      await settle();

      expect(store.getState().markdown).toBe('**Bold**');
      expect(store.getState().markdownRevision).toBe(1);
    });
  });

  describe('rule 2 - suppressed echo', () => {
    it('does not re-convert a derived update that the owning pane echoes back', async () => {
      // The paste path is the case that matters: the rich pane owns the document *and* receives
      // a derived update, so the owner check alone would not save us.
      store.getState().focusPane('rich');
      store.getState().replaceDocument('# Pasted');

      expect(calls).toHaveLength(1);
      calls[0]?.resolve('<h1>Pasted</h1>');
      await settle();

      expect(store.getState().suppressed.rich).toBe(true);

      // TipTap emits onUpdate for the content we just pushed in.
      store.getState().editPane('rich', '<h1>Pasted</h1>');

      expect(calls).toHaveLength(1);
      expect(store.getState().suppressed.rich).toBe(false);
    });

    it('suppresses the echo on the derived Markdown pane too', async () => {
      store.getState().focusPane('rich');
      store.getState().editPane('rich', '<p>one</p>');
      calls[0]?.resolve('one');
      await settle();

      expect(store.getState().suppressed.markdown).toBe(true);

      store.getState().editPane('markdown', 'one');
      expect(calls).toHaveLength(1);
    });

    it('accepts a genuine edit once the echo flag has been consumed', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      calls[0]?.resolve('<p>a</p>');
      await settle();

      // The rich pane echoes, then the user clicks into it and types for real.
      store.getState().editPane('rich', '<p>a</p>');
      store.getState().focusPane('rich');
      store.getState().editPane('rich', '<p>ab</p>');

      expect(calls).toHaveLength(2);
      expect(calls[1]).toMatchObject({ input: '<p>ab</p>', from: 'html', to: 'markdown' });
    });

    it('clears both echo flags when focus moves', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      calls[0]?.resolve('<p>a</p>');
      await settle();

      expect(store.getState().suppressed.rich).toBe(true);
      store.getState().focusPane('rich');
      expect(store.getState().suppressed).toEqual({ markdown: false, rich: false });
    });
  });

  describe('rule 3 - sequence guard', () => {
    it('drops a stale response that lands after a newer one', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      store.getState().editPane('markdown', 'ab');

      expect(calls).toHaveLength(2);
      expect(store.getState().issuedSeq).toBe(2);

      // The second request wins the race...
      calls[1]?.resolve('<p>ab</p>');
      await settle();
      expect(store.getState().html).toBe('<p>ab</p>');
      expect(store.getState().appliedSeq).toBe(2);

      // ...and the first one, arriving late, must be thrown away.
      calls[0]?.resolve('<p>a</p>');
      await settle();

      expect(store.getState().html).toBe('<p>ab</p>');
      expect(store.getState().appliedSeq).toBe(2);
      expect(store.getState().htmlRevision).toBe(1);
    });

    it('applies responses that arrive in order', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      store.getState().editPane('markdown', 'ab');

      calls[0]?.resolve('<p>a</p>');
      await settle();
      expect(store.getState().html).toBe('<p>a</p>');

      calls[1]?.resolve('<p>ab</p>');
      await settle();
      expect(store.getState().html).toBe('<p>ab</p>');
      expect(store.getState().htmlRevision).toBe(2);
    });

    it('does not let a stale success overwrite a newer failure', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      store.getState().editPane('markdown', 'ab');

      calls[1]?.reject(new Error('engine exploded'));
      await settle();
      expect(store.getState().error).toBe('engine exploded');

      calls[0]?.resolve('<p>a</p>');
      await settle();
      expect(store.getState().html).toBe('');
      expect(store.getState().error).toBe('engine exploded');
    });

    it('clears the converting flag only when nothing is outstanding', async () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'a');
      store.getState().editPane('markdown', 'ab');

      calls[0]?.resolve('<p>a</p>');
      await settle();
      expect(store.getState().converting).toBe(true);

      calls[1]?.resolve('<p>ab</p>');
      await settle();
      expect(store.getState().converting).toBe(false);
    });
  });

  describe('paste and canonical text', () => {
    it('replaces the document and derives the rich pane whoever has focus', async () => {
      store.getState().focusPane('rich');
      store.getState().replaceDocument('- one\n- two');

      expect(store.getState().markdown).toBe('- one\n- two');
      expect(store.getState().markdownRevision).toBe(1);
      expect(calls[0]).toMatchObject({ from: 'markdown', to: 'html' });

      calls[0]?.resolve('<ul><li>one</li><li>two</li></ul>');
      await settle();
      expect(store.getState().html).toBe('<ul><li>one</li><li>two</li></ul>');
    });

    it('reports the owning pane as the conversion source', () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', '# Hi');
      expect(store.getState().canonical()).toEqual({ input: '# Hi', from: 'markdown' });

      store.getState().focusPane('rich');
      store.getState().editPane('rich', '<h1>Hi</h1>');
      expect(store.getState().canonical()).toEqual({ input: '<h1>Hi</h1>', from: 'html' });
    });

    it('ignores an edit that does not change the text', () => {
      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'same');
      store.getState().editPane('markdown', 'same');
      expect(calls).toHaveLength(1);
    });
  });
});

describe('documentStore debounce', () => {
  it('coalesces a burst of typing into one conversion', () => {
    vi.useFakeTimers();
    try {
      const { backend, calls } = deferredBackend();
      const store = createDocumentStore({ converter: () => backend, debounceMs: 120 });

      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'h');
      vi.advanceTimersByTime(40);
      store.getState().editPane('markdown', 'he');
      vi.advanceTimersByTime(40);
      store.getState().editPane('markdown', 'hel');

      expect(calls).toHaveLength(0);
      vi.advanceTimersByTime(120);

      expect(calls).toHaveLength(1);
      expect(calls[0]?.input).toBe('hel');
    } finally {
      vi.useRealTimers();
    }
  });

  it('counts a debounced conversion as in flight from the first keystroke', async () => {
    vi.useFakeTimers();
    const hook = installTestHook('mock');
    try {
      const { backend, calls } = deferredBackend();
      const store = createDocumentStore({ converter: () => backend, debounceMs: 120 });

      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'x');
      // Still inside the debounce window: a test waiting for "settled" must not slip through.
      expect(hook.pendingConversions).toBeGreaterThan(0);

      vi.advanceTimersByTime(120);
      expect(hook.pendingConversions).toBeGreaterThan(0);

      calls[0]?.resolve('<p>x</p>');
      await vi.runAllTimersAsync();
      expect(hook.pendingConversions).toBe(0);
    } finally {
      resetTestHook();
      vi.useRealTimers();
    }
  });

  it('cancels a pending conversion when the document is replaced', () => {
    vi.useFakeTimers();
    try {
      const { backend, calls } = deferredBackend();
      const store = createDocumentStore({ converter: () => backend, debounceMs: 120 });

      store.getState().focusPane('markdown');
      store.getState().editPane('markdown', 'typing');
      store.getState().replaceDocument('# Pasted');
      vi.advanceTimersByTime(500);

      expect(calls).toHaveLength(1);
      expect(calls[0]?.input).toBe('# Pasted');
    } finally {
      vi.useRealTimers();
    }
  });
});
