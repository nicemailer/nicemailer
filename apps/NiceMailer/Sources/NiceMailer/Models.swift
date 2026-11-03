import Foundation

// Swift mirrors of the presentation views of the contract (`crates/nm-api`).
//
// The JSON keys are in snake_case (`total_count`) while Swift properties are
// in camelCase (`totalCount`). The decoder converts between the two
// automatically (`keyDecodingStrategy = .convertFromSnakeCase` in
// `CoreClient.decoder`), so no `CodingKeys` are needed.
//
// Swift notes:
// - A `struct` is a value type: assigning it copies it. Views are immutable
//   snapshots, so `let` properties and structs fit well.
// - Conforming to `Decodable` lets `JSONDecoder` build the struct from JSON;
//   the compiler generates the decoding code from the property names.
// - `Identifiable` (an `id` property) lets SwiftUI lists track rows across
//   reloads. `Hashable` lets values be used in sets, as dictionary keys, or
//   as selection tags.
// - `String?` is an optional: either a string or `nil`, matching a JSON
//   `null` or a missing key. Roles are kept as plain strings ("inbox",
//   "todo"...) rather than a Swift enum, so an unknown role sent by a newer
//   core does not make decoding fail.

/// Position in the core's event stream (see `Position` in nm-api).
struct Position: Decodable, Equatable {
    let epoch: String
    let seq: Int
}

/// A folder in the sidebar.
struct FolderView: Decodable, Identifiable, Hashable {
    let id: Int
    let role: String?
    let name: String
    /// Keyboard shortcut that moves a thread to this folder.
    let letter: String?
    let category: String?
    let totalCount: Int
    let unreadCount: Int
    let threadCount: Int
    /// Targets offered by the move command from this folder, computed by
    /// the core: the app never decides on its own what is allowed.
    let moveTargets: [String]
    /// Account of the folder. The sidebar shows one group per account when
    /// there are several (an IMAP account and an mbox archive, say).
    let accountId: Int
    let accountName: String
    /// "imap", or "archive" for an account created by an mbox import.
    let accountKind: String
}

struct FolderList: Decodable {
    let items: [FolderView]
    let asOf: Position
}

/// A row of the thread list.
struct ThreadSummary: Decodable, Identifiable, Hashable {
    let id: Int
    let subject: String
    let participants: [String]
    let snippet: String?
    /// Unix timestamp in seconds.
    let dateLastReceived: Int
    let messageCount: Int
    let unreadCount: Int
    let isFlagged: Bool
    let hasAttachments: Bool
    /// "todo" or "waiting" when the thread came back to the Inbox.
    let returnedFrom: String?
    /// Folder preselected if the user replies.
    let replyTarget: String?
    /// Folder this row describes (the listed folder, or for a list across
    /// folders the folder of the latest message). Moves start from it.
    let folderId: Int
    /// Unix timestamp of the latest change by the user (sort "modified").
    let dateLastModified: Int
    /// Names of the thread's tags.
    let tags: [String]
}

/// A page of a paginated list. `<T: Decodable>` makes it generic: it can
/// hold any decodable item type.
struct Page<T: Decodable>: Decodable {
    let items: [T]
    /// Pass it back to get the next page; `nil` on the last page.
    let nextCursor: String?
    let asOf: Position
}

struct AddressView: Decodable, Hashable {
    let addr: String
    let name: String?
    /// A computed property: evaluated on each access, not stored or decoded.
    /// `??` returns the left side if it is not nil, the right side otherwise.
    var display: String { name ?? addr }
}

/// A message, in the reading pane.
struct MessageView: Decodable, Identifiable, Hashable {
    let id: Int
    let folderId: Int
    let folderRole: String?
    let subject: String
    let fromName: String?
    let fromAddr: String?
    let to: [AddressView]
    let cc: [AddressView]
    let dateSent: Int?
    let dateReceived: Int
    /// `nil` when the body has not been downloaded yet.
    let bodyText: String?
    let isSeen: Bool
    let isFlagged: Bool
    let isAnswered: Bool
    let hasAttachments: Bool
    /// "pass", "fail" or "none".
    let authResult: String
    /// Sanitized HTML body, if the message has an HTML part.
    let bodyHtml: String?
    /// Remote images were removed from `bodyHtml`.
    let hasRemoteContent: Bool
    /// Display remembered for the sender: "text", "html" or nil (default).
    let fromBodyFormat: String?
    /// The text cut into segments by the core, for the simplified display
    /// (`docs/thread-display.md`).
    let segments: [SegmentView]
    /// Brings nothing new: only quotes earlier messages.
    let onlyQuotes: Bool
    /// Unfolded by default (latest message, unread ones).
    let expanded: Bool
    /// Recipients worth showing (they changed since the previous message).
    let showRecipients: Bool
}

/// A piece of a message's text, typed by the core: "text", "quote",
/// "quote_tail", "signature", "boilerplate", "forward" or "copy".
struct SegmentView: Decodable, Hashable {
    let kind: String
    let text: String
    let depth: Int
    /// Hidden by default.
    let hidden: Bool
    /// For a quote tail: its content is in an earlier message.
    let inThread: Bool
}

/// A whole thread, all folders included.
struct ThreadView: Decodable {
    let id: Int
    let subject: String
    let messages: [MessageView]
    let asOf: Position
}

/// A search result (a message, not a thread).
struct SearchHit: Decodable, Identifiable, Hashable {
    let messageId: Int
    let threadId: Int?
    let folderId: Int
    let folderRole: String?
    let subject: String
    let fromName: String?
    let fromAddr: String?
    let dateReceived: Int
    /// Excerpt with matched terms wrapped in `[` and `]`.
    let highlight: String
    /// `Identifiable` requires an `id`; the message id serves as one. Being
    /// computed, it is not expected in the JSON.
    var id: Int { messageId }
}

struct SearchResult: Decodable {
    let items: [SearchHit]
    let asOf: Position
}

/// Result of a write command: number of messages changed.
struct CommandResult: Decodable {
    let changed: Int
}

/// A plain list (`list_tags`, `list_groups`, `list_threads_by`).
struct Items<T: Decodable>: Decodable {
    let items: [T]
    let asOf: Position
}

/// One line of the history (audit log).
struct HistoryEntry: Decodable, Identifiable, Hashable {
    let id: Int
    let date: Int
    /// Who did it: "app", "cli", "core"...
    let source: String
    let entity: String
    let entityId: Int
    let action: String
    /// Readable description built by the core. (The raw `detail` JSON is
    /// not decoded: the app does not need it.)
    let summary: String
}

struct TagView: Decodable, Identifiable, Hashable {
    let id: Int
    let accountId: Int
    let name: String
    let color: String?
    let messageCount: Int
}

struct GroupView: Decodable, Identifiable, Hashable {
    let id: Int
    let name: String
    let color: String?
    let memberCount: Int
}

struct ContactView: Decodable, Identifiable, Hashable {
    let id: Int
    let addr: String
    let name: String?
    let screeningTarget: String?
    let groups: [String]
    let messageCount: Int
}

struct ExportResult: Decodable {
    let path: String
}

struct GroupMemberResult: Decodable {
    let groupId: Int
    let addr: String
    let changed: Bool
}

struct HelloResult: Decodable {
    let protocolVersion: Int
    let coreVersion: String
}

struct SubscribeResult: Decodable {
    let epoch: String
    let seq: Int
    let replayed: Bool
}

/// Display names and letters of the workflow roles, for move targets (which
/// the core sends as bare role strings).
///
/// Swift note: an `enum` without cases cannot be instantiated; it is used
/// here as a namespace for static members.
enum Role {
    static let names: [String: String] = [
        "inbox": "Inbox", "todo": "Todo", "waiting": "Waiting", "archive": "Archive",
        "sent": "Sent", "drafts": "Drafts", "newsletter": "Newsletter", "paperwork": "Paperwork",
        "quarantine": "Quarantine", "deleted": "Deleted", "junk": "Junk", "banned": "Banned",
    ]
    static let letters: [String: String] = [
        "inbox": "I", "todo": "T", "waiting": "W", "archive": "A", "sent": "S", "drafts": "R",
        "newsletter": "N", "paperwork": "P", "quarantine": "Q", "deleted": "D", "junk": "X", "banned": "B",
    ]
    /// Display name of a role, or "?" if unknown. `flatMap` applies the
    /// closure only if `role` is not nil; `$0` is the closure's first argument.
    static func name(_ role: String?) -> String { role.flatMap { names[$0] } ?? "?" }
    /// Shortcut letter of a role. The `_` means callers write
    /// `Role.letter("todo")` rather than `Role.letter(role: "todo")`.
    static func letter(_ role: String) -> String { letters[role] ?? "?" }

    /// Sidebar sections, in display order: (category sent by the core,
    /// French title shown to the user).
    static let categoryTitles: [(String, String)] = [
        ("work", "Travail"), ("filing", "Rangement"), ("rejection", "Rejet"), ("technical", "Technique"),
    ]
}

/// Date formatting helpers.
///
/// `DateFormatter` is expensive to create, so each one is built once and
/// stored in a `static let` (created lazily on first use, then reused). The
/// `= { ... }()` syntax defines a closure and calls it immediately to build
/// the value.
enum Format {
    private static let time: DateFormatter = {
        let f = DateFormatter(); f.dateFormat = "HH:mm"; return f
    }()
    private static let dayMonth: DateFormatter = {
        let f = DateFormatter(); f.locale = Locale(identifier: "fr_FR"); f.dateFormat = "d MMM"; return f
    }()
    private static let full: DateFormatter = {
        let f = DateFormatter(); f.locale = Locale(identifier: "fr_FR"); f.dateStyle = .short; return f
    }()
    private static let long: DateFormatter = {
        let f = DateFormatter(); f.locale = Locale(identifier: "fr_FR")
        f.dateStyle = .full; f.timeStyle = .short; return f
    }()

    /// Compact date for the lists: the time if today, day and month if this
    /// year, the short date otherwise.
    static func listDate(_ ts: Int) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        let cal = Calendar.current
        if cal.isDateInToday(d) { return time.string(from: d) }
        if cal.isDate(d, equalTo: Date(), toGranularity: .year) { return dayMonth.string(from: d) }
        return full.string(from: d)
    }

    private static let geekShort: DateFormatter = {
        let f = DateFormatter(); f.dateFormat = "dd/MM/yy"; return f
    }()
    private static let geekDayMonth: DateFormatter = {
        let f = DateFormatter(); f.locale = .autoupdatingCurrent; f.setLocalizedDateFormatFromTemplate("dMMM"); return f
    }()

    /// Date of the geek lists, at most 8 characters so that the column has
    /// a fixed width: "14:05" today, "16 oct." this year, "14/09/21" before.
    /// Follows the user's language, unlike the classic theme.
    static func geekListDate(_ ts: Int) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        let cal = Calendar.current
        if cal.isDateInToday(d) { return time.string(from: d) }
        if cal.isDate(d, equalTo: Date(), toGranularity: .year) { return geekDayMonth.string(from: d) }
        return geekShort.string(from: d)
    }

    /// Date of a message header in the geek reader: "14:05" today,
    /// "yesterday 14:05", "3 Nov 14:05" this year, "3 Nov 2021" before.
    static func geekMessageDate(_ ts: Int) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        let cal = Calendar.current
        if cal.isDateInToday(d) { return time.string(from: d) }
        if cal.isDateInYesterday(d) { return L("yesterday %@", time.string(from: d)) }
        if cal.isDate(d, equalTo: Date(), toGranularity: .year) {
            return geekDayMonth.string(from: d) + " " + time.string(from: d)
        }
        return geekDayMonthYear.string(from: d)
    }

    private static let geekDayMonthYear: DateFormatter = {
        let f = DateFormatter(); f.locale = .autoupdatingCurrent; f.setLocalizedDateFormatFromTemplate("dMMMyyyy"); return f
    }()

    /// Full date and time, for the message header.
    static func longDate(_ ts: Int) -> String {
        long.string(from: Date(timeIntervalSince1970: TimeInterval(ts)))
    }
}
