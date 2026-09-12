import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

import { updateStore } from '../../stores/updateStore';
import type { UpdateState, UpdateStore } from '../../stores/updateStore';
import { TitleBar } from './TitleBar';

describe('TitleBar updates group', () => {
  let initial: UpdateStore;

  beforeEach(() => {
    initial = updateStore.getState();
  });

  afterEach(() => {
    act(() => {
      updateStore.setState(initial, true);
    });
  });

  function renderWith(state: UpdateState, currentVersion: string | null = '0.1.0'): void {
    updateStore.setState({ state, currentVersion });
    render(<TitleBar />);
    fireEvent.click(screen.getByTestId('theme-toggle'));
  }

  const status = () => screen.getByTestId('update-status');

  it.each<UpdateState>([
    { kind: 'idle' },
    { kind: 'checking' },
    { kind: 'up-to-date' },
    { kind: 'error' },
  ])('$kind is the quiet version line: no button, no dot', (state) => {
    renderWith(state);
    expect(screen.getByTestId('updates-group')).toHaveTextContent('Updates');
    expect(status()).toHaveTextContent('Richochet 0.1.0');
    expect(status().tagName).not.toBe('BUTTON');
    expect(screen.queryByTestId('update-dot')).toBeNull();
  });

  it('falls back to the bare name when the version is unreadable', () => {
    renderWith({ kind: 'error' }, null);
    expect(status()).toHaveTextContent(/^Richochet$/);
  });

  it('available is a button that starts the download, and lights the gear', () => {
    const startDownload = vi.fn(() => Promise.resolve());
    updateStore.setState({ startDownload });
    renderWith({ kind: 'available', version: '0.1.1', notes: null });

    expect(screen.getByTestId('update-dot')).toBeInTheDocument();
    expect(status().tagName).toBe('BUTTON');
    expect(status()).toHaveTextContent('0.1.1 available');
    fireEvent.click(status());
    expect(startDownload).toHaveBeenCalledTimes(1);
  });

  it('downloading shows a percentage only when there is one', () => {
    renderWith({ kind: 'downloading', version: '0.1.1', percent: 47 });
    expect(status()).toHaveTextContent('Downloading… 47%');
    expect(status().tagName).not.toBe('BUTTON');

    act(() => {
      updateStore.setState({ state: { kind: 'downloading', version: '0.1.1', percent: null } });
    });
    expect(status()).toHaveTextContent(/^Downloading…$/);
    expect(screen.getByTestId('update-dot')).toBeInTheDocument();
  });

  it('ready-to-install is a button that restarts', () => {
    const restart = vi.fn(() => Promise.resolve());
    updateStore.setState({ restart });
    renderWith({ kind: 'ready-to-install', version: '0.1.1' });

    expect(status()).toHaveTextContent('Restart to update');
    fireEvent.click(status());
    expect(restart).toHaveBeenCalledTimes(1);
  });
});
