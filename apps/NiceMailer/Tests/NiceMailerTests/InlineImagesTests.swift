import Foundation
import Testing
import WebKit
@testable import NiceMailer

/// Images embedded in a mail (`cid:`): the sources the core writes, read
/// back; the parts that may be shown; and the handler serving them to a web
/// view configured like the HTML reader, Content Security Policy included.
@MainActor
@Suite(.serialized)
struct InlineImagesTests {
    /// A 3×2 PNG.
    static let png = Data(base64Encoded:
        "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAIAAAASFvFNAAAAEElEQVR4nGO44OAAQQxwFgBSNAfh9tMsJgAAAABJRU5ErkJggg==")!

    @Test func theIdsOfThePageAreThoseTheCoreWrote() {
        let html = #"<p><img src="nm-cid://image/12"> <img src="nm-cid://image/7" width="20"></p>"#
            + #"<img data-x="nm-cid://image/99"><img src="cid:logo@x"><img src="nm-cid://image/">"#
        #expect(InlineImages.ids(in: html) == [12, 7])
        #expect(InlineImages.ids(in: "<p>rien</p>").isEmpty)
    }

    @Test func onlyAnAttachmentIdIsAccepted() {
        #expect(InlineImages.attachmentId(URL(string: "nm-cid://image/42")!) == 42)
        for refused in ["nm-cid://image/", "nm-cid://image/4a", "nm-cid://image/../1", "nm-cid://other/4",
                        "nm-remote://image?u=x", "nm-cid://image/1234567890123456789"] {
            #expect(InlineImages.attachmentId(URL(string: refused)!) == nil, "\(refused)")
        }
    }

    @Test func onlyImagesOfAReasonableSizeAreShown() {
        #expect(InlineImages.showable(mimeType: "image/png", size: 2_000))
        #expect(InlineImages.showable(mimeType: "IMAGE/JPEG", size: 2_000))
        #expect(!InlineImages.showable(mimeType: "image/svg+xml", size: 2_000))
        #expect(!InlineImages.showable(mimeType: "application/pdf", size: 2_000))
        #expect(!InlineImages.showable(mimeType: "image/png", size: 0))
        #expect(!InlineImages.showable(mimeType: "image/png", size: InlineImages.maxBytes + 1))
    }

    @Test func thePolicyLoadsOnlyTheAppsSchemesAndData() {
        let csp = ContentPolicy.header
        #expect(csp.contains("default-src 'none'"))
        #expect(csp.contains("img-src nm-remote: nm-cid: data:"))
        #expect(!csp.contains("http"))
        #expect(!csp.contains("script-src"))
    }

    @Test func theHandlerServesTheAllowedPartsOnly() async throws {
        let loader = InlineImageLoader()
        loader.allowed = [12]
        var asked: [Int] = []
        loader.load = { id in
            asked.append(id)
            return (Self.png, "image/png")
        }
        let page = """
        <html><head><meta http-equiv="Content-Security-Policy" content="\(ContentPolicy.header)"></head>
        <body><img id="a" src="nm-cid://image/12"><img id="b" src="nm-cid://image/13"></body></html>
        """
        let view = try await load(page, loader: loader)
        let drawn = try await view.evaluateJavaScript(
            "[document.getElementById('a').naturalWidth, document.getElementById('b').naturalWidth].join(',')") as? String
        // The allowed image is drawn through the policy; the other one is
        // refused without asking for its part.
        #expect(drawn == "3,0")
        #expect(asked == [12])
    }

    /// Loads `html` in a web view with the handler, as the reader does,
    /// and waits for the page (images included).
    private func load(_ html: String, loader: InlineImageLoader) async throws -> WKWebView {
        let config = WKWebViewConfiguration()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.websiteDataStore = .nonPersistent()
        config.setURLSchemeHandler(loader, forURLScheme: InlineImages.scheme)
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 400, height: 300), configuration: config)
        let waiter = Waiter()
        view.navigationDelegate = waiter
        await withCheckedContinuation { continuation in
            waiter.done = { continuation.resume() }
            view.loadHTMLString(html, baseURL: nil)
        }
        return view
    }
}

/// Calls `done` once, when the page has loaded or failed.
private final class Waiter: NSObject, WKNavigationDelegate {
    var done: (() -> Void)?
    private func finish() {
        done?()
        done = nil
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { finish() }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) { finish() }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
        finish()
    }
}
