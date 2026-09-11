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
      // The blink lives on the layer, not the caret itself.
      animation: layer ? getComputedStyle(layer).animationName : 'none',
      height: rect.height,
    };
  });
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

    await page.getByTestId('editor-rich').click();
    await expect(page.getByTestId('editor-markdown')).not.toBeFocused();

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
    await page.getByTestId('editor-rich').click();
    const blurred = await caretState(page);

    expect(focused).not.toBeNull();
    expect(blurred).not.toBeNull();

    // Focused: 2px, so it is findable in a wall of monospace. Unfocused: a hairline.
    expect(focused!.width).toBeCloseTo(2, 1);
    expect(blurred!.width).toBeLessThan(focused!.width);

    // Focused is the accent colour; unfocused is dimmed, and visibly a different colour.
    expect(blurred!.color).not.toBe(focused!.color);

    // And it must not blink at you from a pane you are not typing in.
    expect(blurred!.animation).toBe('none');
    expect(focused!.animation).not.toBe('none');
  });
});
