import { StarterKit } from '@tiptap/starter-kit';
import { TableKit } from '@tiptap/extension-table';
import type { Extensions } from '@tiptap/react';

/**
 * The rich editor's schema, restricted to exactly what the document AST can hold.
 *
 * Everything the AST cannot represent must be impossible to create here, otherwise the user can
 * type something that silently disappears on the next conversion. The AST supports:
 *
 *   inline - bold, italic, strike, code, link, hard break
 *   blocks - paragraph, heading 1-3, bullet list, ordered list, blockquote, code block,
 *            horizontal rule, table
 *
 * Underline is the notable exclusion: Markdown has no underline and the AST has no node for it.
 * trailingNode is off because the phantom paragraph it appends round-trips as a stray blank line.
 *
 * The converse matters just as much: anything the AST *can* hold must have a node here, or it
 * arrives as something else. Without a table node ProseMirror parsed a `<table>` into one
 * paragraph per row — which lost the table on the next conversion, and knocked every block after
 * it out of step with the scroll map, since that map assumes one top-level node per AST block.
 */
export function createExtensions(): Extensions {
  return [
    StarterKit.configure({
      // Kept, narrowed.
      heading: { levels: [1, 2, 3] },
      link: { openOnClick: false, autolink: true, protocols: ['http', 'https', 'mailto'] },
      codeBlock: { languageClassPrefix: 'language-' },

      // Not representable in the AST.
      underline: false,
      trailingNode: false,
    }),

    // `Block::Table` is a real block in the AST, so the editor needs a node for it. `resizable` is
    // off: column widths are presentation the AST cannot carry, so offering the handle would let
    // the user make an adjustment that vanishes on the next conversion.
    TableKit.configure({
      table: { resizable: false, allowTableNodeSelection: true },
    }),
  ];
}
