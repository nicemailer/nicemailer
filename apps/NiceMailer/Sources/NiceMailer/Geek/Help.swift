import Foundation

// The help of the geek theme (`?`, `ui.md`): the keys, grouped by what they
// are for, and how the groups are laid out in columns.
//
// Everything here is pure (no view, no controller state), so the content
// and the layout are unit-tested without a window (`HelpTests`). The view
// (`HelpPanel`, `GeekViews.swift`) only draws what these functions decide.

/// One key of the help.
struct HelpEntry: Equatable {
    let key: String
    let label: String
    /// Announced but not available yet: drawn muted.
    var notYet = false
}

/// A block of the help: a title, then its keys.
struct HelpGroup: Equatable {
    let title: String
    let entries: [HelpEntry]

    /// Lines the group takes in a column: its title and one per key.
    var height: Int { entries.count + 1 }
}

enum Help {
    /// Characters between a key and its label.
    static let keyGap = 2
    /// Characters between two columns.
    static let columnGap = 4
    /// Most columns: beyond three, a column gets too short to read groups
    /// as blocks, and the panel too wide to take in at a glance.
    static let maxColumns = 3

    /// The keys, by group, in reading order: moving and reading first, then
    /// what one does to mails, then finding them, then the window. One key
    /// (or one pair of opposite keys) per line, so the eye never has to
    /// match "u  *" with "read / unread, flag".
    ///
    /// A computed property, not a constant: `L` reads the language the app
    /// runs in, which the UI scripts and the tests may change.
    static var groups: [HelpGroup] {
        [
            HelpGroup(title: L("Navigate"), entries: [
                HelpEntry(key: "j k", label: L("next / previous mail")),
                HelpEntry(key: "^d ^u", label: L("half a page down / up in the list")),
                HelpEntry(key: "^k ^j", label: L("previous / next message of the conversation")),
                HelpEntry(key: "esc", label: L("cancel, unmark, back")),
            ]),
            HelpGroup(title: L("Read"), entries: [
                HelpEntry(key: "ret", label: L("read the mail in the whole window (one column)")),
                HelpEntry(key: "space ^f", label: L("scroll the mail a page down")),
                HelpEntry(key: "⇧space ^b", label: L("scroll the mail a page up")),
                HelpEntry(key: "T", label: L("show / hide quotes, signatures, folded mails")),
                HelpEntry(key: "S", label: L("skip to the text after the next quote")),
                HelpEntry(key: "l", label: L("links of the mail")),
                HelpEntry(key: "^c", label: L("sender's card in Contacts, or copy address")),
            ]),
            HelpGroup(title: L("Select"), entries: [
                HelpEntry(key: "J K", label: L("mark and move (multiple selection)")),
                HelpEntry(key: "⇧↓ ⇧↑", label: L("the same, with the arrows")),
                HelpEntry(key: "⌘A", label: L("mark every mail of the list")),
            ]),
            HelpGroup(title: L("Act on mails"), entries: [
                HelpEntry(key: "d", label: L("done (archive)")),
                HelpEntry(key: "m…", label: L("move to a folder")),
                // Quarantine drops the prefix (`GeekController.directTargets`).
                HelpEntry(key: "i n p x b d", label: L("in Quarantine: move without m")),
                HelpEntry(key: "del", label: L("delete")),
                HelpEntry(key: "u", label: L("read / unread")),
                HelpEntry(key: "U", label: L("mark as read")),
                HelpEntry(key: "*", label: L("flag")),
                HelpEntry(key: "t", label: L("add / remove a tag")),
                HelpEntry(key: "g", label: L("add / remove a group")),
                HelpEntry(key: "s…", label: L("save")),
                HelpEntry(key: "z", label: L("undo the last move or decision")),
                HelpEntry(key: "⌘U", label: L("unsubscribe from the list")),
            ]),
            HelpGroup(title: L("Write"), entries: [
                HelpEntry(key: "r", label: L("reply to all")),
                HelpEntry(key: "R", label: L("reply to the sender only")),
                HelpEntry(key: "F", label: L("forward")),
                HelpEntry(key: "c", label: L("write a new mail")),
                HelpEntry(key: "⌃I", label: L("in a reply: AI menu (proofread, rewrite, translate, subjects)")),
                HelpEntry(key: "⇧⌘T", label: L("in a reply: text only or formatted text")),
                HelpEntry(key: "a", label: L("AI menu of the mail: summary, translation, quick replies, security")),
            ]),
            HelpGroup(title: L("Find and go"), entries: [
                HelpEntry(key: "/", label: L("search everywhere, in All Mail")),
                HelpEntry(key: "f", label: L("filter by attributes")),
                HelpEntry(key: "^C", label: L("mails of the sender")),
                HelpEntry(key: "@", label: L("go to an account")),
                HelpEntry(key: ";…", label: L("go to a folder or a screen")),
            ]),
            HelpGroup(title: L("Window"), entries: [
                HelpEntry(key: "v…", label: L("display")),
                HelpEntry(key: "⌘+ ⌘− ⌘0", label: L("text size: bigger, smaller, default")),
                HelpEntry(key: "⌃⌘F", label: L("full screen")),
                HelpEntry(key: "?", label: L("this list of keys")),
            ]),
        ]
    }

    /// Height of a column, in lines: its groups and a blank line between
    /// two of them.
    static func height(_ column: [HelpGroup]) -> Int {
        column.map(\.height).reduce(0, +) + max(0, column.count - 1)
    }

    /// Width of the key column of a column, in characters: its longest
    /// key. Shared by the column's groups, so that their labels line up.
    static func keyWidth(_ column: [HelpGroup]) -> Int {
        column.flatMap(\.entries).map(\.key.count).max() ?? 0
    }

    /// Width of a column, in characters: key, the gap after it, the longest
    /// label (or a title, if longer).
    static func width(_ column: [HelpGroup]) -> Int {
        let labels = column.flatMap(\.entries).map(\.label.count).max() ?? 0
        let titles = column.map(\.title.count).max() ?? 0
        return max(titles, keyWidth(column) + keyGap + labels)
    }

    /// Width of the whole layout, in characters.
    static func width(_ columns: [[HelpGroup]]) -> Int {
        columns.map(width).reduce(0, +) + columnGap * max(0, columns.count - 1)
    }

    /// Splits `groups` into at most `count` columns, keeping their order
    /// (read down the first column, then the next) and never cutting a
    /// group, so that the tallest column is as short as possible: the panel
    /// is then as short as it can be.
    ///
    /// Every way of cutting the list is tried. With 7 groups and 3 columns
    /// that is 15 ways (cutting points chosen among 6 places): nothing to
    /// optimise. On a tie, the later cut wins (cuts are tried from the
    /// earliest), so the first columns are the fuller ones, as in a
    /// newspaper.
    static func columns(_ groups: [HelpGroup], count: Int) -> [[HelpGroup]] {
        let k = max(1, min(count, groups.count))
        guard k > 1 else { return groups.isEmpty ? [] : [groups] }
        var best: [[HelpGroup]] = [groups]
        var bestHeight = height(groups)
        // Swift note: a nested function can call itself and read the
        // variables around it (`best`, `bestHeight`): a closure that
        // captures them.
        func split(from start: Int, columnsLeft: Int, built: [[HelpGroup]]) {
            if columnsLeft == 1 {
                let all = built + [Array(groups[start...])]
                let h = all.map(height).max() ?? 0
                if h <= bestHeight { best = all; bestHeight = h }
                return
            }
            // Leave at least one group for each of the columns after this one.
            for end in (start + 1)...(groups.count - columnsLeft + 1) {
                split(from: end, columnsLeft: columnsLeft - 1, built: built + [Array(groups[start..<end])])
            }
        }
        split(from: 0, columnsLeft: k, built: [])
        return best
    }

    /// The most columns (up to `maxColumns`) whose layout fits in
    /// `available` points, for characters `charWidth` points wide and a
    /// panel margin of `padding` points on each side. A narrow window gets
    /// two columns, then one; one column is used even when it does not fit
    /// (the text is then cut, rather than the panel lost).
    static func columnCount(_ groups: [HelpGroup], available: CGFloat, charWidth: CGFloat, padding: CGFloat) -> Int {
        for count in stride(from: maxColumns, to: 1, by: -1) {
            let chars = width(columns(groups, count: count))
            if CGFloat(chars) * charWidth + 2 * padding <= available { return count }
        }
        return 1
    }
}
