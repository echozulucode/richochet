import { expect, test } from "@playwright/test";
import { openApp } from "./support";

test.describe("layout", () => {
  test("survives the minimum window size without horizontal scroll", async ({
    page,
  }) => {
    // tauri.conf.json sets minWidth 480 / minHeight 360.
    await page.setViewportSize({ width: 480, height: 360 });
    await openApp(page);

    const overflows = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
    );
    expect(overflows).toBe(false);

    await expect(page.getByTestId("copy-teams")).toBeVisible();
  });

  test("collapses to a single pane when narrow", async ({ page }) => {
    await page.setViewportSize({ width: 520, height: 700 });
    await openApp(page);

    // Below the breakpoint only one pane is shown at a time, with a way to switch.
    const panes = page.locator(
      '[data-testid="pane-formatted"], [data-testid="pane-markdown"]',
    );
    const visible = await panes.evaluateAll(
      (els) => els.filter((el) => (el as HTMLElement).offsetParent !== null).length,
    );
    expect(visible).toBe(1);
  });

  test("shows both panes side by side at the default size", async ({ page }) => {
    await openApp(page);

    const formatted = await page.getByTestId("pane-formatted").boundingBox();
    const markdown = await page.getByTestId("pane-markdown").boundingBox();
    expect(formatted).not.toBeNull();
    expect(markdown).not.toBeNull();

    // Formatted on the left, Markdown on the right, per docs/plan.md.
    expect(formatted!.x).toBeLessThan(markdown!.x);
    // Roughly equal widths to start.
    expect(Math.abs(formatted!.width - markdown!.width)).toBeLessThan(40);
  });

  test("the divider resizes the panes", async ({ page }) => {
    await openApp(page);

    const divider = page.getByTestId("divider");
    await expect(divider).toBeVisible();

    const before = (await page.getByTestId("pane-formatted").boundingBox())!.width;
    const box = (await divider.boundingBox())!;

    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 150, box.y + box.height / 2, {
      steps: 10,
    });
    await page.mouse.up();

    const after = (await page.getByTestId("pane-formatted").boundingBox())!.width;
    expect(after).toBeGreaterThan(before + 80);
  });
});
