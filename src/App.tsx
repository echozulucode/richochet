import { useEffect, useMemo, useState } from 'react';

import { CopyButton } from './components/CopyButton/CopyButton';
import { MarkdownEditor } from './components/MarkdownEditor/MarkdownEditor';
import { RichEditor } from './components/RichEditor/RichEditor';
import { SplitPane } from './components/SplitPane/SplitPane';
import { StatusToast } from './components/StatusToast/StatusToast';
import { TitleBar } from './components/TitleBar/TitleBar';
import { getBackend } from './lib/converter';
import { useMediaQuery } from './lib/useMediaQuery';
import { updateStore } from './stores/updateStore';

/** Below this width there is not enough room for two panes side by side. */
const NARROW_QUERY = '(max-width: 699px)';

type PaneKey = 'formatted' | 'markdown';

interface PaneProps {
  label: string;
  testId: string;
  /** Copy affordances, shown at the right of the pane header. */
  actions: React.ReactNode;
  children: React.ReactNode;
}

function Pane({ label, testId, actions, children }: PaneProps): React.JSX.Element {
  return (
    <section data-testid={testId} className="flex h-full min-h-0 flex-col bg-surface">
      {/* The header carries the copy buttons: an action belongs next to the thing it acts on. */}
      <header className="flex shrink-0 items-center justify-between gap-2 pt-2 pr-2 pb-1 pl-6">
        <h2 className="text-[11px] font-medium tracking-[0.08em] text-muted uppercase select-none">
          {label}
        </h2>
        <div className="flex items-center gap-0.5">{actions}</div>
      </header>
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

/** One screen: a title row and two panes, each with its own copy affordance. */
export function App(): React.JSX.Element {
  const backend = useMemo(() => getBackend().name, []);
  const narrow = useMediaQuery(NARROW_QUERY);
  const [visible, setVisible] = useState<PaneKey>('formatted');

  // The launch update check. After paint, never awaited, guarded inside the store so StrictMode's
  // double mount and any later re-render still only produce one request; a failure - no network,
  // no Tauri at all under ?backend=mock - resolves quietly rather than throwing into render.
  useEffect(() => {
    void updateStore.getState().checkOnLaunch();
  }, []);

  const formatted = (
    <Pane
      label="Formatted"
      testId="pane-formatted"
      actions={
        <>
          <CopyButton target="teams" label="Copy for Teams" testId="copy-teams" />
          <CopyButton target="text" label="Copy plain text" testId="copy-text" icon="text" />
        </>
      }
    >
      <RichEditor />
    </Pane>
  );
  const markdown = (
    <Pane
      label="Markdown"
      testId="pane-markdown"
      actions={<CopyButton target="markdown" label="Copy Markdown" testId="copy-markdown" />}
    >
      <MarkdownEditor />
    </Pane>
  );

  return (
    <div
      data-testid="app-root"
      data-backend={backend}
      className="relative flex h-full flex-col bg-app font-sans text-ink"
    >
      <TitleBar />

      {narrow ? <PaneSwitch value={visible} onChange={setVisible} /> : null}

      <main className="flex min-h-0 flex-1 px-4 pb-4">
        <div className="flex min-h-0 flex-1 overflow-hidden rounded-xl border border-line bg-surface shadow-[var(--shadow-pane)]">
          {narrow ? (
            <div className="min-h-0 flex-1">{visible === 'formatted' ? formatted : markdown}</div>
          ) : (
            <SplitPane left={formatted} right={markdown} />
          )}
        </div>
      </main>

      <StatusToast />
    </div>
  );
}
