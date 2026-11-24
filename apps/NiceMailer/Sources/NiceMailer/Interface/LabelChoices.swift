import Foundation

/// A tag or a contact group: the two kinds of label the user names, adds
/// with `t` and `g`, and renames or deletes on their screens (`ui.md`,
/// "Écrans Tags et Groupes").
enum LabelKind: Equatable {
    case tag, group
}

/// The rows of the `t` (tag) and `g` (group) pickers: which names can be
/// added to the mails aimed at, and which removed (`ui.md`, "Ajouter et
/// retirer un tag ou un groupe").
///
/// A selection can mix mails with a tag and mails without it, so one row
/// cannot both add and remove: the picker offers each action on its own
/// row, "remove VIP" apart from "VIP". A name some mails have gets a
/// removal row; a name not every mail has gets an addition row; a name
/// held by some mails only gets both. With a single mail, that is simply
/// its tags (or its person's groups) to remove, and the others to add.
///
/// Before (November 2026), `t` on a tag every mail had removed it, and
/// `g` only added: nothing said which choice removed, and a mixed
/// selection could not lose a tag at all.
///
/// A pure function of plain values, tested alone (`LabelChoicesTests`).
enum LabelChoices {
    struct Row: Equatable {
        let name: String
        /// The row removes the name from the mails that have it.
        let remove: Bool
        /// How many of the mails aimed at have the name.
        let held: Int
    }

    /// Removal rows first, then addition rows.
    ///
    /// - `names`: the names that exist (tags of the account, groups), in
    ///   display order.
    /// - `held`: for each mail aimed at, the names it has. A name a mail
    ///   has but `names` lacks (a tag of another account in "all") can
    ///   still be removed; it comes after the others, sorted.
    ///
    /// Names compare without case, as the core does: "vip" and "VIP" are
    /// one group (`groupes.md`, `name`).
    static func rows(names: [String], held: [[String]]) -> [Row] {
        func key(_ s: String) -> String { s.lowercased() }
        // How many mails have each name, each mail counted once.
        var count: [String: Int] = [:]
        // The spelling shown for a name only the mails know.
        var spelling: [String: String] = [:]
        for mail in held {
            for name in Set(mail.map(key)) { count[name, default: 0] += 1 }
            for name in mail where spelling[key(name)] == nil { spelling[key(name)] = name }
        }
        let known = Set(names.map(key))
        let extra = spelling.filter { !known.contains($0.key) }.map(\.value)
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
        var removals: [Row] = []
        var additions: [Row] = []
        var seen = Set<String>()
        // Swift note: `where` filters the loop; `insert` returns whether
        // the element was new, which drops a name listed twice.
        for name in names + extra where seen.insert(key(name)).inserted {
            let n = count[key(name)] ?? 0
            if n > 0 { removals.append(Row(name: name, remove: true, held: n)) }
            // `extra` names are known only from the mails: no addition row,
            // the picker offers what exists (a typed name creates one).
            if n < max(held.count, 1), known.contains(key(name)) { additions.append(Row(name: name, remove: false, held: n)) }
        }
        return removals + additions
    }

    /// The tooltip of the labels in the header (`ReaderTitleBar`): what
    /// they are, and that `t` and `g` change them.
    static func headerHelp(tags: [String], groups: [String]) -> String {
        var parts: [String] = []
        if !tags.isEmpty { parts.append(L("Tags: %@ (t to change)", tags.joined(separator: ", "))) }
        if !groups.isEmpty { parts.append(L("Groups of the sender: %@ (g to change)", groups.joined(separator: ", "))) }
        return parts.joined(separator: "\n")
    }
}
