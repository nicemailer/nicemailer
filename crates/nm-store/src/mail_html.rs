//! The HTML of a mail written in the formatted mode of the editor, made
//! ready to send (`specifications/edition.md`, section 10).
//!
//! The editor (Tiptap, in the app) stores what it produces in
//! `draft.body`. Whatever it produced, **the core decides what leaves**:
//!
//! 1. [`clean`] reduces the HTML to a closed list of tags, attributes and
//!    values (ammonia), the fonts, sizes and colours of `nm_core::formats`
//!    included. A buggy or hijacked editor cannot send anything else.
//! 2. [`outgoing`] then writes a `style` attribute on each tag, from a
//!    fixed table (Gmail and Outlook often ignore a `<style>` block), wraps
//!    the result in a minimal document, and computes the plain text sent
//!    beside it and whether the mail has any formatting at all.
//!
//! Both work on a small tree read with html5ever's tokenizer, the one
//! ammonia parses with. After [`clean`] the HTML is well formed and holds
//! only known tags, so a tree of elements and text is all that is needed.
//!
//! # Why not the reader's conversion to text
//!
//! `html_text.rs` turns the HTML of *received* mails into text, and is
//! tuned for any HTML (tables, hidden preview text, broken markup). The
//! text part of a mail we send needs more structure from far less HTML:
//! numbered and nested lists, quotations prefixed by `>`, one paragraph
//! per line as in the editor. Changing the reader's conversion would
//! change how every received mail is shown and indexed, so this module
//! has its own, for the closed subset it accepts.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use nm_core::formats;

/// The largest HTML body a draft may hold, a limit against an aberrant
/// content rather than a real constraint (`edition.md`, section 11).
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// The tags an outgoing mail may hold.
const TAGS: [&str; 14] = ["p", "br", "h1", "h2", "h3", "ul", "ol", "li", "blockquote", "strong", "em", "u", "a", "span"];

/// The HTML of a mail to send, cleaned and styled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The whole document, `<!DOCTYPE html>` included.
    pub html: String,
    /// The plain text of the same mail, for the `text/plain` part.
    pub text: String,
    /// The mail has some formatting (`edition.md`, section 10, "Mise en
    /// forme réelle"): without, it leaves as text only.
    pub formatted: bool,
}

/// Reduces `html` to the closed list of `edition.md`, section 10:
/// the tags of [`TAGS`]; `href` on `a`, in `http`, `https` or `mailto`;
/// `start` on `ol`, a number; `style` on `span`, with only `color`,
/// `font-family` and `font-size`, each to a value of `nm_core::formats`.
/// `<script>` and `<style>` go with their content; any other tag leaves
/// its text.
pub fn clean(html: &str) -> String {
    // Rust note: `HashSet<&str>` and `HashMap<&str, HashSet<&str>>` are
    // what ammonia's builder takes; `collect()` builds them from arrays.
    let tags: HashSet<&str> = TAGS.into_iter().collect();
    let attributes: HashMap<&str, HashSet<&str>> =
        [("a", ["href"].into()), ("ol", ["start"].into()), ("span", ["style"].into())].into_iter().collect();
    ammonia::Builder::empty()
        .tags(tags)
        .tag_attributes(attributes)
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .clean_content_tags(["script", "style", "title"].into_iter().collect())
        .attribute_filter(|element, attribute, value| match (element, attribute) {
            ("span", "style") => {
                let css = span_style(value);
                (!css.is_empty()).then(|| css.into())
            }
            ("ol", "start") => value.trim().parse::<u32>().ok().map(|n| n.to_string().into()),
            _ => Some(value.into()),
        })
        .clean(html)
        .to_string()
}

/// The declarations of a `span`'s `style` that may leave, in a fixed order
/// and spelling: `color`, `font-family`, `font-size`, each only to a value
/// of the lists (any font with a safe name, `formats::font_stack`).
/// Everything else is dropped.
fn span_style(value: &str) -> String {
    let (mut color, mut family, mut size) = (None, None, None);
    // A declaration is `name: value`; values here hold no `;` (quotes in a
    // font name hold no `;` either), so a split is enough.
    for decl in value.split(';') {
        let Some((name, v)) = decl.split_once(':') else { continue };
        match name.trim().to_ascii_lowercase().as_str() {
            "color" => color = formats::palette_color(v).map(Cow::Borrowed),
            "font-family" => family = formats::font_stack(v),
            "font-size" => size = formats::size_value(v).map(Cow::Borrowed),
            _ => {}
        }
    }
    let mut out = String::new();
    for (name, v) in [("color", color), ("font-family", family), ("font-size", size)] {
        if let Some(v) = v {
            if !out.is_empty() {
                out.push(' ');
            }
            let _ = write!(out, "{name}: {v};");
        }
    }
    out
}

/// The style written on each tag of the mail sent. Paragraphs have no
/// margin: in the editor one paragraph is one line, as in Apple Mail, and
/// an empty one is a blank line. A quotation is drawn as Gmail and Mail
/// draw theirs: a grey bar on the left.
fn tag_style(name: &str) -> Option<&'static str> {
    Some(match name {
        "p" => "margin: 0;",
        "h1" => "margin: 0.6em 0 0.3em 0; font-size: 24px; font-weight: bold; line-height: 1.25;",
        "h2" => "margin: 0.6em 0 0.3em 0; font-size: 20px; font-weight: bold; line-height: 1.25;",
        "h3" => "margin: 0.5em 0 0.25em 0; font-size: 17px; font-weight: bold; line-height: 1.25;",
        "ul" | "ol" => "margin: 0; padding-left: 1.6em;",
        "li" => "margin: 0;",
        "blockquote" => "margin: 0 0 0 0.8ex; padding-left: 1ex; border-left: 2px solid #c8c8c8;",
        _ => return None,
    })
}

/// A node of the cleaned HTML.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<Node> },
    Text(String),
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Node::Element { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            Node::Text(_) => None,
        }
    }
}

/// Reads cleaned HTML into a tree. The HTML is well formed (ammonia wrote
/// it), but a stray end tag is skipped rather than trusted.
fn parse(html: &str) -> Vec<Node> {
    /// Receives the tokens and builds the tree on a stack of open elements.
    ///
    /// Rust note: html5ever's `TokenSink::process_token` takes `&self`,
    /// so the state lives in a `RefCell`, borrowed mutably at run time.
    struct Sink {
        stack: RefCell<Vec<Node>>,
    }
    fn close(stack: &mut Vec<Node>) {
        if stack.len() > 1 {
            let node = stack.pop().expect("more than one");
            if let Some(Node::Element { children, .. }) = stack.last_mut() {
                children.push(node);
            }
        }
    }
    impl TokenSink for Sink {
        type Handle = ();
        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            let mut stack = self.stack.borrow_mut();
            match token {
                Token::TagToken(tag) => {
                    let name = tag.name.to_string();
                    if tag.kind == TagKind::StartTag {
                        let attrs = tag.attrs.iter().map(|a| (a.name.local.to_string(), a.value.to_string())).collect();
                        stack.push(Node::Element { name: name.clone(), attrs, children: Vec::new() });
                        if name == "br" || tag.self_closing {
                            close(&mut stack);
                        }
                    } else if let Some(depth) = stack.iter().rposition(|n| matches!(n, Node::Element { name: n, .. } if *n == name)) {
                        // The element and any left open inside it.
                        while stack.len() > depth.max(1) {
                            close(&mut stack);
                        }
                    }
                }
                Token::CharacterTokens(text) => {
                    if let Some(Node::Element { children, .. }) = stack.last_mut() {
                        match children.last_mut() {
                            Some(Node::Text(t)) => t.push_str(&text),
                            _ => children.push(Node::Text(text.to_string())),
                        }
                    }
                }
                _ => {}
            }
            TokenSinkResult::Continue
        }
    }
    let root = Node::Element { name: String::new(), attrs: Vec::new(), children: Vec::new() };
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let tokenizer = Tokenizer::new(Sink { stack: RefCell::new(vec![root]) }, TokenizerOpts::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let mut stack = tokenizer.sink.stack.into_inner();
    while stack.len() > 1 {
        close(&mut stack);
    }
    match stack.pop() {
        Some(Node::Element { children, .. }) => children,
        _ => Vec::new(),
    }
}

/// The mail to send from the HTML of a draft: cleaned ([`clean`]), styled,
/// wrapped in a document, with its plain text.
pub fn outgoing(html: &str) -> Outgoing {
    let nodes = parse(&clean(html));
    let mut body = String::with_capacity(html.len() + html.len() / 2);
    for n in &nodes {
        write_styled(&mut body, n);
    }
    let document = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    );
    Outgoing { html: document, text: text_of(&nodes), formatted: nodes.iter().any(formatted) }
}

/// The plain text of the HTML of a draft, and whether it has formatting:
/// what the text part of the mail holds, and what the editor shows when
/// the draft goes back to text only (`edition.md`, section 3).
pub fn to_text(html: &str) -> (String, bool) {
    let nodes = parse(&clean(html));
    (text_of(&nodes), nodes.iter().any(formatted))
}

/// Escapes text for HTML content and attribute values.
fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

/// Writes a node with the style of its tag. An empty paragraph gets a
/// `<br>`: a `<p></p>` takes no height in most readers, and the blank line
/// the user typed would vanish.
fn write_styled(out: &mut String, node: &Node) {
    match node {
        Node::Text(t) => escape(out, t),
        Node::Element { name, attrs, children } => {
            let _ = write!(out, "<{name}");
            if let Some(style) = tag_style(name) {
                let _ = write!(out, " style=\"{style}\"");
            }
            for (k, v) in attrs {
                let _ = write!(out, " {k}=\"");
                escape(out, v);
                out.push('"');
            }
            out.push('>');
            if name == "br" {
                return;
            }
            if name == "p" && children.iter().all(|c| matches!(c, Node::Text(t) if t.trim().is_empty())) {
                out.push_str("<br>");
            } else {
                for c in children {
                    write_styled(out, c);
                }
            }
            let _ = write!(out, "</{name}>");
            // A line break after each block keeps the source readable and
            // the quoted-printable lines short.
            if matches!(name.as_str(), "p" | "h1" | "h2" | "h3" | "ul" | "ol" | "li" | "blockquote") {
                out.push('\n');
            }
        }
    }
}

/// Whether a node, or one inside it, is formatting: a heading, a list, a
/// bold, an italic, an underline, a styled span, or a link whose text is
/// not its address. Paragraphs, line breaks and quotations are not.
fn formatted(node: &Node) -> bool {
    let Node::Element { name, children, .. } = node else { return false };
    match name.as_str() {
        "h1" | "h2" | "h3" | "ul" | "ol" | "strong" | "em" | "u" => true,
        "span" if node.attr("style").is_some() => true,
        "a" => {
            let text = inline_text(children);
            !link_is_its_text(node.attr("href").unwrap_or_default(), &text)
        }
        _ => children.iter().any(formatted),
    }
}

/// Whether a link only repeats its address: `https://x.fr` shown as
/// `https://x.fr`, or `mailto:a@x.fr` shown as `a@x.fr`.
fn link_is_its_text(href: &str, text: &str) -> bool {
    let text = text.trim();
    href == text || href.strip_prefix("mailto:") == Some(text) || href.trim_end_matches('/') == text.trim_end_matches('/')
}

/// The text of inline content, without line structure.
fn inline_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Element { name, .. } if name == "br" => out.push('\n'),
            Node::Element { children, .. } => out.push_str(&inline_text(children)),
        }
    }
    out
}

/// The plain text of the mail (`edition.md`, section 10, "Texte brut tiré
/// du HTML"): one line per paragraph, as in the editor; a heading on its
/// line; `- ` or `1. ` before each list item, the lines after the first
/// indented under its text; `> ` before each quoted line (`>> ` nested,
/// as `compose::quote` writes them); a link whose text is not its address
/// written `text <address>`. Trailing blank lines are dropped.
fn text_of(nodes: &[Node]) -> String {
    let mut lines = Vec::new();
    blocks(nodes, &mut lines);
    while lines.last().is_some_and(|l: &String| l.trim().is_empty()) {
        lines.pop();
    }
    let mut text = lines.join("\n");
    // Non-breaking spaces become plain ones: the text part is for reading.
    text = text.replace('\u{a0}', " ");
    text
}

/// The lines of a sequence of blocks. Inline content found between blocks
/// (text not in a paragraph) is a paragraph of its own.
fn blocks(nodes: &[Node], lines: &mut Vec<String>) {
    let mut run: Vec<Node> = Vec::new();
    let flush = |run: &mut Vec<Node>, lines: &mut Vec<String>| {
        if !run.is_empty() {
            let text = inline_lines(run);
            if !(text.len() == 1 && text[0].trim().is_empty()) {
                lines.extend(text);
            }
            run.clear();
        }
    };
    for n in nodes {
        match n {
            Node::Element { name, children, .. } => match name.as_str() {
                "p" | "h1" | "h2" | "h3" => {
                    flush(&mut run, lines);
                    lines.extend(inline_lines(children));
                }
                "blockquote" => {
                    flush(&mut run, lines);
                    let mut inner = Vec::new();
                    blocks(children, &mut inner);
                    for l in inner {
                        lines.push(if l.is_empty() {
                            ">".to_string()
                        } else if l.starts_with('>') {
                            format!(">{l}")
                        } else {
                            format!("> {l}")
                        });
                    }
                }
                "ul" | "ol" => {
                    flush(&mut run, lines);
                    let ordered = name == "ol";
                    let mut number: u32 = n.attr("start").and_then(|s| s.parse().ok()).unwrap_or(1);
                    for item in children {
                        let Node::Element { name, children: content, .. } = item else { continue };
                        if name != "li" {
                            continue;
                        }
                        let marker = if ordered {
                            format!("{number}. ")
                        } else {
                            "- ".to_string()
                        };
                        number += 1;
                        let mut inner = Vec::new();
                        blocks(content, &mut inner);
                        if inner.is_empty() {
                            inner.push(String::new());
                        }
                        let indent = " ".repeat(marker.chars().count());
                        for (i, l) in inner.into_iter().enumerate() {
                            lines.push(if i == 0 {
                                format!("{marker}{l}")
                            } else if l.is_empty() {
                                l
                            } else {
                                format!("{indent}{l}")
                            });
                        }
                    }
                }
                _ => run.push(n.clone()),
            },
            Node::Text(t) if t.trim().is_empty() && run.is_empty() => {}
            Node::Text(_) => run.push(n.clone()),
        }
    }
    flush(&mut run, lines);
}

/// The lines of inline content: `<br>` breaks them; a link whose text is
/// not its address is followed by ` <address>`.
fn inline_lines(nodes: &[Node]) -> Vec<String> {
    let mut lines = vec![String::new()];
    inline_into(nodes, &mut lines);
    lines
}

fn inline_into(nodes: &[Node], lines: &mut Vec<String>) {
    for n in nodes {
        match n {
            Node::Text(t) => {
                // Text never holds a line break of its own in the editor's
                // HTML; one from elsewhere is a space, as HTML shows it.
                let last = lines.last_mut().expect("at least one line");
                last.push_str(&t.replace(['\n', '\r'], " "));
            }
            Node::Element { name, children, .. } => match name.as_str() {
                "br" => lines.push(String::new()),
                "a" => {
                    inline_into(children, lines);
                    let href = n.attr("href").unwrap_or_default();
                    let text = inline_text(children);
                    if !href.is_empty() && !link_is_its_text(href, &text) {
                        let shown = href.strip_prefix("mailto:").unwrap_or(href);
                        let _ = write!(lines.last_mut().expect("at least one line"), " <{shown}>");
                    }
                }
                _ => inline_into(children, lines),
            },
        }
    }
}

// ─── The AI on a formatted draft ───────────────────────────────────────
//
// `ia.md`, section 5, "Le texte écrit par l'utilisateur", for a draft in
// the formatted mode: the AI rewrites the user's own blocks as HTML (the
// headings, lists, bold and links stay where they were), and a passage
// selected in the editor as text.

/// A formatted draft as the AI reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiView {
    /// The body cleaned ([`clean`]) and written again, block after block,
    /// with nothing between them: the ranges of `own` are in it, and it is
    /// the body the AI's answer is spliced into.
    pub html: String,
    /// The user's own text: byte ranges of `html`, each a run of
    /// consecutive top-level blocks (paragraphs, headings, lists) that are
    /// neither the quotation, its introduction ("… wrote:"), a forwarded
    /// mail nor the signature. Blank paragraphs at the edges of a run stay
    /// outside it, as blank lines do in a text draft.
    pub own: Vec<Range<usize>>,
    /// The plain text of each range of `own` ([`to_text`]), for the prompts
    /// that read the draft without rewriting it (subjects).
    pub own_text: Vec<String>,
    /// The text the editor measures a selection in: one line per text
    /// block (paragraph, heading, the paragraph of a list item, at any
    /// depth), joined by `\n`, a line break inside one being `\n` too.
    /// No marker, no quotation sign: the same text as the editor's
    /// `htmlView` (`apps/editor/src/text.ts`), offset for offset.
    pub view: String,
    /// The draft ends with a signature (`-- `).
    pub signature: bool,
}

/// The blocks of a formatted draft the AI may rewrite, and its text view.
///
/// Which blocks are the user's is decided as for a text draft
/// (`nm_core::segments::own_segments`), on the text each top-level block
/// gives in the text part of the mail ([`to_text`]: `> ` before a quoted
/// line, `- ` before a list item): the rules of the quotation, of its
/// introduction, of a forward and of the signature are the same, and a
/// block is the user's if its lines are.
pub fn ai_view(body: &str) -> AiView {
    let nodes: Vec<Node> = parse(&clean(body))
        .into_iter()
        .filter(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()))
        .collect();
    // Each block written plainly, and its lines in the text of the mail.
    let mut html = String::new();
    let mut spans = Vec::new(); // (byte range in html, byte range in text, blank)
    let mut text = String::new();
    for n in &nodes {
        let start = html.len();
        write_plain(&mut html, n);
        let mut lines = Vec::new();
        blocks(std::slice::from_ref(n), &mut lines);
        if !text.is_empty() {
            text.push('\n');
        }
        let t_start = text.len();
        text.push_str(&lines.join("\n"));
        let blank = lines.iter().all(|l| l.trim().is_empty());
        spans.push((start..html.len(), t_start..text.len(), blank));
    }
    let own_ranges = nm_core::segments::own_segments(&text);
    // A block is the user's if its text lies in one of the user's ranges;
    // a blank one only inside a range, never at its edge.
    let is_own = |t: &Range<usize>, blank: bool| {
        own_ranges.iter().any(|r| if blank { r.start <= t.start && t.start < r.end } else { r.start < t.end && t.start < r.end })
    };
    let mut own: Vec<Range<usize>> = Vec::new();
    let mut previous_own = false;
    for (h, t, blank) in &spans {
        let mine = is_own(t, *blank);
        if mine {
            match own.last_mut() {
                Some(last) if previous_own => last.end = h.end,
                _ => own.push(h.clone()),
            }
        }
        previous_own = mine;
    }
    let own_text = own.iter().map(|r| to_text(&html[r.clone()]).0).collect();
    let mut view_lines = Vec::new();
    view_blocks(&nodes, &mut view_lines);
    AiView { html, own, own_text, view: view_lines.join("\n"), signature: nm_core::segments::has_signature(&text) }
}

/// The lines of the editor's text view (see [`AiView::view`]).
fn view_blocks(nodes: &[Node], lines: &mut Vec<String>) {
    for n in nodes {
        if let Node::Element { name, children, .. } = n {
            match name.as_str() {
                "p" | "h1" | "h2" | "h3" => lines.push(inline_text(children)),
                _ => view_blocks(children, lines),
            }
        }
    }
}

/// Writes a node as it is, without the styles of [`write_styled`]: the
/// HTML of a draft, for the editor to read back.
fn write_plain(out: &mut String, node: &Node) {
    match node {
        Node::Text(t) => escape(out, t),
        Node::Element { name, attrs, children } => {
            let _ = write!(out, "<{name}");
            for (k, v) in attrs {
                let _ = write!(out, " {k}=\"");
                escape(out, v);
                out.push('"');
            }
            out.push('>');
            if name == "br" {
                return;
            }
            for c in children {
                write_plain(out, c);
            }
            let _ = write!(out, "</{name}>");
        }
    }
}

/// A segment the AI gave back for a formatted draft, made a run of blocks
/// again: cleaned ([`clean`]: only the tags and values of the editor, so a
/// model that invents a `<table>` or a `<script>` gets its text only), and
/// text found outside any block made paragraphs, one per line, as the
/// editor makes a pasted text. The model is asked for HTML, but may answer
/// in plain text: the answer is then still the user's paragraphs.
pub fn ai_block(answer: &str) -> String {
    let nodes = parse(&clean(answer));
    let mut out = String::new();
    let mut loose: Vec<Node> = Vec::new();
    // Loose inline content: a paragraph per line of its text, the inline
    // tags (`<strong>`) kept on the words of their line.
    let flush = |loose: &mut Vec<Node>, out: &mut String| {
        if loose.is_empty() {
            return;
        }
        let mut paragraph = String::new();
        let end = |paragraph: &mut String, out: &mut String| {
            out.push_str("<p>");
            out.push_str(paragraph.trim());
            out.push_str("</p>");
            paragraph.clear();
        };
        for n in loose.iter() {
            match n {
                Node::Text(t) => {
                    let mut parts = t.split('\n').peekable();
                    while let Some(part) = parts.next() {
                        escape(&mut paragraph, part);
                        if parts.peek().is_some() {
                            end(&mut paragraph, out);
                        }
                    }
                }
                _ => write_plain(&mut paragraph, n),
            }
        }
        end(&mut paragraph, out);
        loose.clear();
    };
    for n in nodes {
        match &n {
            Node::Element { name, .. } if matches!(name.as_str(), "p" | "h1" | "h2" | "h3" | "ul" | "ol" | "blockquote") => {
                flush(&mut loose, &mut out);
                write_plain(&mut out, &n);
            }
            Node::Text(t) if t.trim().is_empty() && loose.is_empty() => {}
            _ => loose.push(n),
        }
    }
    flush(&mut loose, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(html: &str) -> String {
        to_text(html).0
    }

    #[test]
    fn only_the_closed_list_is_kept() {
        let html = r#"<p onclick="x()">Hi <b>there</b> <img src="https://t.co/p.gif"><script>alert(1)</script><style>p{}</style></p>
<table><tr><td>cell</td></tr></table><h4>Four</h4><div class="c">div</div><s>strike</s><code>code</code>"#;
        let c = clean(html);
        assert_eq!(c, "<p>Hi there </p>\ncellFourdivstrikecode");
    }

    #[test]
    fn links_keep_only_safe_schemes() {
        assert_eq!(clean(r#"<a href="https://x.fr">x</a>"#), r#"<a href="https://x.fr" rel="noopener noreferrer">x</a>"#);
        assert_eq!(clean(r#"<a href="mailto:a@x.fr">a</a>"#), r#"<a href="mailto:a@x.fr" rel="noopener noreferrer">a</a>"#);
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "nicemailer://m/1", "/relative"] {
            let c = clean(&format!(r#"<a href="{bad}" target="_blank">t</a>"#));
            assert!(!c.contains("href"), "{bad}: {c}");
            assert!(c.contains('t'));
        }
    }

    #[test]
    fn span_styles_keep_only_listed_values() {
        let c = clean(r#"<span style="color: rgb(47, 111, 209); font-family: &quot;Georgia&quot;, serif; font-size: 18px; background: red">x</span>"#);
        assert_eq!(c, r#"<span style="color: #2f6fd1; font-family: Georgia, 'Times New Roman', serif; font-size: 18px;">x</span>"#);
        // A value outside the lists goes; the text stays.
        assert_eq!(clean(r#"<span style="color: #123456; font-size: 17px">x</span>"#), "<span>x</span>");
        assert_eq!(clean(r#"<span style="position: fixed; background: url(https://t.co/x)">x</span>"#), "<span>x</span>");
        // Any font of the Mac stays, with its generic family; a name that
        // could end the value goes.
        assert_eq!(
            clean(r#"<span style="font-family: &quot;Avenir Next&quot;, sans-serif">x</span>"#),
            r#"<span style="font-family: 'Avenir Next', sans-serif;">x</span>"#
        );
        assert_eq!(clean(r#"<span style="font-family: x&quot;&gt;&lt;b">x</span>"#), "<span>x</span>");
    }

    #[test]
    fn ordered_lists_keep_a_numeric_start() {
        assert_eq!(clean(r#"<ol start="3"><li>x</li></ol>"#), r#"<ol start="3"><li>x</li></ol>"#);
        assert_eq!(clean(r#"<ol start="x"><li>x</li></ol>"#), "<ol><li>x</li></ol>");
    }

    #[test]
    fn the_document_is_styled_and_minimal() {
        let o = outgoing("<h1>Title</h1><p>One</p><p></p><p>Two</p><blockquote><p>Q</p></blockquote>");
        assert!(o.html.starts_with("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">"));
        assert!(o.html.contains(r#"<h1 style="margin: 0.6em 0 0.3em 0; font-size: 24px; font-weight: bold; line-height: 1.25;">Title</h1>"#));
        assert!(o.html.contains(r#"<p style="margin: 0;">One</p>"#));
        // The empty paragraph keeps its height.
        assert!(o.html.contains(r#"<p style="margin: 0;"><br></p>"#));
        assert!(o.html.contains(r#"<blockquote style="margin: 0 0 0 0.8ex; padding-left: 1ex; border-left: 2px solid #c8c8c8;">"#));
        assert!(!o.html.contains("http"), "no remote resource: {}", o.html);
    }

    #[test]
    fn styling_does_not_change_the_cleaned_html_beyond_styles() {
        // What the sanitizer keeps, the styling writes back: no tag or text
        // is lost on the way (a second barrier, not a second filter).
        let html = r#"<p>a &amp; b &lt;c&gt; "d"</p><ul><li><p><strong>x</strong> <a href="https://x.fr">link</a></p></li></ul>"#;
        let o = outgoing(html);
        assert!(o.html.contains("a &amp; b &lt;c&gt; &quot;d&quot;"));
        assert!(o.html.contains(r#"<strong>x</strong> <a href="https://x.fr" rel="noopener noreferrer">link</a>"#));
        let again = clean(&o.html);
        assert!(again.contains(r#"<a href="https://x.fr" rel="noopener noreferrer">link</a>"#));
    }

    #[test]
    fn paragraphs_are_lines_and_empty_ones_blank_lines() {
        assert_eq!(text("<p>Hello,</p><p></p><p>See you</p><p>A.</p>"), "Hello,\n\nSee you\nA.");
        assert_eq!(text("<p>one<br>two</p>"), "one\ntwo");
        assert_eq!(text("<p>end</p><p></p><p></p>"), "end");
    }

    #[test]
    fn headings_are_lines() {
        assert_eq!(text("<h1>Plan</h1><p>Text</p><h3>Sub</h3>"), "Plan\nText\nSub");
    }

    #[test]
    fn lists_are_marked_numbered_and_nested() {
        let html = "<ul><li><p>one</p></li><li><p>two</p><ul><li><p>deep</p></li></ul></li></ul>\
                    <ol start=\"3\"><li><p>three</p><p>more</p></li><li><p>four</p></li></ol>";
        assert_eq!(text(html), "- one\n- two\n  - deep\n3. three\n   more\n4. four");
        assert_eq!(text("<ol><li><p>a</p></li><li><p>b</p></li></ol>"), "1. a\n2. b");
    }

    #[test]
    fn quotations_are_prefixed_and_nested() {
        let html = "<p>Yes.</p><blockquote><p>On Monday, Alice wrote:</p><blockquote><p>Lunch?</p></blockquote><p></p><p>Sure</p></blockquote>";
        assert_eq!(text(html), "Yes.\n> On Monday, Alice wrote:\n>> Lunch?\n>\n> Sure");
    }

    #[test]
    fn links_show_their_address_when_it_is_not_their_text() {
        assert_eq!(text(r#"<p>See <a href="https://x.fr/doc">the doc</a>.</p>"#), "See the doc <https://x.fr/doc>.");
        assert_eq!(text(r#"<p><a href="https://x.fr">https://x.fr</a></p>"#), "https://x.fr");
        assert_eq!(text(r#"<p><a href="mailto:a@x.fr">a@x.fr</a></p>"#), "a@x.fr");
        assert_eq!(text(r#"<p><a href="mailto:a@x.fr">Alice</a></p>"#), "Alice <a@x.fr>");
    }

    #[test]
    fn formatting_is_detected() {
        assert!(!to_text("<p>Plain</p><p></p><blockquote><p>quoted</p></blockquote>").1);
        assert!(!to_text(r#"<p><a href="https://x.fr">https://x.fr</a></p>"#).1);
        for html in [
            "<h2>T</h2>",
            "<ul><li><p>x</p></li></ul>",
            "<p><strong>x</strong></p>",
            "<p><em>x</em></p>",
            "<p><u>x</u></p>",
            r#"<p><span style="color: #2f6fd1">x</span></p>"#,
            r#"<p><a href="https://x.fr">here</a></p>"#,
        ] {
            assert!(to_text(html).1, "{html}");
        }
        // A style the core drops is no formatting.
        assert!(!to_text(r#"<p><span style="color: #123456">x</span></p>"#).1);
    }

    #[test]
    fn unicode_and_entities_survive() {
        assert_eq!(text("<p>Très bien&nbsp;!&nbsp;🎉 &eacute;</p>"), "Très bien ! 🎉 é");
        assert!(outgoing("<p>Très bien 🎉</p>").html.contains("Très bien 🎉"));
    }

    #[test]
    fn text_between_blocks_is_a_paragraph() {
        assert_eq!(text("loose<p>p</p>tail"), "loose\np\ntail");
        assert_eq!(text(""), "");
    }

    /// A reply as the editor writes it: the user's text, a blank line, the
    /// introduction, the quotation; then a signature.
    const REPLY: &str = "<p>Très <strong>bien</strong>,</p><p></p><ul><li><p>un</p></li></ul><p></p><p>-- </p><p>Alex</p><p>On Nov 16, Victor wrote:</p><blockquote><p>Bonjour</p></blockquote>";

    #[test]
    fn ai_view_keeps_the_users_blocks_only() {
        let v = ai_view(REPLY);
        let own: Vec<&str> = v.own.iter().map(|r| &v.html[r.clone()]).collect();
        assert_eq!(own, ["<p>Très <strong>bien</strong>,</p><p></p><ul><li><p>un</p></li></ul>"]);
        assert_eq!(v.own_text, ["Très bien,\n\n- un"]);
        assert!(v.signature);
        // The text view: a line per text block, no marker, no quotation sign.
        assert_eq!(v.view, "Très bien,\n\nun\n\n-- \nAlex\nOn Nov 16, Victor wrote:\nBonjour");
        // A text written between quoted blocks is a second run.
        let v = ai_view("<p>Oui.</p><blockquote><p>Q1</p></blockquote><p>Non.</p><blockquote><p>Q2</p></blockquote>");
        let own: Vec<&str> = v.own.iter().map(|r| &v.html[r.clone()]).collect();
        assert_eq!(own, ["<p>Oui.</p>", "<p>Non.</p>"]);
    }

    #[test]
    fn ai_view_reads_entities_and_breaks_as_the_editor() {
        let v = ai_view("<p>a&nbsp;&amp;&lt;b<br>c</p><h2>T</h2>");
        assert_eq!(v.view, "a\u{a0}&<b\nc\nT");
        assert_eq!(v.html, "<p>a&nbsp;&amp;&lt;b<br>c</p><h2>T</h2>");
    }

    #[test]
    fn ai_answers_become_blocks_of_the_editor() {
        assert_eq!(ai_block("<p>Très <strong>bien</strong></p><h2>T</h2>"), "<p>Très <strong>bien</strong></p><h2>T</h2>");
        // Plain text: a paragraph per line; inline tags kept on their line.
        assert_eq!(ai_block("Un\n\nDeux <em>x</em>"), "<p>Un</p><p></p><p>Deux <em>x</em></p>");
        // What the editor cannot hold leaves, its text stays.
        assert_eq!(ai_block("<table><tr><td>x</td></tr></table><script>y</script>"), "<p>x</p>");
    }
}
