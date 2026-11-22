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
/// | `…@known.test` | servers found by the domain; the SMTP server refuses the IMAP password and takes `smtp-pw`; a big Inbox: the import screen; the server has Sent, Drafts, Trash, Junk and Archive |
/// | `…@hosted.test` | the domain unknown, its MX `mx.hosted.test` unknown too: the IMAP server is asked; with it, SMTP guesses that take the IMAP password; a small Inbox; the server has Sent and Drafts only |
/// | `…@other.test` | the SMTP server of another organization: its password is asked, any one works |
/// | `…@nosmtp.test` | no SMTP server answers: the SMTP server is asked, or "without sending" |
/// | `…@look.test` | servers found by the domain, the SMTP server takes the IMAP password; a small Inbox: nothing asked but "On the server" |
/// | `…@gmail.com` | as `look.test`, its provider Google's (`googlemail.com`): the app-password hint |
///
/// The password `wrong` is refused by every IMAP server. `set_password` is
/// answered here too, recorded and never saved. An account
/// "added" is listed after the real ones (ids from 9001), and left out of
/// `set_account_order` before it reaches the core. Added `read_only`, it
/// is organized by `prepare_activation` and `activate_account`, answered
/// here too, with the report of its domain.
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
        case "prepare_activation": answer = try prepare(params)
        case "activate_account": answer = try activate(params)
        // Never the real core's: it would write the scenario's Keychain.
        case "set_password": answer = ["changed": 0]
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
            } else if key == "roles", let roles = v as? [String: String] {
                // Only the folders that are not the role's English name:
                // what the user changed, short enough to read.
                let changed = roles.keys.sorted().filter { roles[$0] != Role.name($0) }.map { "\($0):\(roles[$0]!)" }
                parts.append("roles=\(roles.count)\(changed.isEmpty ? "" : "(" + changed.joined(separator: ",") + ")")")
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
        let provider: [String: Any] = ["id": d == "gmail.com" ? "googlemail.com" : d, "name": "Provider of \(d)", "source": "local", "matched": d, "mx_host": NSNull(), "imap_unsupported": false]
        switch d {
        case "known.test", "other.test", "nosmtp.test", "look.test", "gmail.com":
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

    fileprivate static func testAccount(_ p: [String: Any]) throws -> [String: Any] {
        if p["password"] as? String == "wrong" {
            let host = (p["imap"] as? [String: Any])?["host"] as? String ?? "?"
            throw CoreClient.RPCError(code: 1009, message: "the server refused the login: [AUTHENTICATIONFAILED] Invalid credentials (\(host))",
                                      data: ["reason": "imap_auth_refused", "server_text": "[AUTHENTICATIONFAILED] Invalid credentials (\(host))"])
        }
        let big = domain(p) == "known.test"
        // The folders of the server, with the role the core would propose.
        let folders: [(String, String?)] = big
            ? [("INBOX", "inbox"), ("Archive", "archive"), ("Drafts", "drafts"), ("Junk", "junk"), ("Sent", "sent"), ("Trash", "deleted")]
            : [("INBOX", "inbox"), ("Drafts", "drafts"), ("Sent", "sent")]
        let english = ["inbox": "INBOX", "todo": "Todo", "waiting": "Waiting", "archive": "Archive", "sent": "Sent", "drafts": "Drafts",
                       "newsletter": "Newsletter", "paperwork": "Paperwork", "quarantine": "Quarantine", "deleted": "Deleted", "junk": "Junk", "banned": "Banned"]
        let order = ["inbox", "todo", "waiting", "archive", "sent", "drafts", "newsletter", "paperwork", "quarantine", "deleted", "junk", "banned"]
        let roles: [[String: Any]] = order.map { role in
            let found = folders.first { $0.1 == role }?.0
            return ["role": role, "path": found ?? english[role]!, "exists": found != nil]
        }
        return [
            "probe": ["server": "x", "security": "tls", "capabilities_before_login": [String](), "capabilities": [String](), "enabled": [String](),
                      "namespace": NSNull(), "connect_ms": 1, "login_ms": 1, "list_ms": 1,
                      "folders": folders.map { ["name": $0.0, "delimiter": "/", "attributes": [String](), "selectable": true,
                                                "proposed_role": $0.1 ?? NSNull(), "role_from": $0.1 == nil ? NSNull() : "name" as Any,
                                                "messages": NSNull(), "unseen": NSNull()] as [String: Any] }],
            "roles": roles,
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

    fileprivate func lockedIndex(_ key: String) -> Int? {
        lock.withLock { added.firstIndex { $0["key"] as? String == key && $0["read_only"] as? Bool == true } }
    }

    fileprivate func email(at i: Int) -> String {
        lock.withLock { added[i]["email"] as? String ?? "" }
    }

    fileprivate func setReadOnly(at i: Int, _ on: Bool) {
        lock.withLock { added[i]["read_only"] = on }
    }

    private func add(_ p: [String: Any]) -> [String: Any] {
        let id: Int = lock.withLock {
            let id = 9001 + added.count
            added.append([
                "id": id, "key": p["key"] ?? "?", "kind": "imap", "email": p["email"] ?? NSNull(),
                "display_name": p["display_name"] ?? NSNull(), "enabled": true, "error": NSNull(), "position": 1000 + id,
                "read_only": p["read_only"] as? Bool ?? false,
            ])
            return id
        }
        return ["account_id": id, "created": [String](), "roles": [String: String](), "correspondents": 3,
                "archive_queued": p["archive_read_before"] != nil || p["archive_unread_before"] != nil]
    }
}

extension UIScriptSetupCore {
    /// The account added here under a key, read-only, or an error as the
    /// core's.
    fileprivate func lookedAt(_ p: [String: Any]) throws -> Int {
        let key = p["account"] as? String ?? ""
        let i: Int? = lockedIndex(key)
        guard let i else {
            throw CoreClient.RPCError(code: -32602, message: "the account \(key) is not read-only: it is set up already",
                                      data: ["reason": "account_set_up", "account": key])
        }
        return i
    }

    fileprivate func prepare(_ p: [String: Any]) throws -> [String: Any] {
        let i = try lookedAt(p)
        return try Self.testAccount(["email": email(at: i), "password": "secret"])
    }

    fileprivate func activate(_ p: [String: Any]) throws -> [String: Any] {
        let i = try lookedAt(p)
        let report = try Self.testAccount(["email": email(at: i), "password": "secret"])
        let created = (report["roles"] as? [[String: Any]] ?? []).compactMap { r -> String? in
            let path = (p["roles"] as? [String: String])?[r["role"] as? String ?? ""] ?? r["path"] as? String
            let existing = ((report["probe"] as? [String: Any])?["folders"] as? [[String: Any]] ?? []).compactMap { $0["name"] as? String }
            return path.flatMap { existing.contains($0) ? nil : $0 }
        }
        setReadOnly(at: i, false)
        return ["created": created, "roles": p["roles"] ?? [String: String](), "correspondents": 0,
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
                    "enabled": a.enabled, "error": a.error ?? NSNull(), "position": a.position, "read_only": a.isReadOnly]
        }
    }
}
