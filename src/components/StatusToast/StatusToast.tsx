import { useToastStore } from '../../stores/toastStore';

/**
 * The transient status line: "Pasted rich text", "Copied for Teams".
 *
 * plan.md asks for "nothing more intrusive", so this is one small pill, centred above the action
 * bar, that fades itself out.
 */
export function StatusToast(): React.JSX.Element | null {
  const toast = useToastStore((state) => state.toast);
  if (!toast) return null;

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-full flex justify-center pb-3">
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
