//! Inferring semantic marks from presentational HTML.
//!
//! This is where `<span style="font-weight:600">` becomes `Bold`. See
//! `docs/implementation-plan.md` §2.3 for the full signal table.
//!
//! Two properties of this module matter more than the table itself:
//!
//! 1. **A mark can be explicitly *removed*.** Teams nests a `font-weight: normal` span inside a
//!    bold run, and the inner text must come out unbolded. So every field is a tri-state: `None`
//!    means "this element has no opinion", and the opinion of the nearest ancestor that has one
//!    wins.
//! 2. **The CSS parsing is deliberately tolerant.** Clipboard HTML is machine-written by a
//!    program we do not control and has never been validated by anything. A malformed declaration
//!    is skipped, not fatal.

/// The semantic marks a single element contributes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarkSet {
    /// Strong emphasis.
    pub bold: Option<bool>,
    /// Emphasis.
    pub italic: Option<bool>,
    /// Strikethrough.
    pub strike: Option<bool>,
    /// Monospace / code.
    pub code: Option<bool>,
}

impl MarkSet {
    /// A set with no opinion about anything.
    pub fn none() -> Self {
        MarkSet::default()
    }

    /// Layer `child` over `self`: any opinion the child holds overrides the inherited one.
    ///
    /// This is how `font-weight: normal` inside `<b>` cancels the bold, and equally how a
    /// `<span>` that says nothing at all leaves an inherited mark intact.
    #[must_use]
    pub fn merge(self, child: MarkSet) -> MarkSet {
        MarkSet {
            bold: child.bold.or(self.bold),
            italic: child.italic.or(self.italic),
            strike: child.strike.or(self.strike),
            code: child.code.or(self.code),
        }
    }

    /// Whether strong emphasis is in force.
    pub fn is_bold(&self) -> bool {
        self.bold == Some(true)
    }

    /// Whether emphasis is in force.
    pub fn is_italic(&self) -> bool {
        self.italic == Some(true)
    }

    /// Whether strikethrough is in force.
    pub fn is_strike(&self) -> bool {
        self.strike == Some(true)
    }

    /// Whether the text is code.
    pub fn is_code(&self) -> bool {
        self.code == Some(true)
    }
}

/// Font families that mean "this is code" when they appear in a `font-family` declaration.
///
/// The generic `monospace` keyword covers well-behaved producers; the named faces cover the ones
/// that resolve the generic before writing the clipboard. Teams uses Consolas on Windows.
const MONOSPACE_FAMILIES: [&str; 5] = ["monospace", "consolas", "courier", "menlo", "monaco"];

/// Infer marks from a tag name and its `style` attribute.
///
/// The tag is consulted first and the style second, so that
/// `<b style="font-weight:normal">` — which a browser renders unbolded — comes out
/// `bold: Some(false)`.
pub fn marks_for(tag: &str, style: Option<&str>) -> MarkSet {
    let mut marks = marks_for_tag(tag);
    if let Some(style) = style {
        apply_style(&mut marks, style);
    }
    marks
}

/// The marks implied by an element's name alone.
fn marks_for_tag(tag: &str) -> MarkSet {
    let mut marks = MarkSet::none();
    // `eq_ignore_ascii_case` rather than lowercasing: html5ever already lowercases HTML element
    // names, but `marks_for` is public and callers should not have to know that.
    let is = |name: &str| tag.eq_ignore_ascii_case(name);
    if is("b") || is("strong") {
        marks.bold = Some(true);
    }
    if is("i") || is("em") {
        marks.italic = Some(true);
    }
    if is("s") || is("del") || is("strike") {
        marks.strike = Some(true);
    }
    if is("code") || is("tt") || is("kbd") || is("samp") {
        marks.code = Some(true);
    }
    marks
}

/// Fold the declarations of a `style` attribute into `marks`.
fn apply_style(marks: &mut MarkSet, style: &str) {
    for (property, value) in declarations(style) {
        match property.as_str() {
            "font-weight" => {
                if let Some(bold) = weight_is_bold(&value) {
                    marks.bold = Some(bold);
                }
            }
            "font-style" => match value.as_str() {
                "italic" | "oblique" => marks.italic = Some(true),
                "normal" => marks.italic = Some(false),
                _ => {}
            },
            // `text-decoration` is the shorthand and `text-decoration-line` the longhand; Teams
            // has been seen emitting both.
            "text-decoration" | "text-decoration-line" => {
                if value.split_whitespace().any(|word| word == "line-through") {
                    marks.strike = Some(true);
                } else if value == "none" {
                    marks.strike = Some(false);
                }
            }
            "font-family" => {
                if MONOSPACE_FAMILIES
                    .iter()
                    .any(|family| value.contains(family))
                {
                    marks.code = Some(true);
                }
                // A proportional font is deliberately *not* treated as `code: Some(false)`.
                // Producers set `font-family` on nearly every element for reasons that have
                // nothing to do with code, so reading it as a removal would cancel real
                // `<code>` marks far more often than it would help.
            }
            _ => {}
        }
    }
}

/// Interpret a `font-weight` value: `Some(true)` for bold, `Some(false)` for explicitly
/// not-bold, `None` for a value we have no opinion about (`inherit`, garbage, ...).
///
/// The numeric threshold is 600 because that — not 700 — is what Teams writes for bold.
fn weight_is_bold(value: &str) -> Option<bool> {
    match value {
        "bold" | "bolder" => return Some(true),
        "normal" | "lighter" => return Some(false),
        _ => {}
    }
    // A bare number. Anything at or above 600 is bold; an explicit lighter weight removes an
    // inherited bold, which is what makes Teams' `font-weight: 400` spans work.
    value.parse::<f32>().ok().map(|weight| weight >= 600.0)
}

/// Split a `style` attribute into `(property, value)` pairs.
///
/// Tolerant by design: property names and values are lowercased and trimmed, empty and
/// colon-less declarations are dropped, and a trailing `;` is fine. Values keep their internal
/// structure (`Consolas, "Courier New", monospace` stays one value) because `font-family`
/// matching needs it.
pub fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            if property.is_empty() || value.is_empty() {
                return None;
            }
            Some((property, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold(tag: &str, style: &str) -> Option<bool> {
        marks_for(tag, Some(style)).bold
    }

    #[test]
    fn semantic_tags_imply_their_marks() {
        assert_eq!(marks_for("strong", None).bold, Some(true));
        assert_eq!(marks_for("b", None).bold, Some(true));
        assert_eq!(marks_for("em", None).italic, Some(true));
        assert_eq!(marks_for("i", None).italic, Some(true));
        assert_eq!(marks_for("del", None).strike, Some(true));
        assert_eq!(marks_for("strike", None).strike, Some(true));
        assert_eq!(marks_for("s", None).strike, Some(true));
        assert_eq!(marks_for("code", None).code, Some(true));
        assert_eq!(marks_for("kbd", None).code, Some(true));
        assert_eq!(marks_for("samp", None).code, Some(true));
        assert_eq!(marks_for("tt", None).code, Some(true));
    }

    #[test]
    fn an_unremarkable_tag_has_no_opinion() {
        assert_eq!(marks_for("span", None), MarkSet::none());
        assert_eq!(marks_for("div", Some("color: red")), MarkSet::none());
    }

    #[test]
    fn font_weight_600_is_bold() {
        // The exact signal Teams emits.
        assert_eq!(bold("span", "font-weight:600"), Some(true));
        assert_eq!(bold("span", "font-weight: 700"), Some(true));
        assert_eq!(bold("span", "font-weight:bold"), Some(true));
        assert_eq!(bold("span", "font-weight: BOLDER"), Some(true));
    }

    #[test]
    fn font_weight_normal_removes_bold() {
        assert_eq!(bold("span", "font-weight:normal"), Some(false));
        assert_eq!(bold("span", "font-weight:400"), Some(false));
        assert_eq!(bold("span", "font-weight: lighter"), Some(false));
        // Even on a tag that would otherwise be bold: this is what a browser renders.
        assert_eq!(bold("b", "font-weight:normal"), Some(false));
    }

    #[test]
    fn an_uninterpretable_weight_is_no_opinion() {
        assert_eq!(bold("span", "font-weight:inherit"), None);
        assert_eq!(bold("span", "font-weight:"), None);
        assert_eq!(bold("b", "font-weight:nonsense"), Some(true));
    }

    #[test]
    fn font_style_signals_italic_both_ways() {
        assert_eq!(marks_for("span", Some("font-style:italic")).italic, Some(true));
        assert_eq!(
            marks_for("span", Some("font-style: oblique")).italic,
            Some(true)
        );
        assert_eq!(
            marks_for("i", Some("font-style:normal")).italic,
            Some(false)
        );
    }

    #[test]
    fn line_through_anywhere_in_text_decoration_is_strike() {
        assert_eq!(
            marks_for("span", Some("text-decoration: underline line-through")).strike,
            Some(true)
        );
        assert_eq!(
            marks_for("span", Some("text-decoration-line:line-through")).strike,
            Some(true)
        );
        assert_eq!(
            marks_for("s", Some("text-decoration:none")).strike,
            Some(false)
        );
        // Underline alone is not strikethrough, and the model has no underline mark.
        assert_eq!(
            marks_for("span", Some("text-decoration:underline")).strike,
            None
        );
    }

    #[test]
    fn monospace_font_families_are_code() {
        for family in [
            "monospace",
            "Consolas, monospace",
            "\"Courier New\", serif",
            "Menlo",
            "Monaco",
        ] {
            let style = format!("font-family:{family}");
            assert_eq!(
                marks_for("span", Some(&style)).code,
                Some(true),
                "{family} should read as code"
            );
        }
        assert_eq!(
            marks_for("span", Some("font-family: Segoe UI, sans-serif")).code,
            None
        );
    }

    #[test]
    fn the_declaration_splitter_is_tolerant() {
        assert_eq!(
            declarations("  FONT-WEIGHT :  600 ; font-style:italic ;"),
            vec![
                ("font-weight".to_string(), "600".to_string()),
                ("font-style".to_string(), "italic".to_string()),
            ]
        );
        // Malformed pairs are skipped rather than poisoning the rest.
        assert_eq!(
            declarations("garbage; font-weight:600; :nothing; empty:"),
            vec![("font-weight".to_string(), "600".to_string())]
        );
        assert_eq!(declarations(""), vec![]);
    }

    #[test]
    fn several_signals_combine_in_one_attribute() {
        let marks = marks_for(
            "span",
            Some("font-weight:600;font-style:italic;text-decoration:line-through"),
        );
        assert_eq!(
            marks,
            MarkSet {
                bold: Some(true),
                italic: Some(true),
                strike: Some(true),
                code: None,
            }
        );
    }

    #[test]
    fn merge_lets_the_child_win_only_where_it_has_an_opinion() {
        let outer = MarkSet {
            bold: Some(true),
            italic: Some(true),
            ..MarkSet::none()
        };
        let inner = marks_for("span", Some("font-weight:normal"));
        let merged = outer.merge(inner);
        assert!(!merged.is_bold());
        assert!(merged.is_italic());
    }
}
