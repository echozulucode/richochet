import { useMemo, useState } from 'react';

import { CopyActions } from './components/CopyActions/CopyActions';
import { MarkdownEditor } from './components/MarkdownEditor/MarkdownEditor';
import { RichEditor } from './components/RichEditor/RichEditor';
import { SplitPane } from './components/SplitPane/SplitPane';
import { StatusToast } from './components/StatusToast/StatusToast';
import { TitleBar } from './components/TitleBar/TitleBar';
import { getBackend } from './lib/converter';
import { useMediaQuery } from './lib/useMediaQuery';

/** Below this width there is not enough room for two panes side by side. */
const NARROW_QUERY = '(max-width: 699px)';

type PaneKey = 'formatted' | 'markdown';

interface PaneProps {
  label: string;
  testId: string;
  children: React.ReactNode;
}

function Pane({ label, testId, children }: PaneProps): React.JSX.Element {
  return (
    <section data-testid={testId} className="flex h-full min-h-0 flex-col bg-surface">
      <h2 className="shrink-0 px-6 pt-3 text-[11px] font-medium tracking-[0.08em] text-muted uppercase select-none">
        {label}
      </h2>
      <div className="min-h-0 flex-1">{children}</div>
    </section>
  );
}

interface PaneSwitchProps {
  value: PaneKey;
  onChange: (value: PaneKey) => void;
}

/** The single-pane-mode switch. Only rendered when the window is too narrow for both. */
function PaneSwitch({ value, onChange }: PaneSwitchProps): React.JSX.Element {
  return (
    <div className="flex shrink-0 justify-center gap-1 px-4 pb-2">
      {(['formatted', 'markdown'] as const).map((key) => (
        <button
          key={key}
          type="button"
          data-testid={`pane-toggle-${key}`}
          aria-pressed={value === key}
          onClick={() => {
            onChange(key);
          }}
          className={
            'rounded-md px-3 py-1 text-[12px] font-medium transition-colors duration-150 ' +
            (value === key ? 'bg-surface text-ink shadow-[var(--shadow-pane)]' : 'text-muted')
          }
        >
          {key === 'formatted' ? 'Formatted' : 'Markdown'}
        </button>
      ))}
    </div>
  );
}

/** One screen: a title row, two panes, one action bar. */
export function App(): React.JSX.Element {
  const backend = useMemo(() => getBackend().name, []);
  const narrow = useMediaQuery(NARROW_QUERY);
  const [visible, setVisible] = useState<PaneKey>('formatted');

  const formatted = (
    <Pane label="Formatted" testId="pane-formatted">
      <RichEditor />
    </Pane>
  );
  const markdown = (
    <Pane label="Markdown" testId="pane-markdown">
      <MarkdownEditor />
    </Pane>
  );

  return (
    <div
      data-testid="app-root"
      data-backend={backend}
      className="flex h-full flex-col bg-app font-sans text-ink"
    >
      <TitleBar />

      {narrow ? <PaneSwitch value={visible} onChange={setVisible} /> : null}

      <main className="flex min-h-0 flex-1 px-4">
        <div className="flex min-h-0 flex-1 overflow-hidden rounded-xl border border-line bg-surface shadow-[var(--shadow-pane)]">
          {narrow ? (
            <div className="min-h-0 flex-1">{visible === 'formatted' ? formatted : markdown}</div>
          ) : (
            <SplitPane left={formatted} right={markdown} />
          )}
        </div>
      </main>

      <footer className="relative shrink-0 px-4 py-3">
        <StatusToast />
        <CopyActions />
      </footer>
    </div>
  );
}
