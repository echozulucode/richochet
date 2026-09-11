import { describe, expect, it } from 'vitest';

import {
  EDGE_BAND,
  clamp01,
  lineForPosition,
  pinToEnds,
  positionForLine,
  sanitizeOutline,
  type BlockPosition,
} from './scrollMapping';

/** Assert a number came back usable — the failure mode that matters here is a silent NaN. */
function expectFinite(value: number): void {
  expect(Number.isFinite(value)).toBe(true);
}

function expectSanePosition(position: BlockPosition): void {
  expectFinite(position.index);
  expectFinite(position.fraction);
  expect(Number.isInteger(position.index)).toBe(true);
  expect(position.index).toBeGreaterThanOrEqual(0);
  expect(position.fraction).toBeGreaterThanOrEqual(0);
  expect(position.fraction).toBeLessThanOrEqual(1);
}

describe('clamp01', () => {
  it('clamps to 0-1 and never lets a NaN through', () => {
    expect(clamp01(0.25)).toBe(0.25);
    expect(clamp01(-3)).toBe(0);
    expect(clamp01(9)).toBe(1);
    expect(clamp01(Number.NaN)).toBe(0);
    // The infinities are out of range, not meaningless: clamp them like any other number.
    expect(clamp01(Number.POSITIVE_INFINITY)).toBe(1);
    expect(clamp01(Number.NEGATIVE_INFINITY)).toBe(0);
  });
});

describe('sanitizeOutline', () => {
  it('passes a well-formed outline through unchanged', () => {
    expect(sanitizeOutline([1, 3, 9, 40])).toEqual([1, 3, 9, 40]);
  });

  it('returns an empty array for anything that is not an array', () => {
    expect(sanitizeOutline(undefined)).toEqual([]);
    expect(sanitizeOutline(null)).toEqual([]);
    expect(sanitizeOutline('1,2,3')).toEqual([]);
    expect(sanitizeOutline({ lines: [1, 2] })).toEqual([]);
  });

  it('repairs garbage entries by carrying the previous anchor forward', () => {
    // Carrying forward, rather than sorting, keeps every remaining entry associated with its own
    // block; the damage is confined to a zero-width interval.
    expect(sanitizeOutline([1, Number.NaN, 7, 'x', 12])).toEqual([1, 1, 7, 7, 12]);
    expect(sanitizeOutline([0, -5, 2])).toEqual([1, 1, 2]);
    expect(sanitizeOutline([5, 3, 9])).toEqual([5, 5, 9]);
    expect(sanitizeOutline([2.7, 4.2])).toEqual([2, 4]);
  });
});

describe('positionForLine', () => {
  it('an empty document maps everything to the start', () => {
    const position = positionForLine([], 1, 0);
    expect(position).toEqual({ index: 0, fraction: 0 });
    expectSanePosition(positionForLine([], 500, 0));
  });

  it('a single block interpolates across the whole document', () => {
    // One block starting at line 1 of a nine-line document. The block runs to line 10 exclusive,
    // so line 5 is four ninths of the way through it.
    const at = (line: number) => positionForLine([1], line, 9).fraction;
    expect(positionForLine([1], 1, 9)).toEqual({ index: 0, fraction: 0 });
    expect(at(5)).toBeCloseTo(4 / 9, 5);
    expect(at(10)).toBe(1);
    expect(at(9999)).toBe(1);
  });

  it('a line before the first block clamps to the first block', () => {
    // Front matter, a leading blank line, or simply a stale outline: never a negative fraction.
    expect(positionForLine([4, 8], 1, 12)).toEqual({ index: 0, fraction: 0 });
    expect(positionForLine([4, 8], -20, 12)).toEqual({ index: 0, fraction: 0 });
  });

  it('a line past the last block clamps to the end of the last block', () => {
    const lines = [1, 5, 9];
    expect(positionForLine(lines, 200, 12)).toEqual({ index: 2, fraction: 1 });
    // The last block still has an inside: line 11 of a 12-line document is most of the way in.
    const inside = positionForLine(lines, 11, 12);
    expect(inside.index).toBe(2);
    expect(inside.fraction).toBeCloseTo(0.5, 5);
  });

  it('interpolates inside a long block instead of snapping to its boundary', () => {
    // Block 1 spans lines 5-24 — a twenty-line fenced code block. Snapping would make the whole
    // block feel stuck; halfway through it must read as halfway.
    const lines = [1, 5, 25];
    expect(positionForLine(lines, 5, 40)).toEqual({ index: 1, fraction: 0 });
    expect(positionForLine(lines, 15, 40).fraction).toBeCloseTo(0.5, 5);
    expect(positionForLine(lines, 24, 40).fraction).toBeCloseTo(0.95, 5);
    expect(positionForLine(lines, 25, 40)).toEqual({ index: 2, fraction: 0 });
  });

  it('carries a fractional line (a part-scrolled wrapped line) through the interpolation', () => {
    const half = positionForLine([1, 3], 2.5, 6);
    expect(half.index).toBe(0);
    expect(half.fraction).toBeCloseTo(0.75, 5);
  });

  it('never throws or yields NaN for a non-monotonic or garbage outline', () => {
    const garbage = [9, 2, Number.NaN, -1, 4, Number.POSITIVE_INFINITY, 7] as number[];
    for (const line of [-10, 0, 1, 3, 9, 500, Number.NaN, Number.POSITIVE_INFINITY]) {
      expectSanePosition(positionForLine(garbage, line, 20));
      expectSanePosition(positionForLine(garbage, line));
    }
    expectSanePosition(positionForLine([5, 5, 5], 5, 5));
    expectSanePosition(positionForLine([1, 1], 1, Number.NaN));
  });
});

describe('lineForPosition', () => {
  it('an empty document maps everything to line 1', () => {
    expect(lineForPosition([], { index: 0, fraction: 0 })).toBe(1);
    expect(lineForPosition([], { index: 7, fraction: 0.5 }, 40)).toBe(1);
  });

  it('a single block interpolates across the whole document', () => {
    expect(lineForPosition([1], { index: 0, fraction: 0 }, 9)).toBe(1);
    expect(lineForPosition([1], { index: 0, fraction: 0.5 }, 9)).toBeCloseTo(5.5, 5);
    expect(lineForPosition([1], { index: 0, fraction: 1 }, 9)).toBe(10);
  });

  it('clamps an index the outline does not have', () => {
    // The rich pane's node count can lead or lag the outline while a conversion is in flight.
    const lines = [1, 5, 9];
    expect(lineForPosition(lines, { index: -4, fraction: 0 }, 12)).toBe(1);
    expect(lineForPosition(lines, { index: 99, fraction: 0 }, 12)).toBe(9);
  });

  it('interpolates inside a long block', () => {
    const lines = [1, 5, 25];
    expect(lineForPosition(lines, { index: 1, fraction: 0.5 }, 40)).toBeCloseTo(15, 5);
    expect(lineForPosition(lines, { index: 2, fraction: 0.5 }, 40)).toBeCloseTo(33, 5);
  });

  it('never throws or yields NaN for a non-monotonic or garbage outline', () => {
    const garbage = [9, 2, Number.NaN, -1, 4, Number.POSITIVE_INFINITY, 7] as number[];
    const positions: BlockPosition[] = [
      { index: 0, fraction: 0 },
      { index: 3, fraction: Number.NaN },
      { index: Number.NaN, fraction: 0.5 },
      { index: -9, fraction: 40 },
      { index: 400, fraction: -2 },
      { index: 1.7, fraction: 0.3 },
    ];
    for (const position of positions) {
      expectFinite(lineForPosition(garbage, position, 30));
      expectFinite(lineForPosition(garbage, position));
      expect(lineForPosition(garbage, position, 30)).toBeGreaterThanOrEqual(1);
    }
  });
});

describe('round trip', () => {
  it('a line maps back to itself through block coordinates', () => {
    // This is the property that keeps the panes from drifting: driving pane A to a line, then
    // reading pane B back, must not creep.
    const lines = [1, 4, 5, 21, 30];
    const total = 44;
    for (const line of [1, 2.5, 4, 4.9, 5, 12, 20, 21, 29, 30, 38, 44]) {
      const back = lineForPosition(lines, positionForLine(lines, line, total), total);
      expect(back).toBeCloseTo(line, 5);
    }
  });

  it('is idempotent when applied twice, so repeated sync cannot creep', () => {
    const lines = [1, 4, 5, 21, 30];
    const once = positionForLine(lines, 17.25, 44);
    const twice = positionForLine(lines, lineForPosition(lines, once, 44), 44);
    expect(twice.index).toBe(once.index);
    expect(twice.fraction).toBeCloseTo(once.fraction, 10);
  });
});

describe('pinToEnds', () => {
  it('pulls the follower onto the top when the driver is at the top', () => {
    // Anchoring alone would leave it at 400; at progress 0 the only right answer is 0.
    expect(pinToEnds(400, 0, 1500)).toBe(0);
  });

  it('pulls the follower onto the bottom when the driver is at the bottom', () => {
    // The complaint: a taller follower stayed anchored mid-document with content still below.
    expect(pinToEnds(1000, 1, 1500)).toBe(1500);
  });

  it('leaves the anchored offset alone in the middle, where anchoring is the point', () => {
    expect(pinToEnds(700, 0.5, 1500)).toBe(700);
    expect(pinToEnds(700, 0.3, 1500)).toBe(700);
    expect(pinToEnds(700, 0.7, 1500)).toBe(700);
  });

  it('fades the correction in rather than jumping at the very edge', () => {
    const range = 1000;
    const anchored = 600;
    const outside = pinToEnds(anchored, 1 - EDGE_BAND, range);
    const halfway = pinToEnds(anchored, 1 - EDGE_BAND / 2, range);
    const atEnd = pinToEnds(anchored, 1, range);

    expect(outside).toBe(anchored);
    expect(halfway).toBeGreaterThan(outside);
    expect(halfway).toBeLessThan(atEnd);
    expect(atEnd).toBe(range);
  });

  it('is monotonic across the whole range', () => {
    const range = 1200;
    let previous = -1;
    for (let i = 0; i <= 100; i += 1) {
      const t = i / 100;
      // A plausible anchored curve: roughly proportional, so pinning only has to fix the ends.
      const value = pinToEnds(t * 900, t, range);
      expect(value).toBeGreaterThanOrEqual(previous);
      previous = value;
    }
    expect(previous).toBe(range);
  });

  it('never leaves the follower outside its own range', () => {
    expect(pinToEnds(99999, 0.5, 1000)).toBe(1000);
    expect(pinToEnds(-50, 0.5, 1000)).toBe(0);
  });

  it('is total for degenerate and hostile input', () => {
    expect(pinToEnds(100, 0.5, 0)).toBe(0);
    expect(pinToEnds(Number.NaN, 0.5, 1000)).toBe(0);
    expect(pinToEnds(100, Number.NaN, 1000)).toBe(100);
    expect(pinToEnds(100, 0.5, Number.NaN)).toBe(0);
    expect(pinToEnds(100, -5, 1000)).toBe(0);
    expect(pinToEnds(100, 5, 1000)).toBe(1000);
  });
});
