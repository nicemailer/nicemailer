import Foundation
import Testing
@testable import NiceMailer

/// Remote images (`RemoteImages.swift`): the rules, the page's sources and
/// the download, against a stand-in for the network (`StubNetwork`): no
/// request leaves the machine.
@MainActor
struct RemoteImagesTests {
    func message(html: String? = "<p>x</p>", remote: Bool = true, role: String = "inbox", auth: String = "pass",
                 format: String? = "images") -> MessageView {
        MessageView(
            id: 1, folderId: 1, folderRole: role, subject: "S", fromName: "A", fromAddr: "a@x.org",
            to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: "x", isSeen: true,
            isFlagged: false, isAnswered: false, hasAttachments: false, authResult: auth, bodyHtml: html,
            hasRemoteContent: remote, fromBodyFormat: format, segments: [], onlyQuotes: false, expanded: true,
            showRecipients: false, attachments: [])
    }

    // MARK: Rules

    @Test func imagesAreOfferedOnlyWithRemoteImagesOutsideBlockedFolders() {
        #expect(RemoteImages.offered(message()))
        #expect(!RemoteImages.offered(message(remote: false)))
        #expect(!RemoteImages.offered(message(html: nil)))
        for role in ["junk", "quarantine", "banned"] {
            #expect(!RemoteImages.offered(message(role: role)), "\(role)")
        }
        #expect(RemoteImages.offered(message(role: "archive")))
    }

    @Test func imagesLoadByThemselvesOnlyForAnAuthenticatedRememberedSender() {
        #expect(RemoteImages.automatic(message()))
        for auth in ["unknown", "none", "fail"] {
            #expect(!RemoteImages.automatic(message(auth: auth)), "\(auth)")
        }
        #expect(!RemoteImages.automatic(message(format: "html")))
        #expect(!RemoteImages.automatic(message(format: nil)))
        #expect(!RemoteImages.automatic(message(role: "junk")))
    }

    @Test func imagesLoadInTheImagesModeForUnfoldedMessagesOnly() {
        let m = message()
        #expect(RemoteImages.loads(m, mode: "images", unfolded: true, once: false))
        #expect(!RemoteImages.loads(m, mode: "html", unfolded: true, once: false))
        #expect(!RemoteImages.loads(m, mode: "images", unfolded: false, once: false))
        // Not authenticated: only when asked for the conversation.
        let unknown = message(auth: "unknown")
        #expect(!RemoteImages.loads(unknown, mode: "images", unfolded: true, once: false))
        #expect(RemoteImages.loads(unknown, mode: "images", unfolded: true, once: true))
        // Another sender of the thread, not remembered: the same.
        let other = message(format: nil)
        #expect(!RemoteImages.loads(other, mode: "images", unfolded: true, once: false))
        #expect(RemoteImages.loads(other, mode: "images", unfolded: true, once: true))
        // Never in Junk, even when asked.
        #expect(!RemoteImages.loads(message(role: "junk"), mode: "images", unfolded: true, once: true))
    }

    // MARK: Sources

    /// The same examples as the core's tests (`remote_images.rs`).
    @Test func sourcesAreDecodedAsTheCoreEncodesThem() {
        #expect(RemoteImages.originalURL("nm-remote://image?u=https%3A%2F%2Fx.org%2Fa%20b") == "https://x.org/a b")
        #expect(RemoteImages.originalURL(
            "nm-remote://image?u=https%3A%2F%2Fx.org%2Fo%3Fid%3D1%26u%3Dme%2540x.org%23f")
            == "https://x.org/o?id=1&u=me%40x.org#f")
        #expect(RemoteImages.originalURL("nm-remote://image?u=http%3A%2F%2Fx.org%2Fcaf%C3%A9.png") == "http://x.org/café.png")
        #expect(RemoteImages.originalURL("https://x.org/a.png") == nil)
        #expect(RemoteImages.originalURL("nm-remote://image?u=%ZZ") == nil)
        #expect(RemoteImages.originalURL("nm-remote://image?u=a&b=c") == nil)
    }

    @Test func imagesNotLoadedKeepNoSourceTheWebViewWouldAsk() {
        let body = #"<p>x</p><img src="nm-remote://image?u=https%3A%2F%2Fx.org%2Fa.png" width="600"><img src="cid:logo">"#
        let off = RemoteImages.body(body, loading: false)
        #expect(!off.contains(#" src="nm-remote:"#))
        #expect(off.contains(#"data-nm-src="nm-remote://image?u=https%3A%2F%2Fx.org%2Fa.png""#))
        #expect(off.contains(#"src="cid:logo""#))
        #expect(RemoteImages.body(body, loading: true) == body)
        #expect(RemoteImages.sources(in: body) == ["https://x.org/a.png"])
        #expect(RemoteImages.sources(in: off).isEmpty)
        #expect(RemoteImages.sources(in: "<p>none</p>").isEmpty)
    }

    // MARK: Requests

    @Test func onlyHTTPSRequestsTowardsPublicHosts() {
        #expect(RemoteImages.requestURL(for: "https://img.x.org/a.png")?.absoluteString == "https://img.x.org/a.png")
        // Upgraded, never sent in clear.
        #expect(RemoteImages.requestURL(for: "http://img.x.org/a.png?t=1")?.absoluteString == "https://img.x.org/a.png?t=1")
        #expect(RemoteImages.requestURL(for: "http://img.x.org:80/a.png")?.absoluteString == "https://img.x.org/a.png")
        #expect(RemoteImages.requestURL(for: "https://8.8.8.8/a.png") != nil)
        #expect(RemoteImages.requestURL(for: "https://[2001:db8::1]/a.png") != nil)
        for refused in [
            "ftp://x.org/a.png", "file:///etc/passwd", "data:image/png;base64,AA", "javascript:x",
            "https://localhost/a.png", "https://a.localhost/a.png", "https://printer.local/a.png",
            "https://intranet/a.png", "https://127.0.0.1/a.png", "https://10.1.2.3/a.png", "https://172.20.0.1/a.png",
            "https://192.168.1.1/a.png", "https://169.254.169.254/latest", "https://100.64.0.1/a.png",
            "https://0.0.0.0/a.png", "https://[::1]/a.png", "https://[fd00::1]/a.png", "https://[fe80::1]/a.png",
            "https://[::ffff:10.0.0.1]/a.png", "https://0x7f.1/a.png", "https://127.1/a.png",
            "https://user:pw@x.org/a.png", "not a url",
        ] {
            #expect(RemoteImages.requestURL(for: refused) == nil, "\(refused)")
        }
    }

    // MARK: Download

    func fetcher(cache: URL? = nil) -> RemoteImageFetcher {
        RemoteImageFetcher(configuration: RemoteImageFetcher.configuration(protocols: [StubNetwork.self]),
                           cacheDirectory: cache)
    }

    /// A host of its own for each test: the tests run in parallel and share
    /// the stand-in's record.
    func host() -> String { "img-\(UUID().uuidString.prefix(8).lowercased()).example.com" }

    @Test func anImageIsRequestedOnceThenServedFromTheCache() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nm-images-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let h = host()
        let url = "http://\(h)/a.png"
        let f = fetcher(cache: dir)
        let first = await f.image(for: url)
        #expect(first?.mimeType == "image/png" && first?.data == StubNetwork.png)
        _ = await f.image(for: url)
        #expect(StubNetwork.requests(to: h).count == 1)
        // HTTPS was asked, never HTTP; the request says nothing it need not.
        let request = try #require(StubNetwork.requests(to: h).first)
        #expect(request.url?.scheme == "https")
        #expect(request.value(forHTTPHeaderField: "User-Agent") == RemoteImageFetcher.userAgent)
        #expect(request.value(forHTTPHeaderField: "Cookie") == nil && request.value(forHTTPHeaderField: "Referer") == nil)
        // A new launch (new fetcher): read from the disk, still one request.
        let again = await fetcher(cache: dir).image(for: url)
        #expect(again?.data == StubNetwork.png)
        #expect(StubNetwork.requests(to: h).count == 1)
        // No URL, and no token in it, in the cache's file names.
        let names = try FileManager.default.contentsOfDirectory(atPath: dir.path)
        #expect(names.count == 1 && !names[0].contains(h))
    }

    @Test func twoPagesAskingTheSameImageAtOnceSendOneRequest() async {
        let h = host()
        let f = fetcher()
        async let a = f.image(for: "https://\(h)/slow.png")
        async let b = f.image(for: "https://\(h)/slow.png")
        let (x, y) = await (a, b)
        #expect(x != nil && y != nil)
        #expect(StubNetwork.requests(to: h).count == 1)
    }

    @Test func whatIsNotAnImageIsRefusedAndNotAskedAgain() async {
        let h = host()
        let f = fetcher()
        for path in ["/page.html", "/missing.png", "/huge.png"] {
            #expect(await f.image(for: "https://\(h)\(path)") == nil, "\(path)")
            #expect(await f.image(for: "https://\(h)\(path)") == nil, "\(path)")
        }
        #expect(StubNetwork.requests(to: h).count == 3)
    }

    @Test func aPrivateHostIsNeverAsked() async {
        let f = fetcher()
        #expect(await f.image(for: "https://192.168.1.1/a.png") == nil)
        #expect(await f.image(for: "http://localhost/a.png") == nil)
        #expect(await f.requestCount == 0)
        #expect(StubNetwork.requests(to: "192.168.1.1").isEmpty && StubNetwork.requests(to: "localhost").isEmpty)
    }
}

/// A stand-in for the network, for `RemoteImageFetcher`'s session: it
/// records every request and answers by path. `/page.html`: an HTML page;
/// `/missing.png`: 404; `/huge.png`: a type and a length over the limit;
/// `/slow.png`: a PNG after 100 ms; anything else: a PNG.
final class StubNetwork: URLProtocol {
    static let png = UIScriptImageProtocol.png
    private static let lock = NSLock()
    nonisolated(unsafe) private static var recorded: [URLRequest] = []

    static func requests(to host: String) -> [URLRequest] {
        lock.lock()
        defer { lock.unlock() }
        return recorded.filter { $0.url?.host == host }
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        Self.lock.lock()
        Self.recorded.append(request)
        Self.lock.unlock()
        let path = request.url?.path ?? ""
        let (status, type, body, length): (Int, String, Data, Int) = switch path {
        case "/page.html": (200, "text/html", Data("<p>hi</p>".utf8), 9)
        case "/missing.png": (404, "image/png", Data(), 0)
        case "/huge.png": (200, "image/png", Self.png, RemoteImageFetcher.maxBytes + 1)
        default: (200, "image/png", Self.png, Self.png.count)
        }
        let respond = { [self] in
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1",
                                           headerFields: ["Content-Type": type, "Content-Length": "\(length)"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: body)
            client?.urlProtocolDidFinishLoading(self)
        }
        if path == "/slow.png" {
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.1, execute: respond)
        } else {
            respond()
        }
    }

    override func stopLoading() {}
}
