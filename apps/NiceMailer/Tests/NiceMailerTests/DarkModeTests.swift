import Foundation
import Testing
import WebKit
@testable import NiceMailer

/// Dark Reader in the HTML reader (`DarkMode`): which pages need it, and,
/// in a web view configured like the reader's (JavaScript of the page off,
/// Content Security Policy, a mail in its frame), what it does to a mail
/// made for a white page.
@MainActor
@Suite(.serialized)
struct DarkModeTests {
    @Test func onlyAPageWithAMailForAWhitePageNeedsIt() {
        let own = "<style>@media (prefers-color-scheme:dark){p{color:#fff}}</style><p>x</p>"
        #expect(!DarkMode.needed([own]))
        #expect(DarkMode.needed([own, "<p style=\"color:#000\">y</p>"]))
        #expect(!DarkMode.needed([]))
    }

    @Test func itIsBundledWithTheApp() {
        #expect(DarkMode.available)
        #expect(DarkMode.theme.contains("darkSchemeBackgroundColor: '#1b1d23'"))
        #expect(DarkMode.theme.contains("immediateModify: true"))
    }

    /// A mail made for a white page: white background, dark text, an
    /// orange button.
    static let mail = """
    <div class="probe" style="background-color:#ffffff;color:#222222;padding:8px">Bonjour
    <a class="button" href="https://example.org" style="background:#ffa800;color:#000000;padding:8px">Télécharger</a></div>
    """

    func colours(dark: Bool) async throws -> (background: [Double], text: [Double], button: [Double], fetches: String) {
        let page = """
        <html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="\(ContentPolicy.header)">
        <style>:root { color-scheme: light dark; }</style></head><body>
        \(MailFrame.wrap(Self.mail, base: MailFrame.baseStyle(hiding: "")))
        </body></html>
        """
        let config = WKWebViewConfiguration()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.websiteDataStore = .nonPersistent()
        config.userContentController.addUserScript(try #require(DarkMode.userScript()))
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 600, height: 400), configuration: config)
        view.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        let waiter = DarkWaiter()
        view.navigationDelegate = waiter
        await withCheckedContinuation { continuation in
            waiter.done = { continuation.resume() }
            view.loadHTMLString(page, baseURL: nil)
        }
        // Dark Reader works in the next frames.
        try await Task.sleep(for: .milliseconds(400))
        let js = """
        (() => { const r = document.querySelector('.nm-msg').shadowRoot;
          const s = e => getComputedStyle(r.querySelector(e));
          return [s('.probe').backgroundColor, s('.probe').color, s('.button').backgroundColor].join('|'); })()
        """
        let result = try #require(try await view.evaluateJavaScript(js) as? String)
        let fetches = try await view.evaluateJavaScript("String(window.__nmDarkFetches)", in: nil, contentWorld: DarkMode.world) as? String ?? ""
        let rgb = result.split(separator: "|").map { part in
            part.filter { "0123456789,.".contains($0) }.split(separator: ",").prefix(3).compactMap { Double($0) }
        }
        return (rgb[0], rgb[1], rgb[2], fetches)
    }

    /// Relative luminance, 0 (black) to 1 (white), roughly.
    func light(_ c: [Double]) -> Double { (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) / 255 }

    @Test func inTheDarkThemeAWhiteMailTakesTheThemesColours() async throws {
        let c = try await colours(dark: true)
        // The white background is the theme's, the dark text light.
        #expect(c.background == [27, 29, 35])
        #expect(light(c.text) > 0.6)
        // The orange button stays orange: red over green over blue, not
        // grey, not the background.
        #expect(c.button[0] > c.button[1] && c.button[1] > c.button[2] && c.button[0] - c.button[2] > 80)
        // Nothing was asked for, or it was refused.
        #expect(c.fetches == "0")
    }

    @Test func inTheLightThemeAMailKeepsItsColours() async throws {
        let c = try await colours(dark: false)
        #expect(c.background == [255, 255, 255])
        #expect(c.text == [34, 34, 34])
        #expect(c.button == [255, 168, 0])
    }
}

private final class DarkWaiter: NSObject, WKNavigationDelegate {
    var done: (() -> Void)?
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        done?()
        done = nil
    }
}
