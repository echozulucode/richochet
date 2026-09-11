import { describe, expect, it } from 'vitest';
import { Editor } from '@tiptap/core';

import { createExtensions } from './extensions';

/**
 * The schema has to mirror the document AST in both directions.
 *
 * The direction that is easy to forget is AST -> editor: anything the AST can hold must have a
 * node here, or it silently arrives as something else. A `<table>` with no table node in the
 * schema became one paragraph per row, which lost the table on the next conversion *and* knocked
 * every later block out of step with the scroll map, which assumes one top-level node per block.
 */
function editorWith(html: string): Editor {
  return new Editor({ extensions: createExtensions(), content: html });
}

describe('rich editor schema', () => {
  it('parses a table as a single top-level node', () => {
    const editor = editorWith(
      '<h1>H</h1><p>a</p><table><tbody><tr><td>1</td><td>2</td></tr>' +
        '<tr><td>3</td><td>4</td></tr></tbody></table><p>b</p>',
    );

    const types = editor.state.doc.children.map((node) => node.type.name);
    // Four blocks in, four top-level nodes out. Before the table node existed this was six.
    expect(types).toEqual(['heading', 'paragraph', 'table', 'paragraph']);
    editor.destroy();
  });

  it('keeps a table a table through a round trip', () => {
    const editor = editorWith('<table><tbody><tr><td>cell</td></tr></tbody></table>');
    // TipTap adds its own presentation - a min-width style, a <colgroup>, and explicit
    // colspan/rowspan of 1 - so match the tag, not the exact markup.
    expect(editor.getHTML()).toMatch(/<table[ >]/);
    expect(editor.getHTML()).toContain('cell');
    editor.destroy();
  });

  it('keeps table headers distinct from cells', () => {
    const editor = editorWith(
      '<table><thead><tr><th>Head</th></tr></thead><tbody><tr><td>Body</td></tr></tbody></table>',
    );
    const html = editor.getHTML();
    expect(html).toContain('<th');
    expect(html).toContain('<td');
    editor.destroy();
  });

  it('still refuses what the AST cannot hold', () => {
    // Underline has no AST node; it must not survive being pasted in.
    const editor = editorWith('<p><u>underlined</u></p>');
    expect(editor.getHTML()).not.toContain('<u>');
    expect(editor.getText()).toContain('underlined');
    editor.destroy();
  });

  it('one top-level node per block, for the document the scroll specs use', () => {
    // The scroll map indexes ProseMirror's top-level children against the engine's block list, so
    // this correspondence is load-bearing, not incidental.
    const editor = editorWith(
      '<h1>one</h1><p>two</p><ul><li>three</li></ul><blockquote><p>four</p></blockquote>' +
        '<pre><code>five</code></pre><hr><table><tbody><tr><td>six</td></tr></tbody></table>',
    );
    expect(editor.state.doc.childCount).toBe(7);
    editor.destroy();
  });
});
