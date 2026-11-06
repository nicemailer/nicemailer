//! Conversion of an HTML body to plain text, for messages that have no
//! text part (newsletters, notifications). The text is what the geek
//! reader shows, what the search indexes, and what `clean.rs` cuts into
//! segments, so its structure matters.
//!
//! mail-parser's own conversion joins blocks: `<h1>News</h1><p>Text</p>`
//! became "NewsText". Here block elements keep their separation:
//! - headings, paragraphs, block quotes, tables, lists: an empty line
//!   between them;
//! - `<br>`, `<div>`, table rows: a line break;
//! - list items: a line starting with "- ";
//! - table cells: separated by a tab;
//! - `<pre>`: whitespace kept as is;
//! - `<script>`, `<style>`, `<head>`, `<title>`: dropped;
//! - elements the mail hides (`display: none` and the like, in their
//!   `style` attribute, `css.rs`): dropped, since a mail app does not show
//!   them; unless the mail's `<style>` may show them again
//!   (`css::shown_classes`). Classes hidden by the `<style>` alone are not
//!   followed: which rule wins depends on specificity, and a wrong guess
//!   would drop the body of the mail. The first of them,
//!   before any visible text, is usually the **preview text** a newsletter
//!   writes for the list of mails ("C'est délirant."): it is kept apart
//!   (`Converted::preview`), for the list's snippet (`ingest.rs`);
//! - entities (`&eacute;`, `&#233;`, `&#xE9;`) decoded.
//!
//! It is a small tokenizer, not a full HTML parser: mail HTML is often
//! invalid, and a forgiving pass over tags is enough to recover the text.

/// What a tag does to the text around it.
enum Break {
    /// Nothing (inline tag: `<b>`, `<a>`, `<span>`…).
    None,
    /// A line break.
    Line,
    /// An empty line.
    Paragraph,
}

fn tag_break(name: &str) -> Break {
    match name {
        "br" | "div" | "tr" | "li" | "dt" | "dd" | "center" | "header" | "footer" | "nav" | "address" | "caption" => {
            Break::Line
        }
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "table" | "ul" | "ol" | "dl" | "pre"
        | "hr" | "section" | "article" | "aside" | "figure" | "form" | "fieldset" => Break::Paragraph,
        _ => Break::None,
    }
}

/// The text of an HTML body, and the preview text the mail hides for the
/// list of mails, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub text: String,
    /// Text of the hidden elements before any visible text, whitespace and
    /// invisible characters collapsed, 300 characters at most.
    pub preview: Option<String>,
}

/// Longest preview kept (the list shows far less).
const MAX_PREVIEW: usize = 300;

/// Elements without content: never "closed", so they cannot start a
/// hidden region.
const VOID: [&str; 14] = ["br", "img", "hr", "input", "meta", "link", "wbr", "col", "area", "base", "source", "track", "embed", "param"];

/// Converts HTML to plain text (see the module documentation).
pub fn html_to_text(html: &str) -> String {
    convert(html).text
}

/// Converts HTML to plain text, the elements the mail hides left out.
///
/// An element hidden but never closed (broken HTML) would hide the rest of
/// the mail: the text is then made again without hiding anything, as it
/// was before: a preview shown in the text is better than a mail lost.
pub fn convert(html: &str) -> Converted {
    let classes: std::collections::HashSet<String> =
        crate::css::style_blocks(html).into_iter().flat_map(crate::css::shown_classes).collect();
    let pass = run(html, Some(&classes));
    if pass.unclosed {
        return Converted { text: run(html, None).text, preview: None };
    }
    let preview: String = pass
        .preview
        .chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let preview = (!preview.is_empty()).then(|| preview.chars().take(MAX_PREVIEW).collect());
    Converted { text: pass.text, preview }
}

/// Characters newsletters pad their preview text with, so that the list
/// does not show the start of the body after it: invisible, taken as
/// spaces.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{034F}' | '\u{2007}' | '\u{00A0}')
}

/// What one pass of the conversion produced.
struct Pass {
    text: String,
    preview: String,
    /// A hidden element was still open at the end.
    unclosed: bool,
}

/// Whether the start tag `inner` (the inside of `<…>`) hides its element:
/// its `style` does, and none of its classes is one the mail's sheet may
/// show (`shown`).
fn hides(inner: &str, shown: &std::collections::HashSet<String>) -> bool {
    let Some(style) = crate::links::attribute(inner, "style") else { return false };
    crate::css::hides(&decode_entities(&style))
        && !crate::links::attribute(inner, "class").is_some_and(|c| c.split_whitespace().any(|c| shown.contains(c)))
}

/// One pass over the HTML; `hiding`: the classes the mail's sheet may show
/// over a hiding `style`, or `None` to hide nothing.
fn run(html: &str, hiding: Option<&std::collections::HashSet<String>>) -> Pass {
    let mut out = String::with_capacity(html.len() / 2);
    let mut preview = String::new();
    // The hidden element being skipped: its name, and how many elements of
    // that name are open inside it, itself included.
    let mut hidden: Option<(String, usize)> = None;
    // Whether text skipped now goes to the preview: nothing visible yet.
    let mut previewing = false;
    // Characters of the current run of text, before entity decoding and
    // whitespace collapsing.
    let mut i = 0;
    let bytes = html.as_bytes();
    let mut skip_until: Option<&'static str> = None;
    let mut pre_depth = 0usize;
    // Pending break to emit before the next text: 0 none, 1 line, 2 empty line.
    let mut pending = 0u8;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Comment.
            if html[i..].starts_with("<!--") {
                i = html[i..].find("-->").map(|e| i + e + 3).unwrap_or(bytes.len());
                continue;
            }
            let end = match html[i..].find('>') {
                Some(e) => i + e,
                None => break,
            };
            let inner = &html[i + 1..end];
            i = end + 1;
            let closing = inner.starts_with('/');
            let name: String = inner
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if let Some(until) = skip_until {
                if closing && name == until {
                    skip_until = None;
                }
                continue;
            }
            if !closing {
                skip_until = match name.as_str() {
                    "script" => Some("script"),
                    "style" => Some("style"),
                    "head" => Some("head"),
                    "title" => Some("title"),
                    _ => None,
                };
                if skip_until.is_some() {
                    continue;
                }
            }
            // Inside a hidden element: only its end matters.
            if let Some((hidden_name, depth)) = &mut hidden {
                if *hidden_name == name {
                    if closing {
                        *depth -= 1;
                    } else if !inner.ends_with('/') {
                        *depth += 1;
                    }
                }
                if *depth == 0 {
                    hidden = None;
                    if previewing {
                        preview.push(' ');
                    }
                } else if previewing && matches!(tag_break(&name), Break::Line | Break::Paragraph) {
                    preview.push(' ');
                }
                continue;
            }
            if let Some(classes) = hiding
                && !closing
                && !name.is_empty()
                && !VOID.contains(&name.as_str())
                && !inner.ends_with('/')
                && hides(inner, classes)
            {
                hidden = Some((name, 1));
                previewing = out.trim().is_empty();
                continue;
            }
            if name == "pre" {
                if closing {
                    pre_depth = pre_depth.saturating_sub(1);
                } else {
                    pre_depth += 1;
                }
            }
            match tag_break(&name) {
                Break::Paragraph => pending = 2,
                Break::Line => pending = pending.max(1),
                Break::None => {}
            }
            if !closing && name == "li" {
                flush_break(&mut out, &mut pending);
                out.push_str("- ");
            }
            if !closing && (name == "td" || name == "th") && !out.ends_with('\n') && !out.is_empty() {
                out.push('\t');
            }
            if name == "br" {
                // A <br> is a line break of its own, even repeated.
                out.push('\n');
                pending = 0;
            }
            continue;
        }
        // Text up to the next tag.
        let end = html[i..].find('<').map(|e| i + e).unwrap_or(bytes.len());
        let raw = &html[i..end];
        i = end;
        if skip_until.is_some() {
            continue;
        }
        if hidden.is_some() {
            if previewing && preview.len() < MAX_PREVIEW * 8 {
                preview.push_str(&decode_entities(raw));
            }
            continue;
        }
        let text = decode_entities(raw);
        if pre_depth > 0 {
            flush_break(&mut out, &mut pending);
            out.push_str(&text);
            continue;
        }
        // Outside <pre>, runs of whitespace (newlines included) are one space.
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if text.chars().any(char::is_whitespace) && !out.ends_with(['\n', ' ', '\t']) && !out.is_empty() && pending == 0 {
                out.push(' ');
            }
            continue;
        }
        flush_break(&mut out, &mut pending);
        if text.starts_with(char::is_whitespace) && !out.ends_with(['\n', ' ', '\t']) && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&collapsed);
        if text.ends_with(char::is_whitespace) {
            out.push(' ');
        }
    }
    Pass { text: tidy(&out), preview, unclosed: hidden.is_some() }
}

/// Writes the pending line break(s) before new text.
fn flush_break(out: &mut String, pending: &mut u8) {
    if *pending > 0 && !out.is_empty() {
        while out.ends_with([' ', '\t']) {
            out.pop();
        }
        let have = out.chars().rev().take_while(|c| *c == '\n').count();
        let want = *pending as usize;
        for _ in have..want {
            out.push('\n');
        }
    }
    *pending = 0;
}

/// Trims spaces at line ends and keeps at most one empty line in a row.
fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        // Leading spaces are kept: they only come from `<pre>` blocks (the
        // conversion never starts a line with a space elsewhere).
        let line = line.trim_end_matches([' ', '\t']);
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Decodes HTML entities: numeric ones and every named one of HTML5.
/// Unknown entities are left as written.
///
/// Also used on the text part of a mail without HTML whose sender wrote
/// entities in it (`ingest.rs`).
pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').filter(|e| *e <= MAX_ENTITY_NAME);
        let decoded = end.and_then(|e| entity(&after[..e]));
        match (end, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// An HTML whose entities were encoded twice by its sender ("Zoom
/// :&amp;nbsp;https://…", shown "Zoom :&nbsp;https://…" by every mail
/// client) with them encoded once: `&amp;` followed by a known entity (named
/// or numeric) and `;` becomes `&`. A plain `&amp;` (in a link, in text) is
/// left alone. Borrowed, without copy, when there is nothing to change.
pub fn undouble_entities(html: &str) -> std::borrow::Cow<'_, str> {
    // Some senders encode three times (`&amp;amp;#039;`): repeated until
    // stable, at most three times.
    let mut out = undouble_once(html);
    for _ in 0..2 {
        match undouble_once(&out) {
            std::borrow::Cow::Owned(again) => out = std::borrow::Cow::Owned(again),
            std::borrow::Cow::Borrowed(_) => break,
        }
    }
    out
}

/// One pass of `undouble_entities`.
fn undouble_once(html: &str) -> std::borrow::Cow<'_, str> {
    if !html.contains("&amp;") {
        return std::borrow::Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut changed = false;
    while let Some(start) = rest.find("&amp;") {
        let after = &rest[start + 5..];
        let name = after.find(';').filter(|e| *e <= MAX_ENTITY_NAME).map(|e| &after[..e]);
        out.push_str(&rest[..start]);
        match name.filter(|n| entity(n).is_some()) {
            Some(n) => {
                out.push('&');
                out.push_str(n);
                out.push(';');
                rest = &after[n.len() + 1..];
                changed = true;
            }
            None => {
                out.push_str("&amp;");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if changed { std::borrow::Cow::Owned(out) } else { std::borrow::Cow::Borrowed(html) }
}

/// A text with its HTML entities decoded until none is left (at most three
/// times): a text decoded once can still hold the entities its sender
/// encoded twice ("&amp;nbsp;" gives "&nbsp;"). Borrowed when it holds none.
pub fn decode_all_entities(text: &str) -> std::borrow::Cow<'_, str> {
    if !has_entity(text) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = decode_entities(text);
    for _ in 0..2 {
        if !has_entity(&out) {
            break;
        }
        out = decode_entities(&out);
    }
    std::borrow::Cow::Owned(out)
}

/// Whether `s` holds at least one HTML entity this module knows
/// (`&nbsp;`, `&#39;`…). A text part with entities was generated from HTML
/// by a sender that forgot to decode them (`ingest.rs`).
pub(crate) fn has_entity(s: &str) -> bool {
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        let after = &rest[start + 1..];
        if let Some(e) = after.find(';').filter(|e| *e <= MAX_ENTITY_NAME)
            && entity(&after[..e]).is_some()
        {
            return true;
        }
        rest = after;
    }
    false
}

/// Whether a text holds HTML markup a sender left when generating it from
/// HTML: closing tags (`</p>`, `</a>`…), line breaks (`<br />`), or the
/// remains of attributes (`" target="_blank" style="…`). A lone opening tag
/// is not enough: a text may talk about HTML ("add classes to your <img>").
/// Compared without ASCII case.
pub(crate) fn has_html_markup(s: &str) -> bool {
    if !s.contains('<') && !s.contains("=\"") {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    const MARKS: &[&str] = &[
        "</p>", "</div>", "</span>", "</a>", "</li>", "</td>", "</tr>", "</table>", "</em>", "</strong>", "</b>",
        "</i>", "</u>", "</font>", "</h1>", "</h2>", "</h3>", "<br>", "<br/>", "<br />", " style=\"", " target=\"",
    ];
    MARKS.iter().any(|m| lower.contains(m))
}

/// Longest entity name of HTML5 ("CounterClockwiseContourIntegral"): an
/// `&` farther than that from the next `;` does not start an entity.
const MAX_ENTITY_NAME: usize = 32;

/// The character of an entity (`nbsp`, `#39`, `#x2019`), without `&` and
/// `;`. The names mail uses most are listed below, with the core's own
/// choices (a non-breaking space shown as a plain space); any other HTML5
/// name comes from the table of `markup5ever` (2,231 names, the one the
/// HTML sanitizer uses). The few names worth two characters (a letter and
/// a combining mark) are left as written.
fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        // A non-breaking space is shown as a plain space in text.
        return if code == 0xA0 { Some(' ') } else { char::from_u32(code) };
    }
    Some(match name {
        "nbsp" => ' ',
        // Invisible: a soft hyphen, which newsletters put in long words.
        "shy" => '\u{AD}',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "euml" => 'ë',
        "agrave" => 'à',
        "acirc" => 'â',
        "auml" => 'ä',
        "ccedil" => 'ç',
        "icirc" => 'î',
        "iuml" => 'ï',
        "ocirc" => 'ô',
        "ouml" => 'ö',
        "ugrave" => 'ù',
        "ucirc" => 'û',
        "uuml" => 'ü',
        "Eacute" => 'É',
        "Egrave" => 'È',
        "Agrave" => 'À',
        "Ccedil" => 'Ç',
        "oelig" => 'œ',
        "OElig" => 'Œ',
        "szlig" => 'ß',
        "ntilde" => 'ñ',
        "laquo" => '«',
        "raquo" => '»',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        "ndash" => '–',
        "mdash" => '—',
        "euro" => '€',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "middot" => '·',
        "bull" => '•',
        "times" => '×',
        "zwnj" | "zwj" => '\u{200C}',
        _ => {
            let (code, second) = *markup5ever::data::NAMED_ENTITIES.get(format!("{name};").as_str())?;
            if second != 0 {
                return None;
            }
            return char::from_u32(code);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_markup_left_in_a_text_is_recognized() {
        assert!(has_html_markup("disponibles dans votre <em>Espace Client</em>"));
        assert!(has_html_markup("ligne 1 <br />ligne 2"));
        assert!(has_html_markup("(https://myswisslife.fr)\" target=\"_blank\" style=\"color: #231f20"));
        assert!(has_html_markup("<P>Vous trouverez</P>"));
        // A text that talks about HTML, or uses angle brackets.
        assert!(!has_html_markup("Add classes to your <img> and let it handle the rest."));
        assert!(!has_html_markup("Alice <alice@a.example> a écrit : 3 < 4"));
    }

    #[test]
    fn every_html5_named_entity_is_decoded() {
        assert_eq!(decode_entities("Suivant &rarr; &Ecirc;tre &sup2; &acute; &hearts; &rsaquo;"), "Suivant → Être ² ´ ♥ ›");
        assert_eq!(decode_entities("&CounterClockwiseContourIntegral;"), "∳");
        // Unknown, or too long to be one: left as written.
        assert_eq!(decode_entities("&inconnu; & &aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;"), "&inconnu; & &aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;");
        assert!(has_entity("&rarr;"));
    }

    #[test]
    fn entities_encoded_twice_are_encoded_once() {
        assert_eq!(undouble_entities("Zoom :&amp;nbsp;https://x.us/j/1"), "Zoom :&nbsp;https://x.us/j/1");
        assert_eq!(undouble_entities("l&amp;#39;offre &amp;eacute;t&amp;eacute;"), "l&#39;offre &eacute;t&eacute;");
        // A plain ampersand of a link or a text stays encoded.
        assert_eq!(undouble_entities("<a href=\"x?a=1&amp;b=2\">R&amp;D; Tom &amp; Jerry</a>"), "<a href=\"x?a=1&amp;b=2\">R&amp;D; Tom &amp; Jerry</a>");
        assert!(matches!(undouble_entities("<p>rien</p>"), std::borrow::Cow::Borrowed(_)));
        // Encoded three times.
        assert_eq!(undouble_entities("Je n&amp;amp;#039;ai"), "Je n&#039;ai");
        // Decoded twice, a text loses them too.
        assert_eq!(decode_all_entities("Zoom :&amp;nbsp;https://x"), "Zoom : https://x");
        assert_eq!(decode_all_entities("Tom & Jerry; R&D"), "Tom & Jerry; R&D");
    }

    #[test]
    fn entities_are_recognized() {
        assert!(has_entity("l'État de droit&nbsp;(elle"));
        assert!(has_entity("N&#176;0009"));
        assert!(!has_entity("Tom & Jerry; R&D"));
        assert!(!has_entity("&inconnu;"));
    }

    #[test]
    fn blocks_are_separated() {
        assert_eq!(html_to_text("<h1>Novembre</h1><p>Les nouveautés du mois.</p>"), "Novembre\n\nLes nouveautés du mois.");
        assert_eq!(html_to_text("<div>Ligne 1</div><div>Ligne 2</div>"), "Ligne 1\nLigne 2");
        assert_eq!(html_to_text("Bonjour,<br>à demain<br><br>Alice"), "Bonjour,\nà demain\n\nAlice");
    }

    #[test]
    fn lists_and_tables() {
        assert_eq!(html_to_text("<p>Menu :</p><ul><li>Entrée</li><li>Plat</li></ul><p>Fin</p>"), "Menu :\n\n- Entrée\n- Plat\n\nFin");
        assert_eq!(html_to_text("<table><tr><td>A</td><td>1</td></tr><tr><td>B</td><td>2</td></tr></table>"), "A\t1\nB\t2");
    }

    #[test]
    fn inline_tags_keep_words_together_and_spaces_collapse() {
        assert_eq!(html_to_text("<p>Le <b>rapport</b> est <a href=\"x\">ici</a>.</p>"), "Le rapport est ici.");
        assert_eq!(html_to_text("<p>un\n   deux\t trois</p>"), "un deux trois");
        assert_eq!(html_to_text("<span>a</span> <span>b</span>"), "a b");
    }

    #[test]
    fn scripts_styles_head_and_comments_are_dropped() {
        let html = "<html><head><title>T</title><style>p{color:red}</style></head><body><!-- c --><script>x()</script><p>Texte</p></body></html>";
        assert_eq!(html_to_text(html), "Texte");
    }

    #[test]
    fn hidden_elements_are_left_out_and_the_first_is_the_preview() {
        // TTSO's preview text, then the body.
        let html = r#"<div style="max-width:680px"><div style="max-height:0; mso-hide:all; overflow:hidden; display:none; color:#09144b;">
            <div style="padding:6px 0;">C'est délirant.</div>&#847;&zwnj;&nbsp;&#847;&zwnj;&nbsp;</div>
            <p>Mardi 29 septembre 2026</p><span style="display:none">plus tard</span><p>Fin</p></div>"#;
        let c = convert(html);
        assert_eq!(c.text, "Mardi 29 septembre 2026\n\nFin");
        assert_eq!(c.preview.as_deref(), Some("C'est délirant."));
        // A class the mail's sheet hides is not followed (which rule wins
        // depends on specificity); one it shows over the `style` wins.
        let html = "<style>.preheader{display:none!important}</style><span class=\"x preheader\">Aperçu</span><p>Corps</p>";
        assert_eq!(convert(html), Converted { text: "Aperçu\n\nCorps".into(), preview: None });
        let html = "<style>.show{display:block!important}</style><div class=\"show\" style=\"display:none;mso-hide:all\">Pour tous</div><p>Corps</p>";
        assert_eq!(convert(html).text, "Pour tous\n\nCorps");
        // A hidden element after visible text is no preview.
        let html = "<p>Corps</p><div style=\"display:none\"><div>caché</div></div><p>Suite</p>";
        assert_eq!(convert(html), Converted { text: "Corps\n\nSuite".into(), preview: None });
        // Nested elements of the same name close the right one.
        assert_eq!(html_to_text("<div style=\"display:none\"><div>a</div><div>b</div></div><div>visible</div>"), "visible");
        // An image in a hidden element does not end it.
        assert_eq!(html_to_text("<span style=\"display:none\">x<img src=\"a\">y</span>z"), "z");
    }

    #[test]
    fn a_hidden_element_never_closed_hides_nothing() {
        // Broken HTML: rather the preview in the text than a mail lost.
        let c = convert("<div style=\"display:none\">Aperçu<p>Tout le mail</p>");
        assert_eq!(c.text, "Aperçu\n\nTout le mail");
        assert_eq!(c.preview, None);
    }

    #[test]
    fn pre_keeps_its_whitespace() {
        assert_eq!(html_to_text("<p>Code :</p><pre>a  b\n  c</pre><p>fin</p>"), "Code :\n\na  b\n  c\n\nfin");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(html_to_text("<p>Caf&eacute; &amp; th&#233; &#xE0; 5&nbsp;&euro; &laquo;ok&raquo; &unknown; R&D</p>"), "Café & thé à 5 € «ok» &unknown; R&D");
    }

    #[test]
    fn empty_and_broken_html() {
        assert_eq!(html_to_text(""), "");
        assert_eq!(html_to_text("<p>texte sans fin"), "texte sans fin");
        assert_eq!(html_to_text("a < b"), "a");
    }
}
