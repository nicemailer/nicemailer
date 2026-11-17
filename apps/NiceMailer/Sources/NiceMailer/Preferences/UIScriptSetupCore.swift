import Foundation

/// The core of the Preferences in UI script mode (`UITests/README.md`,
/// `preferences`): the real core for everything, except the methods that
/// create an account, answered here for made-up domains. A scenario then
/// plays every step of the form (servers not found, SMTP password asked,
/// import screen) without any DNS lookup nor mail server, and without
/// writing an account into the scenario's `accounts.toml`.
///
/// The domains, by what they make the flow ask for:
///
/// | Address | What happens |
/// | --- | --- |
/// | `…@known.test` | servers found by the domain; the SMTP server refuses the IMAP password and takes `smtp-pw`; a big Inbox: the import screen |
/// | `…@hosted.test` | the domain unknown, its MX `mx.hosted.test` unknown too: the IMAP server is asked; with it, SMTP guesses that take the IMAP password; a small Inbox |
/// | `…@other.test` | the SMTP server of another organization: its password is asked, any one works |
/// | `…@nosmtp.test` | no SMTP server answers: the SMTP server is asked, or "without sending" |
///
/// The password `wrong` is refused by every IMAP server. An account
/// "added" is listed after the real ones (ids from 9001), and left out of
/// `set_account_order` before it reaches the core.
final class UIScriptSetupCore: CoreCalling, @unchecked Sendable {
    private let real: any CoreCalling
    private let lock = NSLock()
    private var added: [[String: Any]] = []
    /// Every creation method answered here, in order: `method key=value…`,
    /// passwords reduced to whether they were given.
    private var recorded: [String] = []

    init(real: any CoreCalling) {
        self.real = real
    }

    var calls: [String] { lock.withLock { recorded } }

    func call<R: Decodable>(_ method: String, _ params: [String: Any]) async throws -> R {
        let answer: Any
        switch method {
        case "discover_account": answer = Self.discover(params)
        case "test_account": answer = try Self.testAccount(params)
        case "test_smtp": answer = Self.testSMTP(params)
        case "add_account": answer = add(params)
        case "list_accounts":
            let r: AccountInfoListJSON = try await real.call(method, params)
            // Swift note: `withLock` holds the lock for the closure only,
            // the way to lock from an `async` function.
            let mine = lock.withLock { added }
            answer = ["items": r.items + mine]
        case "set_account_order":
            var p = params
            p["account_ids"] = (params["account_ids"] as? [Int] ?? []).filter { $0 < 9001 }
            return try await real.call(method, p)
        default:
            return try await real.call(method, params)
        }
        record(method, params)
        let data = try JSONSerialization.data(withJSONObject: answer, options: .fragmentsAllowed)
        return try CoreClient.decoder.decode(R.self, from: data)
    }

    private func record(_ method: String, _ p: [String: Any]) {
        var parts = [method]
        for key in p.keys.sorted() {
            let v = p[key]!
            if key.contains("password") {
                parts.append("\(key)=\((v as? String)?.isEmpty == false ? "set" : "empty")")
            } else if let server = v as? [String: Any] {
                parts.append("\(key)=\(server["host"] ?? "?"):\(server["port"] ?? "?")")
            } else if key == "candidates" {
                let hosts = (v as? [[String: Any]] ?? []).map { (($0["server"] as? [String: Any])?["host"] as? String) ?? "?" }
                parts.append("\(key)=\(hosts.joined(separator: "+"))")
            } else {
                parts.append("\(key)=\(v)")
            }
        }
        lock.withLock { recorded.append(parts.joined(separator: " ")) }
    }

    private static func domain(_ p: [String: Any]) -> String {
        AccountSetup.domain(of: p["email"] as? String ?? "")
    }

    private static func server(_ host: String, _ port: Int, _ security: String = "tls") -> [String: Any] {
        ["host": host, "port": port, "security": security]
    }

    private static func discover(_ p: [String: Any]) -> [String: Any] {
        let d = domain(p)
        let given = p["imap"] as? [String: Any]
        var r: [String: Any] = [
            "key": d.split(separator: ".").first.map(String.init) ?? d, "key_source": "domain", "domain_key": d,
            "domain_account": NSNull(), "domain": d, "by_domain": NSNull(), "mx": [String](), "mx_looked_up": false,
            "mx_error": NSNull(), "by_mx": NSNull(), "imap": NSNull(), "smtp": [Any](),
        ]
        let provider: [String: Any] = ["id": d, "name": "Provider of \(d)", "source": "local", "matched": d, "mx_host": NSNull(), "imap_unsupported": false]
        switch d {
        case "known.test", "other.test", "nosmtp.test":
            r["by_domain"] = provider
            r["imap"] = ["server": server("imap.\(d)", 993), "origin": "provider"]
            let smtpHost = d == "other.test" ? "smtp.elsewhere.example" : "smtp.\(d)"
            r["smtp"] = [["server": server(smtpHost, 465), "origin": "provider"]]
        default:
            r["mx_looked_up"] = true
            r["mx"] = ["mx.\(d)"]
        }
        if let given, let host = given["host"] as? String {
            r["imap"] = ["server": given, "origin": "given"]
            let smtpName = host.replacingOccurrences(of: "imap", with: "smtp")
            r["smtp"] = [["server": server(smtpName, 465), "origin": "smtp_name"], ["server": server(smtpName, 587, "starttls"), "origin": "smtp_name"]]
        }
        return r
    }

    private static func testAccount(_ p: [String: Any]) throws -> [String: Any] {
        if p["password"] as? String == "wrong" {
            let host = (p["imap"] as? [String: Any])?["host"] as? String ?? "?"
            throw CoreClient.RPCError(code: 1009, message: "the server refused the login: [AUTHENTICATIONFAILED] Invalid credentials (\(host))", data: ["reason": "auth"])
        }
        let big = domain(p) == "known.test"
        return [
            "probe": ["server": "x", "security": "tls", "capabilities_before_login": [String](), "capabilities": [String](), "enabled": [String](),
                      "namespace": NSNull(), "folders": [Any](), "connect_ms": 1, "login_ms": 1, "list_ms": 1],
            "inbox_messages": big ? 18052 : 40, "inbox_unseen": big ? 3921 : 2, "inbox_uidnext": 20000,
            "correspondents": big ? 284 : 3, "offer_archive": big,
            "read_before": "2026-11-17", "unread_before": "2026-10-18",
            "archive_read": big ? 14_100 : 0, "archive_unread": big ? 3_860 : 0,
        ]
    }

    private static func testSMTP(_ p: [String: Any]) -> [String: Any] {
        let candidates = p["candidates"] as? [[String: Any]] ?? []
        let smtpPassword = p["smtp_password"] as? String
        var attempts: [[String: Any]] = []
        var chosen: Any = NSNull()
        var refused: Any = NSNull()
        for c in candidates {
            let host = (c["server"] as? [String: Any])?["host"] as? String ?? ""
            let (outcome, detail): (String, Any)
            switch host {
            case "smtp.known.test":
                (outcome, detail) = smtpPassword == "smtp-pw" ? ("logged_in", NSNull()) : ("auth_refused", "the server refused the login: 535 5.7.8 Application-specific password required")
            case "smtp.elsewhere.example":
                (outcome, detail) = smtpPassword == nil ? ("reachable", NSNull()) : ("logged_in", NSNull())
            case "smtp.nosmtp.test":
                (outcome, detail) = ("failed", "cannot connect to smtp.nosmtp.test:465")
            default:
                (outcome, detail) = host.hasSuffix(".test") || host.hasSuffix(".example") ? ("logged_in", NSNull()) : ("failed", "cannot connect to \(host)")
            }
            attempts.append(["candidate": c, "outcome": outcome, "detail": detail])
            if outcome == "logged_in" || outcome == "reachable" {
                chosen = attempts.count - 1
                break
            }
        }
        if chosen is NSNull, let i = attempts.firstIndex(where: { $0["outcome"] as? String == "auth_refused" }) { refused = i }
        return ["attempts": attempts, "chosen": chosen, "refused": refused]
    }

    private func add(_ p: [String: Any]) -> [String: Any] {
        let id: Int = lock.withLock {
            let id = 9001 + added.count
            added.append([
                "id": id, "key": p["key"] ?? "?", "kind": "imap", "email": p["email"] ?? NSNull(),
                "display_name": p["display_name"] ?? NSNull(), "enabled": true, "error": NSNull(), "position": 1000 + id,
            ])
            return id
        }
        return ["account_id": id, "created": [String](), "roles": [String: String](), "correspondents": 3,
                "archive_queued": p["archive_read_before"] != nil || p["archive_unread_before"] != nil]
    }
}

/// `list_accounts` as plain JSON, to append the accounts added here.
private struct AccountInfoListJSON: Decodable {
    let items: [Any]

    init(from decoder: Decoder) throws {
        // Swift note: the rows are kept as JSON objects, not decoded into
        // `AccountInfo`, so that they go back to the caller unchanged.
        let c = try decoder.container(keyedBy: Key.self)
        let rows = try c.decode([Row].self, forKey: .items)
        items = rows.map(\.json)
    }

    private enum Key: String, CodingKey { case items }

    private struct Row: Decodable {
        let json: [String: Any]
        init(from decoder: Decoder) throws {
            let a = try AccountInfo(from: decoder)
            json = ["id": a.id, "key": a.key, "kind": a.kind, "email": a.email ?? NSNull(), "display_name": a.displayName ?? NSNull(),
                    "enabled": a.enabled, "error": a.error ?? NSNull(), "position": a.position]
        }
    }
}
