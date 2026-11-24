import Foundation

// The help of the geek theme (`?`, `ui.md`): the keys, grouped by what they
// are for, and how the groups are laid out in columns. The keys are those
// of the keymap in effect (`Keymap`, `keymap.toml`).
//
// Everything here is pure (no view, no controller state), so the content
// and the layout are unit-tested without a window (`HelpTests`). The view
// (`HelpPanel`, `InterfaceViews.swift`) only draws what these functions decide.

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
    /// The keys come from `keymap` (`keymap.toml`, `docs/keymap.md`): a line
    /// names its actions, and shows the keys they have in `chain`, the mail
    /// screen. A line whose actions have no key is left out, so the help
    /// never shows a key that does nothing.
    ///
    /// A function, not a constant: `L` reads the language the app runs in,
    /// which the UI scripts and the tests may change, and the keymap may
    /// change while the app runs.
    ///
    /// `severalAccounts` false (a single account) leaves out the account
    /// picker, which then does nothing.
    static func groups(_ keymap: Keymap = .defaults,
                       chain: [KeyContext] = [.mail, .normal, .everywhere],
                       severalAccounts: Bool = true) -> [HelpGroup] {
        let compose: [KeyContext] = [.compose, .everywhere]
        // Swift note: a nested function, used as a shorthand below.
        func line(_ actions: [KeyAction], _ label: String, arrows: Arrows = .yes, suffix: String = "",
                  in c: [KeyContext]? = nil) -> HelpEntry? {
            let key = keys(keymap, actions, in: c ?? chain, arrows: arrows)
            return key.isEmpty ? nil : HelpEntry(key: key + suffix, label: label)
        }
        // The direct moves of Quarantine: its own keys, in file order.
        let quarantine = (keymap.contexts[.quarantine] ?? [])
            .filter { KeyAction(rawValue: $0.action)?.moveRole != nil }
            .map { KeyChord.display($0.key) }
            .joined(separator: " ")
        let move = keymap.label(.menuMove, in: chain)
        let groups: [(String, [HelpEntry?])] = [
            (L("Navigate"), [
                line([.listDown, .listUp], L("next / previous mail"), arrows: .no),
                line([.listHalfPageDown, .listHalfPageUp], L("half a page down / up in the list")),
                line([.messagePrevious, .messageNext], L("previous / next message of the conversation")),
                // Escape is not in the keymap: it always goes back.
                HelpEntry(key: "esc", label: L("cancel, unmark, back")),
            ]),
            (L("Read"), [
                line([.mailOpen], L("read the mail in the whole window (one column)")),
                line([.readerOpenOrPageDown, .readerPageDown], L("scroll the mail a page down")),
                line([.readerPageUp], L("scroll the mail a page up")),
                line([.readerToggleAll], L("show / hide quotes, signatures, folded mails")),
                line([.readerSkipQuoted], L("skip to the text after the next quote")),
                line([.pickerLinks], L("links of the mail")),
                line([.senderCardOrCopy], L("sender's card in Contacts, or copy address")),
            ]),
            (L("Select"), [
                line([.listMarkDown, .listMarkUp], L("mark and move (multiple selection)"), arrows: .no),
                line([.listMarkDown, .listMarkUp], L("the same, with the arrows"), arrows: .only),
                line([.listMarkAll], L("mark every mail of the list")),
            ]),
            (L("Act on mails"), [
                line([.mailArchive], L("done (archive)")),
                line([.menuMove], L("move to a folder"), suffix: "…"),
                quarantine.isEmpty ? nil
                    : HelpEntry(key: quarantine, label: move.isEmpty ? L("in Quarantine: move")
                        : L("in Quarantine: move without %@", move)),
                line([.mailDelete], L("delete")),
                line([.mailToggleSeen], L("read / unread")),
                line([.mailMarkSeen], L("mark as read")),
                line([.mailToggleFlag], L("flag")),
                line([.pickerTag], L("add / remove a tag")),
                line([.pickerGroup], L("add / remove a group")),
                line([.menuSave], L("save"), suffix: "…"),
                line([.undoLast], L("undo the last move or decision")),
                line([.mailUnsubscribe], L("unsubscribe from the list")),
            ]),
            (L("Write"), [
                line([.composeReplyAll], L("reply to all")),
                line([.composeReplySender], L("reply to the sender only")),
                line([.composeForward], L("forward")),
                line([.composeNew], L("write a new mail")),
                line([.aiComposeMenu], L("in a reply: AI menu (proofread, rewrite, translate, subjects)"), in: compose),
                line([.composeTogglePlain], L("in a reply: text only or formatted text"), in: compose),
                line([.aiMenu], L("AI menu of the mail: summary, translation, quick replies, security")),
            ]),
            (L("Find and go"), [
                line([.pickerSearch], L("search everywhere, in All Mail")),
                line([.pickerAttributes], L("filter by attributes")),
                line([.senderMails], L("mails of the sender")),
                severalAccounts ? line([.pickerAccounts], L("go to an account")) : nil,
                line([.menuGoto], L("go to a folder or a screen"), suffix: "…"),
            ]),
            (L("Window"), [
                line([.menuView], L("show"), suffix: "…"),
                line([.textBigger, .textSmaller, .textReset], L("text size: bigger, smaller, default")),
                line([.windowFullScreen], L("full screen")),
                line([.helpShow], L("this list of keys")),
            ]),
        ]
        // The menus of the user: their key, and their name.
        let menus: [HelpEntry] = keymap.menus.filter { !$0.builtin }.compactMap { m in
            let key = keymap.label(raw: "menu." + m.name, in: chain)
            return key.isEmpty ? nil : HelpEntry(key: key + "…", label: m.name)
        }
        var out = groups.map { HelpGroup(title: $0.0, entries: $0.1.compactMap { $0 }) }
        if !menus.isEmpty { out.append(HelpGroup(title: L("Your menus"), entries: menus)) }
        return out.filter { !$0.entries.isEmpty }
    }

    /// Whether a line of the help shows the arrow keys of its actions.
    enum Arrows {
        /// Yes, with the other keys.
        case yes
        /// Not when the action has another key: `j k`, not `j ↓ k ↑` (the
        /// arrows go without saying, or have a line of their own).
        case no
        /// Only them: "⇧↓ ⇧↑, the same, with the arrows".
        case only
    }

    /// The keys a line shows, as text: for one action, all its keys
    /// (`⇧space ⌃B`); for several (a pair of opposite moves), the first key
    /// of each (`j k`). Empty when none has a key.
    static func keys(_ keymap: Keymap, _ actions: [KeyAction], in chain: [KeyContext], arrows: Arrows = .yes) -> String {
        let perAction: [[String]] = actions.map { a in
            let all = keymap.keys(a, in: chain)
            switch arrows {
            case .yes: return all
            case .only: return all.filter(KeyChord.isArrow)
            case .no:
                let other = all.filter { !KeyChord.isArrow($0) }
                return other.isEmpty ? all : other
            }
        }
        let shown = actions.count == 1 ? perAction[0] : perAction.compactMap(\.first)
        return shown.map(KeyChord.display).joined(separator: " ")
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
        columnCount(groups, available: available, measure: .fixed(char: charWidth), padding: padding)
    }

    /// The same, with the texts measured in a font (`TextMeasure`): a
    /// proportional font gives each key and label its own width, the group
    /// titles in bold. The gaps stay counted in characters.
    static func columnCount(_ groups: [HelpGroup], available: CGFloat, measure: TextMeasure, padding: CGFloat) -> Int {
        for count in stride(from: maxColumns, to: 1, by: -1) {
            if width(columns(groups, count: count), measure) + 2 * padding <= available { return count }
        }
        return 1
    }

    /// Width of the key column of `column`, in points.
    static func keyWidth(_ column: [HelpGroup], _ m: TextMeasure) -> CGFloat {
        if m.fixedPitch { return CGFloat(keyWidth(column)) * m.char }
        return column.flatMap(\.entries).map { m.width($0.key) }.max() ?? 0
    }

    /// Width of a column, in points: as `width(_:)` in characters.
    static func width(_ column: [HelpGroup], _ m: TextMeasure) -> CGFloat {
        if m.fixedPitch { return CGFloat(width(column)) * m.char }
        let labels = column.flatMap(\.entries).map { m.width($0.label) }.max() ?? 0
        let titles = column.map { m.width($0.title, bold: true) }.max() ?? 0
        return max(titles, keyWidth(column, m) + CGFloat(keyGap) * m.char + labels)
    }

    /// Width of the whole layout, in points.
    static func width(_ columns: [[HelpGroup]], _ m: TextMeasure) -> CGFloat {
        if m.fixedPitch { return CGFloat(width(columns)) * m.char }
        return columns.map { width($0, m) }.reduce(0, +) + CGFloat(columnGap) * m.char * CGFloat(max(0, columns.count - 1))
    }
}
