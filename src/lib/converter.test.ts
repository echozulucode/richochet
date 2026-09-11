import { afterEach, describe, expect, it } from 'vitest';

import { createMockBackend, passthrough } from './backends/mock';
import { detectBackendName, parseLatency } from './converter';
import { installTestHook, peekTestHook, resetTestHook } from './testHook';

describe('backend selection', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it('falls back to the mock backend in a plain browser', () => {
    expect(detectBackendName('')).toBe('mock');
  });

  it('uses the Tauri backend when the runtime injected its globals', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(detectBackendName('')).toBe('tauri');
  });

  it('honours the ?backend override in both directions', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(detectBackendName('?backend=mock')).toBe('mock');
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    expect(detectBackendName('?backend=tauri')).toBe('tauri');
  });

  it('ignores a nonsense override', () => {
    expect(detectBackendName('?backend=carrier-pigeon')).toBe('mock');
  });

  it('parses the latency cycle', () => {
    expect(parseLatency('')).toEqual([]);
    expect(parseLatency('?latency=50')).toEqual([50]);
    expect(parseLatency('?latency=200,10')).toEqual([200, 10]);
    expect(parseLatency('?latency=abc')).toEqual([]);
  });
});

describe('mock backend', () => {
  afterEach(() => {
    resetTestHook();
  });

  it('marks passthrough HTML so it cannot be mistaken for engine output', () => {
    const html = passthrough('one\n\ntwo', 'markdown', 'html');
    expect(html).toContain('data-richochet-mock="passthrough"');
    expect(html).toContain('one');
    expect(html).toContain('two');
  });

  it('strips tags when passing HTML through to Markdown', () => {
    expect(passthrough('<p>hello <strong>there</strong></p>', 'html', 'markdown')).toBe(
      'hello there',
    );
  });

  it('falls back to passthrough when the oracle file is absent', async () => {
    const backend = createMockBackend();
    const out = await backend.convert('# Hi', 'markdown', 'html');
    expect(out).toContain('data-richochet-mock="passthrough"');
  });

  it('cycles the latency list so responses can be forced out of order', async () => {
    const backend = createMockBackend({ latency: [40, 0] });
    const finished: string[] = [];
    const slow = backend.convert('first', 'markdown', 'html').then(() => {
      finished.push('first');
    });
    const fast = backend.convert('second', 'markdown', 'html').then(() => {
      finished.push('second');
    });
    await Promise.all([slow, fast]);
    expect(finished).toEqual(['second', 'first']);
  });

  it('records clipboard writes and serves staged clipboard reads', async () => {
    const hook = installTestHook('mock');
    const backend = createMockBackend();

    hook.stageClipboard({ kind: 'rich', html: '<p>from Teams</p>', text: 'from Teams' });
    const read = await backend.readClipboard();
    expect(read).toEqual({
      kind: 'rich',
      html: '<p>from Teams</p>',
      rtf: null,
      text: 'from Teams',
    });
    expect(peekTestHook()?.clipboardReads).toBe(1);

    await backend.writeClipboard({ html: '<p>out</p>', text: 'out' });
    expect(peekTestHook()?.clipboardWrites).toEqual([{ html: '<p>out</p>', text: 'out' }]);

    // The staged payload is consumed once.
    const second = await backend.readClipboard();
    expect(second.kind).toBe('text');
  });

  it('tracks in-flight conversions on the test hook', async () => {
    installTestHook('mock');
    const backend = createMockBackend({ latency: [10] });
    const inFlight = backend.convert('x', 'markdown', 'html');
    expect(peekTestHook()?.pendingConversions).toBe(1);
    await inFlight;
    expect(peekTestHook()?.pendingConversions).toBe(0);
  });
});
