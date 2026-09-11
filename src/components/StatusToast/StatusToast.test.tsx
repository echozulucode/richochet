import { act } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

import { toastStore } from '../../stores/toastStore';
import { StatusToast } from './StatusToast';

describe('StatusToast', () => {
  afterEach(() => {
    act(() => {
      toastStore.getState().dismiss();
    });
    vi.useRealTimers();
  });

  it('renders nothing when there is no message', () => {
    render(<StatusToast />);
    expect(screen.queryByTestId('toast')).toBeNull();
  });

  it('shows the message and then removes itself', () => {
    vi.useFakeTimers();
    render(<StatusToast />);

    act(() => {
      toastStore.getState().show('Pasted rich text');
    });
    expect(screen.getByTestId('toast')).toHaveTextContent('Pasted rich text');

    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(screen.queryByTestId('toast')).toBeNull();
  });

  it('tints an error tone differently', () => {
    render(<StatusToast />);
    act(() => {
      toastStore.getState().show('clipboard unavailable', 'error');
    });
    expect(screen.getByTestId('toast').className).toContain('text-danger');
  });
});
