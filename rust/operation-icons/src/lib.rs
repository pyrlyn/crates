//! Icons for what a status-line verb is doing, and the gutter they sit in.
//!
//! The crate is tone-agnostic: a caller that has its own fallback icon per
//! line kind (success, warning, ...) keeps that table and uses this one first.

use unicode_width::UnicodeWidthStr;

/// Icons for what a verb is doing, looked up before a line kind's own icon.
///
/// Matched as substrings of the lowercased verb, in order, so `installing`,
/// `installed` and `already installed` share one row. `uninstall` comes before
/// `install` because it contains it.
pub const OPERATION_ICONS: &[(&str, &str)] = &[
    ("uninstall", "🧹"),
    ("remov", "🧹"),
    ("prun", "🧹"),
    ("install", "📦"),
    ("upgrad", "⏫"),
    ("updat", "⏫"),
    ("download", "⏬"),
    ("fetch", "⏬"),
    ("link", "🔗"),
    ("roll", "⏪"),
    ("search", "🔍"),
    ("doctor", "🩺"),
];

/// Columns every icon is padded to. Every icon is one code point that is wide
/// by default: a narrow symbol made wide by U+FE0F (`⬆️`, `⚠️`) is two columns
/// to `unicode-width` but one to a terminal that ignores the selector, which
/// then draws the glyph over the space after it and shifts the verb left. Each
/// icon is still measured rather than assumed, so a narrower one added later
/// pads out instead of pulling its line left.
pub const ICON_WIDTH: usize = 2;

/// The icon of the first row of [`OPERATION_ICONS`] whose key `verb` contains.
pub fn icon(verb: &str) -> Option<&'static str> {
    icon_in(OPERATION_ICONS, verb)
}

/// Like [`icon`] over a caller's own rows, so a program can add verbs of its
/// own after the shared ones without the shared table growing for everyone.
pub fn icon_in(rows: &[(&str, &'static str)], verb: &str) -> Option<&'static str> {
    let verb = verb.to_ascii_lowercase();
    rows.iter()
        .find(|(key, _)| verb.contains(key))
        .map(|(_, icon)| *icon)
}

/// Blanks that bring `icon` up to [`ICON_WIDTH`] columns.
pub fn padding(icon: &str) -> usize {
    ICON_WIDTH.saturating_sub(UnicodeWidthStr::width(icon))
}

/// The gutter in front of a verb column: the icon, its padding and one space,
/// or as many blanks when the line has no icon, so lines with and without one
/// align.
pub fn gutter(icon: Option<&str>) -> String {
    let icon = icon.unwrap_or("");
    format!("{icon}{} ", " ".repeat(padding(icon)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_verb_gets_its_icon() {
        for (verb, want) in [
            ("uninstalling", "🧹"),
            ("removed", "🧹"),
            ("pruning", "🧹"),
            ("installed", "📦"),
            ("upgrading", "⏫"),
            ("updated", "⏫"),
            ("downloading", "⏬"),
            ("fetching", "⏬"),
            ("linked", "🔗"),
            ("rolled back", "⏪"),
            ("searching", "🔍"),
            ("doctor", "🩺"),
        ] {
            assert_eq!(icon(verb), Some(want), "{verb}");
        }
    }

    #[test]
    fn uninstall_wins_over_install_and_case_is_ignored() {
        assert_eq!(icon("Uninstalled"), Some("🧹"));
        assert_eq!(icon("ALREADY INSTALLED"), Some("📦"));
    }

    #[test]
    fn unknown_verb_has_no_icon() {
        assert_eq!(icon("thinking"), None);
        assert_eq!(icon(""), None);
    }

    #[test]
    fn caller_rows_extend_the_lookup() {
        let rows = [("index", "📚")];
        assert_eq!(icon_in(&rows, "indexing"), Some("📚"));
        assert_eq!(icon_in(&rows, "installing"), None);
    }

    #[test]
    fn every_icon_fills_the_same_width() {
        for (_, icon) in OPERATION_ICONS {
            assert_eq!(UnicodeWidthStr::width(*icon), ICON_WIDTH, "{icon}");
            assert_eq!(padding(icon), 0, "{icon}");
        }
    }

    #[test]
    fn narrow_icon_is_padded_and_empty_is_blank() {
        assert_eq!(padding("→"), 1);
        assert_eq!(gutter(Some("→")), "→  ");
        assert_eq!(gutter(Some("📦")), "📦 ");
        assert_eq!(gutter(None), "   ");
    }

    #[test]
    fn gutters_with_and_without_icon_have_equal_width() {
        let with = UnicodeWidthStr::width(gutter(icon("install")).as_str());
        let without = UnicodeWidthStr::width(gutter(None).as_str());
        assert_eq!(with, without);
        assert_eq!(with, ICON_WIDTH + 1);
    }
}
