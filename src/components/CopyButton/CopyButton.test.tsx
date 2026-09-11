import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { setBackend } from '../../lib/converter';
import type { ConversionBackend, OutboundPayload } from '../../lib/types';
import { documentStore } from '../../stores/documentStore';
import { toastStore } from '../../stores/toastStore';
import { CopyButton } from './CopyButton';

function stubBackend(): { backend: ConversionBackend; writes: OutboundPayload[] } {
  const writes: OutboundPayload[] = [];
  const backend: ConversionBackend = {
    name: 'mock',
    convert: vi.fn((input: string, _from, to) => Promise.resolve(`${to}:${input}`)),
    outline: () => Promise.resolve([]),
    readClipboard: () => Promise.resolve({ kind: 'text', html: null, rtf: null, text: '' }),
    writeClipboard: (payload: OutboundPayload) => {
      writes.push(payload);
      return Promise.resolve();
    },
  };
  return { backend, writes };
}

describe('CopyButton', () => {
  afterEach(() => {
    setBackend(null);
    documentStore.getState().reset();
    toastStore.getState().dismiss();
    vi.useRealTimers();
  });

  it('writes HTML and a plain-text fallback together for Teams', async () => {
    const { backend, writes } = stubBackend();
    setBackend(backend);
    documentStore.setState({ markdown: '# Hi', owner: 'markdown' });

    render(<CopyButton target="teams" label="Copy for Teams" testId="copy-teams" />);
    await userEvent.click(screen.getByTestId('copy-teams'));

    await waitFor(() => {
      expect(writes).toEqual([{ html: 'html:# Hi', text: 'text:# Hi' }]);
    });
  });

  it('writes text only for Markdown and plain text', async () => {
    const { backend, writes } = stubBackend();
    setBackend(backend);
    documentStore.setState({ markdown: 'body', owner: 'markdown' });

    const { unmount } = render(
      <CopyButton target="markdown" label="Copy Markdown" testId="copy-markdown" />,
    );
    await userEvent.click(screen.getByTestId('copy-markdown'));
    await waitFor(() => {
      expect(writes).toHaveLength(1);
    });
    unmount();

    render(<CopyButton target="text" label="Copy plain text" testId="copy-text" icon="text" />);
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

    render(<CopyButton target="markdown" label="Copy Markdown" testId="copy-markdown" />);
    await userEvent.click(screen.getByTestId('copy-markdown'));

    await waitFor(() => {
      expect(writes[0]).toEqual({ html: null, text: 'markdown:<p>x</p>' });
    });
    expect(backend.convert).toHaveBeenCalledWith('<p>x</p>', 'html', 'markdown');
  });

  it('confirms with a tick and then returns to normal', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { backend } = stubBackend();
    setBackend(backend);
    documentStore.setState({ markdown: 'x', owner: 'markdown' });

    render(<CopyButton target="markdown" label="Copy Markdown" testId="copy-markdown" />);
    const button = screen.getByTestId('copy-markdown');
    expect(button).toHaveAttribute('data-copied', 'false');

    await userEvent.click(button);
    await waitFor(() => {
      expect(button).toHaveAttribute('data-copied', 'true');
    });
    // The tick is the confirmation, so there must not also be a toast saying the same thing.
    expect(toastStore.getState().toast).toBeNull();

    await vi.advanceTimersByTimeAsync(2000);
    await waitFor(() => {
      expect(button).toHaveAttribute('data-copied', 'false');
    });
  });

  it('surfaces a failure as an error toast and does not show a tick', async () => {
    const backend: ConversionBackend = {
      name: 'mock',
      convert: () => Promise.reject(new Error('engine exploded')),
      outline: () => Promise.resolve([]),
      readClipboard: () => Promise.resolve({ kind: 'text', html: null, rtf: null, text: '' }),
      writeClipboard: () => Promise.resolve(),
    };
    setBackend(backend);
    documentStore.setState({ markdown: 'x', owner: 'markdown' });

    render(<CopyButton target="markdown" label="Copy Markdown" testId="copy-markdown" />);
    await userEvent.click(screen.getByTestId('copy-markdown'));

    await waitFor(() => {
      expect(toastStore.getState().toast?.message).toBe('engine exploded');
    });
    expect(toastStore.getState().toast?.tone).toBe('error');
    expect(screen.getByTestId('copy-markdown')).toHaveAttribute('data-copied', 'false');
  });

  it('names the action for screen readers, since the icon cannot', () => {
    render(<CopyButton target="teams" label="Copy for Teams" testId="copy-teams" />);
    expect(screen.getByRole('button', { name: 'Copy for Teams' })).toBeInTheDocument();
  });
});
