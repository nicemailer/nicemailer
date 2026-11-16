//! The fonts, sizes and colours of the formatted mode of the editor
//! (`specifications/edition.md`, section 5, "Polices", "Tailles",
//! "Couleurs").
//!
//! The sizes and the colours are closed lists, defined here once: the core
//! keeps only these values when it cleans the HTML of a mail to send
//! (`nm-store`, `mail_html.rs`), and the app hands the same lists to the
//! editor (`compose_formats`), so that the editor offers exactly what the
//! core lets through.
//!
//! The fonts are open: the editor offers every font of the Mac, which the
//! core cannot list (it may run elsewhere, a draft may come from another
//! machine). The core keeps any font whose *name* is safe in CSS
//! ([`font_stack`]), and [`FONTS`] only gives the fonts found on every
//! system their fallbacks, so that a reader without Georgia still reads a
//! serif.
//!
//! Each value is also *normalized* here: a pasted HTML or a draft written
//! elsewhere says the same font or colour in many ways (`"Georgia", serif`,
//! `rgb(47, 111, 209)`, `#2F6FD1`); [`font_stack`] and [`palette_color`]
//! map each spelling to the one canonical value, or to nothing.

use std::borrow::Cow;

/// A font with known fallbacks: its name, and the CSS stack sent, with
/// fallbacks for a reader that lacks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Font {
    pub name: &'static str,
    pub stack: &'static str,
}

/// The fonts found on Windows, macOS, iOS and Android, and rendered by
/// Gmail, Outlook and Apple Mail, with their stacks. Any other font leaves
/// as its name and the generic family the editor gave it
/// (`'Avenir Next', sans-serif`). "Default" (no font at all, the reader's
/// own) is the absence of a value.
pub const FONTS: [Font; 8] = [
    Font { name: "Arial", stack: "Arial, Helvetica, sans-serif" },
    Font { name: "Helvetica", stack: "Helvetica, Arial, sans-serif" },
    Font { name: "Verdana", stack: "Verdana, Geneva, sans-serif" },
    Font { name: "Tahoma", stack: "Tahoma, Verdana, sans-serif" },
    Font { name: "Trebuchet MS", stack: "'Trebuchet MS', Helvetica, sans-serif" },
    Font { name: "Georgia", stack: "Georgia, 'Times New Roman', serif" },
    Font { name: "Times New Roman", stack: "'Times New Roman', Times, serif" },
    Font { name: "Courier New", stack: "'Courier New', Courier, monospace" },
];

/// A size of the list: its key, the number shown in the menu (`18`), and
/// the CSS value sent. "Normal" is the absence of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub key: &'static str,
    pub css: &'static str,
}

/// The sizes of Apple Mail's menu, in pixels: the unit every reader
/// renders the same, Outlook included. A list rather than any number: a
/// size pasted from a page (`17.5px`, `1.2em`, `larger`) would otherwise
/// leave as written, and the menu could not show it.
pub const SIZES: [Size; 15] = [
    Size { key: "9", css: "9px" },
    Size { key: "10", css: "10px" },
    Size { key: "11", css: "11px" },
    Size { key: "12", css: "12px" },
    Size { key: "13", css: "13px" },
    Size { key: "14", css: "14px" },
    Size { key: "18", css: "18px" },
    Size { key: "24", css: "24px" },
    Size { key: "36", css: "36px" },
    Size { key: "48", css: "48px" },
    Size { key: "64", css: "64px" },
    Size { key: "72", css: "72px" },
    Size { key: "96", css: "96px" },
    Size { key: "144", css: "144px" },
    Size { key: "288", css: "288px" },
];

/// A colour of the palette: a key the app translates, and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub key: &'static str,
    pub hex: &'static str,
}

/// Eight hues of middle lightness, readable on a white page **and** on a
/// dark one: the reader may read in a dark theme, and its client does not
/// always change the colours chosen. No black and no white: "Default"
/// leaves the reader's text colour. Each has a contrast of at least 3:1
/// against white and against `#1e1e1e` (checked by a test below).
pub const COLORS: [Color; 8] = [
    Color { key: "gray", hex: "#7a7a7a" },
    Color { key: "red", hex: "#d6312b" },
    Color { key: "orange", hex: "#c8660a" },
    Color { key: "green", hex: "#2e8b47" },
    Color { key: "teal", hex: "#13857f" },
    Color { key: "blue", hex: "#2f6fd1" },
    Color { key: "purple", hex: "#8f52d6" },
    Color { key: "pink", hex: "#c93d8c" },
];

/// The generic families of CSS, kept as the fallback of a font.
const GENERICS: [&str; 5] = ["serif", "sans-serif", "monospace", "cursive", "system-ui"];

/// The longest font name kept, in characters (the longest of a Mac is
/// about 40).
const MAX_FONT_NAME: usize = 80;

/// Whether `name` can be written in a `style` attribute as a font name:
/// letters (any script), digits, spaces and `- . _ &`. No quote, `;`,
/// parenthesis or backslash, which could end the value or the attribute,
/// and no leading `-` or `.` (`-apple-system`, `.SF NS`: names of the
/// system, not fonts a reader has).
fn safe_font_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_alphanumeric());
    first_ok
        && name.chars().count() <= MAX_FONT_NAME
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.' | '_' | '&'))
}

/// The canonical stack of the font a `font-family` value names. Its first
/// family decides, quotes and case ignored:
///
/// - one of [`FONTS`]: its stack (`"Georgia", serif` and `georgia` both
///   give Georgia's);
/// - any other font with a safe name: `'Name'`, then the generic family
///   the value ends with, if any (`"Avenir Next", sans-serif` gives
///   `'Avenir Next', sans-serif`), as the editor writes it;
/// - a name that is not safe, a generic family alone: `None`.
///
/// Rust note: `Cow<'static, str>` ("clone on write") is either a borrowed
/// `&'static str` (a stack of the table, no allocation) or an owned
/// `String` (a stack built here); the caller reads both alike.
pub fn font_stack(value: &str) -> Option<Cow<'static, str>> {
    let unquote = |f: &str| f.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string();
    let families: Vec<String> = value.split(',').map(unquote).collect();
    let first = families.first()?;
    if let Some(f) = FONTS.iter().find(|f| f.name.eq_ignore_ascii_case(first)) {
        return Some(Cow::Borrowed(f.stack));
    }
    if !safe_font_name(first) || GENERICS.iter().any(|g| g.eq_ignore_ascii_case(first)) {
        return None;
    }
    let generic = families.last().filter(|_| families.len() > 1).and_then(|l| GENERICS.iter().find(|g| g.eq_ignore_ascii_case(l)));
    Some(Cow::Owned(match generic {
        Some(g) => format!("'{first}', {g}"),
        None => format!("'{first}'"),
    }))
}

/// The CSS value of a size, if `value` is one of [`SIZES`] (`18px`, any
/// case and spacing).
pub fn size_value(value: &str) -> Option<&'static str> {
    let v = value.trim();
    SIZES.iter().find(|s| s.css.eq_ignore_ascii_case(v)).map(|s| s.css)
}

/// The palette colour a CSS colour value is, if it is one: `#2f6fd1`,
/// `#2F6FD1` and `rgb(47, 111, 209)` all give `#2f6fd1`. Any other colour
/// (another hue, a name, a transparency) gives `None`.
pub fn palette_color(value: &str) -> Option<&'static str> {
    let hex = to_hex(value.trim())?;
    COLORS.iter().find(|c| c.hex == hex).map(|c| c.hex)
}

/// `#rrggbb` (lowercase) of a CSS colour written `#rgb`, `#rrggbb` or
/// `rgb(r, g, b)` (what a browser gives for `element.style.color`).
fn to_hex(v: &str) -> Option<String> {
    let v = v.to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        let full: String = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect(),
            6 => h.to_string(),
            _ => return None,
        };
        return full.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{full}"));
    }
    let inner = v.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_known_by_their_first_family() {
        let stack = |v| font_stack(v).map(|s| s.into_owned());
        assert_eq!(stack("Georgia").as_deref(), Some("Georgia, 'Times New Roman', serif"));
        assert_eq!(stack("\"Trebuchet MS\", Helvetica, sans-serif").as_deref(), Some("'Trebuchet MS', Helvetica, sans-serif"));
        assert_eq!(stack("courier new").as_deref(), Some("'Courier New', Courier, monospace"));
        // Any font of the Mac: its name, and the generic family it ends with.
        assert_eq!(stack("'Avenir Next', sans-serif").as_deref(), Some("'Avenir Next', sans-serif"));
        assert_eq!(stack("\"Hiragino Kaku Gothic ProN\", Arial, serif").as_deref(), Some("'Hiragino Kaku Gothic ProN', serif"));
        assert_eq!(stack("Comic Sans MS, cursive").as_deref(), Some("'Comic Sans MS', cursive"));
        assert_eq!(stack("Baskerville").as_deref(), Some("'Baskerville'"));
        assert_eq!(stack("Didot, fantasy").as_deref(), Some("'Didot'"));
    }

    #[test]
    fn unsafe_font_names_are_dropped() {
        assert_eq!(font_stack(""), None);
        assert_eq!(font_stack("serif"), None);
        assert_eq!(font_stack("-apple-system, sans-serif"), None);
        assert_eq!(font_stack(".SF NS"), None);
        assert_eq!(font_stack("x); background: url(http://e.fr/t.png"), None);
        assert_eq!(font_stack("a\\62 c"), None);
        assert_eq!(font_stack(&"A".repeat(81)), None);
    }

    #[test]
    fn sizes_are_kept_only_from_the_list() {
        assert_eq!(size_value(" 18PX "), Some("18px"));
        assert_eq!(size_value("9px"), Some("9px"));
        assert_eq!(size_value("288px"), Some("288px"));
        assert_eq!(size_value("17px"), None);
        assert_eq!(size_value("16px"), None);
        assert_eq!(size_value("1.2em"), None);
        assert_eq!(size_value("large"), None);
    }

    #[test]
    fn colours_are_matched_in_any_spelling() {
        assert_eq!(palette_color("#2F6FD1"), Some("#2f6fd1"));
        assert_eq!(palette_color("rgb(47, 111, 209)"), Some("#2f6fd1"));
        assert_eq!(palette_color("rgb(47,111,209)"), Some("#2f6fd1"));
        assert_eq!(palette_color("#000"), None);
        assert_eq!(palette_color("red"), None);
        assert_eq!(palette_color("rgba(47, 111, 209, 0.5)"), None);
        assert_eq!(palette_color("#zzzzzz"), None);
    }

    /// The WCAG relative luminance of a `#rrggbb` colour.
    fn luminance(hex: &str) -> f64 {
        let c = |i: usize| {
            let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.0;
            if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
    }

    fn contrast(a: f64, b: f64) -> f64 {
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_colour_reads_on_white_and_on_dark() {
        let white = luminance("#ffffff");
        let dark = luminance("#1e1e1e");
        for c in COLORS {
            let l = luminance(c.hex);
            assert!(contrast(l, white) >= 3.0, "{} on white: {:.2}", c.key, contrast(l, white));
            assert!(contrast(l, dark) >= 3.0, "{} on dark: {:.2}", c.key, contrast(l, dark));
        }
    }
}
