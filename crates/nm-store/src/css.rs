//! Sanitization of the sender's CSS: the `style` attributes and the
//! `<style>` blocks of an HTML mail (`docs/thread-display.md`, "HTML of the
//! sender").
//!
//! A modern newsletter is laid out by CSS alone: a centered column of fixed
//! width, a hidden preview text, colours, its own dark mode. Dropping the
//! CSS, as the sanitizer did until parser version 10, shows a different
//! mail. Keeping it as written would let the mail load what it wants: a
//! `url()` is a request, hence a signal that the mail was opened, and
//! can be made to depend on the reader's actions (`:hover`) or screen
//! (`@media (max-width…)`).
//!
//! The CSS is therefore read with a real tokenizer (`cssparser`, the one of
//! Servo and Firefox), never with regular expressions: CSS escapes
//! (`u\72l(`), comments and nesting make text matching unreliable. What
//! comes out is rebuilt from the tokens, so nothing the tokenizer did not
//! understand passes:
//!
//! - every `url()` is rewritten: a remote image to the app's
//!   `nm-remote://image?u=…` (loaded only in the images display, like an
//!   `<img>`, `remote_images.rs`), a remote font to `nm-remote://font?u=…`,
//!   a `data:` image or font kept; anything else drops its declaration;
//! - no `url()` in a rule that depends on the reader: under `:hover`,
//!   `:focus`, `:active`… or in a `@media` on the size of the screen;
//! - `@import`, `@charset`, `@namespace`, `@keyframes` and the other
//!   at-rules are dropped; `@media`, `@supports` and `@font-face` are kept;
//! - `expression()`, `image-set()`, `behavior`, `-moz-binding`,
//!   `position: fixed` and `sticky` are dropped;
//! - selectors are made to fit the reader: each message is shown in its own
//!   shadow root (`InlineStyle` in the app), so `:root` and `html` become
//!   `:host`, `body` becomes `.nm-body` (the wrapper of the message), and an
//!   id `#x` becomes `#nm-x`, as the sanitizer prefixes the ids of the HTML.
//!
//! The result never holds `<`, so a block cannot close its `<style>`
//! element early, whatever the mail wrote.

use cssparser::{ParseError, Parser, ToCss, Token};

use crate::remote_images;

/// Bytes of CSS read per mail, all `<style>` blocks together: a real
/// newsletter has a few kB; beyond, the rest is dropped (the mail stays
/// readable, less styled) rather than slowing every display.
pub const MAX_SHEET: usize = 256 * 1024;
/// Bytes of one `style` attribute.
pub const MAX_INLINE: usize = 16 * 1024;
/// Nesting of `@media` / `@supports` and of functions in a value.
const MAX_DEPTH: usize = 8;

/// The remote fonts of a mail: `nm-remote://font?u=<percent-encoded URL>`,
/// the same encoding as images (`remote_images::rewrite_source`).
pub const FONT_SOURCE_PREFIX: &str = "nm-remote://font?u=";

/// What a sanitization found.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Sanitized {
    pub css: String,
    /// A remote image was kept (`nm-remote://image`).
    pub remote_images: bool,
}

/// What a `url()` may load where it is written.
#[derive(Debug, Clone, Copy, PartialEq)]
enum UrlUse {
    /// No URL: the declaration is dropped.
    None,
    Image,
    Font,
}

#[derive(Default)]
struct Context {
    remote_images: bool,
}

/// Rust note: `cssparser` reports errors through `ParseError<E>`, `E`
/// being an error type of the caller. This sanitizer never fails (bad
/// input is dropped, not an error), so `E` is `()`.
type Result<T> = std::result::Result<T, ParseError<()>>;

/// A `<style>` block: its rules, sanitized.
pub fn sanitize_stylesheet(css: &str) -> Sanitized {
    if css.len() > MAX_SHEET {
        return Sanitized::default();
    }
    let mut parser = Parser::new(css);
    let mut context = Context::default();
    let mut out = String::new();
    rules(&mut parser, &mut context, &mut out, true, 0);
    Sanitized { css: no_markup(out), remote_images: context.remote_images }
}

/// A `style` attribute: its declarations, sanitized.
pub fn sanitize_declarations(css: &str) -> Sanitized {
    if css.len() > MAX_INLINE {
        return Sanitized::default();
    }
    let mut parser = Parser::new(css);
    let mut context = Context::default();
    let css = declarations(&mut parser, &mut context, Block::Style { urls: true });
    Sanitized { css: no_markup(css), remote_images: context.remote_images }
}

/// The kind of a block of declarations.
#[derive(Debug, Clone, Copy)]
enum Block {
    /// A rule or a `style` attribute; `urls`: whether a `url()` may stay.
    Style { urls: bool },
    /// `@font-face`: only the descriptors of a font.
    FontFace { urls: bool },
}

/// The rules of a sheet or of an `@media` block, appended to `out`.
fn rules(p: &mut Parser, context: &mut Context, out: &mut String, urls: bool, depth: usize) {
    loop {
        // Rust note: the token is borrowed from the parser; `.clone()`
        // makes it ours, so the parser can be used again below.
        let token = match p.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return,
        };
        match token {
            Token::WhiteSpace(_) | Token::CDO | Token::CDC | Token::Semicolon => {}
            Token::AtKeyword(name) => {
                let name = name.to_ascii_lowercase();
                let (prelude, has_block) = at_prelude(p);
                match name.as_str() {
                    "media" | "supports" if has_block && depth < MAX_DEPTH => {
                        // `@supports` asks about the browser, not the
                        // reader: no restriction on URLs.
                        let inner_urls = urls && (name == "supports" || media_allows_urls(&prelude));
                        let mut inner = String::new();
                        let _ = p.parse_nested_block(|p| -> Result<()> {
                            rules(p, context, &mut inner, inner_urls, depth + 1);
                            Ok(())
                        });
                        if !inner.is_empty() {
                            out.push('@');
                            out.push_str(&name);
                            out.push(' ');
                            out.push_str(prelude.trim());
                            out.push('{');
                            out.push_str(&inner);
                            out.push('}');
                        }
                    }
                    "font-face" if has_block => {
                        let body = nested(p, |p| declarations(p, context, Block::FontFace { urls }));
                        if !body.is_empty() {
                            out.push_str("@font-face{");
                            out.push_str(&body);
                            out.push('}');
                        }
                    }
                    _ => {
                        if has_block {
                            skip_block(p);
                        }
                    }
                }
            }
            // A stray block or closing token: ignored.
            Token::CurlyBracketBlock => skip_block(p),
            Token::CloseCurlyBracket | Token::CloseParenthesis | Token::CloseSquareBracket => {}
            first => {
                let Some(selector) = selector_prelude(p, first) else { return };
                // A URL that loads on hover or focus would tell the sender
                // what the reader does.
                let interactive = ["hover", "focus", "active", "visited", "target", "checked"]
                    .iter()
                    .any(|pseudo| selector.to_ascii_lowercase().contains(&format!(":{pseudo}")));
                let block = Block::Style { urls: urls && !interactive };
                let body = nested(p, |p| declarations(p, context, block));
                if !selector.trim().is_empty() && !body.is_empty() {
                    out.push_str(selector.trim());
                    out.push('{');
                    out.push_str(&body);
                    out.push('}');
                }
            }
        }
    }
}

/// Runs `f` on the block the parser just opened; an empty string if the
/// block cannot be read.
fn nested(p: &mut Parser, f: impl FnOnce(&mut Parser) -> String) -> String {
    p.parse_nested_block(|p| -> Result<String> {
        let s = f(p);
        // Whatever `f` left is consumed: a nested parser must be read to
        // its end.
        while p.next_including_whitespace().is_ok() {}
        Ok(s)
    })
    .unwrap_or_default()
}

fn skip_block(p: &mut Parser) {
    let _ = p.parse_nested_block(|p| -> Result<()> {
        while p.next_including_whitespace().is_ok() {}
        Ok(())
    });
}

/// The prelude of an at-rule, up to its block (`true`) or its `;`
/// (`false`), as text.
fn at_prelude(p: &mut Parser) -> (String, bool) {
    let mut out = String::new();
    loop {
        let token = match p.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return (out, false),
        };
        match token {
            Token::CurlyBracketBlock => return (out, true),
            Token::Semicolon => return (out, false),
            other => copy_token(p, &other, &mut out, 0),
        }
    }
}

/// Whether a `@media` query leaves the URLs of its rules: not when it
/// depends on the reader's screen or device (a different image per width
/// tells the sender which device read the mail). `prefers-color-scheme`
/// is allowed: the mail's own dark mode is worth it.
fn media_allows_urls(prelude: &str) -> bool {
    let prelude = prelude.to_ascii_lowercase();
    !["width", "height", "resolution", "orientation", "aspect", "hover", "pointer", "device", "monochrome", "color-gamut"]
        .iter()
        .any(|word| prelude.contains(word))
}

/// A piece of a selector: a token, or a nested part (`:not(…)`, `[…]`)
/// already written as text.
enum Piece<'i> {
    Token(Token<'i>),
    Raw(String),
}

/// The selector of a rule, from its first token up to its block, adapted
/// to the reader (see the module's comment). `None` if the sheet ends
/// before a block.
fn selector_prelude<'i>(p: &mut Parser<'i>, first: Token<'i>) -> Option<String> {
    let mut pieces = Vec::new();
    let mut next = Some(first);
    loop {
        let token = match next.take() {
            Some(t) => t,
            None => match p.next_including_whitespace() {
                Ok(Token::CurlyBracketBlock) => break,
                Ok(t) => t.clone(),
                Err(_) => return None,
            },
        };
        if is_block_start(&token) {
            let mut text = String::new();
            copy_token(p, &token, &mut text, 0);
            pieces.push(Piece::Raw(text));
        } else {
            pieces.push(Piece::Token(token));
        }
    }
    Some(rewrite_selector(&pieces))
}

/// The selector's pieces as text: `:root` and `html` become `:host`,
/// `body` becomes `.nm-body`, an id `#x` becomes `#nm-x`.
fn rewrite_selector(pieces: &[Piece]) -> String {
    let token = |i: usize| match pieces.get(i) {
        Some(Piece::Token(t)) => Some(t),
        _ => None,
    };
    let mut out = String::new();
    let mut i = 0;
    while i < pieces.len() {
        let previous = i.checked_sub(1).and_then(token);
        match &pieces[i] {
            Piece::Raw(text) => out.push_str(text),
            Piece::Token(Token::Colon)
                if matches!(token(i + 1), Some(Token::Ident(name)) if name.eq_ignore_ascii_case("root"))
                    && !matches!(previous, Some(Token::Colon)) =>
            {
                out.push_str(":host");
                i += 1;
            }
            // A type selector: not a class (`.body`), a pseudo-class or a
            // part of a longer name.
            Piece::Token(t @ Token::Ident(name))
                if !matches!(previous, Some(Token::Delim('.')) | Some(Token::Colon) | Some(Token::Delim('-'))) =>
            {
                if name.eq_ignore_ascii_case("html") {
                    out.push_str(":host");
                } else if name.eq_ignore_ascii_case("body") {
                    out.push_str(".nm-body");
                } else {
                    let _ = t.to_css(&mut out);
                }
            }
            Piece::Token(Token::IDHash(id)) => {
                out.push_str("#nm-");
                let _ = cssparser::serialize_identifier(id, &mut out);
            }
            Piece::Token(other) => {
                let _ = other.to_css(&mut out);
            }
        }
        i += 1;
    }
    out
}

fn is_block_start(t: &Token) -> bool {
    matches!(t, Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock)
}

/// Copies a token, and the block it opens, as text, without any
/// filtering: only used for selectors and at-rule preludes, where no URL
/// can load anything.
fn copy_token(p: &mut Parser, token: &Token, out: &mut String, depth: usize) {
    let _ = token.to_css(out);
    let close = match token {
        Token::Function(_) | Token::ParenthesisBlock => ')',
        Token::SquareBracketBlock => ']',
        Token::CurlyBracketBlock => '}',
        _ => return,
    };
    let _ = p.parse_nested_block(|p| -> Result<()> {
        while let Ok(t) = p.next_including_whitespace() {
            let t = t.clone();
            if depth < MAX_DEPTH {
                copy_token(p, &t, out, depth + 1);
            }
        }
        Ok(())
    });
    out.push(close);
}

/// The declarations of a block, sanitized, as `name:value;` text.
fn declarations(p: &mut Parser, context: &mut Context, block: Block) -> String {
    let mut out = String::new();
    loop {
        let token = match p.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return out,
        };
        let name = match token {
            Token::WhiteSpace(_) | Token::Semicolon => continue,
            Token::Ident(name) => name.to_ascii_lowercase(),
            // Not a declaration: skipped up to the next `;`.
            _ => {
                let _ = p.parse_until_after(cssparser::Delimiter::Semicolon, |p| -> Result<()> {
                    while p.next_including_whitespace().is_ok() {}
                    Ok(())
                });
                continue;
            }
        };
        let value = p
            .parse_until_after(cssparser::Delimiter::Semicolon, |p| -> Result<Option<String>> {
                Ok(declaration_value(p, context, &name, block))
            })
            .ok()
            .flatten();
        if let Some(value) = value {
            let _ = cssparser::serialize_identifier(&name, &mut out);
            out.push(':');
            out.push_str(value.trim());
            out.push(';');
        }
    }
}

/// The value of the declaration `name`, or `None` to drop it.
fn declaration_value(p: &mut Parser, context: &mut Context, name: &str, block: Block) -> Option<String> {
    // The colon after the name.
    let colon = loop {
        match p.next_including_whitespace() {
            Ok(Token::WhiteSpace(_)) => continue,
            Ok(Token::Colon) => break true,
            _ => break false,
        }
    };
    let mut value = String::new();
    let mut refused = !colon;
    let url_use = match block {
        Block::FontFace { urls } => {
            const DESCRIPTORS: [&str; 7] =
                ["font-family", "src", "font-style", "font-weight", "font-stretch", "font-display", "unicode-range"];
            if !DESCRIPTORS.contains(&name) {
                refused = true;
            }
            if name == "src" && urls { UrlUse::Font } else { UrlUse::None }
        }
        Block::Style { urls } => {
            // Old ways of running code from CSS, in Internet Explorer and
            // Firefox: dropped whatever their value.
            if matches!(name, "behavior" | "-ms-behavior" | "-moz-binding" | "-webkit-binding") {
                refused = true;
            }
            let image = matches!(
                name,
                "background" | "background-image" | "list-style" | "list-style-image" | "border-image"
                    | "border-image-source" | "content"
            );
            if image && urls { UrlUse::Image } else { UrlUse::None }
        }
    };
    let mut remote = false;
    value_tokens(p, url_use, &mut value, &mut refused, &mut remote, 0);
    if refused || value.trim().is_empty() {
        return None;
    }
    // Out of the message's frame: a fixed or sticky element could cover
    // the app's own lines (a fake sender, a fake attachment).
    if name == "position" {
        let v = value.to_ascii_lowercase();
        if v.contains("fixed") || v.contains("sticky") {
            return None;
        }
    }
    if remote {
        context.remote_images = true;
    }
    Some(value)
}

/// The tokens of a value, rebuilt in `out`; `refused` set when the value
/// holds something that must not pass.
fn value_tokens(p: &mut Parser, url_use: UrlUse, out: &mut String, refused: &mut bool, remote: &mut bool, depth: usize) {
    while let Ok(token) = p.next_including_whitespace() {
        let token = token.clone();
        match token {
            Token::UnquotedUrl(url) => push_url(&url, url_use, out, refused, remote),
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                if lower == "url" || lower == "src" {
                    // `url("…")`: one string inside.
                    let url = p
                        .parse_nested_block(|p| -> Result<Option<String>> {
                            let mut found = None;
                            while let Ok(t) = p.next() {
                                match t {
                                    Token::QuotedString(s) if found.is_none() => found = Some(s.to_string()),
                                    _ => found = None,
                                }
                            }
                            Ok(found)
                        })
                        .ok()
                        .flatten();
                    match url {
                        Some(url) => push_url(&url, url_use, out, refused, remote),
                        None => *refused = true,
                    }
                } else if lower.contains("expression") || lower.ends_with("image-set") || lower == "element"
                    || lower == "image" || lower == "cross-fade" || depth >= MAX_DEPTH
                {
                    // Code (old Internet Explorer), or functions whose
                    // strings are URLs.
                    *refused = true;
                    skip_block(p);
                } else {
                    let _ = Token::Function(name).to_css(out);
                    let _ = p.parse_nested_block(|p| -> Result<()> {
                        value_tokens(p, url_use, out, refused, remote, depth + 1);
                        Ok(())
                    });
                    out.push(')');
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let close = match token {
                    Token::ParenthesisBlock => ')',
                    Token::SquareBracketBlock => ']',
                    _ => '}',
                };
                let _ = token.to_css(out);
                let _ = p.parse_nested_block(|p| -> Result<()> {
                    value_tokens(p, url_use, out, refused, remote, depth + 1);
                    Ok(())
                });
                out.push(close);
            }
            Token::BadUrl(_) | Token::BadString(_) => *refused = true,
            other => {
                let _ = other.to_css(out);
            }
        }
    }
}

/// A `url()` of a value: rewritten, kept, or the declaration refused.
fn push_url(url: &str, url_use: UrlUse, out: &mut String, refused: &mut bool, remote: &mut bool) {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let kept = match url_use {
        UrlUse::None => None,
        UrlUse::Image if remote_images::is_remote(trimmed) => {
            *remote = true;
            Some(remote_images::rewrite_source(trimmed))
        }
        UrlUse::Font if remote_images::is_remote(trimmed) => Some(font_source(trimmed)),
        UrlUse::Image if lower.starts_with("data:image/") => Some(trimmed.to_string()),
        UrlUse::Font if lower.starts_with("data:font/") || lower.starts_with("data:application/") => {
            Some(trimmed.to_string())
        }
        _ => None,
    };
    match kept {
        Some(url) => {
            out.push_str("url(");
            let _ = cssparser::serialize_string(&url, out);
            out.push(')');
        }
        None => *refused = true,
    }
}

/// The source the app may load a remote font from (in the images display
/// only): `nm-remote://font?u=…`.
pub fn font_source(url: &str) -> String {
    let image = remote_images::rewrite_source(url);
    format!("{FONT_SOURCE_PREFIX}{}", &image[remote_images::SOURCE_PREFIX.len()..])
}

/// `css` without any `<`: none is needed in the CSS kept (selectors and
/// values use `>`, never `<`), and one could close the `<style>` element
/// (`</style>` in a string). Inside a string `\3c ` is the same character.
fn no_markup(css: String) -> String {
    if css.contains('<') { css.replace('<', "\\3c ") } else { css }
}

/// The text of the `<style>` elements of an HTML document, in order. Those
/// in a comment are not elements, and are left out: the conditional
/// comments of Outlook (`<!--[if mso]><style>…</style><![endif]-->`) hold
/// styles meant for Outlook alone.
///
/// The HTML parser's rule for these elements is simple: the content is raw
/// text up to the first `</style`, whatever it holds. So a scan for that
/// end is exact, and linear.
pub fn style_blocks(html: &str) -> Vec<&str> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(offset) = bytes[i..].iter().position(|&b| b == b'<') {
        i += offset;
        let rest = &bytes[i..];
        if rest.starts_with(b"<!--") {
            // `<!-->` and `<!--->` are empty comments.
            let after = &bytes[i + 4..];
            i = if after.starts_with(b">") {
                i + 5
            } else if after.starts_with(b"->") {
                i + 6
            } else {
                find(bytes, b"-->", i + 4).map_or(bytes.len(), |end| end + 3)
            };
            continue;
        }
        if rest.len() > 6
            && rest[1..6].eq_ignore_ascii_case(b"style")
            && matches!(rest[6], b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/')
        {
            let Some(gt) = bytes[i..].iter().position(|&b| b == b'>') else { break };
            let start = i + gt + 1;
            let end = find_ignore_case(bytes, b"</style", start).unwrap_or(bytes.len());
            // Both ends are at ASCII bytes: valid boundaries of the string.
            out.push(&html[start..end]);
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn find_ignore_case(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack.get(from..)?.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle)).map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(css: &str) -> String {
        sanitize_stylesheet(css).css
    }

    fn inline(css: &str) -> String {
        sanitize_declarations(css).css
    }

    #[test]
    fn layout_colours_and_fonts_pass() {
        let css = inline("max-width:680px; width:100%; margin:0 auto; color:#09144b; font:bold 14px/18px DM Sans, Roboto, sans-serif; display:none !important; mso-hide:all");
        assert_eq!(
            css,
            "max-width:680px;width:100%;margin:0 auto;color:#09144b;font:bold 14px/18px DM Sans, Roboto, sans-serif;display:none !important;mso-hide:all;"
        );
    }

    #[test]
    fn remote_urls_are_rewritten_and_others_dropped() {
        let s = sanitize_declarations("background:url(https://x.org/bg.png) no-repeat; color:red");
        assert_eq!(s.css, r#"background:url("nm-remote://image?u=https%3A%2F%2Fx.org%2Fbg.png") no-repeat;color:red;"#);
        assert!(s.remote_images);
        // Quoted, protocol-relative, data: kept as such.
        assert!(inline("background-image:url('//x.org/a.png')").contains("nm-remote://image?u=https%3A%2F%2Fx.org%2Fa.png"));
        assert_eq!(inline("background-image:url(data:image/png;base64,iVBO)"), r#"background-image:url("data:image/png;base64,iVBO");"#);
        // A URL where none may load, another scheme, an escaped `url(`, a
        // function whose strings are URLs: the declaration goes.
        for refused in [
            "width:url(https://x.org/a)",
            "cursor:url(https://x.org/c.cur),auto",
            "background:url(javascript:alert(1))",
            "background:url(cid:logo)",
            "background-image:image-set('https://x.org/a.png' 1x)",
            "width:expression(alert(1))",
            "behavior:url(x.htc)",
            "-moz-binding:url(x.xml#a)",
            "position:fixed",
            "position: STICKY",
        ] {
            let out = inline(refused);
            assert!(!out.contains("x.org") && !out.contains("expression") && !out.contains("fixed") && !out.to_lowercase().contains("sticky"),
                "{refused} gave {out}");
        }
        assert_eq!(inline("position:relative;top:2px"), "position:relative;top:2px;");
        // An escaped `url(` is a `url(` for the tokenizer: rewritten like
        // the others, never left as written.
        for escaped in [r"background:u\72l(https://x.org/e.png)", r"background:\75 rl(https://x.org/e.png)"] {
            assert_eq!(inline(escaped), r#"background:url("nm-remote://image?u=https%3A%2F%2Fx.org%2Fe.png");"#, "{escaped}");
        }
    }

    #[test]
    fn selectors_fit_the_messages_frame() {
        assert_eq!(sheet(":root{color-scheme:light dark}"), ":host{color-scheme:light dark;}");
        assert_eq!(sheet("html body p, body>.x{margin:0}"), ":host .nm-body p, .nm-body>.x{margin:0;}");
        assert_eq!(sheet("#main .body, a.html{color:red}"), "#nm-main .body, a.html{color:red;}");
        assert_eq!(sheet(".a:not([class^=\"x_\"]) img{color:#fff}"), ".a:not([class^=\"x_\"]) img{color:#fff;}");
        // A pseudo-element stays.
        assert_eq!(sheet("p::first-line{color:red}"), "p::first-line{color:red;}");
    }

    #[test]
    fn at_rules_are_kept_or_dropped() {
        let css = "@import url(https://x.org/a.css); @charset 'utf-8'; @namespace x url(y);
            @media (prefers-color-scheme:dark) { .a:not([class^=\"x_\"]) { background-color:#040a25 !important; background-image:url(https://x.org/d.png) } }
            @media only screen and (max-width:480px) { .logo { width:100% !important; background:url(https://x.org/m.png) } }
            @keyframes spin { from { transform:rotate(0) } }
            @font-face { font-family:'DM Sans'; font-weight:700; src:url(https://x.org/f.woff2) format('woff2'), local(Arial); size-adjust:90% }
            .b:hover { color:#fda200 !important; background:url(https://x.org/h.png) }";
        let out = sheet(css);
        assert!(!out.contains("@import") && !out.contains("@charset") && !out.contains("@namespace") && !out.contains("@keyframes"), "{out}");
        // The mail's dark mode, its image included.
        assert!(out.contains("@media (prefers-color-scheme:dark){.a:not([class^=\"x_\"]){background-color:#040a25 !important;background-image:url(\"nm-remote://image?u=https%3A%2F%2Fx.org%2Fd.png\");}}"), "{out}");
        // By screen width: the layout stays, not the image.
        assert!(out.contains("@media only screen and (max-width:480px){.logo{width:100% !important;}}"), "{out}");
        // The font, in the app's scheme, without an unknown descriptor.
        assert!(out.contains("@font-face{font-family:\"DM Sans\";font-weight:700;src:url(\"nm-remote://font?u=https%3A%2F%2Fx.org%2Ff.woff2\") format(\"woff2\"), local(Arial);}"), "{out}");
        // On hover: the colour, not the image.
        assert!(out.contains(".b:hover{color:#fda200 !important;}"), "{out}");
        assert!(!out.contains("h.png") && !out.contains("m.png"), "{out}");
    }

    #[test]
    fn nothing_can_close_the_style_element() {
        let out = sheet("p{content:'</style><script>alert(1)</script>'} </style><img src=x>{color:red}");
        assert!(!out.contains('<'), "{out}");
        let out = inline("font-family:'</style>'");
        assert!(!out.contains('<'), "{out}");
    }

    #[test]
    fn broken_css_does_not_break_the_rest() {
        let out = sheet("p{color:red;;; : ; 12px; width:}  } .a{color:blue} @media {  .b{color:#000");
        assert!(out.contains("p{color:red;}") && out.contains(".a{color:blue;}"), "{out}");
        assert!(sheet(&"a{".repeat(10_000)).len() < 100_000);
        assert!(inline(&"x:(".repeat(3_000)).is_empty() || !inline(&"x:(".repeat(3_000)).contains('<'));
        // Too large: dropped as a whole.
        assert!(sheet(&"p{color:red}".repeat(30_000)).is_empty());
    }

    #[test]
    fn style_blocks_outside_comments_only() {
        let html = "<html><head><STYLE type=\"text/css\">a{color:red}</style>
            <!--[if mso]><style>.mso{x:1}</style><![endif]-->
            <!--[if !mso]><!--><style>.notmso{x:2}</STYLE ><!--<![endif]-->
            <!--><style>.after{x:3}</style><styles>no</styles></head><body><p>style</p></body></html>";
        assert_eq!(style_blocks(html), ["a{color:red}", ".notmso{x:2}", ".after{x:3}"]);
        // Unclosed: to the end.
        assert_eq!(style_blocks("<style>p{}"), ["p{}"]);
        assert!(style_blocks("<p>no style</p><!-- <style>x</style>").is_empty());
    }
}
