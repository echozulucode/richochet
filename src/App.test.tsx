import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import { App } from './App';

describe('App', () => {
  it('mounts the whole screen: title row, both panes, both editors, the copy buttons', () => {
    render(<App />);

    const root = screen.getByTestId('app-root');
    expect(root).toHaveAttribute('data-backend', 'mock');

    expect(screen.getByTestId('pane-formatted')).toBeInTheDocument();
    expect(screen.getByTestId('pane-markdown')).toBeInTheDocument();
    expect(screen.getByTestId('editor-rich')).toHaveAttribute('contenteditable', 'true');
    expect(screen.getByTestId('editor-markdown')).toBeInTheDocument();
    expect(screen.getByTestId('divider')).toBeInTheDocument();

    for (const id of ['copy-teams', 'copy-markdown', 'copy-text', 'theme-toggle']) {
      expect(screen.getByTestId(id)).toBeInTheDocument();
    }

    expect(screen.getByText('Richochet')).toBeInTheDocument();
    expect(screen.getByText('Formatted')).toBeInTheDocument();
    expect(screen.getByText('Markdown')).toBeInTheDocument();
  });
});
