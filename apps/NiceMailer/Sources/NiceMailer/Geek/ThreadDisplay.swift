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
    }

    enum Block: Hashable {
        /// A message header. `folded`: one line with a snippet; otherwise
        /// the sender and date, then the recipients if worth showing.
        case header(messageId: Int, folded: Bool, title: String, detail: String?)
        /// A visible segment.
        case segment(messageId: Int, index: Int, kind: String, text: String, depth: Int)
        /// A hidden segment, replaced by a "···" line that opens it.
        case hidden(messageId: Int, index: Int, label: String)
        /// A remark on an unfolded message (it only quotes earlier ones).
        case note(messageId: Int, text: String)
        /// Separation between two messages.
        case separator
    }

    static func blocks(_ thread: ThreadView, _ state: State) -> [Block] {
        var out: [Block] = []
        for (i, m) in thread.messages.enumerated() {
            if i > 0 { out.append(.separator) }
            let unfolded = state.showAll || (state.folding[m.id] ?? m.expanded)
            let who = m.fromName ?? m.fromAddr ?? ""
            let date = Format.geekMessageDate(m.dateReceived)
            if !unfolded {
                out.append(.header(messageId: m.id, folded: true, title: "▸ \(who) · \(date)", detail: snippet(m)))
                continue
            }
            var recipients: String?
            if state.showAll || m.showRecipients {
                var parts: [String] = []
                if !m.to.isEmpty { parts.append(L("To: %@", m.to.map(\.display).joined(separator: ", "))) }
                if !m.cc.isEmpty { parts.append(L("Cc: %@", m.cc.map(\.display).joined(separator: ", "))) }
                recipients = parts.isEmpty ? nil : parts.joined(separator: "   ")
            }
            out.append(.header(messageId: m.id, folded: false, title: "▾ \(who) · \(date)", detail: recipients))
            if m.onlyQuotes && !state.showAll {
                out.append(.note(messageId: m.id, text: L("only quotes earlier messages")))
            }
            if m.segments.isEmpty {
                // Body not downloaded, or an empty message.
                let text = m.bodyText ?? L("(body not downloaded)")
                out.append(.segment(messageId: m.id, index: 0, kind: "text", text: text, depth: 0))
                continue
            }
            for (j, s) in m.segments.enumerated() {
                if !s.hidden || state.showAll || state.revealed.contains("\(m.id):\(j)") {
                    out.append(.segment(messageId: m.id, index: j, kind: s.kind, text: s.text, depth: s.depth))
                } else {
                    out.append(.hidden(messageId: m.id, index: j, label: label(s)))
                }
            }
        }
        return out
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
        let text = m.segments.first { !$0.hidden && ($0.kind == "text" || $0.kind == "forward") }?.text ?? m.bodyText ?? ""
        let flat = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return String(flat.prefix(120))
    }

    /// Number of folded messages (UI script state, tests).
    static func foldedCount(_ blocks: [Block]) -> Int {
        blocks.filter { if case .header(_, true, _, _) = $0 { return true } else { return false } }.count
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
