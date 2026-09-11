import { useToastStore } from '../../stores/toastStore';

/**
 * The transient status line: "Pasted rich text", or a copy that failed.
 *
 * plan.md asks for "nothing more intrusive", so this is one small pill that floats over the bottom
 * of the window and fades itself out. It no longer confirms a successful copy — the tick on the
 * button that was clicked does that, and saying it twice is noise.
 */
export function StatusToast(): React.JSX.Element | null {
  const toast = useToastStore((state) => state.toast);
  if (!toast) return null;

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-5 z-20 flex justify-center">
      <div
        key={toast.id}
        data-testid="toast"
        role="status"
        aria-live="polite"
        className={
          'rounded-full border border-line bg-surface px-3 py-1 text-[12px] shadow-[var(--shadow-pop)] ' +
          (toast.tone === 'error' ? 'text-danger' : 'text-muted')
        }
      >
        {toast.message}
      </div>
    </div>
  );
}
