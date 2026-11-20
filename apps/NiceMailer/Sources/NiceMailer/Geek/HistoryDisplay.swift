import Foundation

/// What the History screen says of the audit log (`tracabilite.md`, section
/// 6; `docs/geek-ui.md`). The core gives stable codes (`HistoryGroup.kind`),
/// and the words are the app's, in the language of the interface. Pure
/// functions, tested without a window.
enum HistoryDisplay {
    /// A row of the screen: a group, or a line of an unfolded group.
    struct Row: Identifiable, Hashable {
        let group: HistoryGroup
        /// A line shown under its group.
        let isDetail: Bool
        /// A group whose lines are shown under it.
        let isUnfolded: Bool

        /// Unique among the rows: a group of one line and that line have the
        /// same id in the log.
        var id: String { "\(isDetail ? "line" : "group")-\(group.lastId)" }
    }

    /// The groups in order, the lines of each unfolded one under it.
    static func rows(_ groups: [HistoryGroup], unfolded: [Int: [HistoryGroup]]) -> [Row] {
        var rows: [Row] = []
        rows.reserveCapacity(groups.count)
        for group in groups {
            let lines = unfolded[group.lastId]
            rows.append(Row(group: group, isDetail: false, isUnfolded: lines != nil))
            for line in lines ?? [] { rows.append(Row(group: line, isDetail: true, isUnfolded: false)) }
        }
        return rows
    }

    /// Who did it, as the user says it (`tracabilite.md`, section 3).
    static func who(_ source: String) -> String {
        switch source {
        case "app": L("Me")
        case "cli", "tui": L("Terminal")
        case "core": "NiceMailer"
        case "mcp": L("AI agent")
        case "imap": L("Mail server")
        case "config_sync": L("Another Mac")
        case "web": "Web"
        default: source
        }
    }

    /// Who the screen shows the actions of (`history.filter_source`): one
    /// of the authors of the column "who", or everyone. The terminal is the
    /// CLI and the TUI together, as the column says them both "Terminal".
    /// The other sources (the mail server, another Mac, the web) are rare in
    /// the log and shown only with everyone.
    ///
    /// Swift note: `CaseIterable` gives `allCases`, the cases in the order
    /// written, which is the order of the answers of the choice.
    enum SourceFilter: String, CaseIterable {
        case me, nicemailer, terminal, agent, everyone

        /// The sources sent to the core (`list_history_groups`, `sources`);
        /// empty: every source.
        var sources: [String] {
            switch self {
            case .me: ["app"]
            case .nicemailer: ["core"]
            case .terminal: ["cli", "tui"]
            case .agent: ["mcp"]
            case .everyone: []
            }
        }

        /// The key of its answer in the choice: the first letter of the
        /// English word, `-` for "no filter", as on the other choices.
        var key: String {
            switch self {
            case .me: "m"
            case .nicemailer: "n"
            case .terminal: "t"
            case .agent: "a"
            case .everyone: "-"
            }
        }

        /// Its name, the same words as the column "who" (`who`).
        var label: String {
            switch self {
            case .everyone: L("everyone")
            default: HistoryDisplay.who(sources[0])
            }
        }
    }

    /// What the row is about: the subject of the mail, the address, the name.
    static func title(_ g: HistoryGroup) -> String {
        if g.entity == "config" { return L("Settings") }
        guard let title = g.title else { return L("Deleted item") }
        return title.isEmpty ? L("(no subject)") : title
    }

    /// A folder or a screening target: its name when it has a role, else
    /// the path the core gave.
    private static func place(_ value: String?, _ names: FolderNames) -> String {
        guard let value else { return "?" }
        return Role.names[value] == nil ? value : names.name(value)
    }

    /// What was done, in two columns: the verb, and what qualifies it
    /// ("Moved" · "to Archive", "Marked" · "as read"). Each is a text of its
    /// own, so that every language words both.
    struct Action: Equatable {
        let verb: String
        var detail = ""
    }

    /// What was done, in words, the folders named by `names`. A code this
    /// version does not know, from a newer core, is shown rather than left
    /// blank.
    static func action(_ g: HistoryGroup, names: FolderNames = .english) -> Action {
        let value = g.value ?? "?"
        switch g.kind {
        case "read": return Action(verb: L("Marked"), detail: L("as read"))
        case "unread": return Action(verb: L("Marked"), detail: L("as unread"))
        case "flagged": return Action(verb: L("Marked"), detail: L("with a flag"))
        case "unflagged": return Action(verb: L("Removed"), detail: L("the flag"))
        case "move": return Action(verb: L("Moved"), detail: L("to %@", place(g.value, names)))
        case "screen": return Action(verb: L("Sorted"), detail: L("into %@", place(g.value, names)))
        case "decision": return Action(verb: L("Filed"), detail: L("in %@", place(g.value, names)))
        case "decision_removed": return Action(verb: L("Removed"), detail: L("the decision"))
        case "tag_added": return Action(verb: L("Added"), detail: L("tag %@", value))
        case "tag_removed": return Action(verb: L("Removed"), detail: L("tag %@", value))
        case "send": return Action(verb: L("Sent"))
        case "delete": return Action(verb: L("Deleted"))
        case "revert": return Action(verb: L("Undone"), detail: L("after a server error"))
        case "add_member": return Action(verb: L("Added"), detail: L("member %@", value))
        case "remove_member": return Action(verb: L("Removed"), detail: L("member %@", value))
        case "create": return Action(verb: L("Created"))
        case "rename": return Action(verb: L("Renamed"))
        case "update": return Action(verb: L("Updated"))
        case "remove": return Action(verb: L("Removed"))
        case "enable": return Action(verb: L("Enabled"))
        case "disable": return Action(verb: L("Disabled"))
        case "import_mbox": return Action(verb: L("Imported"), detail: value)
        case "reorder": return Action(verb: L("Changed"), detail: L("display order"))
        case "set_password": return Action(verb: L("Saved"), detail: L("password"))
        case "reload": return Action(verb: L("Reloaded"), detail: L("settings"))
        case "role": return Action(verb: L("Changed"), detail: L("folder role"))
        case "initial_archive": return Action(verb: L("Archived"), detail: L("old mail"))
        case "sender_kind": return Action(verb: L("Changed"), detail: L("sender type"))
        case "unsubscribe": return Action(verb: L("Unsubscribed"))
        case "auth_policy": return Action(verb: L("Changed"), detail: L("authentication rule"))
        default: return Action(verb: g.kind.replacingOccurrences(of: "_", with: " "))
        }
    }
}
