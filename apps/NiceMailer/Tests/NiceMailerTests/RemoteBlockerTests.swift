import Foundation
import Network
import Testing
import WebKit
@testable import NiceMailer

/// WebKit's barrier against remote loads (`RemoteBlocker`): the rules
/// compile, and a page given raw `http://` sources (as if the core's
/// sanitization had let one pass) sends no request.
///
/// The requests go to a server of the test on 127.0.0.1: nothing leaves the
/// machine. Without the rules, the same page reaches it, which shows the
/// test can see a request when there is one.
@MainActor
@Suite(.serialized)
struct RemoteBlockerTests {
    @Test func theRulesCompile() async {
        // WebKit refused the first version of the rules ("Disjunctions are
        // not supported yet"), and the app showed pages without them.
        let list = await RemoteBlocker.list()
        #expect(list != nil)
        #expect(RemoteBlocker.status == "on")
    }

    @Test func eachRuleIsASingleSchemeWithoutAlternative() {
        #expect(RemoteBlocker.schemes.allSatisfy { !$0.contains("|") })
        #expect(RemoteBlocker.rules.contains("\"block\""))
    }

    @Test func aPageWithTheRulesSendsNoRemoteRequest() async throws {
        let server = try await LocalServer.start()
        defer { server.stop() }
        let list = try #require(await RemoteBlocker.list())
        try await load(page(server.port), rules: list)
        // Give a stray request time to arrive.
        try await Task.sleep(for: .milliseconds(300))
        #expect(server.connections == 0)
    }

    @Test func thePageWithoutTheRulesDoesReachTheServer() async throws {
        let server = try await LocalServer.start()
        defer { server.stop() }
        try await load(page(server.port), rules: nil)
        let deadline = Date().addingTimeInterval(3)
        while server.connections == 0, Date() < deadline { try await Task.sleep(for: .milliseconds(50)) }
        #expect(server.connections > 0)
    }

    @Test func thePolicyAloneStopsTheRemoteRequests() async throws {
        // The third barrier on its own: no rules, the page's policy only.
        let server = try await LocalServer.start()
        defer { server.stop() }
        let html = page(server.port).replacingOccurrences(
            of: "<head>", with: #"<head><meta http-equiv="Content-Security-Policy" content="\#(ContentPolicy.header)">"#)
        try await load(html, rules: nil)
        try await Task.sleep(for: .milliseconds(300))
        #expect(server.connections == 0)
    }

    /// An image, a style sheet and a background, all on the test server.
    private func page(_ port: UInt16) -> String {
        let base = "http://127.0.0.1:\(port)"
        return """
        <html><head><link rel="stylesheet" href="\(base)/s.css"></head>
        <body><img src="\(base)/a.png"><div style="background:url(\(base)/b.png)">x</div></body></html>
        """
    }

    /// Loads `html` in a web view configured like the HTML reader, and
    /// waits until the page has finished loading (images included).
    private func load(_ html: String, rules: WKContentRuleList?) async throws {
        let config = WKWebViewConfiguration()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.websiteDataStore = .nonPersistent()
        if let rules { config.userContentController.add(rules) }
        let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 400, height: 300), configuration: config)
        let waiter = LoadWaiter()
        view.navigationDelegate = waiter
        // Swift note: a continuation turns a callback (the delegate's
        // `didFinish`) into something `await` can wait for; it must be
        // resumed exactly once.
        await withCheckedContinuation { continuation in
            waiter.done = { continuation.resume() }
            view.loadHTMLString(html, baseURL: nil)
        }
        _ = view
    }
}

/// Calls `done` once, when the page has loaded or failed.
private final class LoadWaiter: NSObject, WKNavigationDelegate {
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

/// A TCP server on 127.0.0.1 that counts the connections it gets and
/// closes them at once.
private final class LocalServer: @unchecked Sendable {
    private let listener: NWListener
    private let lock = NSLock()
    private var count = 0
    private var started = false
    var connections: Int { lock.withLock { count } }
    var port: UInt16 { listener.port?.rawValue ?? 0 }

    private init(_ listener: NWListener) { self.listener = listener }

    static func start() async throws -> LocalServer {
        let parameters = NWParameters.tcp
        // Loopback only: no firewall question, nothing reachable from
        // outside.
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        let server = LocalServer(try NWListener(using: parameters))
        server.listener.newConnectionHandler = { [server] connection in
            server.lock.withLock { server.count += 1 }
            connection.cancel()
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            // The handler runs on the listener's queue: the "resumed" flag
            // is kept under the lock, the continuation resumed once.
            server.listener.stateUpdateHandler = { [server] state in
                switch state {
                case .ready, .failed, .cancelled:
                    let first = server.lock.withLock {
                        defer { server.started = true }
                        return !server.started
                    }
                    if first { continuation.resume() }
                default: break
                }
            }
            server.listener.start(queue: .global())
        }
        return server
    }

    func stop() { listener.cancel() }
}
