import { expect, test } from "@playwright/test";
import { markdownText, openApp, richText, setMarkdown, settled } from "./support";

/**
 * The sync engine is the hardest part of the frontend and the part unit tests reach least well.
 * Two editors that each regenerate the other will loop, fight over the caret, and corrupt input;
 * these tests assert the three rules from docs/implementation-plan.md §3.1 hold in a real browser.
 */
test.describe("live sync", () => {
  test("Markdown flows to the Formatted pane", async ({ page }) => {
    await openApp(page);
    await setMarkdown(page, "**Hello Eric**");
    await settled(page);

    await expect(page.getByTestId("editor-rich")).toContainText("Hello Eric");
    await expect(page.getByTestId("editor-rich").locator("strong")).toHaveText(
      "Hello Eric",
    );
  });

  test("editing the Formatted pane flows back to Markdown", async ({ page }) => {
    await openApp(page);

    const rich = page.getByTestId("editor-rich");
    await rich.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("Delete");
    await rich.pressSequentially("plain words");
    await settled(page);

    await expect(page.getByTestId("editor-markdown")).toContainText("plain words");
  });

  test("the unfocused pane never steals the caret during fast typing", async ({
    page,
  }) => {
    await openApp(page);

    const editor = page.getByTestId("editor-markdown");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("Delete");

    // Type fast enough that several conversions overlap. If the derived pane writes back, or a
    // stale response is applied, characters get reordered or dropped.
    const typed = "the quick brown fox jumps over the lazy dog";
    await editor.pressSequentially(typed, { delay: 8 });
    await settled(page);

    expect((await markdownText(page)).trim()).toBe(typed);
  });

  test("a stale conversion response is dropped, not applied", async ({ page }) => {
    // With latency, an early request can resolve after a later one. The sequence guard must
    // discard it; without the guard the pane flickers back to an earlier state.
    await openApp(page, { latency: 120 });

    const editor = page.getByTestId("editor-markdown");
    await editor.click();
    await editor.pressSequentially("abcdef", { delay: 10 });
    await settled(page);

    await expect(page.getByTestId("editor-rich")).toContainText("abcdef");

    const seq = await page.evaluate(() => window.__richochet_test?.lastSeq ?? 0);
    expect(seq).toBeGreaterThan(0);
  });

  test("typing does not grow the document on every keystroke", async ({ page }) => {
    // Non-idempotent conversion shows up here first: the text creeps as the user types.
    await openApp(page);
    await setMarkdown(page, "- one\n- two");
    await settled(page);

    const first = await markdownText(page);

    await page.getByTestId("editor-rich").click();
    await settled(page);
    await page.getByTestId("editor-markdown").click();
    await settled(page);

    expect(await markdownText(page)).toBe(first);
  });
});
