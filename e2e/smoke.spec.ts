import { expect, test } from '@playwright/test';
import { openApp } from './support';

test.describe('app shell', () => {
  test('renders both panes and the three copy actions', async ({ page }) => {
    await openApp(page);

    await expect(page.getByTestId('pane-formatted')).toBeVisible();
    await expect(page.getByTestId('pane-markdown')).toBeVisible();
    await expect(page.getByTestId('editor-rich')).toBeVisible();
    await expect(page.getByTestId('editor-markdown')).toBeVisible();

    await expect(page.getByTestId('copy-teams')).toBeVisible();
    await expect(page.getByTestId('copy-markdown')).toBeVisible();
    await expect(page.getByTestId('copy-text')).toBeVisible();
  });

  test('both panes are editable', async ({ page }) => {
    await openApp(page);

    // docs/plan.md: "Both sides should be editable."
    await expect(page.getByTestId('editor-rich')).toHaveAttribute('contenteditable', 'true');
    const markdown = page.getByTestId('editor-markdown');
    await markdown.click();
    await markdown.pressSequentially('typed');
    await expect(markdown).toContainText('typed');
  });

  test('logs no console errors on load', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(msg.text());
    });
    page.on('pageerror', (err) => errors.push(err.message));

    await openApp(page);
    expect(errors).toEqual([]);
  });

  test('the empty-document hint does not paint over real content', async ({ page }) => {
    await openApp(page);
    const rich = page.getByTestId('editor-rich');

    // Shown when there is genuinely nothing there.
    await expect(rich).toHaveText('');

    // A line ending in a hard break is exactly how you write one in Teams, and ProseMirror marks
    // such a paragraph with the same trailing <br> an empty one gets. The hint must not come back.
    await rich.click();
    await rich.pressSequentially('hello');
    await page.keyboard.down('Shift');
    await page.keyboard.press('Enter');
    await page.keyboard.up('Shift');

    const painted = await page.evaluate(() => {
      const p = document.querySelector('[data-testid="editor-rich"] > p');
      return p ? p.matches(':only-child:has(> br.ProseMirror-trailingBreak:only-child)') : false;
    });
    expect(painted).toBe(false);
    await expect(rich).toContainText('hello');
  });

  test('tables are ruled and padded, not a bare grid of text', async ({ page }) => {
    await openApp(page);

    const md = page.getByTestId('editor-markdown');
    await md.click();
    // The `table-simple` fixture verbatim: the mock only answers for inputs the oracle knows,
    // and a passthrough would render no table at all.
    await md.pressSequentially(
      '| Component | Status |\n| --- | --- |\n| Engine | Done |\n| Clipboard | Blocked |',
    );
    await page.waitForTimeout(1200);

    const cell = page.getByTestId('editor-rich').locator('td').first();
    await expect(cell).toBeVisible();

    const style = await cell.evaluate((el) => {
      const s = getComputedStyle(el);
      return {
        border: parseFloat(s.borderTopWidth),
        padX: parseFloat(s.paddingLeft),
        padY: parseFloat(s.paddingTop),
        collapse: getComputedStyle(el.closest('table')).borderCollapse,
      };
    });

    // Lines and breathing room are most of what makes a grid legible; with neither it barely
    // reads as a table at all, which is how it arrived before anything styled it.
    expect(style.border).toBeGreaterThan(0);
    expect(style.padX).toBeGreaterThan(4);
    expect(style.padY).toBeGreaterThan(2);
    // Without collapse every cell draws its own box and the rules come out doubled.
    expect(style.collapse).toBe('collapse');
  });
});
