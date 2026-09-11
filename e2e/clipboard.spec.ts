import { expect, test } from "@playwright/test";
import { openApp, setMarkdown, settled } from "./support";

test.describe("copy actions", () => {
  test("Copy for Teams writes HTML and a plain-text fallback together", async ({
    page,
  }) => {
    await openApp(page);
    await setMarkdown(page, "**bold** and _italic_");
    await settled(page);

    await page.getByTestId("copy-teams").click();

    const writes = await page.evaluate(
      () => window.__richochet_test?.clipboardWrites ?? [],
    );
    expect(writes).toHaveLength(1);

    // Both representations must go on in one operation, or Teams gets only the plain text.
    expect(writes[0].html).toContain("<strong>bold</strong>");
    expect(writes[0].text).toContain("bold");
    expect(writes[0].text).not.toContain("<strong>");
  });

  test("Copy Markdown writes the Markdown source, not HTML", async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, "**bold**");
    await settled(page);

    await page.getByTestId("copy-markdown").click();

    const writes = await page.evaluate(
      () => window.__richochet_test?.clipboardWrites ?? [],
    );
    expect(writes).toHaveLength(1);
    expect(writes[0].html).toBeNull();
    expect(writes[0].text).toContain("**bold**");
  });

  test("Copy Plain Text strips every mark", async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, "**Important**");
    await settled(page);

    await page.getByTestId("copy-text").click();

    const writes = await page.evaluate(
      () => window.__richochet_test?.clipboardWrites ?? [],
    );
    expect(writes).toHaveLength(1);
    expect(writes[0].html).toBeNull();
    // docs/plan.md: **Important** becomes Important.
    expect(writes[0].text.trim()).toBe("Important");
  });

  test("copying shows a transient confirmation", async ({ page }) => {
    await openApp(page);
    await page.getByTestId("copy-markdown").click();

    const toast = page.getByTestId("toast");
    await expect(toast).toBeVisible();
    // "Nothing more intrusive" — it must go away on its own.
    await expect(toast).toBeHidden({ timeout: 8_000 });
  });
});

test.describe("paste detection", () => {
  test("pasting rich content produces Markdown and says so", async ({ page }) => {
    await openApp(page);

    await page.evaluate(() => {
      window.__richochet_test?.stageClipboard({
        kind: "rich",
        html: '<div><span style="font-weight:600"> Important </span></div>',
        text: "Important",
      });
    });

    await page.getByTestId("editor-rich").click();
    await page.keyboard.press("ControlOrMeta+v");
    await settled(page);

    await expect(page.getByTestId("editor-markdown")).toContainText("**Important**");
    await expect(page.getByTestId("toast")).toContainText(/rich text/i);
  });

  test("pasting plain content says plain text", async ({ page }) => {
    await openApp(page);

    await page.evaluate(() => {
      window.__richochet_test?.stageClipboard({ kind: "text", text: "just words" });
    });

    await page.getByTestId("editor-rich").click();
    await page.keyboard.press("ControlOrMeta+v");
    await settled(page);

    await expect(page.getByTestId("editor-markdown")).toContainText("just words");
    await expect(page.getByTestId("toast")).toContainText(/plain text/i);
  });
});
