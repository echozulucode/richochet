import { StarterKit } from '@tiptap/starter-kit';
import type { Extensions } from '@tiptap/react';

/**
 * The rich editor's schema, restricted to exactly what the document AST can hold.
 *
 * Everything the AST cannot represent must be impossible to create here, otherwise the user can
 * type something that silently disappears on the next conversion. The AST supports:
 *
 *   inline - bold, italic, strike, code, link, hard break
 *   blocks - paragraph, heading 1-3, bullet list, ordered list, blockquote, code block,
 *            horizontal rule
 *
 * Underline is the notable exclusion: Markdown has no underline and the AST has no node for it.
 * trailingNode is off because the phantom paragraph it appends round-trips as a stray blank line.
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
  ];
}
