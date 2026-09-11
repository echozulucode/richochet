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
});
