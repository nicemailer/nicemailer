import Foundation

/// How a picker row matches what was typed (`ui.md`, "Sélecteur
/// d'attributs" and the pickers of the geek theme): a rank, lower is
/// better, or `nil` when the row does not match and is hidden.
///
/// Why ranks rather than a plain filter: pickers are driven by the
/// keyboard, and Return takes the first row. With a plain "contains"
/// filter in the order of the list, typing `read` in the attribute
/// selector left "unread" first (it contains "read" and is listed before
/// "read"), so Return chose the opposite of what was typed. Ranking makes
/// the row that *starts* with the typed text come first, and lets a few
/// letters reach any row: initials (`nf` for "not flagged") and hidden
/// search words (`cc` for "To…").
///
/// Pure function, tested without any view
/// (`Tests/NiceMailerTests/PickerMatchTests.swift`).
enum PickerMatch {
    /// Ranks, from the best match to the loosest one.
    enum Rank: Int, Comparable {
        /// The label (or a search word) starts with the typed text:
        /// `read` → "read", `de` → "De…".
        case labelPrefix = 0
        /// Each typed word starts a word of the label or a search word, in
        /// any order: `lus` → "non lus", `jointe sans` → "sans pièce jointe".
        case wordPrefix
        /// The typed letters start the initials of the label's words:
        /// `nf` → "not flagged", `spj` → "sans pièce jointe".
        case initials
        /// Each typed word appears somewhere in the label or a search word,
        /// in any order (the former filter, kept so that nothing found
        /// before is lost): `lagg` → "flagged".
        case substring

        // Swift note: `Comparable` needs `<`; comparing the raw values makes
        // the order of the cases the order of the ranks.
        static func < (a: Rank, b: Rank) -> Bool { a.rawValue < b.rawValue }
    }

    /// The rank of a row with `label` and hidden search `keywords` for
    /// `query`, ignoring case and accents (`a` finds "À…"). An empty query
    /// matches every row at the best rank, which keeps the list's order.
    static func rank(_ query: String, label: String, keywords: [String] = []) -> Rank? {
        let q = normalize(query).trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return .labelPrefix }
        let typed = q.split(separator: " ").map(String.init)
        let text = normalize(label)
        let terms = keywords.map(normalize)
        let labelWords = words(text)
        let allWords = labelWords + terms.flatMap(words)

        if text.hasPrefix(q) || terms.contains(where: { $0.hasPrefix(q) }) { return .labelPrefix }
        if typed.allSatisfy({ t in allWords.contains { $0.hasPrefix(t) } }) { return .wordPrefix }
        // Initials only for a single typed word of two letters or more: one
        // letter is already covered by the prefixes, and several words mean
        // the user is spelling words, not initials.
        if typed.count == 1, q.count >= 2, String(labelWords.compactMap(\.first)).hasPrefix(q) { return .initials }
        if typed.allSatisfy({ t in text.contains(t) || terms.contains { $0.contains(t) } }) { return .substring }
        return nil
    }

    /// The rows of `entries` matching `query`, best ranks first; rows of
    /// the same rank keep their order in the list. `pinned` rows (the
    /// "+ typed text" row of the tag and group pickers) always stay, first.
    ///
    /// Swift note: `sort` is not stable (equal elements may swap), so the
    /// index in the list is part of the sort key.
    static func filter<T>(_ entries: [T], query: String, label: (T) -> String,
                          keywords: (T) -> [String] = { _ in [] }, pinned: (T) -> Bool = { _ in false }) -> [T] {
        let ranked: [(rank: Int, index: Int, entry: T)] = entries.enumerated().compactMap { i, e in
            if pinned(e) { return (-1, i, e) }
            guard let r = rank(query, label: label(e), keywords: keywords(e)) else { return nil }
            return (r.rawValue, i, e)
        }
        return ranked.sorted { ($0.rank, $0.index) < ($1.rank, $1.index) }.map(\.entry)
    }

    /// Lower case, without accents: "À…" → "a…".
    static func normalize(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
    }

    /// The words of a normalized text: runs of letters and digits ("De…" →
    /// ["de"], "Alice <alice@x.fr>" → ["alice", "alice", "x", "fr"]).
    static func words(_ s: String) -> [String] {
        s.split { !$0.isLetter && !$0.isNumber }.map(String.init)
    }
}
