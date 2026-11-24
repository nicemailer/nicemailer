import Foundation

/// What the geek reader shows for a thread, as a list of blocks built from
/// the core's segments (`docs/thread-display.md`). A pure function of the
/// thread and of the user's choices, so it is tested without any view
/// (`Tests/NiceMailerTests/ThreadDisplayTests.swift`).
///
/// The core decides what is hidden and what is folded by default; this
/// only applies the user's toggles on top of it:
/// - `showAll` (key `T`, like mutt): every message unfolded, every hidden
///   segment shown;
/// - `folding`: messages the user unfolded or folded by clicking their
///   header;
/// - `revealed`: hidden segments the user opened by clicking their "···"
///   line.
enum ThreadDisplay {
    /// The user's choices for the open thread.
    struct State: Equatable {
        var showAll = false
        /// Message id → unfolded (true) or folded (false), overriding the
        /// core's default.
        var folding: [Int: Bool] = [:]
        /// Hidden segments shown anyway, as "messageId:segmentIndex".
        var revealed: Set<String> = []
        /// The message `ctrl-k` / `ctrl-j` start from (`step`); nil: the
        /// last unfolded one.
        var current: Int? = nil
    }

    /// `ctrl-k` (`delta` -1) / `ctrl-j` (+1): the message before or after
    /// the current one in a conversation (`ui.md`). The current one is
    /// folded and the other unfolded, unless `T` shows everything. Returns
    /// the new state and the message to scroll to; nil at either end (or
    /// for a single mail).
    static func step(_ thread: ThreadView, _ state: State, by delta: Int) -> (state: State, message: Int)? {
        let ids = thread.messages.map(\.id)
        guard ids.count > 1 else { return nil }
        let unfolded = { (m: MessageView) in state.showAll || (state.folding[m.id] ?? m.expanded) }
        let current = state.current.flatMap { ids.firstIndex(of: $0) }
            ?? thread.messages.lastIndex(where: unfolded) ?? ids.count - 1
        let target = current + delta
        guard ids.indices.contains(target) else { return nil }
        var s = state
        if !s.showAll {
            s.folding[ids[current]] = false
            s.folding[ids[target]] = true
        }
        s.current = ids[target]
        return (s, ids[target])
    }

    /// The message `l` gives the link of (`DeepLink`, `docs/deep-links.md`):
    /// the one the reader is on (`current`: `ctrl-k` / `ctrl-j`, a click on
    /// a header), so that each message of a conversation can be linked from
    /// a note; else the latest received, not one of the user's own (Sent,
    /// Drafts), so that the link opens the conversation in its folder
    /// rather than in Sent; else the latest. Nil for an empty thread.
    static func linkMessage(_ thread: ThreadView, _ state: State) -> MessageView? {
        if let current = state.current, let m = thread.messages.first(where: { $0.id == current }) { return m }
        return thread.messages.last { $0.folderRole != "sent" && $0.folderRole != "drafts" } ?? thread.messages.last
    }

    /// A deep link opened on `message` (`docs/deep-links.md`): it becomes
    /// the current message, where `ctrl-k` / `ctrl-j`, `r` and the exports
    /// start from, and it is unfolded, even when the core folds it by
    /// default (an older message). The other choices of the thread stay.
    /// Needs no thread: the link is opened before the thread is loaded.
    static func focus(_ state: State, on message: Int) -> State {
        var s = state
        s.current = message
        if !s.showAll { s.folding[message] = true }
        return s
    }

    /// The message whose sender `ctrl-c` and `ctrl-C` act on (`ui.md`,
    /// "Expéditeur"): the current message (`ctrl-k` / `ctrl-j`, a click on
    /// a header), else the last unfolded one, as `step` starts from. A
    /// message of the user's own (Sent, Drafts) stands for the latest mail
    /// received before it, else after it: the user does not look
    /// themselves up. Nil for an empty thread.
    static func senderMessage(_ thread: ThreadView, _ state: State) -> MessageView? {
        let messages = thread.messages
        let unfolded = { (m: MessageView) in state.showAll || (state.folding[m.id] ?? m.expanded) }
        guard let current = state.current.flatMap({ id in messages.firstIndex { $0.id == id } })
            ?? messages.lastIndex(where: unfolded) ?? messages.indices.last else { return nil }
        let received = { (m: MessageView) in m.folderRole != "sent" && m.folderRole != "drafts" }
        if received(messages[current]) { return messages[current] }
        return messages[..<current].last(where: received) ?? messages[current...].first(where: received)
            ?? messages[current]
    }

    enum Block: Hashable {
        /// A message header, one line (`ui.md`, "Expéditeur"). `folded`:
        /// "▸ name · date · @domain" and a snippet (`detail`); otherwise
        /// "▾ name · date", then the sender's `address` in full (alert
        /// colour when `alert`, `senderAlert`), and the recipients on a
        /// second line if worth showing (`detail`). The arrow, and folding
        /// by a click (`foldable`), only in a conversation: a single mail
        /// has nothing to fold. `files`: attachments of a folded message,
        /// shown as a paperclip and their number. `book`: whether the
        /// sender of an unfolded message is in the address book, an icon at
        /// the end of the line; nil when not known (`SenderBook`).
        case header(messageId: Int, folded: Bool, title: String, detail: String?, files: Int = 0,
                    address: String? = nil, alert: Bool = false, foldable: Bool = true, book: SenderBook? = nil)
        /// A visible segment.
        case segment(messageId: Int, index: Int, kind: String, text: String, depth: Int)
        /// A hidden segment, replaced by a "···" line that opens it.
        case hidden(messageId: Int, index: Int, label: String)
        /// A remark on an unfolded message (it only quotes earlier ones).
        case note(messageId: Int, text: String)
        /// The attachments of an unfolded message, as tiles at its foot
        /// (`AttachmentDisplay`).
        case attachments(messageId: Int, files: [AttachmentView])
        /// Separation between two messages.
        case separator
    }

    /// `book`: the senders found or not in the address book, by message id
    /// (`AppModel.senderBook`).
    static func blocks(_ thread: ThreadView, _ state: State, book: [Int: SenderBook] = [:]) -> [Block] {
        var out: [Block] = []
        // A conversation: each message folds, and shows an arrow.
        let conversation = thread.messages.count > 1
        for (i, m) in thread.messages.enumerated() {
            if i > 0 { out.append(.separator) }
            let unfolded = !conversation || state.showAll || (state.folding[m.id] ?? m.expanded)
            let who = m.fromName ?? m.fromAddr ?? ""
            let date = Format.geekMessageDate(m.dateReceived)
            if !unfolded {
                // The sender's domain last: short, and the part that tells
                // whether the mail is genuine. Without a name, the address
                // is already shown.
                let domain = m.fromName != nil ? m.fromAddr.flatMap(Self.domain).map { " · @" + $0 } ?? "" : ""
                out.append(.header(messageId: m.id, folded: true, title: "▸ \(who) · \(date)\(domain)", detail: snippet(m),
                                   files: m.attachments.count))
                continue
            }
            let recipients = Self.recipients(m, state)
            // The address after the date, unless it is the name already.
            let address = m.fromName != nil ? m.fromAddr.flatMap { $0.isEmpty ? nil : $0 } : nil
            out.append(.header(messageId: m.id, folded: false, title: "\(conversation ? "▾ " : "")\(who) · \(date)",
                               detail: recipients, address: address, alert: senderAlert(m), foldable: conversation,
                               book: book[m.id]))
            if m.onlyQuotes && !state.showAll {
                out.append(.note(messageId: m.id, text: L("only quotes earlier messages")))
            } else if m.noText && !state.showAll {
                // Nothing written, nothing quoted (an empty body, a
                // signature alone, files without a word): said so, rather
                // than "only quotes", which it does not. The core decides
                // (`no_text`, `clean.rs`), as it does for `only_quotes`.
                out.append(.note(messageId: m.id, text: L("no text")))
            }
            if m.segments.isEmpty, !m.noText {
                // Body not downloaded (an empty one has its note above).
                let text = m.bodyText ?? L("(body not downloaded)")
                out.append(.segment(messageId: m.id, index: 0, kind: "text", text: text, depth: 0))
            }
            for (j, s) in m.segments.enumerated() {
                if !s.hidden || state.showAll || state.revealed.contains("\(m.id):\(j)") {
                    out.append(.segment(messageId: m.id, index: j, kind: s.kind, text: s.text, depth: s.depth))
                } else {
                    out.append(.hidden(messageId: m.id, index: j, label: label(s)))
                }
            }
            if !m.attachments.isEmpty {
                out.append(.attachments(messageId: m.id, files: m.attachments))
            }
        }
        return out
    }

    /// The recipients line of an unfolded message ("To: Alice, Bob   Cc:
    /// Carol"), or nil when not worth showing: the core decides
    /// (`show_recipients`: they changed since the previous message, or the
    /// user wrote the mail), `T` shows them always. Shared by the text and
    /// the HTML readers, so both show the same header (`ui.md`,
    /// "Expéditeur": "En texte comme en HTML").
    static func recipients(_ m: MessageView, _ state: State) -> String? {
        guard state.showAll || m.showRecipients else { return nil }
        var parts: [String] = []
        if !m.to.isEmpty { parts.append(L("To: %@", m.to.map(\.display).joined(separator: ", "))) }
        if !m.cc.isEmpty { parts.append(L("Cc: %@", m.cc.map(\.display).joined(separator: ", "))) }
        return parts.isEmpty ? nil : parts.joined(separator: "   ")
    }

    /// One line describing a hidden segment.
    static func label(_ s: SegmentView) -> String {
        let lines = s.text.split(separator: "\n", omittingEmptySubsequences: false).count
        switch s.kind {
        case "quote_tail":
            return s.inThread
                ? L("··· %lld quoted lines", lines)
                : L("··· %lld quoted lines, not in this conversation", lines)
        case "signature": return L("··· signature")
        case "copy": return L("··· copy of an earlier message")
        default: return "··· " + (s.text.split(separator: "\n").first.map(String.init) ?? "")
        }
    }

    /// Beginning of what a folded message brings, or a note when it brings
    /// nothing new.
    static func snippet(_ m: MessageView) -> String {
        if m.onlyQuotes { return L("only quotes earlier messages") }
        if m.noText { return L("no text") }
        let text = m.segments.first { !$0.hidden && ($0.kind == "text" || $0.kind == "forward") }?.text ?? m.bodyText ?? ""
        let flat = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return String(flat.prefix(120))
    }

    /// The domain of an address: what follows its last `@`, in lowercase;
    /// nil without one.
    static func domain(_ address: String) -> String? {
        guard let at = address.lastIndex(of: "@") else { return nil }
        let d = address[address.index(after: at)...].lowercased()
        return d.isEmpty ? nil : d
    }

    /// Whether the sender's address deserves a second look, shown in the
    /// alert colour:
    /// - the mail failed its authentication (`auth_result`): anybody may
    ///   have written in this name;
    /// - its name shows another address or domain than the real one
    ///   (`"service@paypal.com" <x7@promo-mail.ru>`, "PayPal.com"): the
    ///   usual way to pass for someone else.
    ///
    /// A name's domain matches when the real domain ends with it ("Le
    /// Monde.fr" from `news.lemonde.fr`).
    static func senderAlert(_ m: MessageView) -> Bool {
        if m.authResult == "fail" { return true }
        guard let name = m.fromName, let real = m.fromAddr.flatMap(domain) else { return false }
        return namedDomains(name).contains { !real.hasSuffix($0) }
    }

    /// The domains a sender's name shows: those of the addresses in it,
    /// and words such as "paypal.com" (letters, a dot, a final part of two
    /// letters or more).
    static func namedDomains(_ name: String) -> [String] {
        name.lowercased()
            .split(whereSeparator: { $0.isWhitespace || "<>()\"'[],;:".contains($0) })
            .compactMap { word -> String? in
                let w = word.hasSuffix(".") ? String(word.dropLast()) : String(word)
                if let d = domain(w), d.contains(".") { return d }
                let parts = w.split(separator: ".", omittingEmptySubsequences: false)
                guard parts.count >= 2, parts.allSatisfy({ !$0.isEmpty && $0.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" } }),
                      let last = parts.last, last.count >= 2, last.allSatisfy(\.isLetter) else { return nil }
                return w
            }
    }

    /// Number of folded messages (UI script state, tests).
    static func foldedCount(_ blocks: [Block]) -> Int {
        blocks.filter { if case .header(_, true, _, _, _, _, _, _, _) = $0 { return true } else { return false } }.count
    }

    /// `S` (like mutt's "skip quoted"): from the scroll position `y`, the
    /// position of the first text that follows the next quoted block.
    /// `quotes` are the (start, end) positions of quoted blocks, `texts` the
    /// starts of the other blocks, both sorted. `nil` when there is none.
    static func skipTarget(from y: Double, quotes: [(Double, Double)], texts: [Double]) -> Double? {
        // The next quote that ends below the top of the view.
        guard let quote = quotes.first(where: { $0.1 > y + 1 }) else { return nil }
        return texts.first { $0 >= quote.1 - 0.5 } ?? quote.1
    }
}
