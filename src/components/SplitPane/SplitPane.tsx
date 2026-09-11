import { useCallback, useRef, useState } from 'react';

const MIN_RATIO = 0.2;
const MAX_RATIO = 0.8;
const KEY_STEP = 0.02;

export interface SplitPaneProps {
  left: React.ReactNode;
  right: React.ReactNode;
  /** Starting split, 0-1. Defaults to an even one. */
  initialRatio?: number;
}

function clamp(value: number): number {
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, value));
}

/**
 * Two panes of equal width with a draggable divider between them.
 *
 * The divider is a real focusable separator: arrow keys nudge it, which keeps the layout usable
 * without a pointer and costs nothing.
 */
export function SplitPane({ left, right, initialRatio = 0.5 }: SplitPaneProps): React.JSX.Element {
  const container = useRef<HTMLDivElement | null>(null);
  const [ratio, setRatio] = useState(() => clamp(initialRatio));

  const moveTo = useCallback((clientX: number) => {
    const box = container.current?.getBoundingClientRect();
    if (!box || box.width === 0) return;
    setRatio(clamp((clientX - box.left) / box.width));
  }, []);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      const handle = event.currentTarget;
      handle.setPointerCapture(event.pointerId);

      const onMove = (moveEvent: PointerEvent) => {
        moveTo(moveEvent.clientX);
      };
      const onUp = () => {
        handle.releasePointerCapture(event.pointerId);
        handle.removeEventListener('pointermove', onMove);
        handle.removeEventListener('pointerup', onUp);
        handle.removeEventListener('pointercancel', onUp);
      };

      handle.addEventListener('pointermove', onMove);
      handle.addEventListener('pointerup', onUp);
      handle.addEventListener('pointercancel', onUp);
    },
    [moveTo],
  );

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      setRatio((current) => clamp(current - KEY_STEP));
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      setRatio((current) => clamp(current + KEY_STEP));
    } else if (event.key === 'Home') {
      event.preventDefault();
      setRatio(0.5);
    }
  }, []);

  return (
    <div ref={container} className="flex min-h-0 flex-1 items-stretch">
      <div className="min-w-0" style={{ flex: `0 0 ${(ratio * 100).toFixed(3)}%` }}>
        {left}
      </div>
      <div
        data-testid="divider"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize panes"
        aria-valuenow={Math.round(ratio * 100)}
        aria-valuemin={Math.round(MIN_RATIO * 100)}
        aria-valuemax={Math.round(MAX_RATIO * 100)}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        onDoubleClick={() => {
          setRatio(0.5);
        }}
        className="group relative w-px shrink-0 cursor-col-resize bg-line outline-none"
      >
        <span className="absolute inset-y-0 -left-2 -right-2 block group-focus-visible:bg-accent/20" />
      </div>
      <div className="min-w-0 flex-1">{right}</div>
    </div>
  );
}
