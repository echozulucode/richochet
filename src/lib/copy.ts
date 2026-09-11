import { getBackend } from './converter';
import { documentStore } from '../stores/documentStore';

/** What a copy button puts on the clipboard. */
export type CopyTarget = 'teams' | 'markdown' | 'text';

/**
 * Render the document and put it on the clipboard.
 *
 * Always renders from the owning pane's text through the engine, never from whatever the editor
 * happens to be showing — the editors hold view state, the store holds the document.
 *
 * `teams` is the only target that puts HTML on the clipboard, and it always writes a plain-text
 * fallback alongside it in the same operation: Teams takes the rich flavour, anything else gets
 * readable text.
 */
export async function copyToClipboard(target: CopyTarget): Promise<void> {
  const backend = getBackend();
  const { input, from } = documentStore.getState().canonical();

  if (target === 'teams') {
    const [html, text] = await Promise.all([
      backend.convert(input, from, 'html'),
      backend.convert(input, from, 'text'),
    ]);
    await backend.writeClipboard({ html, text });
    return;
  }

  const text = await backend.convert(input, from, target === 'markdown' ? 'markdown' : 'text');
  await backend.writeClipboard({ html: null, text });
}
