/**
 * Put real browser-produced rich text on the clipboard, so `mdcli capture` can turn it into a
 * fixture.
 *
 * Why this exists: Teams Desktop is a WebView2 (Chromium) app, so the HTML it writes to the
 * clipboard is Chromium's — the same `<meta charset>` preamble, the same habit of expressing
 * formatting as inline styles on spans rather than semantic tags. That makes a real Chromium copy
 * the closest available proxy for Teams while Teams itself is unavailable.
 *
 * It is a *proxy*, not the real thing. Fixtures captured this way say so in their notes, and they
 * do not substitute for Phase 1: only a real Teams window can answer what Teams emits, and nothing
 * here says anything at all about what Teams *accepts* on paste.
 *
 * Runs headed on purpose. A real Ctrl+C in a real browser window is what puts real CF_HTML on the
 * OS clipboard; a scripted `document.execCommand` in a headless browser does not reliably.
 *
 * Usage: node tools/capture-web.mjs <file-or-url>
 */
import { chromium } from '@playwright/test';
import { pathToFileURL } from 'node:url';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

const target = process.argv[2];
if (!target) {
  console.error('usage: node tools/capture-web.mjs <file-or-url>');
  process.exit(2);
}

const url = /^https?:\/\//.test(target)
  ? target
  : pathToFileURL(resolve(target)).href;

if (!/^https?:\/\//.test(target) && !existsSync(resolve(target))) {
  console.error(`no such file: ${target}`);
  process.exit(2);
}

const browser = await chromium.launch({ headless: false });
const context = await browser.newContext({
  permissions: ['clipboard-read', 'clipboard-write'],
});
const page = await context.newPage();

await page.goto(url);
await page.waitForLoadState('networkidle').catch(() => {});
await page.bringToFront();

// Select the rendered document and copy it the way a person would.
await page.click('body');
await page.keyboard.press('ControlOrMeta+A');
await page.keyboard.press('ControlOrMeta+C');
// The copy is asynchronous on the browser side; give it a moment to reach the OS clipboard.
await page.waitForTimeout(700);

await browser.close();
console.log(`copied ${url} to the clipboard`);
