import { expect, test } from '@playwright/test';
import { markdownText, openApp, richText, setMarkdown, settled } from './support';

/**
 * The sync engine is the hardest part of the frontend and the part unit tests reach least well.
 * Two editors that each regenerate the other will loop, fight over the caret, and corrupt input;
 * these tests assert the three rules from docs/implementation-plan.md §3.1 hold in a real browser.
 */
test.describe('live sync', () => {
  test('Markdown flows to the Formatted pane', async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, '**Hello Eric**');
    await settled(page);

    await expect(page.getByTestId('editor-rich')).toContainText('Hello Eric');
    await expect(page.getByTestId('editor-rich').locator('strong')).toHaveText('Hello Eric');
  });

  test('editing the Formatted pane flows back to Markdown', async ({ page }) => {
    await openApp(page);

    const rich = page.getByTestId('editor-rich');
    await rich.click();
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.press('Delete');
    await rich.pressSequentially('plain words');
    await settled(page);

    await expect(page.getByTestId('editor-markdown')).toContainText('plain words');
  });

  test('the unfocused pane never steals the caret during fast typing', async ({ page }) => {
    await openApp(page);

    const editor = page.getByTestId('editor-markdown');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.press('Delete');

    // Type fast enough that several conversions overlap. If the derived pane writes back, or a
    // stale response is applied, characters get reordered or dropped.
    const typed = 'the quick brown fox jumps over the lazy dog';
    await editor.pressSequentially(typed, { delay: 8 });
    await settled(page);

    expect((await markdownText(page)).trim()).toBe(typed);
  });

  test('a stale conversion response is dropped, not applied', async ({ page }) => {
    // The first request is delayed far longer than the second, so the second lands first and the
    // first arrives afterwards as a stale response. Without the sequence guard the pane would
    // visibly snap back to the earlier text. A single fixed latency cannot test this: every
    // request would wait the same time and they would resolve in order.
    await openApp(page, { latency: '400,20' });

    const editor = page.getByTestId('editor-markdown');
    await editor.click();

    await editor.pressSequentially('ab');
    // Wait for the debounce to actually *issue* a request. Waiting on `pendingConversions` is not
    // enough — it counts from the keystroke, so the second edit would land inside the same debounce
    // window and coalesce into one request, and there would be no race to test.
    await page.waitForFunction(() => (window.__richochet_test?.lastSeq ?? 0) >= 1);
    await editor.pressSequentially('cd');
    await settled(page);

    // The later request wins; the stale one must not overwrite it.
    await expect(page.getByTestId('editor-rich')).toContainText('abcd');
    expect(await richText(page)).not.toBe('ab');

    const { issued, applied } = await page.evaluate(() => ({
      issued: window.__richochet_test?.lastSeq ?? 0,
      applied: window.__richochet_test?.lastAppliedSeq ?? 0,
    }));
    // A response was genuinely dropped rather than merely arriving in order.
    expect(issued).toBeGreaterThan(1);
    expect(applied).toBeLessThanOrEqual(issued);
  });

  test('typing does not grow the document on every keystroke', async ({ page }) => {
    // Non-idempotent conversion shows up here first: the text creeps as the user types.
    await openApp(page);
    await setMarkdown(page, '- one\n- two');
    await settled(page);

    const first = await markdownText(page);

    await page.getByTestId('editor-rich').click();
    await settled(page);
    await page.getByTestId('editor-markdown').click();
    await settled(page);

    expect(await markdownText(page)).toBe(first);
  });
});
