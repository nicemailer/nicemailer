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
        /// `@`: go to another account.
        case accounts
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
    var columns: Int {
        didSet { defaults.set(columns, forKey: "geek.columns") }
    }
    var statusPosition: StatusPosition {
        didSet { defaults.set(statusPosition.rawValue, forKey: "geek.status") }
    }
    /// Width of the mail list, as a fraction of the window ("un petit
    /// tiers", resizable by dragging the separator).
    var listFraction: Double {
        didSet { defaults.set(listFraction, forKey: "geek.listFraction") }
    }
    /// Where the layout choices are remembered (`AppDefaults.store`, or a
    /// throwaway domain in tests).
    @ObservationIgnored private let defaults: UserDefaults

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

    /// Display choices per thread (`ThreadDisplay.State`), for the session.
    var readerStates: [Int: ThreadDisplay.State] = [:]
    /// Positions of the quoted blocks and of the other blocks in the reader,
    /// set by the reader after layout, for `S`.
    @ObservationIgnored var readerSkip: (quotes: [(Double, Double)], texts: [Double]) = ([], [])

    /// Display choices of the open thread.
    var readerState: ThreadDisplay.State {
        get { model.openThread.flatMap { readerStates[$0.id] } ?? ThreadDisplay.State() }
        set { if let id = model.openThread?.id { readerStates[id] = newValue } }
    }

    /// `T` (like mutt's toggle-quoted): shows everything the simplified
    /// display hides in this thread (folded messages, quotes, signatures…),
    /// or goes back to the simplified display.
    func toggleShowAll() {
        var s = readerState
        s.showAll.toggle()
        if !s.showAll {
            s.folding = [:]
            s.revealed = []
        }
        readerState = s
        model.flash = s.showAll ? L("Whole conversation shown (T)") : L("Simplified conversation (T)")
    }

    /// `S` (like mutt's skip-quoted): scrolls to the text after the next
    /// quoted block.
    func skipQuoted() {
        guard let scroll = readerScroll, let doc = scroll.documentView else { return }
        let clip = scroll.contentView
        let y = Double(clip.bounds.origin.y)
        guard let target = ThreadDisplay.skipTarget(from: y, quotes: readerSkip.quotes, texts: readerSkip.texts) else {
            model.flash = L("No more quoted text")
            return
        }
        let maxY = max(0, doc.frame.height - clip.bounds.height)
        clip.scroll(to: NSPoint(x: 0, y: min(maxY, CGFloat(target))))
        scroll.reflectScrolledClipView(clip)
    }

    /// A click in the reader on a header (fold / unfold the message) or on
    /// a "···" line (show that hidden part). Links use the `nm:` scheme.
    func readerLink(_ url: URL) -> Bool {
        guard url.scheme == "nm" else { return false }
        let parts = url.pathComponents.filter { $0 != "/" }
        var s = readerState
        switch url.host {
        case "fold":
            if let id = parts.first.flatMap(Int.init),
               let m = model.openThread?.messages.first(where: { $0.id == id }) {
                let unfolded = s.folding[id] ?? m.expanded
                s.folding[id] = !unfolded
            }
        case "reveal":
            if parts.count == 2 { s.revealed.insert("\(parts[0]):\(parts[1])") }
        default:
            return false
        }
        readerState = s
        return true
    }

    /// Last row clicked and when, to detect double-clicks (see `clickSearch`).
    @ObservationIgnored var lastClick: (tag: String, time: TimeInterval) = ("", 0)

    /// The text size it changes on ⌘+ / ⌘− / ⌘0 (a separate one in tests).
    @ObservationIgnored let textSize: TextSize

    init(model: AppModel, defaults: UserDefaults = AppDefaults.store, textSize: TextSize? = nil) {
        self.model = model
        self.defaults = defaults
        self.textSize = textSize ?? .shared
        columns = defaults.object(forKey: "geek.columns") as? Int ?? 2
        statusPosition = StatusPosition(rawValue: defaults.string(forKey: "geek.status") ?? "") ?? .bottom
        listFraction = defaults.object(forKey: "geek.listFraction") as? Double ?? 0.32
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
            return "\(model.scopeName) · \(model.currentFolderName)"
        }
    }

    // MARK: - Key handling

    /// Handles one key press. Returns `true` if the key was used (it then
    /// goes no further), `false` to let AppKit process it (⌘ shortcuts).
    func handle(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if mods.contains(.command) {
            // Text size (`TextSize`): handled here so that ⌘= works too (on
            // AZERTY, `+` needs Shift) and in every keyboard layout. The
            // View menu has the same commands.
            switch event.characters ?? "" {
            case "+", "=": textSize.bigger()
            case "-": textSize.smaller()
            case "0": textSize.reset()
            // Other ⌘ shortcuts belong to the menus (⌘Q, ⌘R…).
            default: return false
            }
            model.flash = L("Text size: %lld pt", Int(textSize.size))
            return true
        }
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
            } else if model.listFilter != nil {
                // Back from a tag, group or contact list to the folder.
                model.leaveFilter()
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
        case "@": openPicker(.accounts)
        case "l": openPicker(.links)
        case " ": scrollReader(pages: 1)
        case "d": model.moveTargets(to: "archive")
        case "u": model.toggleSeenTargets()
        case "*": model.toggleFlagTargets()
        case "t": openPicker(.tag)
        case "T": toggleShowAll()
        case "S": skipQuoted()
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
            // Folders of the current view ("all" or one account), then the
            // other screens.
            var entries = scopeFolders.filter { !$0.letter.isEmpty }.map { f in
                Entry(key: f.letter.lowercased(), label: f.name) { [weak self] in self?.open(f) }
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

    /// A folder of the current view: a real folder of the account, or in
    /// the unified view a role gathering the folders of every account.
    struct ScopeFolder: Identifiable {
        /// Folder id, or nil for a unified role.
        let folderId: Int?
        let role: String?
        let name: String
        let letter: String
        let unread: Int
        var id: String { folderId.map { "f\($0)" } ?? "r\(role ?? "")" }
    }

    /// Folders of the current view, in display order. In the unified view,
    /// one entry per role, with the unread counts of every account added up;
    /// folders without a role (user folders) stay per account and are not
    /// listed there.
    var scopeFolders: [ScopeFolder] {
        if let account = model.scopeAccountId {
            return model.folders.filter { $0.accountId == account }.map {
                ScopeFolder(folderId: $0.id, role: $0.role, name: $0.name, letter: $0.letter ?? "", unread: $0.unreadCount)
            }
        }
        var out: [ScopeFolder] = []
        for f in model.folders {
            guard let role = f.role else { continue }
            if let i = out.firstIndex(where: { $0.role == role }) {
                let o = out[i]
                out[i] = ScopeFolder(folderId: nil, role: role, name: o.name, letter: o.letter, unread: o.unread + f.unreadCount)
            } else {
                out.append(ScopeFolder(folderId: nil, role: role, name: f.name, letter: f.letter ?? "", unread: f.unreadCount))
            }
        }
        return out
    }

    /// Whether `f` is the folder shown.
    func isCurrent(_ f: ScopeFolder) -> Bool {
        guard model.listFilter == nil else { return false }
        if let id = f.folderId { return id == model.selectedFolderId }
        return f.role == model.unifiedRole
    }

    /// Opens a folder of the current view.
    func open(_ f: ScopeFolder) {
        if let id = f.folderId {
            goToFolder(id)
        } else if let role = f.role {
            screen = .mail
            model.searchText = ""
            model.selectUnified(role)
        }
    }

    func goToFolder(_ id: Int) {
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
            ("@", L("go to an account")),
            ("l", L("links of the mail")),
            ("d", L("done (archive)")),
            ("m…", L("move to a folder")),
            ("del", L("delete")),
            ("t", L("tag")),
            ("g", L("add the sender to a group")),
            ("u  *", L("read / unread, flag")),
            ("T", L("show / hide quotes, signatures, folded mails")),
            ("S", L("skip to the text after the next quote")),
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
        case .folders, .links, .accounts:
            break
        }
    }

    /// Opens a picker from a click in the status bar (`h` and `@`).
    func openPickerFromStatusBar(_ kind: PickerKind) {
        model.flash = nil
        if screen != .mail { screen = .mail }
        openPicker(kind)
    }

    /// Accounts, in the order of the folder list: (id, name, unread mails
    /// in their workflow folders).
    var accounts: [(id: Int, name: String, unread: Int)] {
        var out: [(id: Int, name: String, unread: Int)] = []
        for f in model.folders {
            if let i = out.firstIndex(where: { $0.id == f.accountId }) {
                out[i].unread += f.unreadCount
            } else {
                out.append((f.accountId, f.accountName, f.unreadCount))
            }
        }
        return out
    }

    /// Goes to another account (`nil`: the unified view "all"), in the
    /// folder of the same role as the current one (Archive to Archive), or
    /// its Inbox, or its first folder.
    func goToAccount(_ accountId: Int?) {
        let role = model.currentRole
        guard let accountId else {
            screen = .mail
            model.searchText = ""
            model.selectUnified(role ?? "inbox")
            return
        }
        let folders = model.folders.filter { $0.accountId == accountId }
        let target = folders.first { $0.role == role && role != nil }
            ?? folders.first { $0.role == "inbox" }
            ?? folders.first
        if let target { goToFolder(target.id) }
    }

    /// Rows of a picker, filtered by the text typed: every word typed must
    /// appear in the row, in any order, ignoring case and accents ("hey arc"
    /// finds "hey · Archive"). Simpler than Helix's fuzzy match.
    func pickerEntries(_ kind: PickerKind) -> [Entry] {
        let all: [Entry]
        switch kind {
        case .folders:
            // The folders of the current view only: the account is chosen
            // with `@`.
            all = scopeFolders.map { f in
                let unread = f.unread > 0 ? "  \(f.unread)" : ""
                return Entry(key: f.letter.isEmpty ? " " : f.letter, label: f.name + unread) { [weak self] in self?.open(f) }
            }
        case .accounts:
            // "all" first (the unified view), then each account; the current
            // one is marked.
            let unified = model.unifiedRole != nil
            let total = accounts.reduce(0) { $0 + $1.unread }
            var entries = [Entry(key: unified ? "•" : " ", label: AppModel.unifiedName + (total > 0 ? "  \(total)" : "")) {
                [weak self] in self?.goToAccount(nil)
            }]
            let current = model.scopeAccountId
            entries += accounts.map { a in
                let unread = a.unread > 0 ? "  \(a.unread)" : ""
                return Entry(key: !unified && a.id == current ? "•" : " ", label: a.name + unread) { [weak self] in
                    self?.goToAccount(a.id)
                }
            }
            all = entries
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
            activateScreenRow()
            return
        }
        if key == "/", screen == .contacts {
            overlay = .contactFilter
        }
    }

    /// Return (or a double-click) on a screen row: lists the mails of the
    /// contact, tag or group under the cursor.
    private func activateScreenRow() {
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

// MARK: - Mouse

extension GeekController {
    /// Clicks on rows. A click acts at once; a second click on the same row
    /// within the system's double-click interval also "opens" it. Detecting
    /// the double-click by hand avoids SwiftUI's `onTapGesture(count: 2)`,
    /// which delays every single click until it knows no second one follows.
    private func isDoubleClick(_ tag: String) -> Bool {
        let now = ProcessInfo.processInfo.systemUptime
        defer { lastClick = (tag, now) }
        return lastClick.tag == tag && now - lastClick.time < NSEvent.doubleClickInterval
    }

    /// Click on a mail of the list, Finder-style:
    /// - click: select it, unmark everything;
    /// - ⌘-click: mark or unmark it (the mail under the cursor joins the
    ///   selection first, as when building a selection in the Finder);
    /// - ⇧-click: mark every mail from the cursor to it.
    ///
    /// The modifiers come from the click gesture itself (`View.onClick`).
    func clickThread(_ id: Int, _ mods: ClickModifiers) {
        model.flash = nil
        let result = Selection.click(
            on: id, in: model.threads.map(\.id), cursor: model.selectedThreadId, marked: model.marked,
            command: mods == .command, shift: mods == .shift)
        model.marked = result.marked
        model.selectedThreadId = result.cursor
    }

    /// Click on a search result; a double-click opens it in its folder.
    func clickSearch(_ index: Int) {
        searchIndex = index
        if isDoubleClick("search-\(index)"), model.searchResults.indices.contains(index) {
            model.openSearchHit(model.searchResults[index])
        }
    }

    /// Click on a row of the contacts, tags or groups screen; a
    /// double-click lists its mails, like Return.
    func clickScreenRow(_ index: Int) {
        screenIndex = index
        if isDoubleClick("screen-\(screen)-\(index)") { activateScreenRow() }
    }

    /// Click on a picker row: chooses it, like Return.
    func clickPicker(_ kind: PickerKind, _ index: Int) {
        let entries = pickerEntries(kind)
        overlay = .none
        if entries.indices.contains(index) { entries[index].action() }
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
