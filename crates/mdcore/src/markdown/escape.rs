//! Contextual Markdown escaping.
//!
//! Over-escaping is ugly but correct; under-escaping is silently wrong. Prefer the former.
//!
//! Two contexts matter when emitting a text node:
//!
//! * [`escape_inline`] — the text is somewhere in the middle of a line. Only the *inline*
//!   constructs are dangerous here: emphasis, code spans, links, autolinks, entities.
//! * [`escape_line_start`] — the text is the first thing on its own line, so the *block* markers
//!   are dangerous too. This matters after every soft or hard break as well as at the start of a
//!   paragraph, because an ATX heading, a list marker, a blockquote marker or a setext underline
//!   can all interrupt a paragraph on a continuation line.
//!
//! `escape_line_start` is a superset of `escape_inline`: it applies the inline rules first and
//! then, if the result still begins with a block marker, escapes that too. Callers therefore pick
//! exactly one of the two.
//!
//! # What gets escaped, and why
//!
//! | Character | Escaped | Reason |
//! |---|---|---|
//! | backslash | always, first | otherwise it would eat the next character |
//! | backtick | always | would open a code span |
//! | `*` | always | would open emphasis (`*` is valid intraword, so there is no safe case) |
//! | `_` | unless intraword | CommonMark forbids intraword `_` emphasis, so `snake_case` is safe |
//! | `[` `]` | always | would open a link, image or footnote reference |
//! | `<` | always | would open an autolink or raw HTML |
//! | `&` | always | would open a character reference |
//! | `~` | always | would open GFM strikethrough |
//!
//! `(`, `)`, `!`, `|` and `.` are deliberately **not** escaped in inline context: they are only
//! meaningful next to a construct whose opener (`[`, a `|` delimiter row, a digit at line start)
//! is already escaped or already impossible. `!` in particular is harmless because `[` is always
//! escaped, so `![` can never form an image.
//!
//! Every character escaped here is ASCII punctuation, which is exactly the set CommonMark allows a
//! backslash escape to apply to.

/// Characters that are always escaped in inline context.
const ALWAYS: &[char] = &['\\', '`', '*', '[', ']', '<', '&', '~'];

/// Escape text appearing in an inline context.
///
/// `_` is left alone when it sits between two alphanumerics, because CommonMark does not treat
/// intraword `_` as emphasis — so `snake_case_word` survives unescaped, which is the single most
/// visible escaping decision in the whole program.
///
/// ```
/// use mdcore::markdown::escape::escape_inline;
///
/// assert_eq!(escape_inline("snake_case_word"), "snake_case_word");
/// assert_eq!(escape_inline("_lead"), "\\_lead");
/// assert_eq!(escape_inline("a*b"), "a\\*b");
/// ```
pub fn escape_inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if ALWAYS.contains(&c) {
            out.push('\\');
            out.push(c);
        } else if c == '_' {
            if !is_intraword(&chars, i) {
                out.push('\\');
            }
            out.push('_');
        } else {
            out.push(c);
        }
    }
    out
}

/// True when the character at `i` has an alphanumeric neighbour on both sides.
///
/// A node boundary counts as "not alphanumeric", which over-escapes rather than under-escapes:
/// the renderer escapes each text node independently and cannot see across marks.
fn is_intraword(chars: &[char], i: usize) -> bool {
    let before = i
        .checked_sub(1)
        .and_then(|j| chars.get(j))
        .is_some_and(|c| c.is_alphanumeric());
    let after = chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
    before && after
}

/// Escape text appearing at the start of a line, where block markers are significant.
///
/// Applies [`escape_inline`], then neutralizes a leading block marker: `#` (ATX heading), `>`
/// (blockquote), `-` and `+` (bullet, thematic break or setext underline), `=` (setext
/// underline), and a run of digits followed by `.` or `)` (ordered list marker). `*` and `_`
/// thematic breaks and `~` code fences are already handled by the inline pass.
///
/// Up to three leading spaces are skipped before looking for the marker, because CommonMark
/// allows a block marker to be indented that far. Four or more leading spaces would make the line
/// an indented code block, which no backslash can prevent — parsers never produce a text node
/// shaped like that, so the renderer does not try to.
///
/// ```
/// use mdcore::markdown::escape::escape_line_start;
///
/// assert_eq!(escape_line_start("# not a heading"), "\\# not a heading");
/// assert_eq!(escape_line_start("1. not a list"), "1\\. not a list");
/// ```
pub fn escape_line_start(text: &str) -> String {
    let escaped = escape_inline(text);
    let chars: Vec<char> = escaped.chars().collect();

    // Skip the up-to-three leading spaces CommonMark tolerates before a block marker.
    let mut i = 0;
    while i < chars.len() && i < 3 && chars[i] == ' ' {
        i += 1;
    }
    let Some(&first) = chars.get(i) else {
        return escaped;
    };

    let insert_at = if matches!(first, '#' | '>' | '-' | '+' | '=') {
        Some(i)
    } else if first.is_ascii_digit() {
        let mut j = i;
        while chars.get(j).is_some_and(char::is_ascii_digit) {
            j += 1;
        }
        match chars.get(j) {
            Some('.' | ')') => Some(j),
            _ => None,
        }
    } else {
        None
    };

    match insert_at {
        Some(at) => {
            let mut out: String = chars[..at].iter().collect();
            out.push('\\');
            out.extend(&chars[at..]);
            out
        }
        None => escaped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_backslash_first_so_it_does_not_eat_the_next_escape() {
        assert_eq!(escape_inline(r"\*"), r"\\\*");
    }

    #[test]
    fn escapes_emphasis_and_code_and_link_punctuation() {
        assert_eq!(escape_inline("a*b`c[d]e"), r"a\*b\`c\[d\]e");
    }

    #[test]
    fn escapes_autolink_and_entity_openers() {
        assert_eq!(escape_inline("<b> &amp;"), r"\<b\> \&amp;");
    }

    #[test]
    fn escapes_tilde_so_strikethrough_does_not_reappear() {
        assert_eq!(escape_inline("~~nope~~"), r"\~\~nope\~\~");
    }

    #[test]
    fn snake_case_underscores_stay_unescaped() {
        assert_eq!(escape_inline("snake_case_word"), "snake_case_word");
        assert_eq!(escape_inline("a_1_b"), "a_1_b");
    }

    #[test]
    fn word_boundary_underscores_are_escaped() {
        assert_eq!(escape_inline("_lead"), r"\_lead");
        assert_eq!(escape_inline("trail_"), r"trail\_");
        assert_eq!(escape_inline("a _b_ c"), r"a \_b\_ c");
        assert_eq!(escape_inline("__bold__"), r"\_\_bold\_\_");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(
            escape_inline("Hello, world! (really)"),
            "Hello, world! (really)"
        );
    }

    #[test]
    fn line_start_escapes_atx_headings() {
        assert_eq!(escape_line_start("# not a heading"), r"\# not a heading");
        assert_eq!(escape_line_start("### deep"), r"\### deep");
    }

    #[test]
    fn line_start_escapes_blockquote_and_bullets() {
        assert_eq!(escape_line_start("> not a quote"), r"\> not a quote");
        assert_eq!(escape_line_start("- not a bullet"), r"\- not a bullet");
        assert_eq!(escape_line_start("+ not a bullet"), r"\+ not a bullet");
    }

    #[test]
    fn line_start_escapes_setext_underlines() {
        assert_eq!(escape_line_start("=== not a heading"), r"\=== not a heading");
        assert_eq!(escape_line_start("---"), r"\---");
    }

    #[test]
    fn line_start_escapes_ordered_list_markers() {
        assert_eq!(escape_line_start("1. not a list"), r"1\. not a list");
        assert_eq!(escape_line_start("42) nope"), r"42\) nope");
        // A digit not followed by a delimiter is harmless.
        assert_eq!(escape_line_start("2026 was a year"), "2026 was a year");
    }

    #[test]
    fn line_start_respects_up_to_three_spaces_of_indent() {
        assert_eq!(escape_line_start("   # indented"), r"   \# indented");
        // Four spaces is an indented code block, which escaping cannot undo; leave it alone.
        assert_eq!(escape_line_start("    # deep"), "    # deep");
    }

    #[test]
    fn line_start_also_applies_the_inline_rules() {
        assert_eq!(escape_line_start("* a [b]"), r"\* a \[b\]");
    }

    #[test]
    fn line_start_on_empty_text_is_empty() {
        assert_eq!(escape_line_start(""), "");
    }

    #[test]
    fn a_hash_that_is_not_at_line_start_is_left_alone() {
        assert_eq!(escape_inline("still #1"), "still #1");
    }
}
