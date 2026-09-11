import { useEffect, useRef } from 'react';
import { markdown as markdownLanguage } from '@codemirror/lang-markdown';
import { EditorState } from '@codemirror/state';
import { EditorView, placeholder } from '@codemirror/view';
import { minimalSetup } from 'codemirror';

import { documentStore, useDocumentStore } from '../../stores/documentStore';

/**
 * The Markdown pane.
 *
 * No line numbers, no gutter, no fold markers - the Markdown is the content, not a code file.
 * Derived text arrives through markdownRevision; a full-document replace keeps the caret where
 * it was if it still fits.
 */
export function MarkdownEditor(): React.JSX.Element {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  const markdown = useDocumentStore((state) => state.markdown);
  const markdownRevision = useDocumentStore((state) => state.markdownRevision);
  const appliedRevision = useRef(-1);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;

    const instance = new EditorView({
      parent,
      state: EditorState.create({
        doc: documentStore.getState().markdown,
        extensions: [
          minimalSetup,
          markdownLanguage(),
          EditorView.lineWrapping,
          placeholder('# Type Markdown here'),
          EditorView.contentAttributes.of({
            'data-testid': 'editor-markdown',
            'aria-label': 'Markdown',
          }),
          EditorView.updateListener.of((update) => {
            if (update.focusChanged && update.view.hasFocus) {
              documentStore.getState().focusPane('markdown');
            }
            if (update.docChanged) {
              documentStore.getState().editPane('markdown', update.state.doc.toString());
            }
          }),
        ],
      }),
    });
    view.current = instance;
    appliedRevision.current = documentStore.getState().markdownRevision;

    return () => {
      instance.destroy();
      view.current = null;
    };
  }, []);

  useEffect(() => {
    const instance = view.current;
    if (!instance) return;
    if (appliedRevision.current === markdownRevision) return;
    appliedRevision.current = markdownRevision;
    const current = instance.state.doc.toString();
    if (current === markdown) return;
    const anchor = Math.min(instance.state.selection.main.anchor, markdown.length);
    instance.dispatch({
      changes: { from: 0, to: current.length, insert: markdown },
      selection: { anchor },
    });
  }, [markdown, markdownRevision]);

  return <div ref={host} className="markdown-surface h-full overflow-y-auto px-6 py-4" />;
}
