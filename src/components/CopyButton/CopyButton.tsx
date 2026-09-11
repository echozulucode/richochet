import { useEffect, useRef, useState } from 'react';

import { copyToClipboard, type CopyTarget } from '../../lib/copy';
import { toastStore } from '../../stores/toastStore';

/** How long the tick stays up before the icon returns to normal. */
const CONFIRM_MS = 1600;

type Phase = 'idle' | 'busy' | 'done';

export interface CopyButtonProps {
  target: CopyTarget;
  /** Tooltip and accessible name. Says what will be copied, since the icon cannot. */
  label: string;
  testId: string;
  /** `copy` for the pane's own content, `text` for the strip-formatting variant. */
  icon?: 'copy' | 'text';
}

/**
 * A quiet icon button that copies the document and briefly confirms with a tick.
 *
 * Deliberately low-key: no fill, no border, muted until hovered. The copy affordance sits in the
 * pane header next to what it copies, so it does not need a label to be understood, and the tick
 * is the whole confirmation — no toast, because the button that changed *is* the feedback and says
 * which action succeeded. Errors still raise a toast, since a silent failure is the one outcome the
 * user must not miss.
 */
export function CopyButton({ target, label, testId, icon = 'copy' }: CopyButtonProps) {
  const [phase, setPhase] = useState<Phase>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  async function run(): Promise<void> {
    if (timer.current) clearTimeout(timer.current);
    setPhase('busy');
    try {
      await copyToClipboard(target);
      setPhase('done');
      timer.current = setTimeout(() => {
        setPhase('idle');
      }, CONFIRM_MS);
    } catch (error) {
      setPhase('idle');
      toastStore.getState().show(error instanceof Error ? error.message : String(error), 'error');
    }
  }

  const done = phase === 'done';

  return (
    <button
      type="button"
      data-testid={testId}
      data-copied={done ? 'true' : 'false'}
      title={done ? 'Copied' : label}
      aria-label={label}
      disabled={phase === 'busy'}
      onClick={() => {
        void run();
      }}
      className={
        'inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md ' +
        'transition-colors duration-150 outline-none ' +
        'hover:bg-line/70 focus-visible:bg-line/70 ' +
        'focus-visible:ring-2 focus-visible:ring-accent/60 ' +
        'disabled:opacity-40 ' +
        (done ? 'text-ink' : 'text-muted hover:text-ink')
      }
    >
      {done ? <CheckIcon /> : icon === 'copy' ? <CopyIcon /> : <TextIcon />}
      <span className="sr-only" aria-live="polite">
        {done ? 'Copied' : ''}
      </span>
    </button>
  );
}

/** Shared geometry so the three icons sit on the same optical baseline. */
const SVG = {
  width: 15,
  height: 15,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.9,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
} as const;

function CopyIcon() {
  return (
    <svg {...SVG} aria-hidden="true">
      <rect x="9" y="9" width="11" height="11" rx="2.5" />
      <path d="M5 15V6.5A2.5 2.5 0 0 1 7.5 4H15" />
    </svg>
  );
}

/** Stacked lines: formatting stripped away, just text. */
function TextIcon() {
  return (
    <svg {...SVG} aria-hidden="true">
      <path d="M4 6.5h16M4 12h16M4 17.5h9" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg {...SVG} aria-hidden="true">
      <path d="M4.5 12.5 9.5 17.5 19.5 6.5" />
    </svg>
  );
}
