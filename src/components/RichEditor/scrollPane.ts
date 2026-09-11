import type { Editor } from '@tiptap/react';

import { clamp01 } from '../../lib/scrollMapping';
import type { SyncPane } from '../../lib/scrollSync';

/**
 * The Formatted pane as the scroll-sync controller sees it.
 *
 * This pane is already *in* block coordinates: we render exactly one top-level HTML element per
 * top-level block, so the i-th child of `editor.state.doc` is the i-th entry of the outline. No
 * `data-*` attributes are needed to tie them together — the index is the key, and `nodeDOM` turns
 * a node's position into the element that renders it.
 */
export function createRichScrollPane(editor: Editor, scroller: HTMLElement): SyncPane {
  /**
   * Each top-level node's offset from the top of the scrollable content, in document order.
   *
   * Measured against a base that already accounts for the current scroll position, so the values
   * are stable while scrolling and directly comparable with `scrollTop`.
   */
  function offsets(): number[] {
    const base = scroller.getBoundingClientRect().top - scroller.scrollTop;
    const out: number[] = [];
    let previous = 0;
    editor.state.doc.forEach((_node, position) => {
      const dom = editor.view.nodeDOM(position);
      // A node can briefly have no element of its own (mid-update, or a text node at the top
      // level). Carry the previous offset forward so the array stays parallel to the blocks —
      // an entry that is merely imprecise is far better than one that shifts every later index.
      const top = dom instanceof HTMLElement ? dom.getBoundingClientRect().top - base : previous;
      out.push(top);
      previous = top;
    });
    return out;
  }

  /** Where the block at `index` ends, which is where the next one starts. */
  function endOf(values: readonly number[], index: number): number {
    const next = values[index + 1];
    if (next !== undefined) return next;
    return Math.max(values[index] ?? 0, scroller.scrollHeight);
  }

  return {
    scroller() {
      return scroller;
    },

    blockCount() {
      return editor.state.doc.childCount;
    },

    positionAtTop() {
      const values = offsets();
      if (values.length === 0) return null;

      const top = scroller.scrollTop;
      let index = 0;
      for (let i = 1; i < values.length; i += 1) {
        if ((values[i] ?? 0) > top) break;
        index = i;
      }

      const start = values[index] ?? 0;
      const span = endOf(values, index) - start;
      return { index, fraction: span > 0 ? clamp01((top - start) / span) : 0 };
    },

    scrollToPosition(position) {
      const values = offsets();
      if (values.length === 0) return;

      const requested = Number.isFinite(position.index) ? Math.floor(position.index) : 0;
      const index = Math.min(values.length - 1, Math.max(0, requested));
      const start = values[index] ?? 0;
      const span = Math.max(0, endOf(values, index) - start);
      scroller.scrollTop = start + clamp01(position.fraction) * span;
    },
  };
}
