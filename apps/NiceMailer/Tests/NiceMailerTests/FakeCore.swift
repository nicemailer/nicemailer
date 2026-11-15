import Foundation
@testable import NiceMailer

/// A core that answers from Swift dictionaries and records every request.
///
/// `handler` returns the JSON result (dictionaries, arrays, strings,
/// numbers) of a request; it is turned into bytes and decoded with the real
/// decoder, so the app's models are exercised as with the real core.
///
/// Requests can arrive from several tasks at once (the model starts them in
/// parallel), so the record is protected by a lock: unprotected appends
/// from two threads corrupted the array and crashed the test process now
/// and then.
final class FakeCore: CoreCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(method: String, params: [String: Any])] = []
    var handler: (String, [String: Any]) -> Any
    /// How long the core takes to answer a method, for a test of requests
    /// that overlap (none by default: it answers at once).
    var delays: [String: Duration] = [:]

    var calls: [(method: String, params: [String: Any])] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    init(handler: @escaping (String, [String: Any]) -> Any) {
        self.handler = handler
    }

    func call<R: Decodable>(_ method: String, _ params: [String: Any]) async throws -> R {
        lock.lock()
        recorded.append((method, params))
        let delay = delays[method]
        lock.unlock()
        if let delay { try await Task.sleep(for: delay) }
        let result = handler(method, params)
        // A handler answers an error by returning it.
        if let error = result as? CoreClient.RPCError { throw error }
        let data = try JSONSerialization.data(withJSONObject: result, options: .fragmentsAllowed)
        return try CoreClient.decoder.decode(R.self, from: data)
    }

    /// Requests made with `method`, in order.
    func calls(_ method: String) -> [[String: Any]] {
        calls.filter { $0.method == method }.map(\.params)
    }

    /// Every thread moved, in order, as the parameters of a single move
    /// (`thread_id`, `from_folder_id`, `target`): the threads of each
    /// `move_threads`, and each `move_thread`.
    func moveCalls() -> [[String: Any]] {
        calls.filter { $0.method == "move_threads" || $0.method == "move_thread" }.flatMap { c -> [[String: Any]] in
            guard c.method == "move_threads" else { return [c.params] }
            return (c.params["threads"] as? [[String: Any]] ?? []).map { t in
                t.merging(["target": c.params["target"] ?? ""]) { mine, _ in mine }
            }
        }
    }
}

/// A small mailbox: one account, Inbox / Archive / Todo / Deleted, eight
/// threads in the Inbox (ids 1 to 8), the third one with an HTML body.
enum Fixture {
    static let position: [String: Any] = ["epoch": "e_test", "seq": 0]

    static func folder(_ id: Int, _ role: String, _ name: String, _ letter: String, targets: [String]) -> [String: Any] {
        [
            "id": id, "role": role, "name": name, "letter": letter, "category": "work",
            "total_count": 8, "unread_count": 2, "thread_count": 8, "move_targets": targets,
            "account_id": 1, "account_name": "perso", "account_kind": "imap",
        ]
    }

    static let folders: [[String: Any]] = [
        folder(1, "inbox", "Inbox", "I", targets: ["todo", "waiting", "archive", "deleted", "junk"]),
        folder(2, "todo", "Todo", "T", targets: ["inbox", "archive", "deleted"]),
        folder(3, "archive", "Archive", "A", targets: ["inbox", "todo", "deleted"]),
        folder(4, "deleted", "Deleted", "D", targets: ["inbox", "archive"]),
    ]

    /// A row of the list. Thread 3 is one the user answered: its latest
    /// mail is theirs, and the row names the person answered.
    static func thread(_ id: Int, folder: Int = 1, tags: [String] = []) -> [String: Any] {
        [
            "id": id, "subject": "Subject \(id)",
            "participants": id == 3 ? ["Sender 3", "Me"] : ["Sender \(id)"],
            "correspondent": "Sender \(id)", "last_is_mine": id == 3, "snippet": "…",
            "date_last_received": 1_790_000_000 - id * 60, "message_count": 1, "unread_count": id % 2,
            "is_flagged": false, "has_attachments": false, "returned_from": NSNull(), "reply_target": NSNull(),
            "folder_id": folder, "date_last_modified": 1_790_000_000, "tags": tags,
        ]
    }

    /// Files attached to the message of thread 4.
    static let attachments: [[String: Any]] = [
        ["id": 41, "filename": "devis.pdf", "mime_type": "application/pdf", "size": 245_000],
        ["id": 42, "filename": "photo.jpg", "mime_type": "image/jpeg", "size": 1_200_000],
        ["id": 43, "filename": NSNull(), "mime_type": "application/zip", "size": 512],
    ]

    /// Files attached to the message of thread 6: a text file and a logo.
    static let otherAttachments: [[String: Any]] = [
        ["id": 61, "filename": "notes.txt", "mime_type": "text/plain", "size": 120],
        ["id": 62, "filename": "logo.png", "mime_type": "image/png", "size": 2_000],
    ]

    /// How to unsubscribe, by thread: 5 one click, 3 one click that fails,
    /// 7 a page of the list, 8 an address, 2 a link of the body.
    static let unsubscribe: [Int: [String: Any]] = [
        5: ["method": "one_click", "source": "header", "url": "https://x.org/u5", "mailto": NSNull(), "sender_unsubscribed_at": NSNull()],
        3: ["method": "one_click", "source": "header", "url": "https://x.org/u3", "mailto": NSNull(), "sender_unsubscribed_at": NSNull()],
        7: ["method": "browser", "source": "header", "url": "https://x.org/u7", "mailto": NSNull(), "sender_unsubscribed_at": NSNull()],
        8: ["method": "mail", "source": "header", "url": NSNull(), "mailto": "mailto:leave@x.org", "sender_unsubscribed_at": NSNull()],
        2: ["method": "browser", "source": "body", "url": "https://x.org/b2", "mailto": NSNull(), "sender_unsubscribed_at": NSNull()],
    ]

    static func threadView(_ id: Int) -> [String: Any] {
        let message: [String: Any] = [
            "id": id * 10, "folder_id": 1, "folder_role": "inbox", "subject": "Subject \(id)",
            "from_name": "Sender \(id)", "from_addr": "sender\(id)@example.org", "to": [], "cc": [],
            "date_sent": NSNull(), "date_received": 1_790_000_000, "body_text": "Hello, see https://example.org/\(id)",
            "is_seen": true, "is_flagged": false, "is_answered": false, "has_attachments": false,
            "auth_result": "unknown", "body_html": id == 3 ? "<p>Hello</p>" : NSNull(),
            "has_remote_content": false, "from_body_format": NSNull(),
            "segments": [["kind": "text", "text": "Hello, see https://example.org/\(id)", "depth": 0, "hidden": false, "in_thread": false]],
            "only_quotes": false, "expanded": true, "show_recipients": false,
            "attachments": id == 4 ? Fixture.attachments : id == 6 ? Fixture.otherAttachments : [],
            "unsubscribe": Fixture.unsubscribe[id] ?? NSNull(),
            "tracking_pixels": 0,
            // What the core finds in the text above (`links.rs`).
            "links": [["url": "https://example.org/\(id)", "text": NSNull()]],
        ]
        return ["id": id, "subject": "Subject \(id)", "messages": [message], "as_of": position]
    }

    /// The draft of a reply to the message `replyTo` (thread × 10 in the
    /// fixture), as the core prepares it: to its sender, the quotation
    /// under two empty lines (`reponse.md`, section 4).
    static func draft(replyTo: Int, body: String? = nil, revision: Int = 1, state: String = "editing") -> [String: Any] {
        let thread = replyTo / 10
        return [
            "id": 900 + thread, "account_id": 1, "send_account_id": 1,
            "from": ["addr": "me@example.org", "name": "Me"],
            "to": [["addr": "sender\(thread)@example.org", "name": "Sender \(thread)"]],
            "cc": thread == 2 ? [["addr": "carol@example.org", "name": NSNull()]] : [],
            "bcc": [],
            "attachments": [],
            "subject": "Re: Subject \(thread)",
            "body": body ?? "\n\nOn Nov 7, Sender \(thread) wrote:\n> Hello",
            "revision": revision, "state": state, "kind": "reply", "reply_to": replyTo, "forward_of": NSNull(), "thread_id": thread,
            "after_send_target": "archive", "after_send": NSNull(),
            "send_at": state == "sending" ? 1_790_000_005 : NSNull(),
        ]
    }

    /// The addresses `list_senders` gives: the account of the fixture, and
    /// a second account with two addresses.
    static let senders: [[String: Any]] = [
        ["account_id": 1, "account": "perso", "addr": "me@example.org", "name": "Me"],
        ["account_id": 2, "account": "work", "addr": "me@work.example", "name": "Me at work"],
        ["account_id": 2, "account": "work", "addr": "sales@work.example", "name": NSNull()],
    ]

    /// A draft as `prepare_draft` gives it (`method` "prepare_draft"):
    /// proposed, id 0 and revision 0, no row yet; as is for `create_draft`.
    static func opened(_ method: String, _ d: [String: Any]) -> [String: Any] {
        guard method == "prepare_draft" else { return d }
        var p = d
        p["id"] = 0
        p["revision"] = 0
        // The files of a forward proposed: listed, negative ids.
        if let files = d["attachments"] as? [[String: Any]] {
            p["attachments"] = files.enumerated().map { i, f in f.merging(["id": -(i + 1)]) { $1 } }
        }
        return p
    }

    /// The default answers of the fake core.
    static func answer(_ method: String, _ params: [String: Any]) -> Any {
        switch method {
        case "create_draft", "prepare_draft":
            let replyTo = params["reply_to"] as? Int ?? 10
            return ["draft": opened(method, draft(replyTo: replyTo, body: params["body"] as? String)), "reused": false]
        case "update_draft":
            return ["revision": (params["revision"] as? Int ?? 1) + 1]
        case "add_draft_attachment":
            let name = params["filename"] as? String ?? "file"
            return ["revision": (params["revision"] as? Int ?? 1) + 1,
                    "attachment": ["id": 70 + name.count, "filename": name, "mime_type": params["mime_type"] ?? "application/octet-stream", "size": 1234],
                    "warning": NSNull()]
        case "remove_draft_attachment":
            return ["revision": (params["revision"] as? Int ?? 1) + 1]
        case "parse_addresses":
            // A small stand-in for `compose::parse_address_list`: pieces
            // separated by commas, an address when it has an `@`.
            let pieces = (params["text"] as? String ?? "").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
            let ok = pieces.filter { $0.contains("@") }.map { ["addr": $0.lowercased(), "name": NSNull()] as [String: Any] }
            return ["addresses": ok, "invalid": pieces.filter { !$0.isEmpty && !$0.contains("@") }]
        case "suggest_recipients":
            let q = (params["query"] as? String ?? "").lowercased()
            let book: [[String: Any]] = [
                ["addr": "alice@example.org", "name": "Alice Martin"],
                ["addr": "albert@example.org", "name": NSNull()],
                ["addr": "bob@example.org", "name": "Bob"],
            ]
            return ["items": book.filter { ($0["addr"] as! String).hasPrefix(q) || (($0["name"] as? String)?.lowercased().hasPrefix(q) ?? false) }]
        case "list_senders":
            return ["items": senders]
        case "send_draft":
            return ["send_at": Int(Date().timeIntervalSince1970) + 5]
        case "cancel_send":
            let id = params["id"] as? Int ?? 901
            return draft(replyTo: (id - 900) * 10, body: "Merci", revision: 3)
        case "discard_draft":
            let id = params["id"] as? Int ?? 901
            return draft(replyTo: (id - 900) * 10)
        case "list_folders":
            return ["items": folders, "as_of": position]
        case "list_threads":
            let folder = (params["folder_ids"] as? [Int])?.first ?? params["folder_id"] as? Int ?? 1
            let items = folder == 1 ? (1...8).map { thread($0) } : [thread(100 + folder, folder: folder)]
            return ["items": items, "next_cursor": NSNull(), "as_of": position]
        case "get_thread":
            return threadView(params["thread_id"] as? Int ?? 1)
        case "search":
            return ["items": [], "as_of": position]
        case "list_contacts":
            // c1 a person, c2 a robot forced by the user, c3 not counted
            // yet (`expediteurs.md`).
            let kinds: [Any] = ["person", "robot", NSNull()]
            let items: [[String: Any]] = (1...3).map {
                ["id": $0, "addr": "c\($0)@example.org", "name": NSNull(), "screening_target": NSNull(),
                 "groups": [], "message_count": $0, "sender_kind": kinds[$0 - 1], "sender_kind_forced": $0 == 2]
            }
            return ["items": items, "next_cursor": NSNull(), "as_of": position]
        case "list_history":
            return ["items": [], "next_cursor": NSNull(), "as_of": position]
        case "list_tags", "list_groups":
            return ["items": [], "as_of": position]
        case "move_thread":
            // A move from the Inbox decides nothing; one to Newsletter
            // decides on the sender and files two other mails.
            let thread = params["thread_id"] as? Int ?? 0
            let decision: Any = params["target"] as? String == "newsletter"
                ? ["kind": "address", "name": "sender\(thread)@example.org", "before": NSNull(), "after": "newsletter"]
                : NSNull()
            let filed = params["target"] as? String == "newsletter" ? 2 : 0
            return ["changed": 1 + filed, "moved": 1, "filed": filed, "decision": decision, "undo": "\(thread):\(thread)"]
        case "move_threads":
            // Every thread moved, by the rule of `move_thread` above; one
            // undo token for all.
            let ids = (params["threads"] as? [[String: Any]] ?? []).compactMap { $0["thread_id"] as? Int }
            let newsletter = params["target"] as? String == "newsletter"
            let decisions: [[String: Any]] = newsletter
                ? ids.map { ["kind": "address", "name": "sender\($0)@example.org", "before": NSNull(), "after": "newsletter"] }
                : []
            let filed = newsletter ? 2 * ids.count : 0
            return ["changed": ids.count + filed, "moved": ids.count, "filed": filed, "decisions": decisions,
                    "undo": "\(ids.first ?? 0):\(ids.last ?? 0)", "moved_threads": ids, "refused": [Int](),
                    "suspects": [[String: Any]]()]
        case "undo":
            return ["changed": 1, "moved": 1, "filed": 0, "decision": NSNull(), "undo": "u"]
        case "set_address_screening", "set_domain_screening":
            let name = params["addr"] ?? params["domain"] ?? ""
            return ["changed": 3, "moved": 0, "filed": 3,
                    "decision": ["kind": params["addr"] != nil ? "address" : "domain", "name": name,
                                 "before": NSNull(), "after": params["target"] ?? NSNull()],
                    "undo": "7:9"]
        case "set_sender_kind":
            return ["changed": 0, "moved": 0, "filed": 0, "decision": NSNull(), "undo": "11:11"]
        case "set_group_member":
            return ["group_id": 1, "addr": "sender1@example.org", "changed": true]
        case "export_message":
            return ["path": "/tmp/x.eml"]
        case "message_record":
            // Message ids are thread ids × 10 in the fixture.
            let id = params["message_id"] as? Int ?? 0
            return FakeCore.record(id)
        case "message_text":
            // Message ids are thread ids × 10 in the fixture.
            let id = params["message_id"] as? Int ?? 0
            return ["text": "From: sender\(id / 10)@example.org\nSubject: Subject \(id / 10)\n\nParagraph of \(id)."]
        case "unsubscribe":
            // Message ids are thread ids × 10 in the fixture.
            let thread = (params["message_id"] as? Int ?? 0) / 10
            let u = unsubscribe[thread] ?? [:]
            switch thread {
            case 5: return ["method": "one_click", "done": true, "open": NSNull(), "error": NSNull()]
            case 3: return ["method": "browser", "done": false, "open": u["url"]!, "error": "le serveur a répondu 500"]
            case 8: return ["method": "mail", "done": false, "open": u["mailto"]!, "error": NSNull()]
            default: return ["method": "browser", "done": false, "open": u["url"] ?? NSNull(), "error": NSNull()]
            }
        case "get_attachment":
            // A real file, as the core extracts one, read-only.
            let id = params["attachment_id"] as? Int ?? 0
            let name = (attachments + otherAttachments).first { $0["id"] as? Int == id }?["filename"] as? String
                ?? "attachment-\(id)"
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-fake-\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let path = dir.appendingPathComponent(name)
            FileManager.default.createFile(atPath: path.path, contents: Data("content \(id)".utf8),
                                           attributes: [.posixPermissions: 0o400])
            return ["path": path.path, "filename": name, "mime_type": "application/pdf", "size": 9]
        default:
            return ["changed": 1]
        }
    }
}

/// Polls `condition` until it holds or a second passes (actions of the
/// model run in tasks; the test waits for their effect).
@MainActor
func eventually(_ condition: () -> Bool) async -> Bool {
    for _ in 0..<100 {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(10))
    }
    return condition()
}

extension FakeCore {
    /// A `message_record` answer for message `id` (thread `id / 10`), as
    /// the core makes it from a raw message: one author, two recipients,
    /// a copy, an attachment and two trace headers.
    static func record(_ id: Int) -> [String: Any] {
        [
            "message_id": "\(id)@example.org", "subject": "Subject \(id / 10)",
            "from": [["addr": "sender\(id / 10)@example.org", "name": "Sender \(id / 10)"]],
            "sender": NSNull(), "reply_to": [[String: Any]](),
            "to": [["addr": "me@example.org", "name": "Me"], ["addr": "bob@example.org", "name": NSNull()]],
            "cc": [["addr": "carol@example.org", "name": "Carol"]], "bcc": [[String: Any]](),
            "date_header": "Tue, 23 Jun 2020 21:20:56 +0200", "date_sent": 1_592_940_056, "date_received": 1_592_940_060,
            "in_reply_to": NSNull(), "references": [String](), "account": "perso", "folder": "INBOX",
            "auth_result": "pass", "body": "Hello,\n\nParagraph of \(id).",
            "attachments": [["part_id": "2", "filename": "devis.pdf", "mime_type": "application/pdf", "size": 2048,
                             "inline": false, "sha256": String(repeating: "ab", count: 32)]],
            "headers": [["name": "Received", "value": "from mx.example.org by mx.me.org"],
                        ["name": "DKIM-Signature", "value": "v=1; d=example.org; s=sel; b=xyz"]],
            "raw_size": 4096, "raw_sha256": String(repeating: "0f", count: 32),
        ]
    }
}
