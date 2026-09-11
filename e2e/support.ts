import { expect, type Page } from "@playwright/test";

/** The shape the app exposes on `window` when the mock conversion backend is active. */
export interface TestHooks {
  pendingConversions: number;
  lastSeq: number;
  clipboardWrites: Array<{ html: string | null; text: string }>;
  clipboardReads: number;
  stageClipboard(payload: {
    kind: "rich" | "text";
    html?: string;
    text: string;
  }): void;
}

declare global {
  interface Window {
    __richochet_test?: TestHooks;
  }
}

export interface AppOptions {
  /** Artificial delay on mock conversion responses, in ms. Used to force races. */
  latency?: number;
}

/** Open the app against the mock backend and wait for it to be interactive. */
export async function openApp(page: Page, opts: AppOptions = {}): Promise<void> {
  const params = new URLSearchParams({ backend: "mock" });
  if (opts.latency !== undefined) params.set("latency", String(opts.latency));

  await page.goto(`/?${params.toString()}`);

  const root = page.getByTestId("app-root");
  await expect(root).toBeVisible();
  // Guard against silently testing the wrong backend: if Tauri detection ever regressed, these
  // tests would pass against a passthrough and prove nothing.
  await expect(root).toHaveAttribute("data-backend", "mock");
  await page.waitForFunction(() => window.__richochet_test !== undefined);
}

/** Replace the Markdown pane's entire contents. */
export async function setMarkdown(page: Page, text: string): Promise<void> {
  const editor = page.getByTestId("editor-markdown");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  // Typing rather than pasting so the debounce and sync path are exercised the way a user does.
  await page.keyboard.press("Delete");
  await editor.pressSequentially(text);
}

/** The Markdown pane's current text. */
export async function markdownText(page: Page): Promise<string> {
  return (await page.getByTestId("editor-markdown").innerText()).replace(/ /g, " ");
}

/** The Formatted pane's rendered text, with formatting stripped. */
export async function richText(page: Page): Promise<string> {
  return (await page.getByTestId("editor-rich").innerText()).replace(/ /g, " ");
}

/** Wait until no conversion is in flight. */
export async function settled(page: Page): Promise<void> {
  await page.waitForFunction(
    () => (window.__richochet_test?.pendingConversions ?? 0) === 0,
  );
}
