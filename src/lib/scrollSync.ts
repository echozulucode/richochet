import { documentStore } from '../stores/documentStore';
import { getBackend } from './converter';
import { clamp01, pinToEnds, sanitizeOutline, type BlockPosition } from './scrollMapping';
import { updateTestHook } from './testHook';

/**
 * Synchronized scrolling between the two panes.
 *
 * The mapping itself is pure and lives in `scrollMapping.ts`; this module is the wiring — the
 * outline it needs, the DOM events it listens to, and the loop suppression that stops the two
 * panes pushing each other around.
 *
 * ## The loop hazard
 *
 * Scrolling pane A programmatically fires pane B's scroll handler, which would scroll A back,
 * which fires A's handler, and so on. This is the same shape as the two-editor text sync in
 * §3.1 of the implementation plan and gets the same treatment: one pane is the *authority* for a
 * short window and the other's scroll events are ignored as echo.
 *
 * The authority is a pane id plus a deadline, not a boolean. A boolean set before a programmatic
 * scroll and cleared when the echo arrives gets stuck forever the moment an echo does not arrive
 * — an interrupted smooth scroll, a scroll that was already at the clamp, a pane with nothing to
 * scroll — and sync silently dies. A deadline cannot get stuck: the worst case is that the other
 * pane is ignored for one `DRIVER_HOLD_MS` window and then works again.
 */

/** Which pane. Deliberately matches `PaneId` in the document store. */
export type SyncPaneId = 'markdown' | 'rich';

/**
 * One pane, as the controller sees it.
 *
 * Both panes speak block coordinates, so the controller never learns anything about CodeMirror or
 * ProseMirror. The adapters live next to the editors they wrap.
 */
export interface SyncPane {
  /** The element that actually scrolls. Null before the editor has mounted. */
  scroller(): HTMLElement | null;
  /**
   * How many top-level blocks this pane currently renders, or null if it has no independent
   * opinion (the Markdown pane's block count *is* the outline's length, by definition).
   *
   * The block index is only a shared key while the two panes agree on how many blocks there are,
   * and they can stop agreeing — transiently, because a conversion is still in flight, or
   * lastingly, if the AST ever grows a block the rich editor's schema cannot represent as a single
   * node. That has happened before and will happen again: the fix each time is to add the node,
   * but scroll sync must not be the thing that breaks while nobody has noticed yet. So the count
   * is checked rather than assumed, and a mismatch falls back to proportional scrolling — vaguely
   * right beats confidently wrong, and it fails quietly in the app rather than only in a test.
   */
  blockCount(): number | null;
  /** Where the top of this pane's viewport sits, in block coordinates, or null if unmeasurable. */
  positionAtTop(lines: readonly number[]): BlockPosition | null;
  /** Scroll so `position` sits at the top of the viewport. */
  scrollToPosition(position: BlockPosition, lines: readonly number[]): void;
}

/**
 * How long the driving pane keeps the floor after its last scroll event.
 *
 * Long enough to cover the follower's echo (one or two frames) and the gap between the discrete
 * scroll events of a trackpad flick; short enough that grabbing the other pane's scrollbar
 * mid-glide feels immediate.
 */
const DRIVER_HOLD_MS = 220;

/** How long typing must pause before the outline is refetched. Conversion is debounced too. */
const OUTLINE_DEBOUNCE_MS = 150;

/** Real user input on a pane hands it the floor immediately, without waiting for the deadline. */
const GESTURE_EVENTS = ['wheel', 'pointerdown', 'touchstart', 'keydown', 'focusin'] as const;

/** `?sync=off` disables synchronized scrolling entirely. */
export function isScrollSyncEnabled(search: string = globalThis.location?.search ?? ''): boolean {
  return new URLSearchParams(search).get('sync') !== 'off';
}

function otherPane(pane: SyncPaneId): SyncPaneId {
  return pane === 'markdown' ? 'rich' : 'markdown';
}

/** How far a scroller can travel. Zero when its content fits. */
function scrollRange(element: HTMLElement): number {
  return Math.max(0, element.scrollHeight - element.clientHeight);
}

export interface ScrollSync {
  /** Register a pane. Returns a detach function; calling it twice is harmless. */
  attach(id: SyncPaneId, pane: SyncPane): () => void;
  /** The outline currently in use. Exposed for tests. */
  outline(): readonly number[];
  /** Drop every registration and pending work. Unit tests only. */
  reset(): void;
}

export function createScrollSync(enabled: boolean): ScrollSync {
  const panes = new Map<SyncPaneId, SyncPane>();
  const teardown = new Map<SyncPaneId, () => void>();

  /** The outline for the current Markdown, sanitized. Empty means "no anchors, go proportional". */
  let lines: number[] = [];
  /** Rule 3 again, for outlines: a slow response for old text must not overwrite a newer one. */
  let outlineSeq = 0;
  let outlineRevision = 0;
  let outlineFor: string | null = null;
  let outlineTimer: ReturnType<typeof setTimeout> | null = null;

  /** The pane currently allowed to drive, and until when. */
  let driver: SyncPaneId | null = null;
  let driverUntil = 0;
  let frame: number | null = null;
  let framePane: SyncPaneId | null = null;

  let unsubscribe: (() => void) | null = null;

  function now(): number {
    return typeof performance !== 'undefined' ? performance.now() : Date.now();
  }

  /**
   * Try to become the driving pane.
   *
   * Succeeds when nobody holds the floor, when this pane already holds it (which extends the
   * hold), or when the previous holder's window has expired. Returns false for a follower's echo.
   */
  function claimDriver(pane: SyncPaneId): boolean {
    const time = now();
    if (driver !== null && driver !== pane && time < driverUntil) return false;
    driver = pane;
    driverUntil = time + DRIVER_HOLD_MS;
    return true;
  }

  /** A real user gesture on a pane takes the floor outright — no ambiguity about who is driving. */
  function takeDriver(pane: SyncPaneId): void {
    driver = pane;
    driverUntil = now() + DRIVER_HOLD_MS;
  }

  /** Whether a pane's own block count still matches the outline we are mapping through. */
  function agrees(pane: SyncPane): boolean {
    const count = pane.blockCount();
    return count === null || count === lines.length;
  }

  /** Push the driving pane's position onto the follower. Runs at most once per frame. */
  function project(from: SyncPaneId): void {
    const source = panes.get(from);
    const target = panes.get(otherPane(from));
    if (!source || !target) return;

    const sourceElement = source.scroller();
    const targetElement = target.scroller();
    if (!sourceElement || !targetElement) return;
    // Nothing for the follower to do, and writing scrollTop anyway would only generate echo.
    if (scrollRange(targetElement) < 1) return;

    const sourceRange = scrollRange(sourceElement);
    const progress = sourceRange > 0 ? clamp01(sourceElement.scrollTop / sourceRange) : 0;

    if (lines.length >= 2 && agrees(source) && agrees(target)) {
      const position = source.positionAtTop(lines);
      if (position) {
        target.scrollToPosition(position, lines);
        // Anchoring aligns the tops of the viewports, which is wrong at the document ends when the
        // two panes are different heights. Pin the ends so scrolling one to its bottom reaches the
        // other's bottom; see `pinToEnds`.
        const pinned = pinToEnds(targetElement.scrollTop, progress, scrollRange(targetElement));
        if (Math.abs(pinned - targetElement.scrollTop) >= 1) targetElement.scrollTop = pinned;
        return;
      }
    }

    // Degrade gracefully. The backend has no outline for this document (the mock's table is
    // missing an entry, or the command failed), or there is only one block to anchor on and a
    // single anchor says nothing the proportion does not already, or the panes disagree about how
    // many blocks there are. Proportional scrolling drifts on tall blocks, which is the whole
    // reason for the outline — but drifting is better than not moving.
    targetElement.scrollTop = progress * scrollRange(targetElement);
  }

  function requestProjection(pane: SyncPaneId): void {
    framePane = pane;
    if (frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      const pending = framePane;
      framePane = null;
      if (pending) project(pending);
    });
  }

  function onScroll(pane: SyncPaneId): void {
    // A follower's scroll event is the echo of our own write. Dropping it is what breaks the loop.
    if (!claimDriver(pane)) return;
    requestProjection(pane);
  }

  function applyOutline(markdown: string, next: number[]): void {
    lines = next;
    outlineFor = markdown;
    outlineRevision += 1;
    updateTestHook({ outlineLines: next, outlineRevision });
    // A new outline means the anchors moved. Without re-projecting here the panes stay where the
    // last *scroll* left them and only realign when the user scrolls again, which reads as sync
    // having stopped working the moment you start typing.
    reproject();
  }

  /**
   * Push the panes back into alignment after the document changed rather than after a scroll.
   *
   * The editing pane drives: it is the one whose scroll position the user is holding steady while
   * they type, so it is the one the other should follow. If the user happens to be scrolling the
   * *other* pane at that moment they keep the floor — a live gesture always outranks a re-render.
   */
  function reproject(): void {
    const owner = documentStore.getState().owner;
    const pane: SyncPaneId = owner === 'rich' ? 'rich' : 'markdown';
    if (driver !== null && driver !== pane && now() < driverUntil) return;
    if (!panes.has(pane) || !panes.has(otherPane(pane))) return;
    requestProjection(pane);
  }

  function fetchOutline(markdown: string): void {
    if (markdown === outlineFor) return;
    const seq = outlineSeq + 1;
    outlineSeq = seq;
    void getBackend()
      .outline(markdown)
      .then(
        (result) => {
          if (seq !== outlineSeq) return;
          applyOutline(markdown, sanitizeOutline(result));
        },
        () => {
          // A failed outline is not a failed app: fall back to proportional scrolling silently.
          if (seq !== outlineSeq) return;
          applyOutline(markdown, []);
        },
      );
  }

  function scheduleOutline(markdown: string): void {
    if (outlineTimer !== null) clearTimeout(outlineTimer);
    outlineTimer = setTimeout(() => {
      outlineTimer = null;
      fetchOutline(markdown);
    }, OUTLINE_DEBOUNCE_MS);
  }

  /** Follow the canonical Markdown so the outline tracks whatever is on screen. */
  /** Both derived-text counters, so either pane re-rendering triggers a re-projection. */
  function revisionKey(): string {
    const state = documentStore.getState();
    return `${state.markdownRevision}:${state.htmlRevision}`;
  }

  function watchDocument(): void {
    if (unsubscribe) return;
    let previous = documentStore.getState().markdown;
    fetchOutline(previous);
    let revisions = revisionKey();
    unsubscribe = documentStore.subscribe((state) => {
      if (state.markdown !== previous) {
        previous = state.markdown;
        scheduleOutline(state.markdown);
      }
      // The derived pane has just been rewritten, so its content is a different height than the
      // projection was computed against. Re-project on the next frame, once it has laid out.
      const next = revisionKey();
      if (next !== revisions) {
        revisions = next;
        reproject();
      }
    });
  }

  function detach(id: SyncPaneId): void {
    const stop = teardown.get(id);
    if (stop) stop();
    teardown.delete(id);
    panes.delete(id);
    if (driver === id) driver = null;
  }

  return {
    attach(id, pane) {
      if (!enabled) return () => undefined;

      detach(id);
      panes.set(id, pane);

      const element = pane.scroller();
      if (!element) {
        panes.delete(id);
        return () => undefined;
      }

      const scrollListener = () => {
        onScroll(id);
      };
      const gestureListener = () => {
        takeDriver(id);
      };

      element.addEventListener('scroll', scrollListener, { passive: true });
      for (const name of GESTURE_EVENTS) {
        element.addEventListener(name, gestureListener, { passive: true });
      }

      teardown.set(id, () => {
        element.removeEventListener('scroll', scrollListener);
        for (const name of GESTURE_EVENTS) {
          element.removeEventListener(name, gestureListener);
        }
      });

      watchDocument();

      return () => {
        detach(id);
      };
    },

    outline() {
      return lines;
    },

    reset() {
      for (const id of [...panes.keys()]) detach(id);
      if (unsubscribe) unsubscribe();
      unsubscribe = null;
      if (outlineTimer !== null) clearTimeout(outlineTimer);
      outlineTimer = null;
      if (frame !== null) cancelAnimationFrame(frame);
      frame = null;
      framePane = null;
      driver = null;
      driverUntil = 0;
      lines = [];
      outlineFor = null;
    },
  };
}

let instance: ScrollSync | null = null;

/** The app's scroll-sync controller, created on first use so `?sync=off` is read at that point. */
export function getScrollSync(): ScrollSync {
  instance ??= createScrollSync(isScrollSyncEnabled());
  return instance;
}

/** Drop the controller. Unit tests only. */
export function resetScrollSync(): void {
  instance?.reset();
  instance = null;
}
