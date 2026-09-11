import { expect, type Page } from '@playwright/test';

/** The shape the app exposes on `window` when the mock conversion backend is active. */
export interface TestHooks {
  pendingConversions: number;
  lastSeq: number;
  clipboardWrites: Array<{ html: string | null; text: string }>;
  clipboardReads: number;
  /** The scroll-sync outline in use: 1-based source line per top-level block. */
  outlineLines: number[];
  /** Bumped each time an outline response is applied. */
  outlineRevision: number;
  stageClipboard(payload: { kind: 'rich' | 'text'; html?: string; text: string }): void;
}

declare global {
  interface Window {
    __richochet_test?: TestHooks;
  }
}

export interface AppOptions {
  /**
   * Artificial delay on mock conversion responses, in ms.
   *
   * A single value delays every response equally, so they still resolve in order — useless for
   * testing the sequence guard. A comma-separated cycle is applied per request, so `"250,20"`
   * makes request 2 overtake request 1 deterministically.
   */
  latency?: number | string;
  /**
   * Synchronized scrolling. **Off by default**, because it moves both panes and a spec that
   * asserts on pane *content* has no business also being a scroll test. `e2e/scroll-sync.spec.ts`
   * turns it on.
   */
  sync?: boolean;
}

/** Open the app against the mock backend and wait for it to be interactive. */
export async function openApp(page: Page, opts: AppOptions = {}): Promise<void> {
  const params = new URLSearchParams({ backend: 'mock' });
  if (opts.latency !== undefined) params.set('latency', String(opts.latency));
  if (!opts.sync) params.set('sync', 'off');

  await page.goto(`/?${params.toString()}`);

  const root = page.getByTestId('app-root');
  await expect(root).toBeVisible();
  // Guard against silently testing the wrong backend: if Tauri detection ever regressed, these
  // tests would pass against a passthrough and prove nothing.
  await expect(root).toHaveAttribute('data-backend', 'mock');
  await page.waitForFunction(() => window.__richochet_test !== undefined);
}

/** Replace the Markdown pane's entire contents. */
export async function setMarkdown(page: Page, text: string): Promise<void> {
  const editor = page.getByTestId('editor-markdown');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+a');
  // Typing rather than pasting so the debounce and sync path are exercised the way a user does.
  await page.keyboard.press('Delete');
  await editor.pressSequentially(text);
}

/** The Markdown pane's current text. */
export async function markdownText(page: Page): Promise<string> {
  return (await page.getByTestId('editor-markdown').innerText()).replace(/\u00a0/g, ' ');
}

/** The Formatted pane's rendered text, with formatting stripped. */
export async function richText(page: Page): Promise<string> {
  return (await page.getByTestId('editor-rich').innerText()).replace(/\u00a0/g, ' ');
}

/** Wait until no conversion is in flight. */
export async function settled(page: Page): Promise<void> {
  await page.waitForFunction(() => (window.__richochet_test?.pendingConversions ?? 0) === 0);
}

/** Which pane a scroll helper is talking about. */
export type PaneName = 'markdown' | 'rich';

const SCROLLER = { markdown: 'scroller-markdown', rich: 'scroller-rich' } as const;
const EDITOR = { markdown: 'editor-markdown', rich: 'editor-rich' } as const;

/** A pane's scrolling element. Both are real scrollers with a stable test id. */
export function scroller(page: Page, pane: PaneName) {
  return page.getByTestId(SCROLLER[pane]);
}

/** How far a pane is scrolled, in pixels. */
export async function scrollTop(page: Page, pane: PaneName): Promise<number> {
  return scroller(page, pane).evaluate((el) => el.scrollTop);
}

/** How far a pane *can* scroll. Zero means its content fits and there is nothing to test. */
export async function scrollRange(page: Page, pane: PaneName): Promise<number> {
  return scroller(page, pane).evaluate((el) => el.scrollHeight - el.clientHeight);
}

/**
 * Scroll a pane the way a user does — a real wheel event over it, which is also what tells the
 * sync controller which pane is driving.
 */
export async function wheelOver(page: Page, pane: PaneName, deltaY: number): Promise<void> {
  const box = await scroller(page, pane).boundingBox();
  if (!box) throw new Error(`no ${pane} scroller`);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, deltaY);
}

/**
 * Every block number visible in a pane's viewport right now, in document order.
 *
 * The two panes agreeing on a *scroll offset* would prove nothing — the two documents are
 * different heights, which is the entire reason this feature is not just a ratio. Agreeing on
 * which blocks you are looking at is the claim worth testing, so the sample document carries its
 * block number on every source line and this reads it back off the DOM.
 */
export async function visibleBlocks(page: Page, pane: PaneName): Promise<number[]> {
  const selector =
    pane === 'markdown'
      ? '[data-testid="editor-markdown"] .cm-line'
      : '[data-testid="editor-rich"] > *';

  const texts = await page.evaluate(
    ({ sel, scrollerId }) => {
      const box = document.querySelector<HTMLElement>(`[data-testid="${scrollerId}"]`);
      if (!box) return [];
      const view = box.getBoundingClientRect();
      const out: string[] = [];
      for (const el of document.querySelectorAll<HTMLElement>(sel)) {
        const rect = el.getBoundingClientRect();
        // Anything with pixels inside the viewport counts as visible.
        if (rect.bottom > view.top + 1 && rect.top < view.bottom - 1) out.push(el.textContent ?? '');
      }
      return out;
    },
    { sel: selector, scrollerId: SCROLLER[pane] },
  );

  const seen = new Set<number>();
  for (const text of texts) {
    for (const match of text.matchAll(/Block (\d+)/g)) {
      const value = match[1];
      if (value !== undefined) seen.add(Number.parseInt(value, 10));
    }
  }
  return [...seen].sort((a, b) => a - b);
}

/** The topmost block visible in a pane, or null if nothing legible is on screen. */
export async function topVisibleBlock(page: Page, pane: PaneName): Promise<number | null> {
  const blocks = await visibleBlocks(page, pane);
  return blocks[0] ?? null;
}

/** Wait until an outline response has been applied, so sync is not running on a stale map. */
export async function outlineSettled(page: Page): Promise<void> {
  await page.waitForFunction(() => (window.__richochet_test?.outlineRevision ?? 0) > 0);
}

/** Put the caret in a pane without leaving it focused — used by the caret spec. */
export async function focusPane(page: Page, pane: PaneName): Promise<void> {
  await page.getByTestId(EDITOR[pane]).click();
}

/**
 * The scroll-sync sample: 24 top-level blocks over 60 source lines, long enough that both panes
 * genuinely scroll at the default window size.
 *
 * Every source line carries its own block's number, so a spec can name the block it is looking at
 * from either pane without knowing anything about the mapping — including from inside the code
 * block, which is the block whose two renderings differ most in height and therefore the one
 * proportional scrolling gets most wrong.
 *
 * No table, deliberately: the rich editor's schema has no table node, so a Markdown table renders
 * as one paragraph *per row* and the one-node-per-block correspondence scroll sync depends on
 * stops holding. The controller detects that and falls back to proportional, which is the right
 * behaviour but not what this spec is here to measure.
 *
 * Must exist verbatim in `E2E_INPUTS` in `crates/mdcli/src/fixtures.rs`, or the mock answers with
 * a passthrough and no outline.
 */
export const SCROLL_SAMPLE = '# Block 01 heading\n\nBlock 02 paragraph text.\n\nBlock 03 paragraph text.\n\n```text\nBlock 04 code line 01\nBlock 04 code line 02\nBlock 04 code line 03\nBlock 04 code line 04\nBlock 04 code line 05\nBlock 04 code line 06\nBlock 04 code line 07\nBlock 04 code line 08\nBlock 04 code line 09\nBlock 04 code line 10\nBlock 04 code line 11\nBlock 04 code line 12\n```\n\nBlock 05 paragraph text.\n\nBlock 06 paragraph text.\n\nBlock 07 paragraph text.\n\nBlock 08 paragraph text.\n\nBlock 09 paragraph text.\n\nBlock 10 paragraph text.\n\nBlock 11 paragraph text.\n\nBlock 12 paragraph text.\n\nBlock 13 paragraph text.\n\nBlock 14 paragraph text.\n\nBlock 15 paragraph text.\n\nBlock 16 paragraph text.\n\nBlock 17 paragraph text.\n\nBlock 18 paragraph text.\n\nBlock 19 paragraph text.\n\nBlock 20 paragraph text.\n\nBlock 21 paragraph text.\n\nBlock 22 paragraph text.\n\nBlock 23 paragraph text.\n\nBlock 24 paragraph text.';

/** Type the sample into the Markdown pane and wait for everything to come to rest. */
export async function setScrollSample(page: Page): Promise<void> {
  await setMarkdown(page, SCROLL_SAMPLE);
  await settled(page);
  await outlineSettled(page);
  // Typing walks the caret to the bottom, which scrolls both panes. Start from the top.
  await scroller(page, 'markdown').evaluate((el) => {
    el.scrollTop = 0;
  });
  await scroller(page, 'rich').evaluate((el) => {
    el.scrollTop = 0;
  });
}
