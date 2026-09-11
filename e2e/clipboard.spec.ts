import { expect, test } from '@playwright/test';
import { openApp, setMarkdown, settled } from './support';

test.describe('copy actions', () => {
  test('Copy for Teams writes HTML and a plain-text fallback together', async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, '**bold** and _italic_');
    await settled(page);

    await page.getByTestId('copy-teams').click();

    const writes = await page.evaluate(() => window.__richochet_test?.clipboardWrites ?? []);
    expect(writes).toHaveLength(1);

    // Both representations must go on in one operation, or Teams gets only the plain text.
    expect(writes[0].html).toContain('<strong>bold</strong>');
    expect(writes[0].text).toContain('bold');
    expect(writes[0].text).not.toContain('<strong>');
  });

  test('Copy Markdown writes the Markdown source, not HTML', async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, '**bold**');
    await settled(page);

    await page.getByTestId('copy-markdown').click();

    const writes = await page.evaluate(() => window.__richochet_test?.clipboardWrites ?? []);
    expect(writes).toHaveLength(1);
    expect(writes[0].html).toBeNull();
    expect(writes[0].text).toContain('**bold**');
  });

  test('Copy Plain Text strips every mark', async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, '**Important**');
    await settled(page);

    await page.getByTestId('copy-text').click();

    const writes = await page.evaluate(() => window.__richochet_test?.clipboardWrites ?? []);
    expect(writes).toHaveLength(1);
    expect(writes[0].html).toBeNull();
    // docs/plan.md: **Important** becomes Important.
    expect(writes[0].text.trim()).toBe('Important');
  });

  test('copying confirms on the button itself, then returns to normal', async ({ page }) => {
    await openApp(page);
    const button = page.getByTestId('copy-markdown');
    await expect(button).toHaveAttribute('data-copied', 'false');

    await button.click();

    // The tick on the clicked button is the whole confirmation — it says *which* action
    // succeeded, which a toast at the bottom of the window cannot.
    await expect(button).toHaveAttribute('data-copied', 'true');
    // "Nothing more intrusive" — no toast piling on top of it.
    await expect(page.getByTestId('toast')).toBeHidden();
    // And it must reset itself.
    await expect(button).toHaveAttribute('data-copied', 'false', { timeout: 8_000 });
  });

  test('each pane carries the copy action for its own content', async ({ page }) => {
    await openApp(page);

    // An action belongs next to the thing it acts on, not in a bar detached from both panes.
    await expect(page.getByTestId('pane-formatted').getByTestId('copy-teams')).toBeVisible();
    await expect(page.getByTestId('pane-formatted').getByTestId('copy-text')).toBeVisible();
    await expect(page.getByTestId('pane-markdown').getByTestId('copy-markdown')).toBeVisible();

    // Icon-only, so each must still name itself for assistive tech.
    await expect(page.getByTestId('copy-teams')).toHaveAttribute('aria-label', 'Copy for Teams');
    await expect(page.getByTestId('copy-markdown')).toHaveAttribute('aria-label', 'Copy Markdown');
    await expect(page.getByTestId('copy-text')).toHaveAttribute('aria-label', 'Copy plain text');
  });
});

test.describe('paste detection', () => {
  test('pasting rich content produces Markdown and says so', async ({ page }) => {
    await openApp(page);

    await page.evaluate(() => {
      window.__richochet_test?.stageClipboard({
        kind: 'rich',
        html: '<div><span style="font-weight:600"> Important </span></div>',
        text: 'Important',
      });
    });

    await page.getByTestId('editor-rich').click();
    await page.keyboard.press('ControlOrMeta+v');
    await settled(page);

    await expect(page.getByTestId('editor-markdown')).toContainText('**Important**');
    await expect(page.getByTestId('toast')).toContainText(/rich text/i);
  });

  test('pasting plain content takes it as Markdown', async ({ page }) => {
    await openApp(page);

    await page.evaluate(() => {
      window.__richochet_test?.stageClipboard({ kind: 'text', text: 'just words' });
    });

    await page.getByTestId('editor-rich').click();
    await page.keyboard.press('ControlOrMeta+v');
    await settled(page);

    await expect(page.getByTestId('editor-markdown')).toContainText('just words');
    await expect(page.getByTestId('toast')).toContainText(/Markdown/i);
  });

  test('pasting Markdown source renders it, rather than escaping it', async ({ page }) => {
    await openApp(page);

    // The regression this pins: the plain-text branch used to convert `text -> markdown`, which
    // escapes every Markdown character. `**Hello Eric**` arrived as `\*\*Hello Eric\*\*` and
    // showed literal asterisks — "paste some Markdown and nothing happens".
    await page.evaluate(() => {
      window.__richochet_test?.stageClipboard({ kind: 'text', text: '**Hello Eric**' });
    });

    await page.getByTestId('editor-rich').click();
    await page.keyboard.press('ControlOrMeta+v');
    await settled(page);

    await expect(page.getByTestId('editor-markdown')).toContainText('**Hello Eric**');
    // No backslash escapes: that is exactly what the old text->markdown conversion introduced.
    await expect(page.getByTestId('editor-markdown')).not.toContainText('\\*');
    // And it actually rendered as bold rather than as four asterisks.
    await expect(page.getByTestId('editor-rich').locator('strong')).toHaveText('Hello Eric');
  });
});
