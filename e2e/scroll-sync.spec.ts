import { expect, test } from '@playwright/test';

import {
  openApp,
  outline,
  scrollRange,
  scrollTop,
  setScrollSample,
  topVisibleBlock,
  visibleBlocks,
  wheelOver,
} from './support';

/**
 * Synchronized scrolling, VS Code style: both panes are anchored on document structure rather
 * than on a scroll ratio, because a long code block on one side and a short rendering on the
 * other drift badly under a ratio.
 *
 * These are the bugs unit tests cannot reach. The mapping itself is pure and covered directly in
 * `src/lib/scrollMapping.test.ts`; what can only fail in a real browser is the wiring — measuring
 * two different scrollers, and the feedback loop where each pane's programmatic scroll fires the
 * other's handler and they push each other around forever.
 */
test.describe('synchronized scrolling', () => {
  // Setting up a document long enough to scroll means typing one, which is not fast.
  test.slow();

  test('scrolling the Markdown pane moves the Formatted pane to the same content', async ({
    page,
  }) => {
    await openApp(page, { sync: true });
    await setScrollSample(page);

    expect(await scrollRange(page, 'markdown')).toBeGreaterThan(100);
    expect(await scrollRange(page, 'rich')).toBeGreaterThan(100);

    // Without this the suite could pass entirely on the proportional fallback and never touch the
    // feature: one outline entry per top-level block of the sample, from the real engine.
    expect(await outline(page)).toHaveLength(24);

    await wheelOver(page, 'markdown', 600);

    // The follower has to actually move...
    await expect
      .poll(async () => scrollTop(page, 'rich'), { message: 'Formatted pane did not follow' })
      .toBeGreaterThan(0);

    // ...and land on the same part of the document, which is the claim that matters.
    await expect
      .poll(async () => {
        const top = await topVisibleBlock(page, 'markdown');
        const rich = await visibleBlocks(page, 'rich');
        return top !== null && rich.includes(top);
      })
      .toBe(true);
  });

  test('scrolling the Formatted pane moves the Markdown pane', async ({ page }) => {
    await openApp(page, { sync: true });
    await setScrollSample(page);

    await wheelOver(page, 'rich', 500);

    await expect
      .poll(async () => scrollTop(page, 'markdown'), { message: 'Markdown pane did not follow' })
      .toBeGreaterThan(0);

    await expect
      .poll(async () => {
        const top = await topVisibleBlock(page, 'rich');
        const markdown = await visibleBlocks(page, 'markdown');
        return top !== null && markdown.includes(top);
      })
      .toBe(true);
  });

  test('the two panes do not fight each other once a scroll settles', async ({ page }) => {
    // The failure this exists for: pane A scrolls B, B's scroll handler scrolls A back, and the
    // pair oscillates or creeps away on its own after the user has stopped touching anything.
    await openApp(page, { sync: true });
    await setScrollSample(page);

    await wheelOver(page, 'markdown', 400);
    await expect.poll(async () => scrollTop(page, 'rich')).toBeGreaterThan(0);

    // Well past the driver's hold window, so a loop would have had every chance to start.
    await page.waitForTimeout(600);
    const settled = {
      markdown: await scrollTop(page, 'markdown'),
      rich: await scrollTop(page, 'rich'),
    };

    await page.waitForTimeout(600);
    expect(await scrollTop(page, 'markdown')).toBeCloseTo(settled.markdown, 0);
    expect(await scrollTop(page, 'rich')).toBeCloseTo(settled.rich, 0);
  });

  test('scrolling back and forth between panes stays in step', async ({ page }) => {
    // A stuck echo-suppression flag shows up here and nowhere else: the first direction works,
    // and then the other pane is ignored forever.
    await openApp(page, { sync: true });
    await setScrollSample(page);

    await wheelOver(page, 'markdown', 500);
    await expect.poll(async () => scrollTop(page, 'rich')).toBeGreaterThan(0);

    await page.waitForTimeout(400);
    const richBefore = await scrollTop(page, 'rich');

    // Now hand the wheel to the other pane. It must take over, not be treated as an echo.
    await wheelOver(page, 'rich', -400);
    await expect
      .poll(async () => scrollTop(page, 'rich'), { message: 'Formatted pane stopped responding' })
      .toBeLessThan(richBefore);

    await expect
      .poll(async () => {
        const top = await topVisibleBlock(page, 'rich');
        const markdown = await visibleBlocks(page, 'markdown');
        return top !== null && markdown.includes(top);
      })
      .toBe(true);
  });

  test('?sync=off leaves the other pane exactly where it was', async ({ page }) => {
    // The escape hatch every other spec in this suite relies on.
    await openApp(page);
    await setScrollSample(page);

    await wheelOver(page, 'markdown', 500);
    await expect.poll(async () => scrollTop(page, 'markdown')).toBeGreaterThan(0);

    await page.waitForTimeout(400);
    expect(await scrollTop(page, 'rich')).toBe(0);
  });
});
