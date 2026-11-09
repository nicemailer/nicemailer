import Foundation

// Replying to a mail, seen from the app (`specifications/reponse.md`).
//
// Who the reply goes to, its subject, its quotation and where the
// conversation is filed are worked out by the core (`create_draft`); the
// app only chooses the mail answered, writes the line introducing the
// quotation in its own language, lets the user type, and shows what the
// core prepared.

/// A draft, as the core gives it (`DraftView` of the contract).
struct DraftView: Decodable, Equatable {
    let id: Int
    let accountId: Int
    let from: AddressView
    let to: [AddressView]
    let cc: [AddressView]
    /// Hidden recipients: never in the mail sent, kept in the user's copies.
    let bcc: [AddressView]
    let subject: String
    let body: String
    let revision: Int
    /// `editing` or `sending`.
    let state: String
    let replyTo: Int?
    let threadId: Int?
    /// Where the conversation goes once the mail is sent, as preselected
    /// by the core; nil: it stays, and no filing is offered.
    let afterSendTarget: String?
    /// The filing the user chose with ⇧⌘↩ (`archive`, `waiting`, `todo` or
    /// `stay`), which wins over the preselection; nil: none chosen.
    let afterSend: String?
    /// When a draft being sent leaves, Unix seconds.
    let sendAt: Int?
}

struct CreateDraftResult: Decodable {
    let draft: DraftView
    let reused: Bool
}

struct UpdateDraftResult: Decodable {
    let revision: Int
}

struct SendDraftResult: Decodable {
    let sendAt: Int
}

/// What `parse_addresses` understood of a recipient line.
struct ParseAddressesResult: Decodable {
    let addresses: [AddressView]
    let invalid: [String]
}

struct SuggestRecipientsResult: Decodable {
    let items: [AddressView]
}

/// Where the keys go in the reply (`reponse.md`, section 6): a recipient
/// line, the subject, or the text. Tab and ⇧Tab go from one to the next,
/// in this order.
enum ComposeField: String, CaseIterable {
    case to, cc, bcc, subject, body

    var isRecipients: Bool { self == .to || self == .cc || self == .bcc }
}

/// The headers of a draft as edited: what `update_draft` saves when it
/// differs from what was saved last.
struct ComposeHeaders: Equatable {
    var to: [AddressView]
    var cc: [AddressView]
    var bcc: [AddressView]
    var subject: String

    init(_ d: DraftView) {
        to = d.to
        cc = d.cc
        bcc = d.bcc
        subject = d.subject
    }

    subscript(field: ComposeField) -> [AddressView] {
        get {
            switch field {
            case .to: to
            case .cc: cc
            case .bcc: bcc
            default: []
            }
        }
        set {
            switch field {
            case .to: to = newValue
            case .cc: cc = newValue
            case .bcc: bcc = newValue
            default: break
            }
        }
    }

    /// Every recipient, To then Cc then Bcc.
    var all: [AddressView] { to + cc + bcc }
}

/// The reply being written: the draft as the core last saw it, and what
/// the user typed since.
struct Composer: Equatable {
    var draft: DraftView
    var buffer: TextBuffer
    /// The revision of the draft the text was last saved at: `update_draft`
    /// sends it, and gets the next one back.
    var revision: Int
    /// The text last saved, to save only what changed.
    var saved: String
    /// The recipients and the subject as edited, and as last saved.
    var headers: ComposeHeaders
    var savedHeaders: ComposeHeaders
    /// Where the keys go.
    var focus: ComposeField = .body
    /// The text typed in the focused recipient line, not yet made an
    /// address (Return, a comma, Tab or leaving the line does).
    var line = LineEdit()
    /// The subject as edited, with its cursor.
    var subjectLine: LineEdit
    /// The Bcc line is shown: it has addresses, or ⌥⌘B opened it.
    var showsBcc: Bool
    /// Addresses suggested for the text typed (`suggest_recipients`), and
    /// the one ↑ ↓ highlighted.
    var suggestions: [AddressView] = []
    var suggestionIndex: Int?

    init(draft: DraftView) {
        self.draft = draft
        // The cursor at the start: the reply is written above the
        // quotation (`reponse.md`, section 4).
        buffer = TextBuffer(draft.body, cursor: 0)
        revision = draft.revision
        saved = buffer.text
        headers = ComposeHeaders(draft)
        savedHeaders = headers
        subjectLine = LineEdit(draft.subject)
        showsBcc = !draft.bcc.isEmpty
    }

    var isDirty: Bool { buffer.text != saved || headers != savedHeaders }

    /// The fields Tab goes through, in order: Bcc only when shown.
    var fields: [ComposeField] {
        ComposeField.allCases.filter { $0 != .bcc || showsBcc }
    }

    /// The next field (Tab) or the previous one (⇧Tab), turning around.
    func field(after f: ComposeField, backward: Bool) -> ComposeField {
        let list = fields
        guard let i = list.firstIndex(of: f) else { return .body }
        let n = list.count
        return list[(i + (backward ? n - 1 : 1)) % n]
    }

    /// Adds addresses to a recipient line, each once across the three
    /// lines: an address already in another line stays where it is.
    /// Returns those that were already there.
    @discardableResult
    mutating func add(_ addresses: [AddressView], to field: ComposeField) -> [AddressView] {
        var already: [AddressView] = []
        for a in addresses {
            if headers.all.contains(where: { $0.addr.caseInsensitiveCompare(a.addr) == .orderedSame }) {
                already.append(a)
            } else {
                headers[field].append(a)
            }
        }
        return already
    }

    /// ⌫ on an empty recipient line: its last address goes back to the
    /// line as text, to be corrected or deleted.
    mutating func takeBackLast(of field: ComposeField) -> Bool {
        guard let last = headers[field].popLast() else { return false }
        line = LineEdit(last.addr)
        return true
    }
}

/// Rules and texts of the reply screen, as pure functions (tested without
/// a core).
enum Compose {
    /// Delay without typing before the text is saved (`reponse.md`,
    /// section 5): a crash loses at most this much typing.
    static let saveDelay: Duration = .milliseconds(500)

    /// The mail `r` answers (`reponse.md`, section 3): the one under the
    /// cursor of the reader (`ThreadDisplay.State.current`, moved by
    /// `ctrl-j` / `ctrl-k`), else the latest mail received, not one of the
    /// user's own (Sent, Drafts), else the latest.
    static func replyTarget(_ thread: ThreadView, current: Int?) -> MessageView? {
        if let current, let m = thread.messages.first(where: { $0.id == current }) { return m }
        let own: Set<String> = ["sent", "drafts"]
        return thread.messages.last { !own.contains($0.folderRole ?? "") } ?? thread.messages.last
    }

    /// The line introducing the quotation, in the app's language:
    /// "On Nov 7, 2026, at 10:12, Alice Martin <alice@x.fr> wrote:". The
    /// date is the mail's, in the Mac's time zone.
    static func quoteHeader(_ m: MessageView, locale: Locale = appLocale, timeZone: TimeZone = .current) -> String {
        let when = Date(timeIntervalSince1970: TimeInterval(m.dateSent ?? m.dateReceived))
        let day = DateFormatter()
        day.locale = locale
        day.timeZone = timeZone
        day.dateStyle = .medium
        day.timeStyle = .none
        let time = DateFormatter()
        time.locale = locale
        time.timeZone = timeZone
        time.dateStyle = .none
        time.timeStyle = .short
        let who: String
        switch (m.fromName?.trimmingCharacters(in: .whitespaces), m.fromAddr) {
        case let (name?, addr?) where !name.isEmpty: who = "\(name) <\(addr)>"
        case let (_, addr?): who = addr
        case let (name?, nil) where !name.isEmpty: who = name
        default: who = L("someone")
        }
        return L("On %@, at %@, %@ wrote:", day.string(from: when), time.string(from: when), who)
    }

    /// An address as shown in a recipient line: "Alice <alice@x.fr>", or
    /// the address alone.
    static func label(_ a: AddressView) -> String {
        a.name.map { "\($0) <\(a.addr)>" } ?? a.addr
    }

    /// The parameters of `update_draft` for what changed since the last
    /// save: the body, and the recipients or the subject only when edited.
    static func saveParams(_ c: Composer) -> [String: Any] {
        var p: [String: Any] = ["id": c.draft.id, "revision": c.revision]
        if c.buffer.text != c.saved { p["body"] = c.buffer.text }
        let h = c.headers, s = c.savedHeaders
        let list = { (l: [AddressView]) -> [[String: Any]] in l.map { ["addr": $0.addr, "name": $0.name ?? NSNull()] } }
        if h.to != s.to { p["to"] = list(h.to) }
        if h.cc != s.cc { p["cc"] = list(h.cc) }
        if h.bcc != s.bcc { p["bcc"] = list(h.bcc) }
        if h.subject != s.subject { p["subject"] = h.subject }
        return p
    }

    /// Whether a key typed in a recipient line ends the address: `,` and
    /// `;` separate addresses, as in every mail app.
    static func endsAddress(_ key: String) -> Bool {
        key == "," || key == ";"
    }

    /// A list of recipients on one line, cut after `limit` characters with
    /// the number of those not shown: "Alice Martin, Bob +3".
    static func recipients(_ list: [AddressView], limit: Int = 80) -> String {
        var shown: [String] = []
        var length = 0
        for (i, a) in list.enumerated() {
            let name = a.name.map { "\($0) <\(a.addr)>" } ?? a.addr
            if !shown.isEmpty, length + 2 + name.count > limit {
                return shown.joined(separator: ", ") + " " + L("+%lld", list.count - i)
            }
            shown.append(name)
            length += (shown.count > 1 ? 2 : 0) + name.count
        }
        return shown.joined(separator: ", ")
    }

    /// The filing that will apply: the one chosen, else the preselection.
    /// Nil when the conversation stays where it is, because the core offers
    /// no filing for it (it is not in the Inbox, Todo or Waiting) or because
    /// the user chose `stay`.
    static func afterSend(_ d: DraftView) -> String? {
        guard d.afterSendTarget != nil else { return nil }
        let chosen = d.afterSend ?? d.afterSendTarget
        return chosen == "stay" ? nil : chosen
    }

    /// The line "After sending: Archive", "After sending: stays where it
    /// is" when the user chose to leave it, or nil when no filing applies.
    static func afterSendLine(_ d: DraftView) -> String? {
        guard d.afterSendTarget != nil else { return nil }
        if d.afterSend == "stay" { return L("After sending: stays where it is") }
        return afterSend(d).map { L("After sending: %@", Role.name($0)) }
    }

    /// The answers of ⇧⌘↩ "Send, then…" (`reponse.md`, section 8): the
    /// three filings of `dossiers.md`, then the trial option `stay`, shown
    /// only here. `current` marks the one ⌘↩ would apply.
    struct SendChoice: Equatable {
        let key: String
        /// What `send_draft` receives as `after_send`.
        let afterSend: String
        let label: String
        let current: Bool
    }

    static func sendChoices(_ d: DraftView) -> [SendChoice] {
        let current = d.afterSend ?? d.afterSendTarget
        return [("a", "archive"), ("w", "waiting"), ("t", "todo"), ("s", "stay")].map { key, value in
            let label = value == "stay" ? L("leave it where it is (trial)") : Role.name(value)
            return SendChoice(key: key, afterSend: value, label: label, current: value == current)
        }
    }

    /// The status bar while a mail waits for its undo delay.
    static func sendingFlash(seconds: Int) -> String {
        seconds > 0 ? L("Sending in %lld s · z to cancel", seconds) : L("Sending…")
    }

    /// The status bar once the mail is sent: where the conversation went,
    /// and that `z` puts it back (the mail itself has left).
    static func sentFlash(movedTo: String?, undo: Bool) -> String {
        var text = L("Sent")
        if let movedTo { text += " · " + L("filed in %@", Role.name(movedTo)) }
        if undo { text += " · " + (movedTo != nil ? L("z: undo the filing") : L("z: undo")) }
        return text
    }
}
