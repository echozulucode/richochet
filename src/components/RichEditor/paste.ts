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

    if (rich) {
      const html = payload.html ?? '';
      if (html.length === 0) return;
      const markdown = await backend.convert(html, 'html', 'markdown');
      documentStore.getState().replaceDocument(markdown);
      toastStore.getState().show('Pasted rich text');
      return;
    }

    // Plain text on the clipboard is *Markdown source* as far as this app is concerned, so it goes
    // straight into the document without a conversion.
    //
    // It used to be converted `text -> markdown`, which escapes every Markdown character —
    // `**bold**` arrived as `\*\*bold\*\*` and rendered as literal asterisks. That silently broke
    // half the point of the app: `docs/plan.md` lists "paste or type Markdown, see an accurate
    // rich preview" as an MVP requirement.
    //
    // The trade is that prose containing a stray `*` or `_` is now read as emphasis. That is the
    // right way round for a Markdown tool: the Markdown pane shows exactly what was understood, so
    // a misreading is visible and fixable, whereas escaping everything made the common case
    // impossible.
    if (payload.text.length === 0) return;
    documentStore.getState().replaceDocument(payload.text);
    toastStore.getState().show('Pasted Markdown');
  } catch (error) {
    toastStore.getState().show(error instanceof Error ? error.message : String(error), 'error');
  }
}
