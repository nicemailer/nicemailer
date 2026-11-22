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
    /// The account of the draft: of the mail answered, whose conversation
    /// and Drafts folder keep it.
    let accountId: Int
    /// The account that sends it (its SMTP server, its Sent folder): the
    /// draft's own, or another one when the draft's cannot send
    /// (`reponse.md`, section 4 ter).
    let sendAccountId: Int
    let from: AddressView
    let to: [AddressView]
    let cc: [AddressView]
    /// Hidden recipients: never in the mail sent, kept in the user's copies.
    let bcc: [AddressView]
    let subject: String
    let body: String
    /// `text`: `body` is the text; `html`: it is the HTML of the editor's
    /// formatted mode (`edition.md`, section 3). Absent from an older core.
    var bodyFormat: String?
    let attachments: [DraftAttachmentView]
    let revision: Int
    /// `editing` or `sending`.
    let state: String
    /// `reply`, `forward` or `new`.
    let kind: String
    let replyTo: Int?
    /// The mail a forward carries.
    let forwardOf: Int?
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

/// The result of `prepare_draft` (what `r`, `R`, `f`, `c` open, written
/// nowhere yet) and of `create_draft` (the draft written).
struct CreateDraftResult: Decodable {
    /// From `prepare_draft`, a draft with `id` 0 unless `reused`: proposed,
    /// no row holds it yet (`Composer.isWritten`).
    let draft: DraftView
    let reused: Bool
    /// A forward whose files could not all be taken: which.
    let warning: String?
}

/// What a draft starts as (`create_draft`'s `kind`): `r`, `R`, `f`, `c`.
enum DraftKind: String {
    case replyAll = "reply_all"
    case replySender = "reply_sender"
    case forward
    case new

    /// The kind of a draft the core shows (`reply`, `forward`, `new`): a
    /// reply already written is given back whatever its recipients.
    init(view kind: String) {
        switch kind {
        case "forward": self = .forward
        case "new": self = .new
        default: self = .replyAll
        }
    }
}

struct UpdateDraftResult: Decodable {
    let revision: Int
}

/// The result of `html_to_text`: the text of a formatted draft, and whether
/// it had formatting that text only loses (`edition.md`, section 3).
struct HtmlToTextResult: Decodable {
    let text: String
    let formatted: Bool
}

struct SendDraftResult: Decodable {
    let sendAt: Int
}

/// A file attached to a draft.
struct DraftAttachmentView: Decodable, Equatable, Hashable {
    let id: Int
    let filename: String
    let mimeType: String
    let size: Int
}

struct AddDraftAttachmentResult: Decodable {
    let revision: Int
    let attachment: DraftAttachmentView
    let warning: String?
}

/// What `parse_addresses` understood of a recipient line.
struct ParseAddressesResult: Decodable {
    let addresses: [AddressView]
    let invalid: [String]
}

struct SuggestRecipientsResult: Decodable {
    let items: [AddressView]
}

/// An address a mail can be sent from (`list_senders`): every account that
/// can send, each with its main address first, then its identities.
struct SenderView: Decodable, Equatable {
    let accountId: Int
    /// The key of the account (`accounts.toml`).
    let account: String
    let addr: String
    let name: String?
}

struct ListSendersResult: Decodable {
    let items: [SenderView]
}

/// Where the keys go in the reply (`reponse.md`, section 6): a recipient
/// line, the subject, or the text. Tab and ⇧Tab go from one to the next,
/// in this order.
enum ComposeField: String, CaseIterable {
    case to, cc, bcc, subject, attachments, body

    var isRecipients: Bool { self == .to || self == .cc || self == .bcc }
}

/// The headers of a draft as edited: what `update_draft` saves when it
/// differs from what was saved last.
struct ComposeHeaders: Equatable {
    var to: [AddressView]
    var cc: [AddressView]
    var bcc: [AddressView]
    var subject: String
    /// Who sends: the address, and the account it belongs to.
    var from: AddressView
    var sendAccountId: Int

    init(_ d: DraftView) {
        to = d.to
        cc = d.cc
        bcc = d.bcc
        subject = d.subject
        from = d.from
        sendAccountId = d.sendAccountId
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
///
/// The draft starts proposed (`prepare_draft`, `reponse.md`, section 5):
/// the core computed it without writing it, so that the screen opens at
/// once even while an import keeps the database busy writing. It is
/// written (`create_draft`, with the text typed) only when there is
/// something to keep: the first pause of typing, a file attached, the
/// sending (`AppModel.ensureWritten`). Closed before, it leaves nothing.
struct Composer: Equatable {
    var draft: DraftView
    /// Which opening of the reply screen this is: the answers of the core
    /// that arrive late are dropped when the user has left it, or opened
    /// another. Not the draft's id, which a proposal does not have yet.
    let opened = UUID()
    /// What the draft was opened as, and the line introducing the
    /// quotation: what `create_draft` needs to write a proposed draft.
    var kind: DraftKind
    var quoteHeader: String
    /// The text (text only) or the HTML (formatted) of the draft, as the
    /// editor last gave it (`EditorBridge`, `changed` and `flushed`): the
    /// editor holds the text being typed, the model what it reported.
    var body: String
    /// The mode of `body`.
    var format: ComposeFormat
    /// The revision of the draft the text was last saved at: `update_draft`
    /// sends it, and gets the next one back.
    var revision: Int
    /// The text and its mode last saved, to save only what changed.
    var saved: String
    var savedFormat: ComposeFormat
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
    /// The Bcc line is shown: it has addresses, or ⌥⌘B or the button at the
    /// end of the Cc line opened it.
    var showsBcc: Bool
    /// The files attached, as the core last listed them, and the one
    /// selected while the line of files has the keys (← → and ⌫).
    var attachments: [DraftAttachmentView]
    var attachmentIndex = 0
    /// Addresses suggested for the text typed (`suggest_recipients`), and
    /// the one ↑ ↓ highlighted.
    var suggestions: [AddressView] = []
    var suggestionIndex: Int?

    init(draft: DraftView, kind: DraftKind? = nil, quoteHeader: String = "") {
        self.draft = draft
        self.kind = kind ?? DraftKind(view: draft.kind)
        self.quoteHeader = quoteHeader
        body = draft.body
        format = ComposeFormat(rawValue: draft.bodyFormat ?? "text") ?? .text
        revision = draft.revision
        saved = draft.body
        savedFormat = format
        headers = ComposeHeaders(draft)
        savedHeaders = headers
        subjectLine = LineEdit(draft.subject)
        showsBcc = !draft.bcc.isEmpty
        attachments = draft.attachments
        // A forward or a new mail starts with whom it goes to; a reply is
        // addressed already, the keys go to the text.
        if draft.kind != "reply", draft.to.isEmpty { focus = .to }
    }

    /// A new mail with nothing in it: the core drops it rather than keep
    /// it in Drafts (`drop_if_blank`).
    var isBlank: Bool {
        draft.kind == "new" && headers.all.isEmpty && headers.subject.trimmingCharacters(in: .whitespaces).isEmpty
            && Compose.visibleText(body, format).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && attachments.isEmpty && line.text.isEmpty
    }

    var isDirty: Bool { body != saved || format != savedFormat || headers != savedHeaders }

    /// The draft is a row of the core: a proposal has id 0 (SQLite ids
    /// start at 1). Until then, no call that takes an id can be made.
    var isWritten: Bool { draft.id > 0 }

    /// The draft as `create_draft` wrote it: its id, its revision, what it
    /// holds now (what was typed, or another client's text if it wrote the
    /// same reply first). The text and the headers typed stay; what
    /// differs from the row is saved next (`isDirty`).
    ///
    /// The files are the core's: a proposal holds only those of a forward
    /// (negative ids), since attaching one writes the draft first; written,
    /// they have their real ids.
    mutating func written(_ d: DraftView) {
        draft = d
        revision = d.revision
        saved = d.body
        savedFormat = ComposeFormat(rawValue: d.bodyFormat ?? "text") ?? .text
        savedHeaders = ComposeHeaders(d)
        attachments = d.attachments
        attachmentIndex = min(attachmentIndex, max(0, attachments.count - 1))
    }

    /// The fields Tab goes through, in order: Bcc only when shown, the
    /// line of files only when there are some.
    var fields: [ComposeField] {
        ComposeField.allCases.filter { ($0 != .bcc || showsBcc) && ($0 != .attachments || !attachments.isEmpty) }
    }

    /// Whether the Bcc line may be hidden: only with nothing in it. Hidden
    /// with an address, or with one being typed, which leaving the line
    /// makes an address, the mail would go to someone the screen no longer
    /// shows.
    var canHideBcc: Bool {
        headers.bcc.isEmpty && !(focus == .bcc && !line.text.trimmingCharacters(in: .whitespaces).isEmpty)
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
    /// line, **selected** (`reponse.md`, section 6, "Corriger").
    ///
    /// Selected, as a macOS field reached with Tab, it is one keystroke
    /// away from either wish: a second ⌫ wipes it, typing replaces it,
    /// and ← or → turns it into an ordinary text to correct letter by
    /// letter. It used to come back with the caret at its end, so
    /// removing the address meant deleting every one of its characters.
    mutating func takeBackLast(of field: ComposeField) -> Bool {
        guard !headers[field].isEmpty else { return false }
        return takeBack(headers[field].count - 1, of: field)
    }

    /// A click on an address: that one goes back to its line, selected,
    /// as ⌫ brings back the last (`takeBackLast`). Any address of the
    /// three lines can so be removed or corrected, not only the last one.
    /// What was being typed in the line is committed first by the caller
    /// (`InterfaceController.clickRecipient`), so nothing typed is lost.
    @discardableResult
    mutating func takeBack(_ index: Int, of field: ComposeField) -> Bool {
        guard headers[field].indices.contains(index) else { return false }
        let taken = headers[field].remove(at: index)
        // The address alone, as before: a name is for reading, and
        // what is typed back is read by `parse_addresses` anyway.
        line = LineEdit(taken.addr, selected: true)
        return true
    }
}

/// Rules and texts of the reply screen, as pure functions (tested without
/// a core).
enum Compose {
    /// The text a reader sees in a body: itself in text only; in the
    /// formatted mode, the HTML without its tags (for "is it blank?" only:
    /// the core makes the real text, `html_to_text`).
    static func visibleText(_ body: String, _ format: ComposeFormat) -> String {
        guard format == .html else { return body }
        let noTags = body.replacingOccurrences(of: "<[^>]*>", with: " ", options: .regularExpression)
        return noTags.replacingOccurrences(of: "&nbsp;", with: " ").replacingOccurrences(of: "&[a-zA-Z#0-9]+;", with: "x", options: .regularExpression)
    }

    /// The part of `old` that `new` replaced, in UTF-16 units of `old` (the
    /// units of the editor's text and of `NSString`), and the new words:
    /// what a change of the AI the core applied does to the text, sent to
    /// the editor as one step of undo (`EditorBridge.replaceRange`).
    static func changedRange(old: String, new: String) -> (start: Int, end: Int, text: String) {
        let a = Array(old.utf16), b = Array(new.utf16)
        if a == b { return (a.count, a.count, "") }
        var prefix = 0
        while prefix < a.count, prefix < b.count, a[prefix] == b[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < a.count - prefix, suffix < b.count - prefix, a[a.count - 1 - suffix] == b[b.count - 1 - suffix] { suffix += 1 }
        // Never inside a surrogate pair (an emoji is two units): the cut
        // moves to the edge of the character.
        if prefix > 0, prefix < a.count, UTF16.isTrailSurrogate(a[prefix]) { prefix -= 1 }
        if prefix > 0, prefix < b.count, UTF16.isTrailSurrogate(b[prefix]) { prefix -= 1 }
        if suffix > 0, a.count - suffix < a.count, UTF16.isTrailSurrogate(a[a.count - suffix]) { suffix -= 1 }
        if suffix > 0, b.count - suffix < b.count, UTF16.isTrailSurrogate(b[b.count - suffix]) { suffix -= 1 }
        suffix = min(suffix, a.count - prefix, b.count - prefix)
        // Whole words: "jeudi" rewritten "vendredi" shares "di", but the
        // word changed, and the cursor goes after it, not inside.
        func isWord(_ u: UInt16) -> Bool {
            guard let s = Unicode.Scalar(u) else { return true }  // half of a pair
            return CharacterSet.alphanumerics.contains(s)
        }
        while prefix > 0, isWord(a[prefix - 1]) { prefix -= 1 }
        while suffix > 0, isWord(a[a.count - suffix]) { suffix -= 1 }
        let text = String(decoding: b[prefix..<(b.count - suffix)], as: UTF16.self)
        return (prefix, a.count - suffix, text)
    }

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
    /// date is the mail's, in the Mac's time zone, written as the Mac's
    /// region writes dates (`uiLocale`, `localisation.md`, section 3.2):
    /// an English interface on a Mac set to France writes "7 Nov 2026" and
    /// "23:13", not "Nov 7" and "11:13 PM".
    static func quoteHeader(_ m: MessageView, locale: Locale = uiLocale, timeZone: TimeZone = .current) -> String {
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

    /// A file in the line of attachments: "devis.pdf (240 KB)".
    static func attachmentLabel(_ a: DraftAttachmentView) -> String {
        "\(a.filename) (\(Format.bytes(a.size)))"
    }

    /// The name under which a file is staged in `incoming.noindex`: what
    /// the core accepts (letters, digits, `-`), never a path.
    static func stagedName() -> String {
        UUID().uuidString
    }

    /// An address as shown in a recipient line: "Alice <alice@x.fr>", or
    /// the address alone.
    static func label(_ a: AddressView) -> String {
        a.name.map { "\($0) <\(a.addr)>" } ?? a.addr
    }

    /// The parameters of `create_draft` that write a proposed draft: what
    /// it was opened as, and the text typed, which replaces the quotation
    /// the core would write. The recipients, the subject and the sender
    /// edited meanwhile follow with `update_draft` (`saveParams`).
    static func createParams(_ c: Composer) -> [String: Any] {
        var p: [String: Any] = ["kind": c.kind.rawValue, "quote_header": c.quoteHeader, "body": c.body,
                                "body_format": c.format.rawValue]
        switch c.kind {
        case .new: p["account"] = c.draft.accountId
        case .forward: if let m = c.draft.forwardOf { p["reply_to"] = m }
        case .replyAll, .replySender: if let m = c.draft.replyTo { p["reply_to"] = m }
        }
        return p
    }

    /// The parameters of `update_draft` for what changed since the last
    /// save: the body, and the recipients or the subject only when edited.
    static func saveParams(_ c: Composer) -> [String: Any] {
        // Only for a draft written (`AppModel.ensureWritten` first).
        var p: [String: Any] = ["id": c.draft.id, "revision": c.revision]
        if c.body != c.saved || c.format != c.savedFormat {
            p["body"] = c.body
            p["body_format"] = c.format.rawValue
        }
        let h = c.headers, s = c.savedHeaders
        let list = { (l: [AddressView]) -> [[String: Any]] in l.map { ["addr": $0.addr, "name": $0.name ?? NSNull()] } }
        if h.to != s.to { p["to"] = list(h.to) }
        if h.cc != s.cc { p["cc"] = list(h.cc) }
        if h.bcc != s.bcc { p["bcc"] = list(h.bcc) }
        if h.subject != s.subject { p["subject"] = h.subject }
        if h.from != s.from || h.sendAccountId != s.sendAccountId {
            p["from"] = ["account_id": h.sendAccountId, "addr": h.from.addr]
        }
        return p
    }

    /// The answers of ⌥⌘F "Send from" (`reponse.md`, section 4 ter): every
    /// address of `list_senders`, keyed 1 to 9 then by letter, the current
    /// one marked.
    struct SenderChoice: Equatable {
        let key: String
        let sender: SenderView
        let label: String
        let current: Bool
    }

    static func senderChoices(_ senders: [SenderView], current h: ComposeHeaders) -> [SenderChoice] {
        let keys = Array("123456789abcdefghijklmnopqrstuvwxy").map(String.init)
        return zip(keys, senders).map { key, s in
            let current = s.accountId == h.sendAccountId && s.addr.caseInsensitiveCompare(h.from.addr) == .orderedSame
            return SenderChoice(key: key, sender: s, label: senderLabel(s), current: current)
        }
    }

    /// An address to send from, with its account: "Me <me@x.fr> · perso".
    static func senderLabel(_ s: SenderView) -> String {
        label(AddressView(addr: s.addr, name: s.name)) + " · " + s.account
    }

    /// The line "From" of the reply: the address, and the key of the
    /// account that sends when the app knows it.
    static func fromLine(_ h: ComposeHeaders, senders: [SenderView]) -> String {
        let account = senders.first { $0.accountId == h.sendAccountId }?.account
        return label(h.from) + (account.map { " · " + $0 } ?? "")
    }

    /// The status bar when a reply starts from another account than the
    /// mail's, which cannot send: which one sends, and how to change it.
    static func otherSenderFlash(_ d: DraftView, senders: [SenderView], keymap: Keymap = .defaults) -> String? {
        guard d.sendAccountId != d.accountId, d.kind != "new" else { return nil }
        let h = ComposeHeaders(d)
        return L("This account cannot send: from %@ · %@ to change", fromLine(h, senders: senders),
                 keymap.mention(.composeFrom, in: Keymap.composeChain))
    }

    /// Whether a key typed in a recipient line ends the address: `,` and
    /// `;` separate addresses, as in every mail app.
    static func endsAddress(_ key: String) -> Bool {
        key == "," || key == ";"
    }

    /// The block that introduces a mail forwarded, in the app's language
    /// (`reponse.md`, `f`), its date as the Mac's region writes it
    /// (`uiLocale`): the core puts it between the user's text and the
    /// mail's.
    static func forwardHeader(_ m: MessageView, locale: Locale = uiLocale, timeZone: TimeZone = .current) -> String {
        let when = Date(timeIntervalSince1970: TimeInterval(m.dateSent ?? m.dateReceived))
        let date = DateFormatter()
        date.locale = locale
        date.timeZone = timeZone
        date.dateStyle = .medium
        date.timeStyle = .short
        let from: String
        switch (m.fromName?.trimmingCharacters(in: .whitespaces), m.fromAddr) {
        case let (name?, addr?) where !name.isEmpty: from = "\(name) <\(addr)>"
        case let (_, addr?): from = addr
        default: from = m.fromName ?? ""
        }
        var lines = [
            L("---------- Forwarded message ---------"),
            L("From: %@", from),
            L("Date: %@", date.string(from: when)),
            L("Subject: %@", m.subject),
        ]
        if !m.to.isEmpty { lines.append(L("To: %@", m.to.map(label).joined(separator: ", "))) }
        if !m.cc.isEmpty { lines.append(L("Cc: %@", m.cc.map(label).joined(separator: ", "))) }
        return lines.joined(separator: "\n")
    }

    /// What the status bar says when a draft is left with Escape.
    static func keptFlash(_ c: Composer) -> String {
        if c.isBlank { return L("Empty mail dropped") }
        // Proposed and left as it was: nothing was ever written. An
        // address still typed in a line is something: it is made a
        // recipient and saved on the way out.
        if !c.isWritten, !c.isDirty, c.line.text.trimmingCharacters(in: .whitespaces).isEmpty {
            return L("Nothing written: no draft kept")
        }
        switch c.draft.kind {
        case "forward": return L("Forward kept in Drafts · f to resume")
        case "new": return L("Mail kept in Drafts")
        default: return L("Reply kept in Drafts · r to resume")
        }
    }

    /// A list of recipients on one line, cut after `limit` characters with
    /// the number of those not shown: "Alice Martin, Bob +3".
    static func recipients(_ list: [AddressView], limit: Int = 80) -> String {
        var shown: [String] = []
        var length = 0
        for (i, a) in list.enumerated() {
            let name = a.name.map { "\($0) <\(a.addr)>" } ?? a.addr
            if !shown.isEmpty, length + 2 + name.count > limit {
                return L("%@ +%lld", shown.joined(separator: ", "), list.count - i)
            }
            shown.append(name)
            length += (shown.count > 1 ? 2 : 0) + name.count
        }
        return shown.joined(separator: ", ")
    }

    /// Whether ⇧⌘↩ has something to ask (`reponse.md`, section 8):
    ///
    /// - a reply whose conversation the core offers to file
    ///   (`afterSendTarget`: it is in the Inbox, Todo or Waiting);
    /// - a **new mail**, which answers nothing and so files itself: the
    ///   copy kept in Sent goes to the folder chosen ("Nouveau mail").
    ///
    /// A forward files nothing, and neither does a reply to a
    /// conversation already in Archive, Newsletter…: ⇧⌘↩ says so.
    static func offersFiling(_ d: DraftView) -> Bool {
        d.afterSendTarget != nil || filesItself(d)
    }

    /// A draft whose own mail is filed, not a conversation: a new mail
    /// (`kind` `new`). The wording and the preselection differ.
    static func filesItself(_ d: DraftView) -> Bool {
        d.kind == "new"
    }

    /// The filing that will apply: the one chosen, else the preselection.
    /// Nil when nothing moves: no filing is offered, the user chose
    /// `stay`, or a new mail was left to Sent (its preselection).
    static func afterSend(_ d: DraftView) -> String? {
        guard offersFiling(d) else { return nil }
        let chosen = d.afterSend ?? (filesItself(d) ? "stay" : d.afterSendTarget)
        return chosen == "stay" ? nil : chosen
    }

    /// The line "After sending: Archive", "After sending: stays where it
    /// is" when the user chose to leave a conversation where it is, or nil
    /// when nothing moves (a new mail left to Sent says nothing: that is
    /// what happens anyway).
    static func afterSendLine(_ d: DraftView, names: FolderNames = .english) -> String? {
        guard offersFiling(d) else { return nil }
        if d.afterSend == "stay" { return filesItself(d) ? nil : L("After sending: stays where it is") }
        return afterSend(d).map { L("After sending: %@", names.name($0)) }
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

    static func sendChoices(_ d: DraftView, names: FolderNames = .english) -> [SendChoice] {
        // A new mail is in Sent and stays there unless the user says
        // otherwise: `stay` is its preselection.
        let own = filesItself(d)
        let current = d.afterSend ?? (own ? "stay" : d.afterSendTarget)
        return [("a", "archive"), ("w", "waiting"), ("t", "todo"), ("s", "stay")].map { key, value in
            let label = value == "stay"
                ? (own ? L("leave it in Sent") : L("leave it where it is (trial)"))
                : names.name(value)
            return SendChoice(key: key, afterSend: value, label: label, current: value == current)
        }
    }

    /// The status bar while a mail waits for its undo delay.
    static func sendingFlash(seconds: Int) -> String {
        seconds > 0 ? L("Sending in %lld s · z to cancel", seconds) : L("Sending…")
    }

    /// The status bar once the mail is sent: where the conversation went,
    /// and that `z` puts it back (the mail itself has left).
    ///
    /// `filing` is a new mail's own filing, which the core applies once
    /// the copy in Sent is made (`reponse.md`, section 8, "Nouveau
    /// mail"): said in the same words, but `z` does not undo it (it is
    /// outside the undo token of the sending), so only the decisions are
    /// offered back.
    static func sentFlash(movedTo: String?, filing: String? = nil, undo: Bool, names: FolderNames = .english) -> String {
        var text = L("Sent")
        if let target = movedTo ?? filing { text += " · " + L("filed in %@", names.name(target)) }
        if undo { text += " · " + (movedTo != nil ? L("z: undo the filing") : L("z: undo")) }
        return text
    }
}
