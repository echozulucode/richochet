import type { EditorView } from '@codemirror/view';

import { clamp01, lineForPosition, positionForLine } from '../../lib/scrollMapping';
import type { SyncPane } from '../../lib/scrollSync';

/**
 * The Markdown pane as the scroll-sync controller sees it.
 *
 * CodeMirror measures in *document coordinates* — a vertical axis whose origin is the top of the
 * first line, independent of scrolling. `view.documentTop` is where that origin currently sits in
 * client coordinates, so the two spaces convert into each other with one subtraction, and doing it
 * that way avoids having to know anything about the editor's padding or its scroller's offset.
 *
 * Line granularity comes from `lineBlockAt`/`lineBlockAtHeight`, which with line wrapping on treat
 * a wrapped line as a single block. That is what we want: the outline is keyed by *source* line,
 * so a visual row is not a meaningful unit. The leftover fraction of a part-scrolled wrapped line
 * rides along as the fractional part of the line number.
 */
export function createMarkdownScrollPane(view: EditorView): SyncPane {
  /** The document-coordinate height currently at the top of the visible area. */
  function heightAtViewportTop(): number {
    return view.scrollDOM.getBoundingClientRect().top - view.documentTop;
  }

  return {
    scroller() {
      return view.scrollDOM;
    },

    blockCount() {
      // The outline *is* this pane's block list; there is nothing independent to compare.
      return null;
    },

    positionAtTop(lines) {
      const doc = view.state.doc;
      if (doc.length === 0) return null;

      const height = heightAtViewportTop();
      const block = view.lineBlockAtHeight(height);
      const line = doc.lineAt(block.from);
      const within = block.height > 0 ? clamp01((height - block.top) / block.height) : 0;
      return positionForLine(lines, line.number + within, doc.lines);
    },

    scrollToPosition(position, lines) {
      const doc = view.state.doc;
      const target = lineForPosition(lines, position, doc.lines);

      const number = Math.min(doc.lines, Math.max(1, Math.floor(target)));
      const within = clamp01(target - number);
      const block = view.lineBlockAt(doc.line(number).from);
      const wanted = block.top + within * block.height;

      // Move by the delta rather than assigning an absolute scrollTop: document coordinates and
      // scroller coordinates differ by a constant we would otherwise have to reconstruct, and the
      // browser clamps the result for us at both ends.
      view.scrollDOM.scrollTop += wanted - heightAtViewportTop();
    },
  };
}
