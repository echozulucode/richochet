//! The Windows `HTML Format` clipboard wrapper.
//!
//! Windows does not put raw HTML on the clipboard. It puts a *CF_HTML block*: a small plain-text
//! header of `Key:value` lines followed by a complete HTML document, where the header carries
//! **byte offsets** into the block itself.
//!
//! ```text
//! Version:0.9
//! StartHTML:0000000105
//! EndHTML:0000000186
//! StartFragment:0000000141
//! EndFragment:0000000150
//! <html>
//! <body>
//! <!--StartFragment--><b>hi</b><!--EndFragment-->
//! </body>
//! </html>
//! ```
//!
//! The offsets are counted in **bytes from the first byte of the block**, zero-padded to a fixed
//! width. Because the width is fixed, the header's own length is a constant and the offsets can be
//! computed before the header is written — see [`HEADER_LEN`]. Getting this wrong is the classic
//! cause of "the paste is blank in one app and fine in another".
//!
//! # Scope
//!
//! This module is **pure and platform-independent**. It is not on the hot clipboard path — on
//! Windows, `clipboard-win`'s `raw::get_html` / `raw::set_html` already handle the header. It
//! exists so captured fixture bytes can be decoded anywhere (including a non-Windows CI box), so
//! what Teams actually emits can be inspected, and so the encoding is pinned by byte-exact tests.

use super::ClipError;

/// The fixed `Version` line, including its CRLF.
const VERSION_LINE: &str = "Version:0.9\r\n";

/// The markup that precedes the fragment, immediately after the header.
const BODY_HEADER: &str = "<html>\r\n<body>\r\n<!--StartFragment-->";

/// The markup that follows the fragment, closing the document.
const BODY_FOOTER: &str = "<!--EndFragment-->\r\n</body>\r\n</html>";

/// Width of a zero-padded offset field, in digits. Fixed by the CF_HTML convention.
const OFFSET_WIDTH: usize = 10;

/// Byte length of one `Key:0000000000\r\n` header line.
const fn field_len(key: &str) -> usize {
    // key + ':' + digits + "\r\n"
    key.len() + 1 + OFFSET_WIDTH + 2
}

/// Byte length of the header [`encode`] emits — and therefore the value of `StartHTML`.
///
/// Constant precisely because every offset field is zero-padded to [`OFFSET_WIDTH`] digits, so the
/// offsets can be computed before a single byte of the header is written.
pub const HEADER_LEN: usize = VERSION_LINE.len()
    + field_len("StartHTML")
    + field_len("EndHTML")
    + field_len("StartFragment")
    + field_len("EndFragment");

/// Wrap an HTML fragment in a CF_HTML header with correct byte offsets.
///
/// The returned block is exactly `EndHTML` bytes long, and `block[StartFragment..EndFragment]` is
/// byte-for-byte `fragment`. Line endings in the wrapper are CRLF; the fragment is copied verbatim.
pub fn encode(fragment: &str) -> String {
    use std::fmt::Write as _;

    let start_html = HEADER_LEN;
    let start_fragment = start_html + BODY_HEADER.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + BODY_FOOTER.len();

    let mut out = String::with_capacity(end_html);
    out.push_str(VERSION_LINE);
    // Writing to a String is infallible, hence the discarded results.
    let _ = write!(out, "StartHTML:{start_html:0w$}\r\n", w = OFFSET_WIDTH);
    let _ = write!(out, "EndHTML:{end_html:0w$}\r\n", w = OFFSET_WIDTH);
    let _ = write!(
        out,
        "StartFragment:{start_fragment:0w$}\r\n",
        w = OFFSET_WIDTH
    );
    let _ = write!(out, "EndFragment:{end_fragment:0w$}\r\n", w = OFFSET_WIDTH);
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.push_str(BODY_HEADER);
    out.push_str(fragment);
    out.push_str(BODY_FOOTER);
    debug_assert_eq!(out.len(), end_html);
    out
}

/// Strip a CF_HTML header and return the fragment between the fragment markers.
///
/// Real producers are sloppy, so this is deliberately forgiving and tries, in order:
///
/// 1. the `<!--StartFragment-->` / `<!--EndFragment-->` comments;
/// 2. the `StartFragment` / `EndFragment` header offsets (any digit width, clamped into range and
///    snapped to UTF-8 character boundaries so a wrong offset cannot panic);
/// 3. the `<body>` … `</body>` element (matched case-insensitively, so Word's
///    `<body lang=EN-US>` is handled);
/// 4. everything after the header lines — which is the whole input when there was no header.
///
/// A missing `Version` line is tolerated. The only error is an empty input.
///
/// # Why the comments outrank the offsets
///
/// The numeric offsets are exactly the thing producers get wrong — they are computed from a header
/// whose length the producer has to predict, and Word in particular ships blocks whose offsets are
/// off by tens of bytes. Some producers also point `StartFragment` *at* the comment rather than
/// past it. The comment markers carry no arithmetic, so when both are present and they disagree,
/// the comments are believed. When a block has no comments, the offsets are used as given.
pub fn decode(raw: &str) -> Result<String, ClipError> {
    if raw.is_empty() {
        return Err(ClipError::Encoding {
            encoding: "CF_HTML",
            detail: "the block was empty".to_owned(),
        });
    }

    let header = Header::parse(raw);

    if let Some(found) = by_comments(raw, header.body_offset) {
        return Ok(found.to_owned());
    }
    if let Some(found) = by_offsets(raw, &header) {
        return Ok(found.to_owned());
    }
    if let Some(found) = by_body(raw, header.body_offset) {
        return Ok(found.to_owned());
    }
    Ok(raw[header.body_offset..].to_owned())
}

/// The header fields [`decode`] cares about, plus where the header stopped.
#[derive(Debug, Default)]
struct Header {
    start_fragment: Option<usize>,
    end_fragment: Option<usize>,
    /// Byte offset of the first line that was not a `Key:value` header line.
    body_offset: usize,
}

impl Header {
    /// Consume `Key:value` lines from the front of `raw` until the markup starts.
    fn parse(raw: &str) -> Self {
        let mut header = Header::default();
        let mut pos = 0usize;

        while pos < raw.len() {
            let rest = &raw[pos..];
            let (line, next) = match rest.find('\n') {
                Some(i) => (&rest[..i], pos + i + 1),
                None => (rest, raw.len()),
            };
            let line = line.trim_end_matches('\r');

            // The markup begins, or the line is not a header line at all.
            if line.is_empty() || line.starts_with('<') {
                break;
            }
            // `SourceURL:https://…` contains further colons; only the first one separates.
            let Some((key, value)) = line.split_once(':') else {
                break;
            };
            match key {
                "StartFragment" => header.start_fragment = parse_offset(value),
                "EndFragment" => header.end_fragment = parse_offset(value),
                // Version, StartHTML, EndHTML, SourceURL and vendor keys are all tolerated and
                // ignored: the fragment bounds are the only thing we need.
                _ => {}
            }
            pos = next;
        }

        header.body_offset = pos;
        header
    }
}

/// Parse a header offset, accepting any zero-padded width and rejecting non-numeric values.
///
/// Some producers write `-1` for an absent fragment marker; that yields `None`.
fn parse_offset(value: &str) -> Option<usize> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<usize>().ok()
}

/// Slice out the fragment using the header offsets, clamping anything out of range.
fn by_offsets<'a>(raw: &'a str, header: &Header) -> Option<&'a str> {
    let start = header.start_fragment?;
    let end = header.end_fragment?;
    let start = floor_boundary(raw, start.min(raw.len()));
    let end = ceil_boundary(raw, end.min(raw.len()).max(start));
    if start >= end {
        return None;
    }
    Some(&raw[start..end])
}

/// Slice out the fragment using the `<!--StartFragment-->` / `<!--EndFragment-->` comments.
///
/// The comments are matched loosely (`<!--StartFragment` up to the next `-->`) because some
/// producers pad them, e.g. `<!--StartFragment -->`.
fn by_comments(raw: &str, body_offset: usize) -> Option<&str> {
    let has_start = raw.contains("<!--StartFragment");
    let start = match raw.find("<!--StartFragment") {
        Some(i) => i + raw[i..].find("-->")? + 3,
        None => body_offset,
    };
    let end = match raw[start..].find("<!--EndFragment") {
        Some(i) => start + i,
        // A start marker with no end marker: take the rest of the block.
        None if has_start => raw.len(),
        None => return None,
    };
    if start > end {
        return None;
    }
    // `start == end` is legitimate: an empty fragment.
    Some(&raw[start..end])
}

/// Slice out the contents of the `<body>` element.
fn by_body(raw: &str, body_offset: usize) -> Option<&str> {
    let open = find_ascii_ci(raw, "<body", body_offset)?;
    let start = open + raw[open..].find('>')? + 1;
    let end = find_ascii_ci(raw, "</body", start).unwrap_or(raw.len());
    if start >= end {
        return None;
    }
    Some(&raw[start..end])
}

/// ASCII-case-insensitive substring search starting at `from`.
///
/// `needle` must be ASCII; since it is, every match position is a UTF-8 character boundary, so the
/// returned index is always safe to slice at.
fn find_ascii_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    debug_assert!(needle.is_ascii());
    let from = from.min(haystack.len());
    let hay = haystack.as_bytes();
    let need = needle.as_bytes();
    if need.is_empty() || hay.len() < need.len() {
        return None;
    }
    (from..=hay.len() - need.len()).find(|&i| hay[i..i + need.len()].eq_ignore_ascii_case(need))
}

/// Round `i` down to the nearest UTF-8 character boundary.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `i` up to the nearest UTF-8 character boundary.
fn ceil_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header length is a constant; if this changes, every offset in every test moves.
    #[test]
    fn header_length_is_the_documented_constant() {
        assert_eq!(HEADER_LEN, 105);
    }

    #[test]
    fn encode_is_byte_exact() {
        let expected = concat!(
            "Version:0.9\r\n",
            "StartHTML:0000000105\r\n",
            "EndHTML:0000000186\r\n",
            "StartFragment:0000000141\r\n",
            "EndFragment:0000000150\r\n",
            "<html>\r\n",
            "<body>\r\n",
            "<!--StartFragment--><b>hi</b><!--EndFragment-->\r\n",
            "</body>\r\n",
            "</html>",
        );
        assert_eq!(encode("<b>hi</b>"), expected);
    }

    #[test]
    fn encoded_offsets_address_the_real_bytes() {
        let fragment = "<p>one</p><p>two</p>";
        let block = encode(fragment);
        let header = Header::parse(&block);
        let start = header.start_fragment.expect("StartFragment");
        let end = header.end_fragment.expect("EndFragment");
        assert_eq!(&block[start..end], fragment);
        assert_eq!(&block[HEADER_LEN..HEADER_LEN + 6], "<html>");
        // EndHTML is the total length of the block.
        assert!(block.contains(&format!("EndHTML:{:010}\r\n", block.len())));
    }

    #[test]
    fn round_trips() {
        for fragment in [
            "<b>hi</b>",
            "plain text with no markup",
            "<p>a</p>\r\n<p>b</p>",
            "<ul><li>one</li><li>two<ul><li>deep</li></ul></li></ul>",
            "<a href=\"https://example.com/?a=1&amp;b=2\">link</a>",
            "<pre><code class=\"lang-rust\">fn main() {}\n</code></pre>",
            "<!-- a comment that is not a fragment marker -->",
        ] {
            let block = encode(fragment);
            assert_eq!(decode(&block).unwrap(), fragment, "fragment: {fragment}");
        }
    }

    #[test]
    fn an_empty_fragment_round_trips_to_empty() {
        // Zero-length fragments have no bytes to point at, so the offset path declines and the
        // comment fallback still produces the empty string.
        let block = encode("");
        assert!(block.contains("StartFragment:0000000141\r\n"));
        assert!(block.contains("EndFragment:0000000141\r\n"));
        assert_eq!(decode(&block).unwrap(), "");
    }

    #[test]
    fn offsets_are_bytes_not_chars() {
        // "café 🎉" is 6 chars but 11 bytes; wrapped, the fragment is 17 bytes / 13 chars.
        let fragment = "<p>café 🎉</p>";
        assert_eq!(fragment.chars().count(), 13);
        assert_eq!(fragment.len(), 17);

        let block = encode(fragment);
        let header = Header::parse(&block);
        let start = header.start_fragment.unwrap();
        let end = header.end_fragment.unwrap();
        assert_eq!(end - start, 17, "offsets must count bytes, not chars");
        assert_eq!(&block[start..end], fragment);
        assert_eq!(decode(&block).unwrap(), fragment);
    }

    #[test]
    fn round_trips_multibyte_fragments() {
        for fragment in [
            "<p>naïve café</p>",
            "<p>🎉🎉🎉</p>",
            "<p>日本語のテキスト</p>",
            "<p>Ω≈ç√∫˜µ≤≥÷</p>",
            "<p>combining: e\u{0301} — zwj: 👨\u{200d}👩\u{200d}👧\u{200d}👦</p>",
        ] {
            let block = encode(fragment);
            assert_eq!(decode(&block).unwrap(), fragment, "fragment: {fragment}");
        }
    }

    #[test]
    fn decodes_a_chromium_shaped_blob() {
        // Chromium adds a SourceURL line, so StartHTML sits past the 105-byte minimum. The offsets
        // below are computed, not guessed, so this blob is genuinely self-consistent.
        let body = concat!(
            "<html>\r\n",
            "<body>\r\n",
            "<!--StartFragment--><b>bold</b> and <i>italic</i><!--EndFragment-->\r\n",
            "</body>\r\n",
            "</html>",
        );
        let fragment = "<b>bold</b> and <i>italic</i>";
        let header_len = concat!(
            "Version:0.9\r\n",
            "StartHTML:0000000000\r\n",
            "EndHTML:0000000000\r\n",
            "StartFragment:0000000000\r\n",
            "EndFragment:0000000000\r\n",
            "SourceURL:https://example.com/page\r\n",
        )
        .len();
        let start_html = header_len;
        let start_fragment = start_html + "<html>\r\n<body>\r\n<!--StartFragment-->".len();
        let end_fragment = start_fragment + fragment.len();
        let end_html = header_len + body.len();
        let blob = format!(
            "Version:0.9\r\n\
             StartHTML:{start_html:010}\r\n\
             EndHTML:{end_html:010}\r\n\
             StartFragment:{start_fragment:010}\r\n\
             EndFragment:{end_fragment:010}\r\n\
             SourceURL:https://example.com/page\r\n\
             {body}"
        );
        assert_eq!(blob.len(), end_html);
        assert_eq!(decode(&blob).unwrap(), fragment);
    }

    #[test]
    fn decodes_a_word_shaped_blob_with_eight_digit_offsets_and_no_version() {
        // Word-flavoured: no Version line, 8-digit offsets, unquoted attributes on <body>. The
        // offsets here are wrong, as Word's frequently are, so the comment fallback must rescue it.
        let blob = concat!(
            "StartHTML:00000097\r\n",
            "EndHTML:00000260\r\n",
            "StartFragment:00000133\r\n",
            "EndFragment:00000160\r\n",
            "<html>\r\n",
            "<body lang=EN-US style='tab-interval:.5in'>\r\n",
            "<!--StartFragment--><p class=MsoNormal>hi</p><!--EndFragment-->\r\n",
            "</body>\r\n",
            "</html>",
        );
        assert_eq!(
            decode(blob).unwrap(),
            "<p class=MsoNormal>hi</p>",
            "the offsets were wrong; the comments must win"
        );
    }

    #[test]
    fn eight_digit_offsets_are_honoured_when_there_are_no_comments() {
        let head = "StartFragment:00000046\r\nEndFragment:00000055\r\n";
        assert_eq!(head.len(), 46);
        let blob = format!("{head}<b>hi</b>");
        assert_eq!(blob.len(), 55);
        assert_eq!(decode(&blob).unwrap(), "<b>hi</b>");
    }

    #[test]
    fn falls_back_to_comments_when_offsets_are_missing() {
        let blob = concat!(
            "Version:0.9\r\n",
            "StartHTML:0000000045\r\n",
            "EndHTML:0000000150\r\n",
            "<html><body><!--StartFragment--><em>x</em><!--EndFragment--></body></html>",
        );
        assert_eq!(decode(blob).unwrap(), "<em>x</em>");
    }

    #[test]
    fn tolerates_padded_fragment_comments() {
        let blob = "<html><body><!--StartFragment --><em>x</em><!--EndFragment --></body></html>";
        assert_eq!(decode(blob).unwrap(), "<em>x</em>");
    }

    #[test]
    fn falls_back_to_body_when_there_are_no_comments() {
        let blob = concat!(
            "Version:0.9\r\n",
            "StartFragment:abc\r\n",
            "<html><body><p>only a body</p></body></html>",
        );
        assert_eq!(decode(blob).unwrap(), "<p>only a body</p>");
    }

    #[test]
    fn falls_back_to_the_whole_string() {
        assert_eq!(decode("<p>bare</p>").unwrap(), "<p>bare</p>");
        assert_eq!(decode("not markup at all").unwrap(), "not markup at all");
    }

    #[test]
    fn malformed_input_never_panics() {
        let cases = [
            "Version:0.9\r\n",
            "Version:0.9\r\nStartFragment:9999999999\r\nEndFragment:0000000001\r\n<html></html>",
            "StartFragment:0000000000\r\nEndFragment:9999999999\r\n<html><body>x</body></html>",
            "StartFragment:-1\r\nEndFragment:-1\r\n<html><body>x</body></html>",
            "StartFragment:\r\nEndFragment:\r\n",
            ":::::",
            "\r\n\r\n\r\n",
            "<body>",
            "</body>",
            "<!--StartFragment-->",
            "<!--EndFragment-->",
            "<!--StartFragment", // truncated marker, no closing `-->`
            "Version:0.9\r\nStartFragment:0000000003\r\nEndFragment:0000000005\r\n🎉🎉🎉",
            "\u{feff}Version:0.9\r\n<html><body>bom</body></html>",
            "<BODY><P>UPPERCASE</P></BODY>",
        ];
        for case in cases {
            let decoded = decode(case);
            assert!(decoded.is_ok(), "decode({case:?}) errored: {decoded:?}");
        }
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(decode("").is_err());
    }

    #[test]
    fn offsets_landing_inside_a_multibyte_char_are_snapped_not_panicked() {
        // The header is 50 bytes; "🎉" is 4. Offsets 51..53 land inside the first emoji.
        let head = "StartFragment:0000000051\r\nEndFragment:0000000053\r\n";
        assert_eq!(head.len(), 50);
        let blob = format!("{head}🎉🎉");
        let decoded = decode(&blob).expect("must not panic");
        // Snapping outwards keeps whole characters, so the first emoji survives intact.
        assert_eq!(decoded, "🎉");
    }
}
