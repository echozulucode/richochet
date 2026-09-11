import { afterEach, describe, expect, it, vi } from 'vitest';

import { setBackend } from '../../lib/converter';
import type { ClipboardPayload, ConversionBackend } from '../../lib/types';
import { documentStore } from '../../stores/documentStore';
import { toastStore } from '../../stores/toastStore';
import { pasteFromClipboard } from './paste';

function backendFor(payload: ClipboardPayload): ConversionBackend {
  return {
    name: 'mock',
    convert: vi.fn((input: string, from) => Promise.resolve(`${from}->md:${input}`)),
    outline: () => Promise.resolve([]),
    readClipboard: () => Promise.resolve(payload),
    writeClipboard: () => Promise.resolve(),
  };
}

describe('pasteFromClipboard', () => {
  afterEach(() => {
    setBackend(null);
    documentStore.getState().reset();
    toastStore.getState().dismiss();
  });

  it('converts HTML from the clipboard and reports rich text', async () => {
    const backend = backendFor({
      kind: 'rich',
      html: '<p><b>Hi</b></p>',
      rtf: null,
      text: 'Hi',
    });
    setBackend(backend);

    await pasteFromClipboard();

    expect(backend.convert).toHaveBeenCalledWith('<p><b>Hi</b></p>', 'html', 'markdown');
    expect(documentStore.getState().markdown).toBe('html->md:<p><b>Hi</b></p>');
    expect(toastStore.getState().toast?.message).toBe('Pasted rich text');
  });

  it('falls back to plain text when no HTML flavour was offered', async () => {
    const backend = backendFor({ kind: 'text', html: null, rtf: null, text: 'just words' });
    setBackend(backend);

    await pasteFromClipboard();

    expect(backend.convert).toHaveBeenCalledWith('just words', 'text', 'markdown');
    expect(toastStore.getState().toast?.message).toBe('Pasted plain text');
  });

  it('does nothing when the clipboard is empty', async () => {
    const backend = backendFor({ kind: 'text', html: null, rtf: null, text: '' });
    setBackend(backend);

    await pasteFromClipboard();

    expect(backend.convert).not.toHaveBeenCalled();
    expect(toastStore.getState().toast).toBeNull();
  });

  it('reports a clipboard failure as an error toast', async () => {
    setBackend({
      name: 'mock',
      convert: () => Promise.resolve(''),
      outline: () => Promise.resolve([]),
      readClipboard: () => Promise.reject(new Error('clipboard busy')),
      writeClipboard: () => Promise.resolve(),
    });

    await pasteFromClipboard();

    expect(toastStore.getState().toast).toMatchObject({
      message: 'clipboard busy',
      tone: 'error',
    });
  });
});
