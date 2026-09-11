/**
 * The line <-> block mapping behind synchronized scrolling.
 *
 * Both panes are scrolled by *document structure*, the way VS Code's Markdown preview works.
 * The backend's `outline` command returns the 1-based source line each top-level block starts on,
 * and that array stays exactly parallel to the blocks — which is the whole trick, because the rich
 * pane's top-level ProseMirror nodes also correspond one-for-one with those blocks. So a block
 * index is a coordinate both panes understand, and neither needs to know anything about the other.
 *
 * Proportional scrolling is not good enough on its own: a fifteen-line code block renders as a
 * tall box on one side and a short one on the other, and the two panes drift apart the further you
 * scroll. Anchoring on blocks fixes the drift; interpolating *within* a block stops a long block
 * from feeling stuck while you scroll through it.
 *
 * Everything here is pure and total. The `lines` array arrives over IPC from another process and
 * may be absent, stale, non-monotonic or outright garbage; nothing in this module may throw or
 * produce `NaN` for any input, because a scroll handler has nowhere useful to report an error.
 */

/**
 * A position in the shared coordinate space: the `index`-th top-level block, and how far through
 * it (0 at its start, 1 at the start of the next block) the viewport sits.
 */
export interface BlockPosition {
  /** Index into the outline / into the rich pane's top-level nodes. */
  index: number;
  /** How far through that block, 0-1. */
  fraction: number;
}

/** Clamp to 0-1. `NaN` becomes 0; the infinities clamp like any other out-of-range number. */
export function clamp01(value: number): number {
  if (Number.isNaN(value)) return 0;
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}

/** Read an index that sanitization has already proved to be in range. */
function valueAt(values: readonly number[], index: number): number {
  return values[index] ?? 1;
}

/**
 * Coerce an untrusted outline into something the interpolation can rely on.
 *
 * Guarantees on the result: every entry is a finite integer >= 1, and the array is non-decreasing.
 * A non-monotonic input is repaired by carrying the previous value forward rather than by sorting —
 * sorting would silently re-associate lines with the wrong blocks, whereas carrying forward only
 * collapses the damaged range into a zero-width interval, which the interpolation already handles.
 */
export function sanitizeOutline(lines: unknown): number[] {
  if (!Array.isArray(lines)) return [];
  const values: unknown[] = lines;
  const out: number[] = [];
  let previous = 1;
  for (const raw of values) {
    const parsed = typeof raw === 'number' && Number.isFinite(raw) ? Math.floor(raw) : previous;
    const value = Math.max(previous, parsed, 1);
    out.push(value);
    previous = value;
  }
  return out;
}

/**
 * The exclusive end of the last block, in source lines.
 *
 * The last block has no successor to interpolate towards, so the end of the document stands in for
 * one. The `+ 1` keeps the interval non-empty even for a one-line document, which is what stops the
 * final block from snapping.
 */
function endLine(anchors: readonly number[], totalLines?: number): number {
  const last = anchors.length > 0 ? valueAt(anchors, anchors.length - 1) : 1;
  const total =
    typeof totalLines === 'number' && Number.isFinite(totalLines) ? Math.floor(totalLines) : last;
  return Math.max(last, total) + 1;
}

/** The largest index whose anchor is <= `line`. `anchors` must be non-decreasing and non-empty. */
function lastAnchorAtOrBefore(anchors: readonly number[], line: number): number {
  let lo = 0;
  let hi = anchors.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (valueAt(anchors, mid) <= line) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/**
 * Where a (possibly fractional) source line sits in block coordinates.
 *
 * A fractional line is how the Markdown pane reports a viewport top that falls part-way down a
 * wrapped line, and it carries straight through the interpolation.
 *
 * @param lines - the outline, 1-based source line per top-level block. Untrusted.
 * @param line - the source line at the top of the Markdown viewport, 1-based, may be fractional.
 * @param totalLines - lines in the document, so the final block can interpolate to the end.
 */
export function positionForLine(
  lines: readonly number[],
  line: number,
  totalLines?: number,
): BlockPosition {
  const anchors = sanitizeOutline(lines);
  if (anchors.length === 0) return { index: 0, fraction: 0 };

  const value = Number.isFinite(line) ? line : 1;
  // Anything above the first block belongs to the first block; there is nothing before it.
  if (value <= valueAt(anchors, 0)) return { index: 0, fraction: 0 };

  const last = anchors.length - 1;
  const index = lastAnchorAtOrBefore(anchors, value);
  const start = valueAt(anchors, index);
  const end = index === last ? endLine(anchors, totalLines) : valueAt(anchors, index + 1);
  const span = end - start;
  // A zero-width interval (two blocks on the same line, or repaired garbage) has no inside.
  return { index, fraction: span > 0 ? clamp01((value - start) / span) : 0 };
}

/**
 * The inverse: the source line a block-coordinate position corresponds to.
 *
 * Used when the rich pane is the one being scrolled — it knows which top-level node is at the top
 * of its viewport and how far through it, and this turns that into a line for CodeMirror.
 *
 * @param lines - the outline, 1-based source line per top-level block. Untrusted.
 * @param position - block index and fraction, as reported by the rich pane.
 * @param totalLines - lines in the document, so the final block can interpolate to the end.
 * @returns a 1-based, possibly fractional source line. Never `NaN`.
 */
export function lineForPosition(
  lines: readonly number[],
  position: BlockPosition,
  totalLines?: number,
): number {
  const anchors = sanitizeOutline(lines);
  if (anchors.length === 0) return 1;

  const last = anchors.length - 1;
  const requested = Number.isFinite(position.index) ? Math.floor(position.index) : 0;
  // The rich pane's node count can briefly disagree with the outline while a conversion is in
  // flight. Clamping is the right answer: scroll to the nearest block we do know about.
  const index = Math.min(last, Math.max(0, requested));
  const fraction = clamp01(position.fraction);

  const start = valueAt(anchors, index);
  const end = index === last ? endLine(anchors, totalLines) : valueAt(anchors, index + 1);
  return start + fraction * Math.max(0, end - start);
}

/**
 * How much of the scroll range at each end is reserved for pulling the follower onto the edge.
 *
 * A fraction rather than a pixel count so it behaves the same in a tall window and a short one.
 */
export const EDGE_BAND = 0.12;

/**
 * Pull an anchored offset onto the document ends as the driver approaches them.
 *
 * Block anchoring aligns the *top* of the two viewports, which is right in the middle of a
 * document and wrong at its ends. When the two panes have different heights — a tall code block
 * rendered short, a table rendered tall — scrolling the shorter pane to its very bottom leaves the
 * taller one anchored on whichever block happens to be at the top of that last viewport, with
 * content still below it. The user can then never reach the end of the taller pane by scrolling
 * the shorter one, which is exactly the complaint.
 *
 * So the ends are pinned: at progress 0 the follower goes to 0, at progress 1 it goes to its
 * maximum, and in between the correction fades out over [`EDGE_BAND`] so there is no jump. Outside
 * the two bands the anchored offset is used untouched, because that is where anchoring earns its
 * keep.
 *
 * @param anchored - the offset block anchoring chose.
 * @param progress - how far the driving pane is through its own range, 0-1.
 * @param range - the follower's maximum scrollTop.
 * @returns a finite offset within `0..range`.
 */
export function pinToEnds(anchored: number, progress: number, range: number): number {
  const limit = Number.isFinite(range) && range > 0 ? range : 0;
  if (limit === 0) return 0;

  const from = Number.isFinite(anchored) ? Math.min(Math.max(anchored, 0), limit) : 0;
  // An unmeasurable driver means we do not know where it is, which is not the same as knowing it
  // is at the top. Leave the anchored offset alone rather than yanking the follower to 0.
  if (!Number.isFinite(progress)) return from;
  const t = clamp01(progress);

  // Distance into whichever band we are in, 0 outside both, 1 at the very edge.
  const weight =
    t <= EDGE_BAND ? 1 - t / EDGE_BAND : t >= 1 - EDGE_BAND ? (t - (1 - EDGE_BAND)) / EDGE_BAND : 0;
  if (weight <= 0) return from;

  const edge = t < 0.5 ? 0 : limit;
  return from + (edge - from) * clamp01(weight);
}
