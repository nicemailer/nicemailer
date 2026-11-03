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

/// Converts HTML to plain text (see the module documentation).
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
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
    tidy(&out)
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

/// Decodes HTML entities: numeric ones, and the named ones mail uses most.
/// Unknown entities are left as written.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').filter(|e| *e <= 10);
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
