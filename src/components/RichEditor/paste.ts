import { getBackend } from '../../lib/converter';
import { documentStore } from '../../stores/documentStore';
import { toastStore } from '../../stores/toastStore';

/**
 * Handle a paste into the Formatted pane.
 *
 * The browser's own paste data is deliberately ignored: on Windows the interesting
 * representation is CF_HTML, and the Rust side is the only place that can read it without the
 * WebView mangling it first. So we ask the backend what is really on the clipboard, convert the
 * richest representation it found to Markdown, and make that the document.
 */
export async function pasteFromClipboard(): Promise<void> {
  const backend = getBackend();
  try {
    const payload = await backend.readClipboard();
    const rich = payload.kind === 'rich' && payload.html !== null && payload.html.length > 0;
    const input = rich ? (payload.html ?? '') : payload.text;
    if (input.length === 0) return;
    const markdown = await backend.convert(input, rich ? 'html' : 'text', 'markdown');
    documentStore.getState().replaceDocument(markdown);
    toastStore.getState().show(rich ? 'Pasted rich text' : 'Pasted plain text');
  } catch (error) {
    toastStore.getState().show(error instanceof Error ? error.message : String(error), 'error');
  }
}
