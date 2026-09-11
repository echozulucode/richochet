import { expect, test } from '@playwright/test';

import {
  openApp,
  outline,
  outlineSettled,
  scroller,
  scrollRange,
  scrollTop,
  setScrollSample,
  settled,
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

    // Deliberately a modest scroll. Anchoring aligns the viewport *tops*, which is the claim
    // being tested; within the end bands the tops are pinned apart on purpose (see the
    // end-of-document specs below), so a scroll that reached the bottom would test the wrong thing.
    await wheelOver(page, 'rich', 200);

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

  test('scrolling one pane to its bottom reaches the other pane bottom', async ({ page }) => {
    await openApp(page, { sync: true });
    await setScrollSample(page);

    const markdownRange = await scrollRange(page, 'markdown');
    const richRange = await scrollRange(page, 'rich');
    expect(markdownRange).toBeGreaterThan(0);
    expect(richRange).toBeGreaterThan(0);

    // Block anchoring aligns the *tops* of the viewports, so at full scroll the follower used to
    // stop on whichever block sat at the top of that last screen, leaving its own tail unreachable.
    // A real wheel, not a programmatic scrollTop: only a gesture takes the floor outright, and a
    // programmatic scroll arriving while the other pane still holds it is dropped as echo.
    await wheelOver(page, 'markdown', 5000);
    await expect.poll(async () => scrollTop(page, 'rich')).toBeGreaterThan(richRange - 4);

    // And the same going back to the top.
    await wheelOver(page, 'markdown', -5000);
    await expect.poll(async () => scrollTop(page, 'rich')).toBeLessThan(4);
  });

  test('the taller pane reaches its own end when driven from the shorter one', async ({ page }) => {
    await openApp(page, { sync: true });
    await setScrollSample(page);

    const markdownRange = await scrollRange(page, 'markdown');
    const richRange = await scrollRange(page, 'rich');
    // The panes are genuinely different heights - a 12-line code block renders compactly - which
    // is the whole reason the ends need pinning rather than just anchoring.
    expect(Math.abs(markdownRange - richRange)).toBeGreaterThan(20);

    const [shorter, taller] =
      markdownRange < richRange ? (['markdown', 'rich'] as const) : (['rich', 'markdown'] as const);

    await wheelOver(page, shorter, 5000);

    const tallerRange = await scrollRange(page, taller);
    await expect
      .poll(async () => scrollTop(page, taller), {
        message: 'the taller pane never reached its own end',
      })
      .toBeGreaterThan(tallerRange - 4);
  });

  test('editing re-aligns the panes without needing a scroll', async ({ page }) => {
    await openApp(page, { sync: true });
    await setScrollSample(page);

    // Park half way down, driven from the Markdown pane.
    const markdownRange = await scrollRange(page, 'markdown');
    await wheelOver(page, 'markdown', markdownRange / 2);
    await page.waitForTimeout(400);

    const richBefore = await scrollTop(page, 'rich');
    expect(richBefore).toBeGreaterThan(0);

    // Now type, without touching either scrollbar. Inserting lines above moves every anchor, so
    // the follower has to be pushed again; before this fix sync appeared to stop working until
    // the user scrolled.
    const editor = scroller(page, 'markdown');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+Home');
    await page.keyboard.type('Block 00 inserted at the top.\n\n');
    await settled(page);
    await outlineSettled(page);
    await page.waitForTimeout(400);

    // The mock only knows outlines for documents in the oracle, and an edited one is not, so this
    // exercises the proportional fallback. That is fine: the bug was that *nothing* re-projected
    // after an edit, on either path. The anchored path is covered by the specs above.
    expect(await scrollTop(page, 'rich')).not.toBe(richBefore);
  });
});
