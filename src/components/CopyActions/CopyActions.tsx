import { useState } from 'react';

import { getBackend } from '../../lib/converter';
import { documentStore } from '../../stores/documentStore';
import { toastStore } from '../../stores/toastStore';

type ActionId = 'teams' | 'markdown' | 'text';

const LABELS: Record<ActionId, string> = {
  teams: 'Copy for Teams',
  markdown: 'Copy Markdown',
  text: 'Copy Plain Text',
};

const TEST_IDS: Record<ActionId, string> = {
  teams: 'copy-teams',
  markdown: 'copy-markdown',
  text: 'copy-text',
};

const DONE: Record<ActionId, string> = {
  teams: 'Copied for Teams',
  markdown: 'Copied Markdown',
  text: 'Copied plain text',
};

/**
 * The three copy actions.
 *
 * Everything is rendered from the owning pane's text through the engine, never from whatever the
 * editor happens to be showing. "Copy for Teams" is the only one that puts HTML on the clipboard,
 * and it always writes a plain-text fallback alongside it - Teams picks the rich flavour, anything
 * else gets readable text.
 */
export function CopyActions(): React.JSX.Element {
  const [busy, setBusy] = useState<ActionId | null>(null);

  async function run(action: ActionId): Promise<void> {
    setBusy(action);
    const backend = getBackend();
    const { input, from } = documentStore.getState().canonical();
    try {
      if (action === 'teams') {
        const [html, text] = await Promise.all([
          backend.convert(input, from, 'html'),
          backend.convert(input, from, 'text'),
        ]);
        await backend.writeClipboard({ html, text });
      } else {
        const target = action === 'markdown' ? 'markdown' : 'text';
        const text = await backend.convert(input, from, target);
        await backend.writeClipboard({ html: null, text });
      }
      toastStore.getState().show(DONE[action]);
    } catch (error) {
      toastStore.getState().show(error instanceof Error ? error.message : String(error), 'error');
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex flex-wrap items-center justify-center gap-x-2 gap-y-1">
      {(['teams', 'markdown', 'text'] as const).map((action) => (
        <button
          key={action}
          type="button"
          data-testid={TEST_IDS[action]}
          disabled={busy !== null}
          onClick={() => {
            void run(action);
          }}
          className={
            'rounded-lg px-4 py-1.5 text-[13px] font-medium transition-colors duration-150 ' +
            'disabled:opacity-50 ' +
            (action === 'teams'
              ? 'bg-accent text-accent-ink hover:brightness-110'
              : 'text-ink hover:bg-line/70')
          }
        >
          {LABELS[action]}
        </button>
      ))}
    </div>
  );
}
