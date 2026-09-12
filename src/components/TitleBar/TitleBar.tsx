import { useEffect, useRef, useState } from 'react';

import { useThemeStore } from '../../stores/themeStore';
import type { ThemePreference } from '../../stores/themeStore';
import { hasPendingUpdate, updateStore, useUpdateStore } from '../../stores/updateStore';

const OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string }> = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

/**
 * The Updates group.
 *
 * Four of the seven states - including `error` - are the same quiet line: the product name and,
 * when we can read it, the running version. An update check that failed looks exactly like one
 * that found nothing, which is the point: offline is not a condition the user has to act on.
 */
function UpdatesGroup(): React.JSX.Element {
  const state = useUpdateStore((store) => store.state);
  const currentVersion = useUpdateStore((store) => store.currentVersion);

  const line = (children: React.ReactNode) => (
    <p
      data-testid="update-status"
      data-update-state={state.kind}
      className="px-2 py-1 text-[13px] text-muted select-none"
    >
      {children}
    </p>
  );

  const action = (label: string, onClick: () => void) => (
    <button
      type="button"
      role="menuitem"
      data-testid="update-status"
      data-update-state={state.kind}
      onClick={onClick}
      className="flex w-full items-center gap-1 rounded-md px-2 py-1 text-left text-[13px] text-ink hover:bg-line/70"
    >
      <span aria-hidden="true" className="text-accent">
        &#9656;
      </span>
      {label}
    </button>
  );

  switch (state.kind) {
    case 'available':
      return action(`${state.version} available`, () => {
        void updateStore.getState().startDownload();
      });
    case 'downloading':
      return line(state.percent === null ? 'Downloading…' : `Downloading… ${state.percent}%`);
    case 'ready-to-install':
      return action('Restart to update', () => {
        void updateStore.getState().restart();
      });
    default:
      return line(currentVersion === null ? 'Richochet' : `Richochet ${currentVersion}`);
  }
}

/** The slim title row: the product name, and one quiet settings affordance. */
export function TitleBar(): React.JSX.Element {
  const preference = useThemeStore((state) => state.preference);
  const setPreference = useThemeStore((state) => state.setPreference);
  const pending = useUpdateStore((store) => hasPendingUpdate(store.state));
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDocumentPointerDown = (event: PointerEvent) => {
      if (!container.current?.contains(event.target as globalThis.Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    document.addEventListener('pointerdown', onDocumentPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDocumentPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <header className="flex h-11 shrink-0 items-center justify-between px-4">
      <span className="text-[13px] font-semibold tracking-[-0.01em] text-ink select-none">
        Richochet
      </span>

      <div ref={container} className="relative">
        <button
          type="button"
          data-testid="theme-toggle"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={pending ? 'Settings — update available' : 'Settings'}
          title={pending ? 'Settings — update available' : 'Settings'}
          onClick={() => {
            setOpen((value) => !value);
          }}
          className="relative flex size-7 items-center justify-center rounded-md text-muted transition-colors duration-150 hover:bg-line/70 hover:text-ink"
        >
          {/* The whole update feature's claim on the user's attention: one 6px dot. */}
          {pending ? (
            <span
              aria-hidden="true"
              data-testid="update-dot"
              className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-accent"
            />
          ) : null}
          <svg viewBox="0 0 16 16" aria-hidden="true" className="size-4">
            <path
              fill="currentColor"
              d="M8 5.6a2.4 2.4 0 1 0 0 4.8 2.4 2.4 0 0 0 0-4.8Zm0 1.2a1.2 1.2 0 1 1 0 2.4 1.2 1.2 0 0 1 0-2.4Z"
            />
            <path
              fill="currentColor"
              d="m13.6 8 .9-1.1-.9-2.2-1.4.2-1-.6-.5-1.3H8.3l-.5 1.3-1 .6-1.4-.2-.9 2.2.9 1.1-.9 1.1.9 2.2 1.4-.2 1 .6.5 1.3h2.4l.5-1.3 1-.6 1.4.2.9-2.2L13.6 8Zm-1.3 1.4.6.7-.3.7-.9-.1-2 1.2-.3.9h-.8l-.3-.9-2-1.2-.9.1-.3-.7.6-.7v-2.8l-.6-.7.3-.7.9.1 2-1.2.3-.9h.8l.3.9 2 1.2.9-.1.3.7-.6.7v2.8Z"
              opacity="0.55"
            />
          </svg>
        </button>

        {open ? (
          <div
            role="menu"
            data-testid="settings-menu"
            className="absolute right-0 z-10 mt-1 w-44 rounded-lg border border-line bg-surface p-1 shadow-[var(--shadow-pop)]"
          >
            <p className="px-2 py-1 text-[11px] tracking-wide text-muted uppercase">Appearance</p>
            {OPTIONS.map((option) => (
              <button
                key={option.value}
                type="button"
                role="menuitemradio"
                aria-checked={preference === option.value}
                data-testid={`theme-${option.value}`}
                onClick={() => {
                  setPreference(option.value);
                  setOpen(false);
                }}
                className="flex w-full items-center justify-between rounded-md px-2 py-1 text-left text-[13px] text-ink hover:bg-line/70"
              >
                {option.label}
                {preference === option.value ? (
                  <span aria-hidden="true" className="text-accent">
                    &#10003;
                  </span>
                ) : null}
              </button>
            ))}

            <hr className="my-1 border-0 border-t border-line" />

            <div data-testid="updates-group">
              <p className="px-2 py-1 text-[11px] tracking-wide text-muted uppercase">Updates</p>
              <UpdatesGroup />
            </div>
          </div>
        ) : null}
      </div>
    </header>
  );
}
