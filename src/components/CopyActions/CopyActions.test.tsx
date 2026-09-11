import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { setBackend } from '../../lib/converter';
import type { ConversionBackend, OutboundPayload } from '../../lib/types';
import { documentStore } from '../../stores/documentStore';
import { toastStore } from '../../stores/toastStore';
import { CopyActions } from './CopyActions';

function stubBackend(): { backend: ConversionBackend; writes: OutboundPayload[] } {
  const writes: OutboundPayload[] = [];
  const backend: ConversionBackend = {
    name: 'mock',
    convert: vi.fn((input: string, _from, to) => Promise.resolve(`${to}:${input}`)),
    readClipboard: () => Promise.resolve({ kind: 'text', html: null, rtf: null, text: '' }),
    writeClipboard: (payload: OutboundPayload) => {
      writes.push(payload);
      return Promise.resolve();
    },
  };
  return { backend, writes };
}

describe('CopyActions', () => {
  afterEach(() => {
    setBackend(null);
    documentStore.getState().reset();
    toastStore.getState().dismiss();
  });

  it('writes HTML and a plain-text fallback together for Teams', async () => {
    const { backend, writes } = stubBackend();
    setBackend(backend);
    documentStore.setState({ markdown: '# Hi', owner: 'markdown' });

    render(<CopyActions />);
    await userEvent.click(screen.getByTestId('copy-teams'));

    await waitFor(() => {
      expect(writes).toEqual([{ html: 'html:# Hi', text: 'text:# Hi' }]);
    });
    expect(toastStore.getState().toast?.message).toBe('Copied for Teams');
  });

  it('writes text only for Copy Markdown and Copy Plain Text', async () => {
    const { backend, writes } = stubBackend();
    setBackend(backend);
    documentStore.setState({ markdown: 'body', owner: 'markdown' });

    render(<CopyActions />);
    await userEvent.click(screen.getByTestId('copy-markdown'));
    await waitFor(() => {
      expect(writes).toHaveLength(1);
    });
    await userEvent.click(screen.getByTestId('copy-text'));
    await waitFor(() => {
      expect(writes).toHaveLength(2);
    });

    expect(writes[0]).toEqual({ html: null, text: 'markdown:body' });
    expect(writes[1]).toEqual({ html: null, text: 'text:body' });
  });

  it('converts from the rich pane when it owns the document', async () => {
    const { backend, writes } = stubBackend();
    setBackend(backend);
    documentStore.setState({ html: '<p>x</p>', owner: 'rich' });

    render(<CopyActions />);
    await userEvent.click(screen.getByTestId('copy-markdown'));

    await waitFor(() => {
      expect(writes[0]).toEqual({ html: null, text: 'markdown:<p>x</p>' });
    });
    expect(backend.convert).toHaveBeenCalledWith('<p>x</p>', 'html', 'markdown');
  });

  it('surfaces a conversion failure as an error toast', async () => {
    const backend: ConversionBackend = {
      name: 'mock',
      convert: () => Promise.reject(new Error('no engine')),
      readClipboard: () => Promise.resolve({ kind: 'text', html: null, rtf: null, text: '' }),
      writeClipboard: () => Promise.resolve(),
    };
    setBackend(backend);

    render(<CopyActions />);
    await userEvent.click(screen.getByTestId('copy-markdown'));

    await waitFor(() => {
      expect(toastStore.getState().toast).toMatchObject({ message: 'no engine', tone: 'error' });
    });
  });
});
