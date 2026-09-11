import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

import { openApp, setMarkdown, settled } from './support';

/** What the caret element looks like right now, from the point of view of someone looking at it. */
async function caretState(page: Page) {
  return page.evaluate(() => {
    const caret = document.querySelector<HTMLElement>('[data-testid="pane-markdown"] .cm-cursor');
    if (!caret) return null;
    const style = getComputedStyle(caret);
    const layer = caret.parentElement;
    const rect = caret.getBoundingClientRect();
    return {
      display: style.display,
      width: Number.parseFloat(style.borderLeftWidth),
      color: style.borderLeftColor,
      /*
       * Whether it blinks. Not `animationName`: CodeMirror writes that (and the duration) onto
       * the layer as an inline style on every selection change, focused or not, so it says
       * "cm-blink" in both states. The iteration count is what its focused-only rule actually
       * changes, and it is the property that decides whether anything moves.
       */
      blinks: layer ? getComputedStyle(layer).animationIterationCount === 'infinite' : false,
      height: rect.height,
    };
  });
}

/**
 * Move focus to the Formatted pane and wait for CodeMirror to actually let go.
 *
 * Reading the caret straight after the click races the blur: the styles are keyed off
 * `.cm-focused`, so the class going away is the event worth waiting for.
 */
async function blurMarkdown(page: Page): Promise<void> {
  await page.getByTestId('editor-rich').click();
  await expect(page.locator('[data-testid="pane-markdown"] .cm-editor')).not.toHaveClass(
    /cm-focused/,
  );
}

test.describe('the Markdown caret', () => {
  test('stays visible after focus moves to the Formatted pane', async ({ page }) => {
    // The complaint this answers: click into the Formatted pane and every trace of where you were
    // in the Markdown vanishes, because CodeMirror only draws its caret while focused.
    await openApp(page);
    await setMarkdown(page, 'the quick brown fox jumps over the lazy dog');
    await settled(page);

    const focused = await caretState(page);
    expect(focused).not.toBeNull();
    expect(focused!.display).not.toBe('none');
    expect(focused!.height).toBeGreaterThan(0);

    await blurMarkdown(page);

    const blurred = await caretState(page);
    expect(blurred).not.toBeNull();
    expect(blurred!.display).not.toBe('none');
    expect(blurred!.height).toBeGreaterThan(0);
  });

  test('reads as a memory, not an invitation, while unfocused', async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, 'abcdef');
    await settled(page);

    const focused = await caretState(page);
    await blurMarkdown(page);
    const blurred = await caretState(page);

    expect(focused).not.toBeNull();
    expect(blurred).not.toBeNull();

    // Focused: 2px, so it is findable in a wall of monospace. Unfocused: a hairline.
    expect(focused!.width).toBeCloseTo(2, 1);
    expect(blurred!.width).toBeLessThan(focused!.width);

    // Focused is the accent colour; unfocused is dimmed, and visibly a different colour.
    expect(blurred!.color).not.toBe(focused!.color);

    // And it must not blink at you from a pane you are not typing in.
    expect(blurred!.blinks).toBe(false);
    expect(focused!.blinks).toBe(true);
  });
});
