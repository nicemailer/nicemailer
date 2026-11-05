import Foundation

/// The frame of one HTML message in the reader's page
/// (`docs/thread-display.md`, "HTML of the sender").
///
/// The core keeps the sender's CSS, sanitized (`css.rs`): `style`
/// attributes, and its `<style>` blocks gathered at the start of the body.
/// A page holds the whole thread, and the app's own lines (headers of the
/// messages, attachment tiles): the CSS of one mail must reach neither.
///
/// Each message is therefore put in a **shadow root**, written in the HTML
/// itself (`<template shadowrootmode="open">`, read by WebKit's parser
/// without any script, macOS 14). Styles inside a shadow root apply to it
/// alone, and the page's styles do not enter it, except inherited
/// properties (the text colour and font of the theme, which a mail without
/// its own keeps). The core rewrote the mail's selectors for this frame:
/// `:root` and `html` are `:host`, `body` is `.nm-body`.
///
/// The frame also contains the mail: `contain: layout paint` makes it the
/// box of any positioned element of the mail (no line of the mail can
/// cover the app's lines), and clips what overflows it.
///
/// Pure functions, tested on their own (`MailFrameTests`), and in a web
/// view for what only WebKit can tell (isolation, containment).
enum MailFrame {
    /// Whether the mail brings its own dark mode: its CSS (the `<style>`
    /// the core puts first) has `prefers-color-scheme` rules. Such a mail
    /// is shown with its colours, its dark ones in a dark theme, as its
    /// author designed them.
    static func hasOwnDarkMode(_ body: String) -> Bool {
        guard body.hasPrefix("<style>"), let end = body.range(of: "</style>") else { return false }
        return body[..<end.lowerBound].contains("prefers-color-scheme")
    }

    /// How the colours of a mail are shown.
    enum Colours: Equatable {
        /// The mail's own colours, light or dark as the theme asks (its
        /// `prefers-color-scheme` rules).
        case own
        /// A mail made for a white page, in a dark theme: shown on a white
        /// sheet, as written, until the reader adapts its colours.
        case paper
    }

    static func colours(_ body: String) -> Colours {
        hasOwnDarkMode(body) ? .own : .paper
    }

    /// The styles of the reader inside each frame, before the mail's own
    /// (so that the mail's win where both apply).
    ///
    /// - `hiding`: the rules that hide quoted history and signatures
    ///   (`threadHTML`), which must be inside the frame to reach them.
    static func baseStyle(hiding: String, scale: Double) -> String {
        """
        :host { display: block; contain: layout paint; overflow-x: auto; overflow-y: hidden; }
        :host(.paper) { color-scheme: light; background: #fff; color: #000; padding: \(Int(8 * scale))px; }
        /* The page has no doctype ("quirks" mode, as mail apps): a table
           would not inherit the size and font of the text around it. */
        table { font-family: inherit; font-size: inherit; }
        img { max-width: 100%; height: auto; }
        /* Images without a source (tracking pixels removed by the core),
           remote images not loaded, `cid:` images without a part. */
        img:not([src]), img[data-nm-src], img[src^="cid:"] { display: none; }
        \(hiding)
        """
    }

    /// The message `body` (sanitized HTML from the core) in its frame.
    static func wrap(_ body: String, base: String) -> String {
        let paper = colours(body) == .paper ? " paper" : ""
        return "<div class=\"nm-msg\(paper)\"><template shadowrootmode=\"open\"><style>\(base)</style>"
            + "<div class=\"nm-body\">\(body)</div></template></div>"
    }
}
