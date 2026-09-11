import { useEffect, useMemo, useRef } from 'react';
import { EditorContent, useEditor } from '@tiptap/react';

import { documentStore, useDocumentStore } from '../../stores/documentStore';
import { createExtensions } from './extensions';
import { pasteFromClipboard } from './paste';

/**
 * The Formatted pane.
 *
 * View state only: the canonical text lives in the store. Derived HTML is pushed in through the
 * htmlRevision counter, which only moves when something other than this editor changed the
 * document - so typing here never fights the caret.
 */
export function RichEditor(): React.JSX.Element {
  const extensions = useMemo(() => createExtensions(), []);
  const html = useDocumentStore((state) => state.html);
  const htmlRevision = useDocumentStore((state) => state.htmlRevision);
  const appliedRevision = useRef(-1);

  const editor = useEditor({
    extensions,
    content: '',
    editorProps: {
      attributes: {
        'data-testid': 'editor-rich',
        'aria-label': 'Formatted',
        class: 'tiptap',
      },
      handlePaste: (_view, event) => {
        // The WebView cannot see CF_HTML; the Rust side reads the real clipboard instead.
        event.preventDefault();
        void pasteFromClipboard();
        return true;
      },
    },
    onFocus: () => {
      documentStore.getState().focusPane('rich');
    },
    onUpdate: ({ editor: instance }) => {
      documentStore.getState().editPane('rich', instance.getHTML());
    },
  });

  useEffect(() => {
    if (!editor) return;
    if (appliedRevision.current === htmlRevision) return;
    appliedRevision.current = htmlRevision;
    // Rule 2 belt and braces: emitUpdate false, and the store's suppression flag is already set.
    editor.commands.setContent(html, { emitUpdate: false });
  }, [editor, html, htmlRevision]);

  return (
    <div className="rich-surface h-full overflow-y-auto px-6 py-4 text-ink">
      <EditorContent editor={editor} className="h-full" />
    </div>
  );
}
