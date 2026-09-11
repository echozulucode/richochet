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

  it('takes plain text as Markdown source, without converting it', async () => {
    const backend = backendFor({ kind: 'text', html: null, rtf: null, text: '**bold** text' });
    setBackend(backend);

    await pasteFromClipboard();

    // Converting `text -> markdown` would escape it to `\*\*bold\*\* text`, which renders as
    // literal asterisks. Plain text on the clipboard is Markdown here, so it goes in untouched.
    // (`markdown -> html` is still called afterwards - that is the store rendering the preview.)
    expect(backend.convert).not.toHaveBeenCalledWith('**bold** text', 'text', 'markdown');
    expect(documentStore.getState().markdown).toBe('**bold** text');
    expect(toastStore.getState().toast?.message).toBe('Pasted Markdown');
  });

  it('does nothing when the clipboard is empty', async () => {
    const backend = backendFor({ kind: 'text', html: null, rtf: null, text: '' });
    setBackend(backend);

    await pasteFromClipboard();

    // (`markdown -> html` is still called afterwards - that is the store rendering the preview.)
    expect(backend.convert).not.toHaveBeenCalledWith('**bold** text', 'text', 'markdown');
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
