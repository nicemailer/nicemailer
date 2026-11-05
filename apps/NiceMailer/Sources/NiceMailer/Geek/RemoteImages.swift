import CryptoKit
import Foundation
import OSLog
import WebKit

// Remote images of the HTML reader (`ui.md`, "Images distantes";
// `docs/geek-ui.md`, "Remote images").
//
// The core never leaves a source a web view would load by itself
// (`crates/nm-store/src/remote_images.rs`): tracking pixels have no source
// any more, other remote images have one in the app's own scheme,
// `nm-remote://image?u=<percent-encoded URL>`. This file decides when such
// an image may be loaded, and loads it:
//
// - `RemoteImages`: the rules, as pure functions (tested without a window);
// - `RemoteImageFetcher`: the download, with a cache, so that a mail opened
//   five times sends one request per image, not five;
// - `RemoteImageLoader`: the web view's handler of the `nm-remote:` scheme;
// - `RemoteBlocker`: a content rule list that blocks every `http(s)` load
//   in the web view, a second barrier should a remote source ever pass
//   the core's sanitization.
//
// A request reveals to the sender that the mail was opened, when, and from
// which IP address: nothing here can hide that. What is done is to send as
// few requests as possible, as anonymous as possible, and only when the
// user asked.

/// The rules of remote images, as pure functions.
enum RemoteImages {
    /// The app's scheme for remote images (`remote_images.rs`,
    /// `SOURCE_PREFIX`).
    static let scheme = "nm-remote"
    static let sourcePrefix = "nm-remote://image?u="

    /// Folders whose mail never loads images: loading them would confirm
    /// to a spammer that the address is read (as unsubscribing would,
    /// `ui.md`, "Désabonnement").
    static let blockedRoles: Set<String> = ["junk", "quarantine", "banned"]

    /// Whether `m` has images that may be loaded at all: remote images left
    /// by the core, and a folder that allows them.
    static func offered(_ m: MessageView) -> Bool {
        m.bodyHtml != nil && m.hasRemoteContent && !blockedRoles.contains(m.folderRole ?? "")
    }

    /// Whether the images of `m` load by themselves: its sender is
    /// remembered in the images mode, and the mail is authenticated
    /// (`pass`). Without authentication anybody can write in the name of a
    /// trusted sender, and the remembered choice would confirm the reading
    /// to them (`authentification-expediteur.md`). The user can still load
    /// such a mail's images for the conversation (`once`).
    static func automatic(_ m: MessageView) -> Bool {
        offered(m) && m.fromBodyFormat == "images" && m.authResult == "pass"
    }

    /// Whether the images of `m` are loaded now: the thread is shown in the
    /// images mode, the message is unfolded (images of a folded message
    /// would be requested although nobody sees them), and either its own
    /// sender's choice applies or the user asked for this conversation.
    static func loads(_ m: MessageView, mode: String, unfolded: Bool, once: Bool) -> Bool {
        mode == "images" && unfolded && offered(m) && (once || automatic(m))
    }

    /// The body of a message for the page: unchanged when its images load;
    /// otherwise each `nm-remote:` source is renamed `data-nm-src`, so that
    /// the web view does not even ask the scheme handler for it.
    ///
    /// The HTML comes from the core's serializer (html5ever through
    /// ammonia), which always writes ` src="…"` with double quotes, so a
    /// plain replacement is reliable. The handler refuses any source it was
    /// not given anyway (`RemoteImageLoader.allowed`).
    static func body(_ html: String, loading: Bool) -> String {
        loading ? html : html.replacingOccurrences(of: " src=\"\(sourcePrefix)", with: " data-nm-src=\"\(sourcePrefix)")
    }

    /// The original URLs of the `nm-remote:` sources of `html`, decoded.
    static func sources(in html: String) -> Set<String> {
        var out = Set<String>()
        var rest = html[...]
        // With its leading space: `data-nm-src="…"` (images not loaded,
        // `body`) must not match.
        let marker = " src=\"\(sourcePrefix)"
        while let start = rest.range(of: marker) {
            let afterPrefix = rest[start.upperBound...]
            guard let end = afterPrefix.firstIndex(of: "\"") else { break }
            if let url = originalURL(sourcePrefix + afterPrefix[..<end]) { out.insert(url) }
            rest = afterPrefix[end...]
        }
        return out
    }

    /// The URL a `nm-remote:` source stands for; `nil` for any other
    /// string or a malformed encoding (the same examples are tested in
    /// Rust, `remote_images.rs`).
    static func originalURL(_ source: String) -> String? {
        guard source.hasPrefix(sourcePrefix) else { return nil }
        let encoded = source.dropFirst(sourcePrefix.count)
        // Only unreserved characters and `%XX` are written by the core.
        guard encoded.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "-._~%".contains($0)) }) else { return nil }
        return String(encoded).removingPercentEncoding
    }

    /// The URL actually requested for `original`: HTTPS only (`http:` is
    /// upgraded, as Safari does, rather than sent in clear with its token
    /// readable on the network), and only towards a public host. `nil`: no
    /// request at all.
    static func requestURL(for original: String) -> URL? {
        guard var parts = URLComponents(string: original), let scheme = parts.scheme?.lowercased() else { return nil }
        switch scheme {
        case "https": break
        case "http":
            parts.scheme = "https"
            // An explicit port 80 would mean plain HTTP on that port.
            if parts.port == 80 { parts.port = nil }
        default: return nil
        }
        guard let host = parts.host, isPublicHost(host), parts.user == nil, parts.password == nil else { return nil }
        return parts.url
    }

    /// Whether `host` may be asked for an image: not this machine, not the
    /// local network. A mail could otherwise make the app send requests to
    /// a router or a local service. Names are checked as written; a public
    /// name that resolves to a private address is not caught (that would
    /// take a resolver of our own).
    static func isPublicHost(_ host: String) -> Bool {
        let h = host.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        let name = h.hasSuffix(".") ? String(h.dropLast()) : h
        if name.isEmpty || name == "localhost" || name.hasSuffix(".localhost") || name.hasSuffix(".local")
            || name.hasSuffix(".internal") || name.hasSuffix(".home.arpa") {
            return false
        }
        if name.contains(":") { return isPublicIPv6(name) }
        let labels = name.split(separator: ".", omittingEmptySubsequences: false)
        // A single label ("intranet") is a local name.
        guard labels.count > 1 else { return false }
        let octets = labels.compactMap { UInt8($0) }
        if octets.count == 4, labels.count == 4 { return isPublicIPv4(octets) }
        // A number as the last label is an address written another way
        // (`0x7f.1`, `127.1`): no real top-level domain is numeric.
        if let last = labels.last, last.allSatisfy(\.isNumber) || last.hasPrefix("0x") { return false }
        return true
    }

    private static func isPublicIPv4(_ o: [UInt8]) -> Bool {
        switch (o[0], o[1]) {
        case (0, _), (10, _), (127, _), (169, 254), (192, 168): return false
        case (172, 16...31), (100, 64...127): return false
        case (224...255, _): return false
        default: return true
        }
    }

    private static func isPublicIPv6(_ h: String) -> Bool {
        if h == "::" || h == "::1" { return false }
        // IPv4-mapped addresses (`::ffff:10.0.0.1`): refused rather than
        // decoded.
        if h.hasPrefix("::ffff:") || h.hasPrefix("::") { return false }
        // fc00::/7 (unique local), fe80::/10 (link local), ff00::/8 (multicast).
        if h.hasPrefix("fc") || h.hasPrefix("fd") || h.hasPrefix("fe8") || h.hasPrefix("fe9")
            || h.hasPrefix("fea") || h.hasPrefix("feb") || h.hasPrefix("ff") {
            return false
        }
        return true
    }
}

/// Downloads remote images, with a cache in memory and on disk.
///
/// Swift note: an `actor` is a class whose methods run one at a time, so
/// its state (`memory`, `inFlight`) needs no lock. Calls from outside are
/// `await`ed.
actor RemoteImageFetcher {
    struct Image: Sendable {
        let data: Data
        let mimeType: String
    }

    /// Largest image accepted, in bytes: beyond, the download stops.
    static let maxBytes = 10_000_000
    /// Bytes of images kept in memory; beyond, the oldest are dropped
    /// (they stay on disk).
    static let memoryBytes = 50_000_000
    /// The same for every user, and no app or OS version in it (URLSession's
    /// default User-Agent names the app, CFNetwork and Darwin): one more
    /// detail less to tell the sender.
    static let userAgent = "Mozilla/5.0"

    static let shared = RemoteImageFetcher(
        configuration: RemoteImageFetcher.configuration(protocols: UIScript.isActive ? [UIScriptImageProtocol.self] : []),
        cacheDirectory: RemoteImageFetcher.defaultCacheDirectory)

    private let session: URLSession
    private let cacheDirectory: URL?
    private var memory: [String: Image] = [:]
    /// Order of insertion in `memory`, for the eviction.
    private var memoryOrder: [String] = []
    private var memorySize = 0
    /// Downloads under way: a second page asking for the same image waits
    /// for the first download instead of sending its own request.
    private var inFlight: [String: Task<Image?, Never>] = [:]
    /// Images that failed during this run: not asked again until the app
    /// restarts (a pixel that answers 404 would otherwise be asked at each
    /// opening).
    private var failed: Set<String> = []
    /// Requests sent since launch (tests and UI script state).
    private(set) var requestCount = 0

    init(configuration: URLSessionConfiguration, cacheDirectory: URL?) {
        session = URLSession(configuration: configuration)
        self.cacheDirectory = cacheDirectory
        if let cacheDirectory {
            try? FileManager.default.createDirectory(at: cacheDirectory, withIntermediateDirectories: true,
                                                     attributes: [.posixPermissions: 0o700])
        }
    }

    /// An ephemeral session that keeps nothing and sends nothing it does
    /// not have to: no cookies (read or written), no URL cache shared with
    /// other apps, no credentials. URLSession sends no `Referer`.
    static func configuration(protocols: [AnyClass]) -> URLSessionConfiguration {
        let c = URLSessionConfiguration.ephemeral
        c.httpCookieStorage = nil
        c.httpShouldSetCookies = false
        c.httpCookieAcceptPolicy = .never
        c.urlCredentialStorage = nil
        c.urlCache = nil
        c.requestCachePolicy = .reloadIgnoringLocalCacheData
        c.timeoutIntervalForRequest = 15
        c.timeoutIntervalForResource = 30
        c.httpMaximumConnectionsPerHost = 4
        if !protocols.isEmpty { c.protocolClasses = protocols }
        return c
    }

    /// `~/Library/Caches/NiceMailer/RemoteImages` (`NICEMAILER_DATA_DIR`'s
    /// `remote-images.noindex` in UI script mode, removed with the test
    /// data). macOS may empty the Caches folder: images are then asked
    /// again.
    static var defaultCacheDirectory: URL? {
        if UIScript.isActive, let dir = ProcessInfo.processInfo.environment["NICEMAILER_DATA_DIR"] {
            return URL(fileURLWithPath: dir).appendingPathComponent("remote-images.noindex")
        }
        return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("NiceMailer/RemoteImages", isDirectory: true)
    }

    /// The image at `original` (an URL as the mail wrote it), from the
    /// cache or downloaded; `nil` if it may not or could not be loaded.
    func image(for original: String) async -> Image? {
        if let image = memory[original] { return image }
        if failed.contains(original) { return nil }
        if let task = inFlight[original] { return await task.value }
        if let image = readDisk(original) {
            remember(original, image)
            return image
        }
        guard let url = RemoteImages.requestURL(for: original) else {
            failed.insert(original)
            return nil
        }
        requestCount += 1
        // Swift note: `Task { … }` inside an actor runs on the actor; only
        // the download itself (`download`, `nonisolated`) leaves it.
        let task = Task { await Self.download(url, session: session) }
        inFlight[original] = task
        let image = await task.value
        inFlight[original] = nil
        if let image {
            remember(original, image)
            writeDisk(original, image)
        } else {
            failed.insert(original)
        }
        return image
    }

    /// Downloads one image: success status, an `image/…` type, at most
    /// `maxBytes`, redirections only towards HTTPS public hosts.
    private nonisolated static func download(_ url: URL, session: URLSession) async -> Image? {
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
        request.httpShouldHandleCookies = false
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        request.setValue("image/*", forHTTPHeaderField: "Accept")
        do {
            let (bytes, response) = try await session.bytes(for: request, delegate: RedirectGuard())
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode),
                  let mime = http.mimeType?.lowercased(), mime.hasPrefix("image/"),
                  http.expectedContentLength <= Int64(maxBytes) else {
                return nil
            }
            var data = Data()
            if http.expectedContentLength > 0 { data.reserveCapacity(Int(http.expectedContentLength)) }
            for try await byte in bytes {
                data.append(byte)
                if data.count > maxBytes { return nil }
            }
            return data.isEmpty ? nil : Image(data: data, mimeType: mime)
        } catch {
            return nil
        }
    }

    private func remember(_ key: String, _ image: Image) {
        guard memory[key] == nil else { return }
        memory[key] = image
        memoryOrder.append(key)
        memorySize += image.data.count
        while memorySize > Self.memoryBytes, !memoryOrder.isEmpty {
            let old = memoryOrder.removeFirst()
            memorySize -= memory.removeValue(forKey: old)?.data.count ?? 0
        }
    }

    /// The file of `key` in the cache: the SHA-256 of the URL, so that no
    /// URL (and no token in it) appears in a file name.
    private func file(_ key: String) -> URL? {
        let digest = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
        return cacheDirectory?.appendingPathComponent(digest)
    }

    /// File format: the MIME type, a line feed, the bytes of the image.
    private func readDisk(_ key: String) -> Image? {
        guard let file = file(key), let data = try? Data(contentsOf: file),
              let newline = data.firstIndex(of: 0x0A),
              let mime = String(data: data[..<newline], encoding: .utf8), mime.hasPrefix("image/") else {
            return nil
        }
        let body = data[data.index(after: newline)...]
        return body.isEmpty ? nil : Image(data: Data(body), mimeType: mime)
    }

    private func writeDisk(_ key: String, _ image: Image) {
        guard let file = file(key) else { return }
        var data = Data(image.mimeType.utf8)
        data.append(0x0A)
        data.append(image.data)
        // Atomic: a reader never sees half a file.
        try? data.write(to: file, options: [.atomic])
    }
}

/// Follows a redirection only towards an HTTPS public host
/// (`RemoteImages.requestURL`): a mail must not reach the local network
/// through a redirecting server.
private final class RedirectGuard: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest) async -> URLRequest? {
        guard let url = request.url, url.scheme?.lowercased() == "https",
              RemoteImages.requestURL(for: url.absoluteString) != nil else { return nil }
        return request
    }
}

/// The web view's handler of `nm-remote:` sources: it answers only the
/// sources of the page it was given (`allowed`), with the fetcher's image.
///
/// Swift note: WebKit calls a scheme handler on the main thread, hence
/// `@MainActor`; the download itself runs on the fetcher's actor.
@MainActor
final class RemoteImageLoader: NSObject, WKURLSchemeHandler {
    /// Original URLs the current page may load (`RemoteImages.sources` of
    /// the bodies whose images load). Anything else is refused.
    var allowed: Set<String> = []
    private let fetcher: RemoteImageFetcher
    /// Tasks WebKit has not stopped: answering a stopped task raises an
    /// exception.
    private var active: Set<ObjectIdentifier> = []

    init(fetcher: RemoteImageFetcher = .shared) {
        self.fetcher = fetcher
    }

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        guard let url = task.request.url, let original = RemoteImages.originalURL(url.absoluteString),
              allowed.contains(original) else {
            task.didFailWithError(URLError(.resourceUnavailable))
            return
        }
        let id = ObjectIdentifier(task)
        active.insert(id)
        Task { @MainActor in
            let image = await fetcher.image(for: original)
            guard active.remove(id) != nil else { return }
            guard let image else {
                task.didFailWithError(URLError(.resourceUnavailable))
                return
            }
            task.didReceive(URLResponse(url: url, mimeType: image.mimeType,
                                        expectedContentLength: image.data.count, textEncodingName: nil))
            task.didReceive(image.data)
            task.didFinish()
            if UIScript.isActive { UIScript.remoteImagesShown += 1 }
        }
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {
        active.remove(ObjectIdentifier(task))
    }
}

/// A content rule list blocking every `http(s)` resource of the page
/// (images, styles, fonts, media…), whatever the display. The core already
/// removes such sources; this is the second barrier, inside WebKit.
/// Documents are left alone: a clicked link is caught by the navigation
/// delegate and opened in the browser (`HTMLReader`).
@MainActor
enum RemoteBlocker {
    /// One rule per scheme. WebKit compiles `url-filter` with a small
    /// subset of regular expressions, without alternatives: a single
    /// `^(https?|wss?|ftp)://` was refused ("Disjunctions are not supported
    /// yet"), and the barrier silently missing. `RemoteBlockerTests`
    /// compiles the list and checks that it blocks.
    static let schemes = ["^https?://", "^wss?://", "^ftp://"]
    static let resourceTypes = ["image", "style-sheet", "script", "font", "raw", "svg-document", "media"]

    /// The rules, as WebKit's JSON.
    static var rules: String {
        let list: [[String: Any]] = schemes.map { filter in
            ["trigger": ["url-filter": filter, "resource-type": resourceTypes], "action": ["type": "block"]]
        }
        // Swift note: `try!` stops the program if the call throws; a
        // dictionary of strings and arrays always turns into JSON, so it
        // cannot happen here.
        let data = try! JSONSerialization.data(withJSONObject: list, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    private enum State {
        case notCompiled
        case compiled(WKContentRuleList)
        /// WebKit refused the rules: they will not compile better later,
        /// so the page is shown without them and the error logged once.
        case refused
    }
    private static var state = State.notCompiled

    /// Swift note: `Logger` writes to the unified system log (Console.app,
    /// or `log stream --predicate 'subsystem == "NiceMailer"'`),
    /// not to the terminal the app was started from.
    private static let log = Logger(subsystem: "NiceMailer", category: "RemoteBlocker")

    /// The compiled list (compiled once, then kept). `nil` if WebKit
    /// refused it: the page is shown anyway, the core's sanitization being
    /// the first barrier, and the refusal is logged.
    static func list() async -> WKContentRuleList? {
        switch state {
        case .compiled(let list): return list
        case .refused: return nil
        case .notCompiled: break
        }
        do {
            let list = try await WKContentRuleListStore.default()
                .compileContentRuleList(forIdentifier: "nicemailer-remote-block", encodedContentRuleList: rules)
            // Two pages opened at once both compile; the first result wins.
            if case .compiled(let first) = state { return first }
            guard let list else { throw CocoaError(.featureUnsupported) }
            state = .compiled(list)
            return list
        } catch {
            if case .notCompiled = state {
                log.error("Remote content rule list refused by WebKit: \(error.localizedDescription, privacy: .public)")
                state = .refused
            }
            return nil
        }
    }

    /// Whether the list is in force: `on`, `off` (refused) or `pending`
    /// (not compiled yet), for the UI scenarios (`remote_blocker`).
    static var status: String {
        switch state {
        case .compiled: "on"
        case .refused: "off"
        case .notCompiled: "pending"
        }
    }
}

/// In UI script mode, the fetcher's only "network": every request gets a
/// small PNG, and nothing leaves the machine. Each request is counted
/// (`UIScript.remoteRequests`).
final class UIScriptImageProtocol: URLProtocol {
    /// A 1×1 transparent PNG.
    static let png = Data(base64Encoded:
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")!

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        DispatchQueue.main.async { UIScript.remoteRequests += 1 }
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1",
                                       headerFields: ["Content-Type": "image/png"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Self.png)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
