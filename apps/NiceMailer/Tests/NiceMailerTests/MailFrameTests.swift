import Foundation
import Testing
import WebKit
@testable import NiceMailer

/// The frame of an HTML message (`MailFrame`): which mails keep their own
/// colours, and, in a web view, that the CSS of a mail stays in its frame.
@MainActor
@Suite(.serialized)
struct MailFrameTests {
    @Test func aMailHasItsOwnDarkModeOnlyThroughItsStyles() {
        let dark = "<style>.a{color:#000}@media (prefers-color-scheme:dark){.a{color:#fff}}</style><p class=\"a\">x</p>"
        #expect(MailFrame.hasOwnDarkMode(dark))
        #expect(MailFrame.colours(dark) == .own)
        // Without styles, or with the words in the text only: a mail for a
        // white page.
        #expect(!MailFrame.hasOwnDarkMode("<p>prefers-color-scheme</p>"))
        #expect(!MailFrame.hasOwnDarkMode("<style>.a{color:#000}</style><p>prefers-color-scheme</p>"))
        #expect(MailFrame.colours("<p>x</p>") == .paper)
    }

    @Test func eachMessageIsInAShadowRoot() {
        let frame = MailFrame.wrap("<p>x</p>", base: "p{}", sheet: true)
        #expect(frame == "<div class=\"nm-msg paper\"><template shadowrootmode=\"open\"><style>p{}</style><div class=\"nm-body\"><p>x</p></div></template></div>")
        let own = MailFrame.wrap("<style>@media (prefers-color-scheme:dark){p{}}</style><p>x</p>", base: "", sheet: true)
        #expect(own.hasPrefix("<div class=\"nm-msg\"><template"))
        // With Dark Reader, no white sheet.
        #expect(MailFrame.wrap("<p>x</p>", base: "").hasPrefix("<div class=\"nm-msg\"><template"))
    }

    @Test func theStylesOfAMailStayInItsFrame() async throws {
        let base = MailFrame.baseStyle(hiding: ".gmail_quote { display: none; }")
        let page = """
        <html><head><style>p { letter-spacing: 3px; } .att { color: rgb(0, 0, 255); }</style></head><body>
        \(MailFrame.wrap("<style>p{color:rgb(255,0,0)} .att{color:rgb(255,0,0)} :host{margin-left:7px}</style><p id=\"a\">A</p><div class=\"gmail_quote\">old</div>", base: base))
        \(MailFrame.wrap("<p id=\"b\">B</p>", base: base))
        <div class="att" id="tile">tile</div>
        </body></html>
        """
        let view = try await load(page)
        let js = """
        (() => {
        const [a, b] = [...document.querySelectorAll('.nm-msg')].map(h => h.shadowRoot);
        const p = (r, id) => getComputedStyle(r.getElementById(id));
        return [p(a, 'a').color, p(b, 'b').color, getComputedStyle(document.getElementById('tile')).color,
                p(a, 'a').letterSpacing, getComputedStyle(a.querySelector('.gmail_quote')).display,
                getComputedStyle(a.host).marginLeft].join('|');
        })()
        """
        let result = try await view.evaluateJavaScript(js) as? String
        // A's rule colours A only; it reaches neither B nor the app's tile;
        // the page's rule does not enter the frames; the hiding rules do;
        // `:host` (the mail's `:root`) styles A's frame.
        #expect(result == "rgb(255, 0, 0)|rgb(0, 0, 0)|rgb(0, 0, 255)|normal|none|7px")
    }

    /// Loads `html` in a web view with JavaScript off, as the reader does.
    private func load(_ html: String) async throws -> WKWebView {
        let config = WKWebViewConfiguration()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 600, height: 400), configuration: config)
        // Light: the colours compared are those of a light page.
        view.appearance = NSAppearance(named: .aqua)
        let waiter = FrameWaiter()
        view.navigationDelegate = waiter
        await withCheckedContinuation { continuation in
            waiter.done = { continuation.resume() }
            view.loadHTMLString(html, baseURL: nil)
        }
        return view
    }
}

private final class FrameWaiter: NSObject, WKNavigationDelegate {
    var done: (() -> Void)?
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        done?()
        done = nil
    }
}
