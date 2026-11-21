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
    /// The person the conversation is with, as the core finds it: the
    /// latest author who is not the user, or the recipient of a mail the
    /// user wrote alone (`ThreadSummary::correspondent` in nm-api).
    let correspondent: String?
    /// Who wrote the mail that names the correspondent: `person`, `robot`,
    /// or `nil` when the core has no verdict for it
    /// (`ThreadSummary::correspondent_kind`, `expediteurs.md`). Shown as an
    /// icon before the name while Quarantine is listed.
    ///
    /// Swift note: the synthesized decoder reads an optional with
    /// `decodeIfPresent`, so a core that does not send the key still
    /// decodes, with `nil`.
    let correspondentKind: String?
    /// The user wrote the latest mail: the conversation waits on the
    /// correspondent.
    let lastIsMine: Bool
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

extension ThreadSummary {
    /// The name of the sender column (`ui.md`, "Liste"): the
    /// correspondent, else the latest participant (a conversation whose
    /// mails name nobody else).
    ///
    /// Swift note: `??` gives the value on its right when the optional on
    /// its left is `nil`; chained, it takes the first that is not.
    var correspondentName: String { correspondent ?? participants.last ?? "" }

    /// A copy with another unread count and flag (a row kept in a filtered
    /// list after its state changed, `AppModel.keepingSelected`).
    ///
    /// Swift note: a struct gets a "memberwise" initializer taking every
    /// stored property in order; it is used here to copy the row.
    func with(unreadCount: Int, isFlagged: Bool) -> ThreadSummary {
        ThreadSummary(
            id: id, subject: subject, participants: participants, correspondent: correspondent,
            correspondentKind: correspondentKind, lastIsMine: lastIsMine, snippet: snippet,
            dateLastReceived: dateLastReceived, messageCount: messageCount, unreadCount: unreadCount,
            isFlagged: isFlagged, hasAttachments: hasAttachments, returnedFrom: returnedFrom,
            replyTarget: replyTarget, folderId: folderId, dateLastModified: dateLastModified, tags: tags)
    }
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
    /// `bodyHtml` holds remote images that can be loaded on request, with
    /// `nm-remote:` sources (`RemoteImages.swift`).
    let hasRemoteContent: Bool
    /// Display remembered for the sender: "text", "html", "images" (HTML
    /// with its remote images) or nil (default).
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
    /// Files attached, in MIME order; images shown by the HTML are left
    /// out by the core (`attachments.rs`).
    let attachments: [AttachmentView]
    /// How to unsubscribe from the list the message comes from, if it
    /// says (`ui.md`, "Désabonnement"). A `var` with a default so that the
    /// tests can build messages without it.
    var unsubscribe: UnsubscribeView? = nil
    /// Tracking pixels the core removed from `bodyHtml`: never requested.
    var trackingPixels: Int = 0
    /// Links of the message for `l`, found by the core in the HTML (with
    /// their text) and in the text part (`links.rs`): the text made from an
    /// HTML has no address left to look for.
    var links: [LinkView] = []
    /// The verification code the message gives, when it is a code mail
    /// (`codes.rs` in the core): offered first by `l`, where Return copies
    /// it.
    var verificationCode: String? = nil
    /// The deep link of the message, `nicemailer://m/<key>`, written by the
    /// core: the same on every Mac that has the mail (`DeepLink`). A `var`
    /// with a default so that the tests can build messages without it; the
    /// app then falls back on the id (`DeepLink.url(of:)`).
    var link: String? = nil
}

/// Where a message is (`locate_message`), to open a deep link.
struct MessageLocation: Decodable {
    let messageId: Int
    /// nil for a message not linked to a thread yet.
    let threadId: Int?
    let folderId: Int
    let accountId: Int
}

/// A link of a message (`MessageView.links`).
struct LinkView: Decodable, Hashable {
    let url: String
    /// Visible text of the link in the HTML; nil for an address written
    /// in the text, or a link around an image.
    let text: String?
    /// Where a tracking link really goes, when its address says so.
    /// Optional, so that an older core (or a test) may leave it out.
    var target: String? = nil
    /// The other texts of the same link in the message, each once.
    var otherTexts: [String]? = nil
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
    /// Names of the thread's tags, and the groups of the person it is with
    /// (`correspondent`, the one `g` adds): shown in the header (`ui.md`,
    /// "Tags et groupes dans l'en-tête").
    ///
    /// Swift note: optionals with a default, so that a core that does not
    /// send them still decodes, and that the tests build a `ThreadView`
    /// without them; `labels` reads them as plain arrays.
    var tags: [String]? = nil
    var correspondent: String? = nil
    var correspondentGroups: [String]? = nil
}

extension ThreadView {
    /// The tags and groups the header shows, `[]` when the core sent none.
    var labels: (tags: [String], groups: [String]) { (tags ?? [], correspondentGroups ?? []) }
}

/// The tags of a thread, the person it is with and that person's groups
/// (`thread_labels`): what `t` and `g` can remove from a selection.
struct ThreadLabels: Decodable, Hashable {
    let threadId: Int
    let tags: [String]
    let correspondent: String?
    let groups: [String]
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

/// What the app reads of `config_status`: the settings of `[ui]` it
/// applies itself (`WindowChrome`), and `[unsubscribe]`.
struct ConfigStatusView: Decodable {
    let ui: UIConfig?
    /// Absent from an older core (nil: closed).
    var unsubscribe: UnsubscribeConfig? = nil
    /// `[folder_names]`: the name of each folder by role id, the 12 roles,
    /// English unless the file names them. Absent from an older core (nil:
    /// the English names of `Role.names`).
    var folderNames: [String: String]? = nil
}

/// `[unsubscribe]` of `config.toml` (`ui.md`, "Désabonnement").
struct UnsubscribeConfig: Decodable {
    /// `unsubscribe.in_banned`: the button shows on the mails of Banned.
    let inBanned: Bool
}

struct UIConfig: Decodable {
    /// `hidden` or `shown`.
    let titleBar: String
    /// `ui.auto_confirm_calls`; absent from an older core (nil: off).
    let autoConfirmCalls: Bool?
    /// `ui.text_size` and `ui.status_bar_text_size`, in points; absent
    /// from an older core (nil: the defaults of `TextSize`).
    var textSize: Double? = nil
    var statusBarTextSize: Double? = nil
}

/// A screening decision taken, changed or removed by a command: on an
/// address or a domain, from `before` to `after` (`nil`: no decision).
struct DecisionView: Decodable, Hashable {
    let kind: String
    let name: String
    let before: String?
    let after: String?
}

/// What a move, a decision or an undo did (`dossiers.md`, "Décision sur un
/// expéditeur"): mails of the thread moved, other mails of the sender
/// filed, the decision recorded, and the token that undoes it all.
struct WorkflowResult: Decodable {
    let changed: Int
    let moved: Int
    let filed: Int
    let decision: DecisionView?
    let undo: String?
}

/// What `move_threads` did (`ui.md`, "Actions groupées"): one request for
/// every thread moved, one undo token for all; the threads left (target
/// not allowed, sender suspected of spoofing) are listed.
struct MoveThreadsResult: Decodable {
    let changed: Int
    let moved: Int
    let filed: Int
    let decisions: [DecisionView]
    let undo: String?
    let movedThreads: [Int]
    let refused: [Int]
    let suspects: [SuspectView]
}

/// A thread `move_threads` left for a confirmation.
struct SuspectView: Decodable {
    let threadId: Int
    let addr: String
    let verdict: String
}

/// A plain list (`list_tags`, `list_groups`).
struct Items<T: Decodable>: Decodable {
    let items: [T]
    let asOf: Position
}

/// One line of the history (audit log).
/// A row of the History screen (`list_history_groups`): one action, on one
/// item or on several at once.
struct HistoryGroup: Decodable, Identifiable, Hashable {
    /// The oldest and the newest line of the log it holds.
    let firstId: Int
    let lastId: Int
    let count: Int
    let date: Int
    /// Who did it: "app", "cli", "core"...
    let source: String
    let entity: String
    let entityId: Int
    /// What was done, as a code the app words (`HistoryDisplay.action`).
    let kind: String
    /// What it was done with: a folder, a tag, an address…
    var value: String? = nil
    /// The subject of the mail, the address, the name; nil: deleted since.
    var title: String? = nil

    var id: Int { lastId }
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
    /// Latest change of the group or of one of its memberships (Unix time),
    /// and the mails received from its members: what ranks the groups when
    /// the folder column has no room for all of them (`GroupRanking`).
    ///
    /// Swift note: a `var` with a default value is still decoded (the core
    /// always sends both keys); the default only lets a test build a
    /// `GroupView` without them through the memberwise initializer.
    var dateModified: Int = 0
    var mailCount: Int = 0
}

struct ContactView: Decodable, Identifiable, Hashable {
    let id: Int
    let addr: String
    let name: String?
    let screeningTarget: String?
    /// Decision of the address's domain, which applies when the address
    /// has none.
    var domainTarget: String? = nil
    /// `auto`, `required` or `not_required`.
    var authPolicy: String? = nil
    let groups: [String]
    let messageCount: Int
    /// Who writes the address's mails (`expediteurs.md`): `person`,
    /// `robot`, or `nil` before any mail was counted. The user's choice
    /// when `forced`, else the core's verdict.
    var senderKind: String? = nil
    /// Swift note: an optional, so that the synthesized decoder accepts a
    /// core that does not send the key (a non-optional `Bool` with a
    /// default value is still required in the JSON).
    var senderKindForced: Bool? = nil
    var forced: Bool { senderKindForced == true }
}

extension ContactView {
    /// The origin column of the contacts screen: "human" or "robot",
    /// followed by "*" when the user chose it; empty before the core counted
    /// any mail.
    var originLabel: String {
        let word: String = switch senderKind {
        case "person": L("human")
        case "robot": L("robot")
        default: ""
        }
        return word.isEmpty || !forced ? word : word + " *"
    }
}

struct ExportResult: Decodable {
    let path: String
}

/// Result of `message_text`: the text of a mail for the clipboard, each
/// paragraph on one line (`nm-store`, `paragraphs.rs`).
struct MessageTextResult: Decodable {
    let text: String
}

/// Result of `message_record`: one mail as a piece of evidence, for the
/// PDF of `sp` (`MailPDF`). Read by the core from the raw message when it
/// has it (`rawSha256` set): the names as written in this mail, every
/// recipient, the `Date` header as written, the trace headers.
struct MessageRecord: Decodable {
    let messageId: String?
    let subject: String
    let from: [AddressView]
    let sender: AddressView?
    let replyTo: [AddressView]
    let to: [AddressView]
    let cc: [AddressView]
    let bcc: [AddressView]
    /// `Date` header as written, with the sender's time zone.
    let dateHeader: String?
    let dateSent: Int?
    let dateReceived: Int
    let inReplyTo: String?
    let references: [String]
    let account: String
    let folder: String
    /// "pass", "fail", "none" or "unknown".
    let authResult: String
    /// Text body, paragraphs whole; nil while it is not downloaded.
    let body: String?
    let attachments: [RecordAttachment]
    let headers: [RecordHeader]
    let rawSize: Int?
    /// SHA-256 of the raw message (the `.eml` of `se`), lowercase hex.
    let rawSha256: String?
}

/// An attachment of `MessageRecord`, with the SHA-256 of its content.
struct RecordAttachment: Decodable, Hashable {
    let partId: String
    let filename: String?
    let mimeType: String
    let size: Int
    let inline: Bool
    let sha256: String?
}

/// A trace header of `MessageRecord` (Received, DKIM-Signature…), on one
/// line, as written.
struct RecordHeader: Decodable, Hashable {
    let name: String
    let value: String
}

/// How to unsubscribe from a mailing list, as found by the core at
/// reception.
struct UnsubscribeView: Decodable, Hashable {
    /// "one_click" (the core does it), "browser" (a page to open) or
    /// "mail" (an address to write to).
    let method: String
    /// "header" (List-Unsubscribe, reliable) or "body" (a link of the text).
    let source: String
    let url: String?
    let mailto: String?
    /// When the user unsubscribed from this sender with one click.
    let senderUnsubscribedAt: Int?
}

/// Result of `unsubscribe`: done by the core, or a page / address to open.
struct UnsubscribeResult: Decodable {
    let method: String
    let done: Bool
    let open: String?
    let error: String?
}

/// An attachment listed under a message. Its content is fetched on demand
/// (`get_attachment`).
struct AttachmentView: Decodable, Identifiable, Hashable {
    let id: Int
    /// Name given by the sender, if any.
    let filename: String?
    let mimeType: String
    /// Size in bytes.
    let size: Int
    /// Whether the core can read its text, for the AI to summarize it
    /// (text, HTML, PDF, DOCX, a mail attached). Swift note: an optional
    /// decodes as `nil` when the key is missing, as from an older core;
    /// a `var` with a default keeps it out of the memberwise initializer.
    var readable: Bool? = nil
}

/// Result of `get_attachment`: the file extracted by the core. It belongs
/// to the core (read-only): the app copies or opens it, and asks again
/// before each use (it may be purged and extracted again).
struct AttachmentFileView: Decodable {
    let path: String
    let filename: String
    let mimeType: String
    let size: Int
}

/// Result of `rename_tag` and `rename_group`.
struct RenameResult: Decodable {
    let changed: Bool
}

/// Result of `delete_group`.
struct DeleteGroupResult: Decodable {
    let groupId: Int
    let members: Int
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

/// The answer of `prepare_update` (`nm_api::PrepareUpdateResult`).
struct PrepareUpdateResult: Decodable {
    let timeoutSecs: Int
}

/// The answer of `cancel_update` (`nm_api::CancelUpdateResult`).
struct CancelUpdateResult: Decodable {
    let cancelled: Bool
}

struct SubscribeResult: Decodable {
    let epoch: String
    let seq: Int
    let replayed: Bool
}

/// English names and letters of the workflow roles, for move targets (which
/// the core sends as bare role strings). What the interface shows is
/// `FolderNames`, which starts from these names; `Role.name` stays for what
/// must be English whatever the file says (the folders the server is told
/// to create, the targets of the UI scripts).
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
    /// title shown to the user, in the app's language).
    ///
    /// A computed property rather than a constant: `L` looks the title up
    /// in the language in effect when the sidebar is drawn.
    static var categoryTitles: [(String, String)] {
        [("work", L("Working")), ("filing", L("Filing")), ("rejection", L("Rejected")), ("technical", L("Technical"))]
    }
}

/// The names the interface shows for the folders of the workflow
/// (`configuration.md`, "Noms des dossiers"): those of `[folder_names]` in
/// `config.toml`, sent by the core with `config_status`, English for a role
/// the file does not name.
///
/// For display only. The role (`waiting`) stays the identifier everywhere:
/// what the app sends to the core, the keymap's actions (`move.waiting`),
/// and the targets of the UI scripts (`folder-Waiting`, in English, so that
/// a scenario does not depend on the names of a file).
///
/// A value held by the model (`AppModel.folderNames`) and passed to the
/// pure functions that word a folder, rather than a global: each unit test
/// then has its own names, and tests running in parallel never see each
/// other's. Those functions take `.english` by default.
struct FolderNames: Equatable, Sendable {
    /// The names by role id, as the core sends them, and by id of a
    /// virtual folder (`work`, `all_mail`, `everything`).
    var byRole: [String: String] = [:]

    /// The English names: the default of the file, and the names of an
    /// older core that does not send them.
    static let english = FolderNames()

    /// The name shown for `role`: the file's, else the English one, "?"
    /// for no role or an unknown one.
    func name(_ role: String?) -> String {
        guard let role else { return "?" }
        return byRole[role] ?? Role.names[role] ?? "?"
    }

    /// The name shown for a folder: its role's, or its IMAP path when it
    /// has no role (the core sends the path as its name).
    func name(of folder: FolderView) -> String {
        folder.role.map { name($0) } ?? folder.name
    }

    /// The name shown for a virtual folder (Work, All Mail, Everything):
    /// the file's, else the English one.
    func name(of virtual: AppModel.Virtual) -> String {
        byRole[virtual.id] ?? virtual.name
    }
}

/// Date formatting helpers.
///
/// Every date the interface shows is written in `uiLocale`
/// (`docs/localization.md`): the words of the app's language, the order
/// and the clock of the Mac's settings. No format is spelled out
/// ("HH:mm", "dd/MM/yy"): each is a template ("jmm": hours and minutes,
/// "j" meaning the hour the locale prefers, 12- or 24-hour), which the
/// locale turns into its own pattern ("23:13", "11:13 PM").
enum Format {
    /// The formatters of one locale.
    ///
    /// `DateFormatter` is expensive to create, so each one is built once:
    /// the app keeps one `Dates` (`Format.dates`), the tests build their own
    /// for a given locale and time zone.
    struct Dates {
        let time: DateFormatter
        let dayMonth: DateFormatter
        let short: DateFormatter
        let long: DateFormatter
        let geekTime: DateFormatter
        let geekDayMonth: DateFormatter
        let geekShort: DateFormatter
        let dayMonthTime: DateFormatter
        let dayMonthYear: DateFormatter

        /// `timeZone` nil: the Mac's.
        init(locale: Locale, timeZone: TimeZone? = nil) {
            func make(_ setup: (DateFormatter) -> Void) -> DateFormatter {
                let f = DateFormatter()
                f.locale = locale
                if let timeZone { f.timeZone = timeZone }
                setup(f)
                return f
            }
            func template(_ t: String) -> DateFormatter { make { $0.setLocalizedDateFormatFromTemplate(t) } }
            time = template("jmm")
            dayMonth = template("dMMM")
            short = make { $0.dateStyle = .short }
            long = make { $0.dateStyle = .full; $0.timeStyle = .short }
            dayMonthTime = template("dMMMjmm")
            dayMonthYear = template("dMMMy")
            // The date column of the geek lists is 8 characters wide
            // (`RowLayout`). A locale whose names are longer ("28. Sept."
            // in German, "3 de fev." in Portuguese) gets the numbers; one
            // whose numbers are still too long (Arabic, whose "ddMM" keeps
            // the month's name) gets fixed digits.
            geekTime = Format.fitting([.template("jmm"), .template("Hmm")], width: 8, make: make)
            geekDayMonth = Format.fitting([.template("dMMM"), .template("ddMM"), .fixed("dd/MM")], width: 8, make: make)
            // The region's short date with two digits per field: its order
            // and its separators, as the classic list and a typed date
            // (`DateChoice.Order`) have them. Not the template "ddMMyy",
            // which follows the language: a French interface on an
            // American Mac would show day first while a typed date is
            // read month first.
            geekShort = Format.fitting([.localized(Format.twoDigits(Format.shortPattern(locale))), .fixed("yy-MM-dd")], width: 8, make: make)
        }
    }

    /// A way to write a date: a template the locale orders as it wants,
    /// or, as a last resort, a fixed pattern.
    enum Pattern {
        case template(String)
        /// A pattern written in the locale's digits and names.
        case localized(String)
        case fixed(String)
    }

    /// The short date of `locale`'s region ("M/d/yy", "dd/MM/y", "y/MM/dd").
    static func shortPattern(_ locale: Locale) -> String {
        let f = DateFormatter()
        f.locale = locale
        f.dateStyle = .short
        f.timeStyle = .none
        return f.dateFormat ?? "dd/MM/yy"
    }

    /// `pattern` with two digits for the day, the month and the year:
    /// "M/d/yy" becomes "MM/dd/yy", "dd.MM.y" "dd.MM.yy". A fixed width for
    /// the date column of the geek lists.
    static func twoDigits(_ pattern: String) -> String {
        var out = ""
        var last: Character? = nil
        for c in pattern {
            if "dMy".contains(c) {
                if c != last { out += String(repeating: c, count: 2) }
            } else {
                out.append(c)
            }
            last = c
        }
        return out
    }

    /// The first of `patterns` whose dates all fit in `width` characters,
    /// tried on the longest dates there are (the 28th of every month, in
    /// the morning and in the evening). The last one is kept whatever its
    /// width: a date cut at its end still reads better than none.
    static func fitting(_ patterns: [Pattern], width: Int,
                        make: ((DateFormatter) -> Void) -> DateFormatter) -> DateFormatter {
        var formatter: DateFormatter!
        for pattern in patterns {
            formatter = make { f in
                switch pattern {
                case .template(let t): f.setLocalizedDateFormatFromTemplate(t)
                case .localized(let p): f.dateFormat = p
                case .fixed(let p):
                    // A fixed pattern is written with ASCII digits and the
                    // Gregorian calendar: it is the one fallback every
                    // reader knows.
                    f.locale = Locale(identifier: "en_US_POSIX")
                    f.dateFormat = p
                }
            }
            if samples(calendar: formatter.calendar, timeZone: formatter.timeZone).allSatisfy({ formatter.string(from: $0).count <= width }) {
                return formatter
            }
        }
        return formatter
    }

    /// The 28th of every month of a year of the formatter's own calendar
    /// (an Islamic month name is not met on the 28th of every Gregorian
    /// month), at 09:05 and 23:13, in `timeZone`.
    private static func samples(calendar base: Calendar, timeZone: TimeZone) -> [Date] {
        var calendar = base
        calendar.timeZone = timeZone
        let year = calendar.component(.year, from: Date(timeIntervalSince1970: 1_700_000_000))
        let months = calendar.range(of: .month, in: .year, for: Date(timeIntervalSince1970: 1_700_000_000)) ?? 1..<13
        return months.flatMap { month in
            [9, 23].compactMap { hour in
                calendar.date(from: DateComponents(year: year, month: month, day: 28, hour: hour, minute: hour == 9 ? 5 : 13))
            }
        }
    }

    /// The formatters of the interface.
    static let dates = Dates(locale: uiLocale)

    /// A size in the interface's locale: "251 ko" in French, "251 kB" in
    /// English (`localisation.md`, section 3.6). `ByteCountFormatter`
    /// takes no locale: it writes the units of the process's language,
    /// English for the app run as a bare executable.
    static func bytes(_ n: Int, locale: Locale = uiLocale) -> String {
        Int64(n).formatted(.byteCount(style: .file).locale(locale))
    }

    /// Subject of a mail quoted in a status message ("“Devis toiture” moved
    /// to Archive"): "(no subject)" when empty, whitespace (line breaks of
    /// folded headers included) collapsed, and cut to `limit` characters
    /// with "…" so that the end of the message (the folder) stays visible.
    static func flashSubject(_ subject: String, limit: Int = 50) -> String {
        let words = subject.split(whereSeparator: \.isWhitespace)
        let s = words.joined(separator: " ")
        if s.isEmpty { return L("(no subject)") }
        return s.count > limit ? String(s.prefix(limit - 1)) + "…" : s
    }

    /// Compact date for the lists: the time if today, day and month if this
    /// year, the short date otherwise.
    static func listDate(_ ts: Int, in f: Dates = dates, now: Date = Date(), calendar: Calendar = .current) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        if calendar.isDate(d, inSameDayAs: now) { return f.time.string(from: d) }
        if calendar.isDate(d, equalTo: now, toGranularity: .year) { return f.dayMonth.string(from: d) }
        return f.short.string(from: d)
    }

    /// Date of the geek lists, at most 8 characters so that the column has
    /// a fixed width: "14:05" today, "16 oct." this year, "14/09/21" before
    /// (in French; "2:05 PM", "Oct 16", "09/14/21" in American English).
    static func geekListDate(_ ts: Int, in f: Dates = dates, now: Date = Date(), calendar: Calendar = .current) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        if calendar.isDate(d, inSameDayAs: now) { return f.geekTime.string(from: d) }
        if calendar.isDate(d, equalTo: now, toGranularity: .year) { return f.geekDayMonth.string(from: d) }
        return f.geekShort.string(from: d)
    }

    /// The day something happened, never its time: "3 Nov" this year,
    /// "3 Nov 2021" before. For a state that lasts ("unsubscribed on…"),
    /// where the time of the lists ("09:57" today) reads as a time, not a
    /// day.
    static func day(_ ts: Int, in f: Dates = dates, now: Date = Date(), calendar: Calendar = .current) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        if calendar.isDate(d, equalTo: now, toGranularity: .year) { return f.dayMonth.string(from: d) }
        return f.dayMonthYear.string(from: d)
    }

    /// Date of a message header in the geek reader: "14:05" today,
    /// "yesterday 14:05", "3 Nov at 14:05" this year, "3 Nov 2021" before.
    /// The day and the time of this year make one template, so that the
    /// locale chooses their order and what joins them ("à" in French).
    static func geekMessageDate(_ ts: Int, in f: Dates = dates, now: Date = Date(), calendar: Calendar = .current) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        if calendar.isDate(d, inSameDayAs: now) { return f.time.string(from: d) }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now), calendar.isDate(d, inSameDayAs: yesterday) {
            return L("yesterday %@", f.time.string(from: d))
        }
        if calendar.isDate(d, equalTo: now, toGranularity: .year) { return f.dayMonthTime.string(from: d) }
        return f.dayMonthYear.string(from: d)
    }

    /// Date of the History screen: "14:05" today, else the day of the lists
    /// and the time ("16 oct. 14:05"): when something was done matters here,
    /// and the full date took a third of the row.
    static func historyDate(_ ts: Int, in f: Dates = dates, now: Date = Date(), calendar: Calendar = .current) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ts))
        let time = f.geekTime.string(from: d).trimmingCharacters(in: .whitespaces)
        if calendar.isDate(d, inSameDayAs: now) { return time }
        return geekListDate(ts, in: f, now: now, calendar: calendar).trimmingCharacters(in: .whitespaces) + " " + time
    }

    /// Full date and time, for the message header.
    static func longDate(_ ts: Int, in f: Dates = dates) -> String {
        f.long.string(from: Date(timeIntervalSince1970: TimeInterval(ts)))
    }
}
