import AppKit
import Foundation
import Observation

/// State and keyboard handling of the geek theme (`ui.md`).
///
/// The keyboard is read with an AppKit event monitor rather than SwiftUI
/// focus: every key reaches `handle(_:)` before any view, whatever has the
/// focus, and nothing waits for an animation. Text typed in a picker or in
/// the search is edited here too (no `TextField`), so a key can never be
/// "eaten" by a field that holds the focus.
///
/// Key model, as in Helix and Vim: in the normal mode a key is an action
/// (`d`, `r`…) or a *prefix* (`m`, `;`, `s`, `v`) that waits for a second
/// key and shows a menu of what it can be.
@MainActor
@Observable
final class GeekController {
    // MARK: - Types

    /// A prefix key and the menu it opens.
    enum Prefix: String {
        case move = "m", goto = ";", save = "s", view = "v"
    }

    /// Full-window screens reached with `;` (`ui.md`, "changer d'écran").
    enum Screen: Equatable {
        case mail, contacts, history, tags, groups
    }

    /// Filterable lists in a popup (Helix's "pickers").
    enum PickerKind {
        /// `h`: go to a folder.
        case folders
        /// `l`: open a link of the mail.
        case links
        /// `t`: tag the threads (existing tag, or a new one typed).
        case tag
        /// `g`: add the senders to a group (existing, or new).
        case group
    }

    /// What currently receives the keys, besides the normal mode.
    enum Overlay: Equatable {
        case none
        /// A prefix waits for its second key; its menu is shown.
        case menu(Prefix)
        /// `?`: the list of keys.
        case help
        /// A picker, with the text typed and the highlighted row.
        case picker(PickerKind)
        /// `/`: the search query is being typed.
        case search
        /// `/` on the contacts screen: the filter is being typed.
        case contactFilter
    }

    /// One row of a menu or a picker.
    struct Entry: Identifiable {
        let key: String
        let label: String
        /// Grayed out: shown for discoverability, not available yet.
        var disabled = false
        let action: @MainActor () -> Void
        var id: String { key + label }
    }

    /// Where the status bar is (`ui.md`: bottom, top or hidden).
    enum StatusPosition: String, CaseIterable {
        case bottom, top, hidden
    }

    // MARK: - State

    let model: AppModel
    var screen: Screen = .mail
    var overlay: Overlay = .none
    /// Text typed in the open picker, and the highlighted row.
    var pickerQuery = ""
    var pickerIndex = 0
    /// Text of the search being typed; results are in `model.searchResults`.
    var searchQuery = ""
    /// Row of the search results under the cursor.
    var searchIndex = 0

    /// Number of columns (`ui.md`, display menu): 1 = list only,
    /// 2 = list and mail, 3 = folders, list and mail.
    var columns: Int = UserDefaults.standard.object(forKey: "geek.columns") as? Int ?? 2 {
        didSet { UserDefaults.standard.set(columns, forKey: "geek.columns") }
    }
    var statusPosition: StatusPosition =
        StatusPosition(rawValue: UserDefaults.standard.string(forKey: "geek.status") ?? "") ?? .bottom {
        didSet { UserDefaults.standard.set(statusPosition.rawValue, forKey: "geek.status") }
    }
    /// Width of the mail list, as a fraction of the window ("un petit
    /// tiers", resizable by dragging the separator).
    var listFraction: Double = UserDefaults.standard.object(forKey: "geek.listFraction") as? Double ?? 0.32 {
        didSet { UserDefaults.standard.set(listFraction, forKey: "geek.listFraction") }
    }

    // Data of the screens, loaded when a screen opens.
    var contacts: [ContactView] = []
    var contactsCursor: String?
    var contactsQuery = ""
    var history: [HistoryEntry] = []
    var historyCursor: String?
    var tags: [TagView] = []
    var groups: [GroupView] = []
    /// Row under the cursor on the current screen.
    var screenIndex = 0

    /// The reading pane's scroll view, to scroll it from the keyboard.
    weak var readerScroll: NSScrollView?

    init(model: AppModel) {
        self.model = model
    }

    // MARK: - Labels

    /// Short mode name for the status bar, like Helix's "NOR".
    var modeLabel: String {
        switch overlay {
        case .none, .help: "NOR"
        case .menu(let p): p.rawValue
        case .picker: "PCK"
        case .search, .contactFilter: "/"
        }
    }

    /// "account · folder", or the tag, group or contact listed.
    var locationLabel: String {
        switch screen {
        case .contacts: return L("Contacts")
        case .history: return L("History")
        case .tags: return L("Tags")
        case .groups: return L("Groups")
        case .mail: break
        }
        if !model.searchText.isEmpty { return L("search: %@", model.searchText) }
        switch model.listFilter {
        case .tag(let t): return L("tag: %@", t.name)
        case .group(let g): return L("group: %@", g.name)
        case .contact(let c): return L("contact: %@", c.addr)
        case nil:
            guard let f = model.selectedFolder else { return "" }
            return "\(f.accountName) · \(f.name)"
        }
    }

    // MARK: - Key handling

    /// Handles one key press. Returns `true` if the key was used (it then
    /// goes no further), `false` to let AppKit process it (⌘ shortcuts).
    func handle(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        // ⌘ shortcuts belong to the menus (⌘Q, ⌘R…).
        if mods.contains(.command) { return false }
        let control = mods.contains(.control)
        let key = (control ? event.charactersIgnoringModifiers : event.characters) ?? ""
        let code = event.keyCode

        // Any key clears the previous message, as in Helix.
        model.flash = nil
        model.lastError = nil

        switch overlay {
        case .menu(let prefix):
            overlay = .none
            if code == KeyCode.escape { return true }
            if let entry = menuEntries(prefix).first(where: { $0.key == key }) {
                if entry.disabled {
                    model.flash = L("Not available yet: %@", entry.label)
                } else {
                    entry.action()
                }
            } else {
                model.flash = L("No command %@%@", prefix.rawValue, key)
            }
            return true
        case .help:
            overlay = .none
            return true
        case .picker(let kind):
            handlePickerKey(kind, key: key, code: code, control: control)
            return true
        case .search:
            handleSearchInput(key: key, code: code)
            return true
        case .contactFilter:
            handleContactFilterInput(key: key, code: code)
            return true
        case .none:
            break
        }

        if control {
            switch key {
            case "d": scrollReader(pages: 0.5)
            case "u": scrollReader(pages: -0.5)
            case "f": scrollReader(pages: 1)
            case "b": scrollReader(pages: -1)
            default: return false
            }
            return true
        }

        // Keys common to every screen.
        switch key {
        case ";":
            overlay = .menu(.goto)
            return true
        case "?":
            overlay = .help
            return true
        default:
            break
        }
        if code == KeyCode.escape, screen != .mail {
            screen = .mail
            return true
        }
        if screen != .mail {
            handleScreenKey(key: key, code: code)
            return true
        }
        handleMailKey(key: key, code: code)
        return true
    }

    /// Normal mode on the mail screen.
    private func handleMailKey(key: String, code: UInt16) {
        let searching = !model.searchText.isEmpty
        switch code {
        case KeyCode.escape:
            if searching {
                model.searchText = ""
            } else if !model.marked.isEmpty {
                model.marked = []
            } else if model.listFilter != nil, let f = model.selectedFolderId {
                // Back from a tag, group or contact list to the folder.
                model.selectedFolderId = nil
                model.selectedFolderId = f
            }
            return
        case KeyCode.returnKey:
            if searching, model.searchResults.indices.contains(searchIndex) {
                model.openSearchHit(model.searchResults[searchIndex])
            }
            return
        case KeyCode.delete, KeyCode.forwardDelete:
            model.moveTargets(to: "deleted")
            return
        case KeyCode.down:
            moveCursor(1)
            return
        case KeyCode.up:
            moveCursor(-1)
            return
        default:
            break
        }
        switch key {
        case "j": moveCursor(1)
        case "k": moveCursor(-1)
        case "J": if !searching { model.markAndMove(1) }
        case "K": if !searching { model.markAndMove(-1) }
        case "h": openPicker(.folders)
        case "l": openPicker(.links)
        case " ": scrollReader(pages: 1)
        case "d": model.moveTargets(to: "archive")
        case "u": model.toggleSeenTargets()
        case "*": model.toggleFlagTargets()
        case "t": openPicker(.tag)
        case "g": openPicker(.group)
        case "/":
            searchQuery = model.searchText
            overlay = .search
        case "r", "R", "f":
            model.flash = L("Not available yet: writing mail needs SMTP")
        case "m", "s", "v":
            if let p = Prefix(rawValue: key) { overlay = .menu(p) }
        default:
            if !key.isEmpty { model.flash = L("No command %@", key) }
        }
    }

    private func moveCursor(_ delta: Int) {
        if !model.searchText.isEmpty {
            searchIndex = max(0, min(model.searchResults.count - 1, searchIndex + delta))
        } else {
            model.selectNext(delta)
        }
    }

    /// Scrolls the reading pane by a fraction of its height.
    private func scrollReader(pages: CGFloat) {
        guard let scroll = readerScroll, let doc = scroll.documentView else { return }
        let clip = scroll.contentView
        let maxY = max(0, doc.frame.height - clip.bounds.height)
        let y = min(maxY, max(0, clip.bounds.origin.y + pages * clip.bounds.height))
        clip.scroll(to: NSPoint(x: 0, y: y))
        scroll.reflectScrolledClipView(clip)
    }

    // MARK: - Menus

    /// Entries of a prefix menu (`ui.md`).
    func menuEntries(_ prefix: Prefix) -> [Entry] {
        switch prefix {
        case .move:
            // Only the targets allowed from the current thread's folder; the
            // core computes them (`move_targets`).
            let folderId = model.selectedThread?.folderId ?? model.selectedFolderId
            let targets = model.folders.first { $0.id == folderId }?.moveTargets ?? []
            return targets.map { role in
                Entry(key: Role.letter(role).lowercased(), label: Role.name(role)) { [model] in
                    model.moveTargets(to: role)
                }
            }
        case .goto:
            // Folders of the current account, then the other screens.
            let accountId = model.selectedFolder?.accountId ?? model.folders.first?.accountId
            var entries = model.folders.filter { $0.accountId == accountId && $0.letter != nil }.map { f in
                Entry(key: (f.letter ?? "").lowercased(), label: f.name) { [weak self] in
                    self?.goToFolder(f.id)
                }
            }
            entries += [
                Entry(key: "c", label: L("Contacts")) { [weak self] in self?.open(.contacts) },
                Entry(key: "h", label: L("History")) { [weak self] in self?.open(.history) },
                Entry(key: "T", label: L("Tags")) { [weak self] in self?.open(.tags) },
                Entry(key: "g", label: L("Groups")) { [weak self] in self?.open(.groups) },
            ]
            return entries
        case .save:
            return [
                Entry(key: "e", label: L("as .eml file")) { [model] in model.export("eml") },
                Entry(key: "t", label: L("as text file")) { [model] in model.export("txt") },
                Entry(key: "m", label: L("as Markdown file")) { [model] in model.export("md") },
                Entry(key: "a", label: L("all attachments"), disabled: true) {},
                Entry(key: "g", label: L("attachments but graphics"), disabled: true) {},
                Entry(key: "p", label: L("as PDF"), disabled: true) {},
            ]
        case .view:
            return [
                Entry(key: "1", label: L("one column")) { [weak self] in self?.columns = 1 },
                Entry(key: "2", label: L("two columns")) { [weak self] in self?.columns = 2 },
                Entry(key: "3", label: L("three columns")) { [weak self] in self?.columns = 3 },
                Entry(key: "c", label: L("sort by reception date")) { [model] in model.sort = "received" },
                Entry(key: "m", label: L("sort by modification date")) { [model] in model.sort = "modified" },
                Entry(key: "h", label: L("text / HTML")) { [model] in model.toggleBodyFormat() },
                Entry(key: "s", label: L("status bar: bottom, top, hidden")) { [weak self] in self?.cycleStatusBar() },
            ]
        }
    }

    private func cycleStatusBar() {
        let all = StatusPosition.allCases
        statusPosition = all[(all.firstIndex(of: statusPosition)! + 1) % all.count]
    }

    private func goToFolder(_ id: Int) {
        screen = .mail
        model.searchText = ""
        if model.selectedFolderId == id, model.listFilter != nil {
            model.selectedFolderId = nil
        }
        model.selectedFolderId = id
    }

    /// Main keys, for `?`.
    var helpLines: [(String, String)] {
        [
            ("j k", L("next / previous mail")),
            ("J K", L("mark and move (multiple selection)")),
            ("h", L("go to a folder")),
            ("l", L("links of the mail")),
            ("d", L("done (archive)")),
            ("m…", L("move to a folder")),
            ("del", L("delete")),
            ("t", L("tag")),
            ("g", L("add the sender to a group")),
            ("u  *", L("read / unread, flag")),
            ("/", L("search")),
            (";…", L("go to a folder or a screen")),
            ("s…", L("save")),
            ("v…", L("display")),
            ("^d ^u  space", L("scroll the mail")),
            ("r R f", L("reply, reply all, forward (not yet)")),
            ("esc", L("cancel, unmark, back")),
        ]
    }

    // MARK: - Pickers

    private func openPicker(_ kind: PickerKind) {
        pickerQuery = ""
        pickerIndex = 0
        overlay = .picker(kind)
        switch kind {
        case .tag:
            Task { await loadTags() }
        case .group:
            Task { await loadGroups() }
        case .folders, .links:
            break
        }
    }

    /// Rows of a picker, filtered by the text typed: every word typed must
    /// appear in the row, in any order, ignoring case and accents ("hey arc"
    /// finds "hey · Archive"). Simpler than Helix's fuzzy match.
    func pickerEntries(_ kind: PickerKind) -> [Entry] {
        let all: [Entry]
        switch kind {
        case .folders:
            let several = Set(model.folders.map(\.accountId)).count > 1
            all = model.folders.map { f in
                let unread = f.unreadCount > 0 ? "  \(f.unreadCount)" : ""
                return Entry(key: f.letter ?? " ", label: (several ? "\(f.accountName) · " : "") + f.name + unread) {
                    [weak self] in self?.goToFolder(f.id)
                }
            }
        case .links:
            all = model.links.enumerated().map { i, url in
                Entry(key: "\(i + 1)", label: url.absoluteString) { NSWorkspace.shared.open(url) }
            }
        case .tag:
            let accountId = model.folders.first { $0.id == model.selectedThread?.folderId }?.accountId
            let names = tags.filter { accountId == nil || $0.accountId == accountId }.map(\.name)
            all = withNew(names) { [model] name in model.tagTargets(name) }
        case .group:
            all = withNew(groups.map(\.name)) { [model] name in model.addSendersToGroup(name) }
        }
        let words = pickerQuery.split(separator: " ").map(String.init)
        guard !words.isEmpty else { return all }
        return all.filter { entry in
            entry.key == "+"
                || words.allSatisfy { entry.label.range(of: $0, options: [.caseInsensitive, .diacriticInsensitive]) != nil }
        }
    }

    /// Existing names, plus "+ <typed text>" to create a new one when the
    /// text matches none of them exactly.
    private func withNew(_ names: [String], _ apply: @escaping @MainActor (String) -> Void) -> [Entry] {
        var entries = names.map { name in Entry(key: " ", label: name) { apply(name) } }
        let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
        if !typed.isEmpty, !names.contains(where: { $0.compare(typed, options: [.caseInsensitive]) == .orderedSame }) {
            entries.insert(Entry(key: "+", label: typed) { apply(typed) }, at: 0)
        }
        return entries
    }

    private func handlePickerKey(_ kind: PickerKind, key: String, code: UInt16, control: Bool) {
        let entries = pickerEntries(kind)
        switch code {
        case KeyCode.escape:
            overlay = .none
            return
        case KeyCode.returnKey:
            overlay = .none
            if entries.indices.contains(pickerIndex) { entries[pickerIndex].action() }
            return
        case KeyCode.down:
            pickerIndex = min(entries.count - 1, pickerIndex + 1)
            return
        case KeyCode.up:
            pickerIndex = max(0, pickerIndex - 1)
            return
        case KeyCode.delete:
            if !pickerQuery.isEmpty { pickerQuery.removeLast() }
            pickerIndex = 0
            return
        default:
            break
        }
        // Helix's picker keys: ctrl-n / ctrl-p (and ctrl-j / ctrl-k).
        if control {
            if key == "n" || key == "j" { pickerIndex = min(entries.count - 1, pickerIndex + 1) }
            if key == "p" || key == "k" { pickerIndex = max(0, pickerIndex - 1) }
            return
        }
        if let c = key.first, !c.isNewline, key.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) {
            pickerQuery += key
            pickerIndex = 0
        }
    }

    // MARK: - Search

    private func handleSearchInput(key: String, code: UInt16) {
        switch code {
        case KeyCode.escape:
            overlay = .none
            model.searchText = ""
        case KeyCode.returnKey:
            // Keep the results; j/k then move in them, Return opens one.
            overlay = .none
            searchIndex = 0
        case KeyCode.delete:
            if !searchQuery.isEmpty { searchQuery.removeLast() }
            model.searchText = searchQuery
        default:
            if !key.isEmpty, key.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) {
                searchQuery += key
                model.searchText = searchQuery
                searchIndex = 0
            }
        }
    }

    // MARK: - Screens

    func open(_ s: Screen) {
        screen = s
        screenIndex = 0
        switch s {
        case .contacts: Task { await loadContacts(reset: true) }
        case .history: Task { await loadHistory(reset: true) }
        case .tags: Task { await loadTags() }
        case .groups: Task { await loadGroups() }
        case .mail: break
        }
    }

    private var screenCount: Int {
        switch screen {
        case .contacts: contacts.count
        case .history: history.count
        case .tags: tags.count
        case .groups: groups.count
        case .mail: 0
        }
    }

    private func handleScreenKey(key: String, code: UInt16) {
        if code == KeyCode.down || key == "j" {
            screenIndex = min(screenCount - 1, screenIndex + 1)
            loadMoreIfNeeded()
            return
        }
        if code == KeyCode.up || key == "k" {
            screenIndex = max(0, screenIndex - 1)
            return
        }
        if code == KeyCode.returnKey || key == "l" {
            switch screen {
            case .contacts where contacts.indices.contains(screenIndex):
                screen = .mail
                model.showThreads(of: .contact(contacts[screenIndex]))
            case .tags where tags.indices.contains(screenIndex):
                screen = .mail
                model.showThreads(of: .tag(tags[screenIndex]))
            case .groups where groups.indices.contains(screenIndex):
                screen = .mail
                model.showThreads(of: .group(groups[screenIndex]))
            default:
                break
            }
            return
        }
        if key == "/", screen == .contacts {
            overlay = .contactFilter
        }
    }

    private func handleContactFilterInput(key: String, code: UInt16) {
        switch code {
        case KeyCode.escape:
            overlay = .none
            contactsQuery = ""
        case KeyCode.returnKey:
            overlay = .none
            return
        case KeyCode.delete:
            if !contactsQuery.isEmpty { contactsQuery.removeLast() }
        default:
            guard !key.isEmpty, key.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) else { return }
            contactsQuery += key
        }
        screenIndex = 0
        Task { await loadContacts(reset: true) }
    }

    private func loadMoreIfNeeded() {
        guard screenIndex >= screenCount - 10 else { return }
        if screen == .contacts, contactsCursor != nil { Task { await loadContacts(reset: false) } }
        if screen == .history, historyCursor != nil { Task { await loadHistory(reset: false) } }
    }

    func loadContacts(reset: Bool) async {
        guard let client = model.core else { return }
        var params: [String: Any] = ["limit": 100]
        if !contactsQuery.isEmpty { params["query"] = contactsQuery }
        if !reset, let c = contactsCursor { params["cursor"] = c }
        let query = contactsQuery
        do {
            let page: Page<ContactView> = try await client.call("list_contacts", params)
            guard query == contactsQuery else { return }
            contacts = reset ? page.items : contacts + page.items
            contactsCursor = page.nextCursor
        } catch { model.report(error) }
    }

    func loadHistory(reset: Bool) async {
        guard let client = model.core else { return }
        var params: [String: Any] = ["limit": 100]
        if !reset, let c = historyCursor { params["cursor"] = c }
        do {
            let page: Page<HistoryEntry> = try await client.call("list_history", params)
            history = reset ? page.items : history + page.items
            historyCursor = page.nextCursor
        } catch { model.report(error) }
    }

    func loadTags() async {
        guard let client = model.core else { return }
        do {
            let list: Items<TagView> = try await client.call("list_tags", [:])
            tags = list.items
        } catch { model.report(error) }
    }

    func loadGroups() async {
        guard let client = model.core else { return }
        do {
            let list: Items<GroupView> = try await client.call("list_groups", [:])
            groups = list.items
        } catch { model.report(error) }
    }
}

/// macOS virtual key codes of the keys handled by code rather than by
/// character (they have no printable character).
enum KeyCode {
    static let returnKey: UInt16 = 36
    static let delete: UInt16 = 51
    static let escape: UInt16 = 53
    static let forwardDelete: UInt16 = 117
    static let down: UInt16 = 125
    static let up: UInt16 = 126
}
