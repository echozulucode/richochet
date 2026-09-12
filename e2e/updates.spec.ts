import { expect, test } from '@playwright/test';
import { openApp } from './support';

/**
 * The update UX under `?backend=mock`: a browser with no Tauri runtime, which is also the closest
 * thing we have to "offline". The launch check must fail down the silent path - one warning, no
 * error, no dot - and the Updates group must still render, inert.
 */
test.describe('updates group', () => {
  test('renders inert under the mock backend, with no dot and no console error', async ({
    page,
  }) => {
    const errors: string[] = [];
    const updateWarnings: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(msg.text());
      if (msg.type() === 'warning' && msg.text().startsWith('[update]'))
        updateWarnings.push(msg.text());
    });
    page.on('pageerror', (err) => errors.push(err.message));

    await openApp(page);

    await page.getByTestId('theme-toggle').click();
    const group = page.getByTestId('updates-group');
    await expect(group).toBeVisible();
    await expect(group).toContainText('Updates');

    const status = page.getByTestId('update-status');
    // The launch check has nowhere to go without Tauri; wait for it to land in `error`.
    await expect(status).toHaveAttribute('data-update-state', 'error');
    // ...which looks exactly like "nothing to do": the bare name, since the version is unreadable.
    await expect(status).toHaveText('Richochet');
    expect(await status.evaluate((el) => el.tagName)).not.toBe('BUTTON');
    await expect(page.getByRole('menuitem')).toHaveCount(0);
    await expect(page.getByTestId('update-dot')).toHaveCount(0);

    // Clicking it does nothing: the menu stays open and the state does not move.
    await status.click();
    await expect(page.getByTestId('settings-menu')).toBeVisible();
    await expect(status).toHaveAttribute('data-update-state', 'error');

    // The theme group next door still works.
    await page.getByTestId('theme-dark').click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

    expect(errors).toEqual([]);
    // Logged once, not once per render or per StrictMode mount.
    expect(updateWarnings).toHaveLength(1);
  });
});
