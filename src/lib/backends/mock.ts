import { loadOracle, oracleKey, outlineKey } from '../oracle';
import { sanitizeOutline } from '../scrollMapping';
import { bumpPending, recordClipboardWrite, takeStagedClipboard } from '../testHook';
import type { ClipboardPayload, ConversionBackend, OutboundPayload, WireFormat } from '../types';

/** Tuning knobs, all driven from the URL so Playwright can set them per test. */
export interface MockOptions {
  /**
   * Per-request delay in milliseconds, cycled across successive conversions.
   * A single value delays everything equally; a list such as `[200, 10]` makes the second
   * response overtake the first, which is how the sequence guard is exercised deterministically.
   */
  latency?: number[];
}

const warned = new Set<string>();

function warnOnce(key: string, message: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[richochet:mock] ${message}`);
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function stripTags(value: string): string {
  return value
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/(p|div|li|h[1-6]|blockquote|pre)>/gi, '\n\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&nbsp;/g, ' ')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, '&')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/**
 * What the mock returns when the oracle has no entry.
 *
 * Deliberately marked: HTML output carries `data-richochet-mock="passthrough"` so a test (or a
 * human squinting at the Formatted pane) can tell real engine output from a stand-in. Markdown
 * and text output cannot carry a marker without corrupting the document, so those only warn.
 */
export function passthrough(input: string, from: WireFormat, to: WireFormat): string {
  if (to === from) return input;
  if (to === 'html') {
    const blocks = input.split(/\n{2,}/).filter((block) => block.trim().length > 0);
    if (blocks.length === 0) return '';
    return blocks
      .map(
        (block) =>
          `<p data-richochet-mock="passthrough">${escapeHtml(block).replace(/\n/g, '<br>')}</p>`,
      )
      .join('\n');
  }
  return from === 'html' ? stripTags(input) : input;
}

function delay(ms: number): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/**
 * The browser-only backend. Conversions resolve from a table of real engine output; the
 * clipboard is simulated through `window.__richochet_test`.
 */
export function createMockBackend(options: MockOptions = {}): ConversionBackend {
  const latencies = options.latency?.length ? options.latency : [0];
  let call = 0;

  return {
    name: 'mock',

    async convert(input: string, from: WireFormat, to: WireFormat): Promise<string> {
      const wait = latencies[call % latencies.length] ?? 0;
      call += 1;
      bumpPending(1);
      try {
        await delay(wait);
        const oracle = await loadOracle();
        const key = oracleKey(from, to, input);
        const hit = oracle[key];
        if (typeof hit === 'string') return hit;
        warnOnce(
          `${from}:${to}`,
          `no oracle entry for ${from} -> ${to}; using passthrough. ` +
            `Run \`cargo run -p mdcli -- export-fixtures\` to regenerate src/test-support/oracle.json.`,
        );
        return passthrough(input, from, to);
      } finally {
        bumpPending(-1);
      }
    },

    /**
     * The scroll-sync outline, from the same table.
     *
     * A miss is not an error and must not reject: the controller has no way to recover from a
     * thrown scroll handler, and an empty outline is exactly the signal it already understands —
     * "no structural anchors here, scroll proportionally instead".
     */
    async outline(markdown: string): Promise<number[]> {
      const oracle = await loadOracle();
      const hit = oracle[outlineKey(markdown)];
      if (Array.isArray(hit)) return sanitizeOutline(hit);
      warnOnce(
        'outline',
        `no oracle outline for this document; scroll sync falls back to proportional. ` +
          `Run \`cargo run -p mdcli -- export-fixtures\` to regenerate src/test-support/oracle.json.`,
      );
      return [];
    },

    async readClipboard(): Promise<ClipboardPayload> {
      const stagedPayload = takeStagedClipboard();
      if (stagedPayload) return stagedPayload;
      // Nothing staged: fall back to the browser clipboard's plain text, which is all a
      // sandboxed page can reliably read.
      try {
        const text = (await navigator.clipboard?.readText()) ?? '';
        return { kind: 'text', html: null, rtf: null, text };
      } catch {
        return { kind: 'text', html: null, rtf: null, text: '' };
      }
    },

    async writeClipboard(payload: OutboundPayload): Promise<void> {
      recordClipboardWrite({ html: payload.html, text: payload.text });
      try {
        await navigator.clipboard?.writeText(payload.text);
      } catch {
        // A browser without clipboard permission is fine; the write is still recorded.
      }
    },
  };
}
