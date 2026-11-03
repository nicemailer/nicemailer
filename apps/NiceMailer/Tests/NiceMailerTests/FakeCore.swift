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
        lock.unlock()
        let result = handler(method, params)
        let data = try JSONSerialization.data(withJSONObject: result, options: .fragmentsAllowed)
        return try CoreClient.decoder.decode(R.self, from: data)
    }

    /// Requests made with `method`, in order.
    func calls(_ method: String) -> [[String: Any]] {
        calls.filter { $0.method == method }.map(\.params)
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

    static func thread(_ id: Int, folder: Int = 1, tags: [String] = []) -> [String: Any] {
        [
            "id": id, "subject": "Subject \(id)", "participants": ["Sender \(id)"], "snippet": "…",
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
        ]
        return ["id": id, "subject": "Subject \(id)", "messages": [message], "as_of": position]
    }

    /// The default answers of the fake core.
    static func answer(_ method: String, _ params: [String: Any]) -> Any {
        switch method {
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
            let items: [[String: Any]] = (1...3).map {
                ["id": $0, "addr": "c\($0)@example.org", "name": NSNull(), "screening_target": NSNull(),
                 "groups": [], "message_count": $0]
            }
            return ["items": items, "next_cursor": NSNull(), "as_of": position]
        case "list_history":
            return ["items": [], "next_cursor": NSNull(), "as_of": position]
        case "list_tags", "list_groups", "list_threads_by":
            return ["items": [], "as_of": position]
        case "set_group_member":
            return ["group_id": 1, "addr": "sender1@example.org", "changed": true]
        case "export_message":
            return ["path": "/tmp/x.eml"]
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
