import AppKit
import WebKit
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
    enum Prefix: String, CaseIterable {
        case move = "m", goto = ";", save = "s", view = "v"

        /// Icon of its block in the status bar (SF Symbols).
        var icon: String {
            switch self {
            case .move: "arrowshape.turn.up.right"
            case .goto: "folder"
            case .save: "square.and.arrow.down"
            case .view: "eye"
            }
        }
    }

    /// Full-window screens reached with `;` (`ui.md`, "changer d'écran").
    enum Screen: Equatable {
        case mail, contacts, history, tags, groups
    }

    /// Filterable lists in a popup (Helix's "pickers").
    enum PickerKind: Equatable {
        /// `@`: go to another account.
        case accounts
        /// `F`: attributes narrowing the list.
        case attributes
        /// The expanded panel of an attribute with a value (From…, To…,
        /// Contains…, Date…).
        case attributeValue(AttributeField)
        /// `/`: the Contains… panel, applied to "Everything" (`ui.md`).
        case search
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
        /// `/` on the contacts screen: the filter is being typed.
        case contactFilter
        /// A small menu asking for a choice.
        case choice(Choice)
    }

    /// Questions asked in a small menu, each key an answer (`ui.md`).
    enum Choice: Equatable {
        /// A move whose sender's mail is suspected of spoofing waits for a
        /// confirmation (`authentification-expediteur.md`, section 5).
        case suspect(SuspectMove)
        /// `m` / `M` on the contacts screen: the decision on the address,
        /// or on its domain.
        case decision(name: String, domain: Bool, current: String?)
    }

    /// One row of a menu or a picker.
    struct Entry: Identifiable {
        let key: String
        let label: String
        /// Grayed out: shown for discoverability, not available yet.
        var disabled = false
        /// Starts a new block: the menu leaves a gap above the row.
        var gapBefore = false
        /// Shown at the right end of the row (a folder's unread count).
        var detail: String? = nil
        /// SF Symbol drawn before the label (pickers: link, file type).
        var icon: String? = nil
        /// What ⌘-Return does, when it differs from Return (a file of `l`:
        /// save, then open).
        var commandAction: (@MainActor () -> Void)? = nil
        /// Hidden words that also find the row when typed (`cc` for "To…"),
        /// never shown (`PickerMatch`).
        var keywords: [String] = []
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
    var overlay: Overlay = .none {
        didSet {
            // A menu opens with no row highlighted: its keys act at once.
            if case .menu = overlay, overlay != oldValue { menuIndex = nil }
        }
    }
    /// Row of the open prefix menu highlighted with the arrows (`nil`: none
    /// yet); Return goes to it.
    var menuIndex: Int?
    /// The same for the open choice.
    var choiceIndex: Int?
    /// Text typed in the open picker, and the highlighted row.
    var pickerQuery = ""
    var pickerIndex = 0
    /// Where `/` was pressed from, to come back there with Escape (`ui.md`):
    /// set when `/` leaves a folder for "Everything", forgotten as soon as
    /// the user goes elsewhere (`;`, `@`, a click on a folder).
    var searchOrigin: SearchOrigin?

    /// The list shown before a `/` search: a folder, a unified role of
    /// "all", or "Everything" itself (then nothing changes on return).
    struct SearchOrigin: Equatable {
        var folderId: Int?
        var unifiedRole: String?
        var everything: AppModel.Everything?
    }

    /// Number of columns (`ui.md`, display menu): 1 = list only,
    /// 2 = list and mail, 3 = folders, list and mail.
    var columns: Int {
        didSet {
            defaults.set(columns, forKey: "geek.columns")
            // The full-window reader only exists in one column: another
            // layout shows the mail beside the list anyway.
            if columns != 1 { reading = false }
            model.readerVisible = readerVisible
        }
    }
    /// In one column, the mail under the cursor fills the window instead of
    /// the list (`ui.md`, `v1`): Return opens it, Escape goes back to the
    /// list. Not remembered: the app always opens on the list.
    var reading = false {
        didSet { model.readerVisible = readerVisible }
    }
    /// Whether the mail is on screen: always beside the list with two or
    /// three columns, only after Return with one. The model needs it for
    /// its reading rules (`AppModel.readerVisible`).
    var readerVisible: Bool { columns != 1 || reading }
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
    /// The HTML reader (`vh`), scrolled the same way. It draws in another
    /// process: its scroll position is not an AppKit scroll view but the
    /// page's own, moved with a line of JavaScript.
    weak var readerWeb: WKWebView?
    /// Scroll position of the HTML page after the last keyboard scroll, for
    /// the UI tests (the page reports it asynchronously).
    @ObservationIgnored var htmlOffset = 0

    /// Rows of the mail list that fit in its pane, set by the view when its
    /// height changes; used by `ctrl-d` / `ctrl-u`. `@ObservationIgnored`:
    /// no view reads it, so writing it during layout must not trigger a
    /// redraw (it would loop).
    @ObservationIgnored var listVisibleRows = 20

    /// Left edge of each prefix block of the status bar (folder `;`, `m`,
    /// `s`, `v`), where its menu drops down from.
    var menuBlockX: [Prefix: CGFloat] = [:]

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
        case "attachment":
            // A tile: saved to Downloads, and opened with ⌘ (the click's own
            // modifiers, read from the event being handled; no app, hence
            // no event, in unit tests).
            if let id = parts.first.flatMap(Int.init) {
                model.saveAttachment(id, NSApp?.currentEvent?.modifierFlags.contains(.command) == true ? .saveAndOpen : .save)
            }
            return true
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
        // Swift note: `didSet` does not run for assignments made in `init`,
        // so the model is told here about the remembered layout.
        model.readerVisible = readerVisible
    }

    // MARK: - Labels

    /// Short mode name for the status bar, like Helix's "NOR".
    var modeLabel: String {
        // A suspect move asks without an overlay of its own.
        if model.suspectMove != nil { return "ASK" }
        return switch overlay {
        case .none, .help: "NOR"
        case .menu(let p): p.rawValue
        case .picker: "PCK"
        case .contactFilter: "/"
        case .choice: "ASK"
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
        // ⌘-Return in a picker: the row's command action (a file of `l`:
        // saved and opened), or its usual one.
        if mods.contains(.command), event.keyCode == KeyCode.returnKey, case .picker(let kind) = overlay {
            model.flash = nil
            overlay = .none
            let entries = pickerEntries(kind)
            if entries.indices.contains(pickerIndex) {
                let e = entries[pickerIndex]
                (e.commandAction ?? e.action)()
            }
            return true
        }
        // ⌘U: unsubscribe from the list of the open mail (`ui.md`,
        // "Désabonnement"), with ⌘ because it cannot be undone and `u` alone
        // toggles read / unread. Only on the mail screen, without a panel.
        if mods.contains(.command), event.charactersIgnoringModifiers?.lowercased() == "u" {
            guard screen == .mail, overlay == .none else { return false }
            model.flash = nil
            model.unsubscribe()
            return true
        }
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

        // A question waits for its answer before anything else.
        if let choice = activeChoice {
            handleChoiceKey(choice, key: key, code: code)
            return true
        }

        switch overlay {
        case .menu(let prefix):
            // The arrows highlight a row without closing the menu: down
            // starts on the first row, up on the last, and both turn around
            // (`stepDown`, `stepUp`); Return chooses it (`ui.md`).
            let count = menuEntries(prefix).count
            switch code {
            case KeyCode.down:
                menuIndex = menuIndex.map { Self.stepDown($0, count: count) } ?? 0
                return true
            case KeyCode.up:
                menuIndex = menuIndex.map { Self.stepUp($0, count: count) } ?? max(0, count - 1)
                return true
            case KeyCode.returnKey:
                if let i = menuIndex { chooseMenuEntry(i) } else { overlay = .none }
                return true
            default:
                break
            }
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
        case .contactFilter:
            handleContactFilterInput(key: key, code: code)
            return true
        case .choice, .none:
            break
        }

        if control {
            switch key {
            // As in Zed and vim, ctrl-d / ctrl-u move by half a page in the
            // list the cursor belongs to (j / k move it too); the mail keeps
            // Space, shift-Space, ctrl-f and ctrl-b (`ui.md`).
            case "d" where screen == .mail: moveCursor(halfPage)
            case "u" where screen == .mail: moveCursor(-halfPage)
            // Other screens: used, but nothing to do (no beep).
            case "d", "u": break
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
        case "z":
            // Undo the last move or decision, from any screen.
            Task {
                await model.undoLast()
                if screen == .contacts { await loadContacts(reset: false, keep: true) }
            }
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
        handleMailKey(key: key, code: code, shift: mods.contains(.shift))
        return true
    }

    /// Normal mode on the mail screen. `shift`: Shift is held, read only
    /// for Space and the arrows (their character does not change with
    /// Shift).
    private func handleMailKey(key: String, code: UInt16, shift: Bool = false) {
        switch code {
        case KeyCode.returnKey:
            // One column shows the list only: Return reads the mail under
            // the cursor in the whole window (`ui.md`, `v1`). With two or
            // three columns the mail is already shown: nothing to do.
            if columns == 1 { openReader() }
            return
        case KeyCode.escape:
            if reading {
                // Back to the list first, before Escape's other steps
                // (unmark, leave a filter…): it undoes the last thing done.
                reading = false
            } else if !model.marked.isEmpty {
                model.marked = []
            } else if model.listFilter != nil {
                // Back from a tag, group or contact list to the folder.
                model.leaveFilter()
            } else if !model.attributes.isEmpty || searchOrigin != nil {
                // Then the attributes go, and after a `/` search, the list
                // goes back to where `/` was pressed.
                model.attributes = ThreadFilter()
                returnFromSearch()
            }
            return
        case KeyCode.delete, KeyCode.forwardDelete:
            model.moveTargets(to: "deleted")
            return
        // Shift with an arrow marks the mail and moves, like J / K (the
        // arrow's character does not change with Shift, so it is read here).
        case KeyCode.down:
            if shift { model.markAndMove(1) } else { moveCursor(1) }
            return
        case KeyCode.up:
            if shift { model.markAndMove(-1) } else { moveCursor(-1) }
            return
        default:
            break
        }
        switch key {
        case "j": moveCursor(1)
        case "k": moveCursor(-1)
        case "J": model.markAndMove(1)
        case "K": model.markAndMove(-1)
        case "@": openPicker(.accounts)
        case "F": openPicker(.attributes)
        case "l": openPicker(.links)
        case " ": scrollReader(pages: shift ? -1 : 1)
        case "d": model.moveTargets(to: "archive")
        case "u": model.toggleSeenTargets()
        case "*": model.toggleFlagTargets()
        case "t": openPicker(.tag)
        case "T": toggleShowAll()
        case "S": skipQuoted()
        case "g": openPicker(.group)
        case "/": openPicker(.search)
        case "r", "R", "f":
            model.flash = L("Not available yet: writing mail needs SMTP")
        case "m", "s", "v":
            if let p = Prefix(rawValue: key) { overlay = .menu(p) }
        default:
            if !key.isEmpty { model.flash = L("No command %@", key) }
        }
    }

    /// One column: shows the mail under the cursor in the whole window and
    /// marks it read at once (`ui.md`, `v1`). Opening a mail is asking to
    /// read it, unlike passing over it with `j` / `k`, which waits a second.
    /// Already open: nothing to do (a mail marked unread with `u` while it
    /// is open stays unread).
    private func openReader() {
        guard !reading, model.selectedThreadId != nil else { return }
        reading = true
        model.markSelectedSeen()
    }

    /// One row up in a picker or in the search results. The list turns
    /// around, as in Helix: from the first row (where a list opens) it goes
    /// to the last one, so the end of a short list is one key away. Not for
    /// the mail list: it is loaded page by page, its last row is not known.
    static func stepUp(_ index: Int, count: Int) -> Int {
        guard count > 0 else { return 0 }
        return index <= 0 ? count - 1 : min(count - 1, index - 1)
    }

    /// One row down, the other way round: from the last row it goes back to
    /// the first one.
    static func stepDown(_ index: Int, count: Int) -> Int {
        guard count > 0 else { return 0 }
        return index >= count - 1 ? 0 : max(0, index + 1)
    }

    /// Half of the visible rows of the mail list, at least one row.
    private var halfPage: Int { Self.halfPage(listVisibleRows) }

    /// Rows moved by `ctrl-d` / `ctrl-u` for a list showing `visibleRows`
    /// rows: half of them, as in Zed, and never zero (a tiny window still
    /// moves).
    static func halfPage(_ visibleRows: Int) -> Int { max(1, visibleRows / 2) }

    /// Rows of height `rowHeight` that fit entirely in `height`.
    static func visibleRows(height: CGFloat, rowHeight: CGFloat) -> Int {
        guard rowHeight > 0, height.isFinite else { return 1 }
        return max(1, Int(height / rowHeight))
    }

    private func moveCursor(_ delta: Int) { model.selectNext(delta) }

    /// Scrolls the reading pane by a fraction of its height: the text
    /// reader, or the HTML page when the mail is shown as HTML.
    private func scrollReader(pages: CGFloat) {
        if model.bodyFormat == "html", let web = readerWeb, web.window != nil {
            // Scripts of the page are off (`HTMLReader`), but the app's own
            // calls still run: `evaluateJavaScript` is not page content.
            web.evaluateJavaScript(Self.htmlScrollScript(pages: pages)) { [weak self] result, _ in
                // Swift note: the completion is called on the main thread;
                // `assumeIsolated` tells the compiler so.
                MainActor.assumeIsolated { self?.htmlOffset = (result as? NSNumber)?.intValue ?? 0 }
            }
            return
        }
        guard let scroll = readerScroll, let doc = scroll.documentView else { return }
        let clip = scroll.contentView
        let maxY = max(0, doc.frame.height - clip.bounds.height)
        let y = min(maxY, max(0, clip.bounds.origin.y + pages * clip.bounds.height))
        clip.scroll(to: NSPoint(x: 0, y: y))
        scroll.reflectScrolledClipView(clip)
    }

    /// JavaScript that scrolls the page by `pages` of its height (negative:
    /// up) and returns the new position. The browser stops at both ends.
    static func htmlScrollScript(pages: CGFloat) -> String {
        "window.scrollBy(0, Math.round(window.innerHeight * \(Double(pages)))); window.scrollY"
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
            // Folders of the current view ("all" or one account) in blocks
            // (`ui.md`, `folderGroups`), Everything, then the screens:
            // Contacts, Tags, Groups, and History on its own.
            var entries: [Entry] = []
            var group: Int?
            for f in scopeFolders where !f.letter.isEmpty {
                entries.append(Entry(
                    key: f.letter.lowercased(), label: f.name,
                    gapBefore: group != nil && group != f.group, detail: f.unread > 0 ? "\(f.unread)" : nil
                ) { [weak self] in self?.open(f) })
                group = f.group
            }
            entries += [
                Entry(key: "c", label: L("Contacts"), gapBefore: true) { [weak self] in self?.open(.contacts) },
                Entry(key: "T", label: L("Tags")) { [weak self] in self?.open(.tags) },
                Entry(key: "g", label: L("Groups")) { [weak self] in self?.open(.groups) },
                Entry(key: "h", label: L("History"), gapBefore: true) { [weak self] in self?.open(.history) },
            ]
            return entries
        case .save:
            return [
                Entry(key: "e", label: L("as .eml file")) { [model] in model.export("eml") },
                Entry(key: "t", label: L("as text file")) { [model] in model.export("txt") },
                Entry(key: "m", label: L("as Markdown file")) { [model] in model.export("md") },
                // Every attachment of the marked threads (or of the one
                // under the cursor), with or without the graphics.
                Entry(key: "a", label: L("all attachments")) { [model] in
                    model.saveAttachmentsOfTargets(includingGraphics: true)
                },
                Entry(key: "g", label: L("attachments but graphics")) { [model] in
                    model.saveAttachmentsOfTargets(includingGraphics: false)
                },
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
        /// The virtual folder "Everything".
        var everything = false
        /// Block of the folder in the lists (`folderGroup`).
        var group: Int { everything ? GeekController.folderGroups.count + 1 : GeekController.folderGroup(role) }
        var id: String { everything ? "everything" : folderId.map { "f\($0)" } ?? "r\(role ?? "")" }
    }

    /// Folders of the current view, in display order. In the unified view,
    /// one entry per role, with the unread counts of every account added up;
    /// folders without a role (user folders) stay per account and are not
    /// listed there.
    var scopeFolders: [ScopeFolder] { Self.ordered(realScopeFolders) + [everythingFolder] }

    /// Blocks of the folder lists (`ui.md`, "Thème geek"), in order: what
    /// is to deal with, what is filed, what was written, what was thrown
    /// away. Folders of other roles, and the user's own folders, form a
    /// block of their own after the filed ones.
    ///
    /// Swift note: `nonisolated` lets code outside the main actor (here the
    /// `group` property of `ScopeFolder`) use these constant rules.
    nonisolated static let folderGroups: [[String]] = [
        ["inbox", "todo", "waiting", "quarantine"],
        ["archive", "newsletter", "paperwork"],
        ["sent", "drafts"],
        ["deleted", "junk", "banned"],
    ]

    /// Block number of a role: its index in `folderGroups` × 2, so that
    /// the other folders (1 + 2 = 3) fall between the filed folders (2)
    /// and the sent ones (4).
    nonisolated static func folderGroup(_ role: String?) -> Int {
        guard let role, let i = folderGroups.firstIndex(where: { $0.contains(role) }) else { return 3 }
        return i * 2
    }

    /// Folders sorted by block, then by their place in the block; folders
    /// outside the blocks keep the core's order.
    static func ordered(_ folders: [ScopeFolder]) -> [ScopeFolder] {
        func rank(_ f: ScopeFolder) -> Int {
            f.role.flatMap { r in folderGroups.lazy.compactMap { $0.firstIndex(of: r) }.first } ?? 0
        }
        // Swift note: `enumerated()` pairs each element with its index; the
        // index breaks ties, which makes the sort stable.
        return folders.enumerated().sorted { a, b in
            (a.element.group, rank(a.element), a.offset) < (b.element.group, rank(b.element), b.offset)
        }.map(\.element)
    }

    /// "Everything", at the foot of the list: every folder of the view, to
    /// search everywhere with the attributes (`ui.md`).
    private var everythingFolder: ScopeFolder {
        ScopeFolder(folderId: nil, role: nil, name: AppModel.everythingName, letter: "E", unread: 0, everything: true)
    }

    private var realScopeFolders: [ScopeFolder] {
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
        if f.everything { return model.everything != nil }
        if model.everything != nil { return false }
        if let id = f.folderId { return id == model.selectedFolderId }
        return f.role == model.unifiedRole
    }

    /// Opens a folder of the current view.
    func open(_ f: ScopeFolder) {
        if f.everything {
            screen = .mail
            searchOrigin = nil
            model.selectEverything(model.scopeAccountId.map { .account($0) } ?? .all)
        } else if let id = f.folderId {
            goToFolder(id)
        } else if let role = f.role {
            screen = .mail
            searchOrigin = nil
            model.selectUnified(role)
        }
    }

    func goToFolder(_ id: Int) {
        screen = .mail
        searchOrigin = nil
        if model.selectedFolderId == id, model.listFilter != nil {
            model.selectedFolderId = nil
        }
        model.selectedFolderId = id
    }

    /// Main keys, for `?`.
    var helpLines: [(String, String)] {
        [
            ("j k", L("next / previous mail")),
            ("^d ^u", L("half a page down / up in the list")),
            ("J K", L("mark and move (multiple selection)")),
            ("@", L("go to an account")),
            ("F", L("filter by attributes")),
            ("l", L("links of the mail")),
            ("d", L("done (archive)")),
            ("m…", L("move to a folder")),
            ("del", L("delete")),
            ("t", L("tag")),
            ("g", L("add the sender to a group")),
            ("u  *", L("read / unread, flag")),
            ("z", L("undo the last move or decision")),
            ("⌘U", L("unsubscribe from the list")),
            ("T", L("show / hide quotes, signatures, folded mails")),
            ("S", L("skip to the text after the next quote")),
            ("/", L("search everywhere (Contains… in Everything)")),
            (";…", L("go to a folder or a screen")),
            ("s…", L("save")),
            ("v…", L("display")),
            ("ret", L("read the mail in the whole window (one column)")),
            ("space ⇧space  ^f ^b", L("scroll the mail")),
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
        case .links, .accounts, .attributes:
            break
        case .attributeValue, .search:
            valueSuggestions = []
        }
    }

    /// Contacts suggested in the From… and To… panels, for the text typed.
    var valueSuggestions: [ContactView] = []

    /// Loads the contacts matching the text typed in a From… or To… panel.
    private func loadSuggestions() {
        let query = pickerQuery.trimmingCharacters(in: .whitespaces)
        guard query.count >= 2, let client = model.core else { valueSuggestions = []; return }
        Task {
            do {
                let page: Page<ContactView> = try await client.call("list_contacts", ["query": query, "limit": 20])
                if query == pickerQuery.trimmingCharacters(in: .whitespaces) { valueSuggestions = page.items }
            } catch { model.report(error) }
        }
    }

    /// Opens a prefix menu from a click on its block in the status bar:
    /// the folder (`;`: folders of the view, "Everything", screens), or the
    /// `m`, `s`, `v` icons.
    func openMenuFromStatusBar(_ prefix: Prefix) {
        model.flash = nil
        overlay = .menu(prefix)
    }

    /// A click on a row of the open prefix menu: same as its key.
    func chooseMenuEntry(_ index: Int) {
        guard case .menu(let prefix) = overlay else { return }
        overlay = .none
        let entries = menuEntries(prefix)
        guard entries.indices.contains(index) else { return }
        if entries[index].disabled {
            model.flash = L("Not available yet: %@", entries[index].label)
        } else {
            entries[index].action()
        }
    }

    /// Opens a picker from a click in the status bar (`@`, `F`).
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
        // From "Everything", stay in "Everything" of the chosen view.
        if model.everything != nil {
            screen = .mail
            searchOrigin = nil
            model.selectEverything(accountId.map { .account($0) } ?? .all)
            return
        }
        guard let accountId else {
            screen = .mail
            searchOrigin = nil
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
        case .accounts:
            // "all" first (the unified view), then each account; the current
            // one is marked.
            let unified = model.unifiedRole != nil
            let total = accounts.reduce(0) { $0 + $1.unread }
            // The unread count is a detail aligned on the right, as in the
            // ; menu.
            var entries = [Entry(key: unified ? "•" : " ", label: AppModel.unifiedName,
                                 detail: total > 0 ? "\(total)" : nil) { [weak self] in self?.goToAccount(nil) }]
            let current = model.scopeAccountId
            entries += accounts.map { a in
                Entry(key: !unified && a.id == current ? "•" : " ", label: a.name,
                      detail: a.unread > 0 ? "\(a.unread)" : nil) { [weak self] in
                    self?.goToAccount(a.id)
                }
            }
            all = entries
        case .attributes:
            all = attributeEntries
        case .attributeValue(let field):
            return valueEntries(field)
        case .search:
            // The rows of Contains…, applied to "Everything".
            let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
            guard !typed.isEmpty else { return [] }
            return [Entry(key: "+", label: typed) { [weak self] in self?.searchEverywhere(typed) }]
        case .links:
            // The links of the text, then one row per attached file
            // (`ui.md`): Return opens a link or saves a file; ⌘-Return saves
            // the file and opens it at once.
            let links = model.links
            all = links.enumerated().map { i, url in
                Entry(key: "\(i + 1)", label: url.absoluteString, icon: "link") { NSWorkspace.shared.open(url) }
            } + model.attachments.enumerated().map { i, a in
                Entry(key: "\(links.count + i + 1)",
                      label: AttachmentDisplay.name(a) + "  " + AttachmentDisplay.size(a.size),
                      icon: AttachmentDisplay.symbol(mimeType: a.mimeType, filename: a.filename),
                      commandAction: { [model] in model.saveAttachment(a.id, .saveAndOpen) }) { [model] in
                    model.saveAttachment(a.id, .save)
                }
            }
        case .tag:
            let accountId = model.folders.first { $0.id == model.selectedThread?.folderId }?.accountId
            let names = tags.filter { accountId == nil || $0.accountId == accountId }.map(\.name)
            all = withNew(names) { [model] name in model.tagTargets(name) }
        case .group:
            all = withNew(groups.map(\.name)) { [model] name in model.addSendersToGroup(name) }
        }
        // Best matches first, so that Return takes the row the user meant
        // (`PickerMatch`); the "+ typed text" row always stays.
        return PickerMatch.filter(all, query: pickerQuery, label: \.label,
                                  keywords: \.keywords, pinned: { $0.key == "+" })
    }

    /// The attribute selector: each yes / no attribute with its two values
    /// (an active one is marked "•", choosing it again removes it), then the
    /// attributes with a value, which expand the panel, then "clear".
    private var attributeEntries: [Entry] {
        let a = model.attributes
        var entries: [Entry] = []
        for flag in ThreadFilter.Flag.allCases {
            // Unread before read, then "with" before "without".
            for value in flag == .seen ? [false, true] : [true, false] {
                entries.append(Entry(key: a.value(flag) == value ? "•" : " ", label: ThreadFilter.label(flag, value)) {
                    [model] in model.attributes.toggle(flag, value)
                })
            }
        }
        for field in AttributeField.allCases {
            let active: Bool = switch field {
            case .from: !a.from.isEmpty
            case .to: !a.to.isEmpty
            case .contains: a.contains != nil
            case .date: a.after != nil || a.before != nil
            }
            entries.append(Entry(key: active ? "•" : " ", label: field.title, keywords: field.keywords) { [weak self] in
                // The panel grows at once to choose the value (`ui.md`).
                self?.pickerQuery = ""
                self?.pickerIndex = 0
                self?.valueSuggestions = []
                self?.overlay = .picker(.attributeValue(field))
            })
        }
        if !a.isEmpty {
            entries.append(Entry(key: "×", label: L("clear all attributes")) { [model] in model.attributes = ThreadFilter() })
        }
        return entries
    }

    /// Rows of the expanded panel of `field`.
    private func valueEntries(_ field: AttributeField) -> [Entry] {
        let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
        switch field {
        case .from, .to:
            var entries: [Entry] = []
            if !typed.isEmpty {
                entries.append(Entry(key: "+", label: typed) { [model] in model.attributes.add(field, typed) })
            }
            entries += valueSuggestions.map { c in
                let label = c.name.map { "\($0) <\(c.addr)>" } ?? c.addr
                return Entry(key: " ", label: label) { [model] in model.attributes.add(field, c.addr) }
            }
            return entries
        case .contains:
            guard !typed.isEmpty else { return [] }
            return [Entry(key: "+", label: typed) { [model] in model.attributes.add(.contains, typed) }]
        case .date:
            let presets = DateChoice.typed(typed) + DateChoice.presets().filter {
                typed.isEmpty || DateChoice.parse(typed) != nil
                    || $0.label.range(of: typed, options: [.caseInsensitive, .diacriticInsensitive]) != nil
            }
            return presets.map { p in
                Entry(key: " ", label: p.label) { [model] in
                    model.attributes.after = p.after
                    model.attributes.before = p.before
                    model.attributes.dateLabel = p.label
                }
            }
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
            pickerIndex = Self.stepDown(pickerIndex, count: entries.count)
            return
        case KeyCode.up:
            pickerIndex = Self.stepUp(pickerIndex, count: entries.count)
            return
        case KeyCode.delete:
            if !pickerQuery.isEmpty { pickerQuery.removeLast() }
            pickerIndex = 0
            if kind == .attributeValue(.from) || kind == .attributeValue(.to) { loadSuggestions() }
            return
        default:
            break
        }
        // Helix's picker keys: ctrl-n / ctrl-p (and ctrl-j / ctrl-k).
        if control {
            if key == "n" || key == "j" { pickerIndex = Self.stepDown(pickerIndex, count: entries.count) }
            if key == "p" || key == "k" { pickerIndex = Self.stepUp(pickerIndex, count: entries.count) }
            return
        }
        if let c = key.first, !c.isNewline, key.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) {
            pickerQuery += key
            pickerIndex = 0
        }
        if kind == .attributeValue(.from) || kind == .attributeValue(.to) { loadSuggestions() }
    }

    // MARK: - Search

    /// `/` then Return: the words become the Contains… attribute of
    /// "Everything" of the current view (every folder, every account in
    /// "all"), shown like any filtered list (`ui.md`). The place `/` was
    /// pressed from is kept for Escape; a second `/` keeps the first one.
    func searchEverywhere(_ words: String) {
        if searchOrigin == nil {
            searchOrigin = SearchOrigin(
                folderId: model.selectedFolderId, unifiedRole: model.unifiedRole, everything: model.everything)
        }
        screen = .mail
        model.attributes.add(.contains, words)
        model.selectEverything(model.scopeAccountId.map { .account($0) } ?? .all)
    }

    /// Escape after a `/` search: back to the folder `/` was pressed from.
    private func returnFromSearch() {
        guard let origin = searchOrigin else { return }
        // Navigating clears `searchOrigin` (`goToFolder`…), already done.
        if let id = origin.folderId {
            goToFolder(id)
        } else if let role = origin.unifiedRole {
            model.selectUnified(role)
        } else if let e = origin.everything {
            model.selectEverything(e)
        }
        searchOrigin = nil
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
        // `m`: decision on the contact's address; `M`: on its domain.
        if key == "m" || key == "M", screen == .contacts, contacts.indices.contains(screenIndex) {
            let c = contacts[screenIndex]
            let domain = key == "M"
            guard let name = domain ? Self.domain(of: c.addr) : c.addr else { return }
            choiceIndex = nil
            overlay = .choice(.decision(name: name, domain: domain, current: domain ? c.domainTarget : c.screeningTarget))
        }
    }

    /// Domain part of an address (`news.shop.fr` for `a@news.shop.fr`).
    static func domain(of addr: String) -> String? {
        guard let at = addr.lastIndex(of: "@") else { return nil }
        let d = addr[addr.index(after: at)...].lowercased()
        return d.isEmpty ? nil : d
    }

    // MARK: - Choices

    /// The question waiting for an answer: a suspect move first, as it
    /// arrives from the core without a key; then a choice opened by a key.
    var activeChoice: Choice? {
        if let s = model.suspectMove { return .suspect(s) }
        if case .choice(let c) = overlay { return c }
        return nil
    }

    func choiceTitle(_ c: Choice) -> String {
        switch c {
        case .suspect(let s): L("Probable spoofing of %@ (verdict %@)", s.addr, s.verdict)
        case .decision(let name, let domain, _): domain ? L("Decision on the domain %@", name) : L("Decision on %@", name)
        }
    }

    /// Answers of a choice. The current decision is marked on the right.
    func choiceEntries(_ c: Choice) -> [Entry] {
        switch c {
        case .suspect(let s):
            return [
                Entry(key: "y", label: L("move it: it is really %@", s.addr)) { [model] in
                    model.confirmSuspect(trustTool: false)
                },
                Entry(key: "t", label: L("move it, and accept this sender without authentication (badly configured tool)")) {
                    [model] in model.confirmSuspect(trustTool: true)
                },
                Entry(key: "esc", label: L("cancel: leave it where it is")) { [model] in model.cancelSuspect() },
            ]
        case .decision(let name, let domain, let current):
            var entries = ["inbox", "newsletter", "paperwork", "banned"].map { role in
                Entry(key: Role.letter(role).lowercased(), label: Role.name(role), detail: current == role ? "•" : nil) {
                    [weak self] in self?.decide(name, domain: domain, target: role)
                }
            }
            entries.append(Entry(key: "-", label: L("no decision"), detail: current == nil ? "•" : nil) { [weak self] in
                self?.decide(name, domain: domain, target: nil)
            })
            return entries
        }
    }

    private func decide(_ name: String, domain: Bool, target: String?) {
        Task {
            await model.setScreening(name, domain: domain, target: target)
            await loadContacts(reset: false, keep: true)
        }
    }

    private func handleChoiceKey(_ c: Choice, key: String, code: UInt16) {
        let entries = choiceEntries(c)
        switch code {
        case KeyCode.down:
            choiceIndex = choiceIndex.map { Self.stepDown($0, count: entries.count) } ?? 0
            return
        case KeyCode.up:
            choiceIndex = choiceIndex.map { Self.stepUp($0, count: entries.count) } ?? max(0, entries.count - 1)
            return
        case KeyCode.returnKey:
            if let i = choiceIndex { chooseChoiceEntry(i) }
            return
        case KeyCode.escape:
            closeChoice()
            if case .suspect = c { model.cancelSuspect() }
            return
        default:
            break
        }
        if let i = entries.firstIndex(where: { $0.key == key }) {
            chooseChoiceEntry(i)
        } else if !key.isEmpty {
            model.flash = L("Answer with one of the keys, or esc")
        }
    }

    /// A row of the open choice, by key, Return or click.
    func chooseChoiceEntry(_ index: Int) {
        guard let c = activeChoice else { return }
        let entries = choiceEntries(c)
        closeChoice()
        if entries.indices.contains(index) { entries[index].action() }
    }

    private func closeChoice() {
        choiceIndex = nil
        if case .choice = overlay { overlay = .none }
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

    /// Loads the contacts: the first page (`reset`), the next one, or with
    /// `keep` the rows already shown again (after a decision), so that the
    /// cursor stays on its row.
    func loadContacts(reset: Bool, keep: Bool = false) async {
        guard let client = model.core else { return }
        var params: [String: Any] = ["limit": keep ? max(100, contacts.count) : 100]
        if !contactsQuery.isEmpty { params["query"] = contactsQuery }
        if !reset, !keep, let c = contactsCursor { params["cursor"] = c }
        let query = contactsQuery
        do {
            let page: Page<ContactView> = try await client.call("list_contacts", params)
            guard query == contactsQuery else { return }
            contacts = reset || keep ? page.items : contacts + page.items
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
    /// - ⇧-click: mark every mail from the cursor to it;
    /// - double-click, with one column: read it in the whole window, like
    ///   Return (`ui.md`, `v1`). With two or three columns the mail is
    ///   already shown beside the list: the double-click is a plain click.
    ///
    /// The modifiers come from the click gesture itself (`View.onClick`).
    func clickThread(_ id: Int, _ mods: ClickModifiers) {
        model.flash = nil
        // Only two plain clicks make a double-click: a ⌘-click or a ⇧-click
        // is recorded under another tag, so that it never counts as the
        // first half of one.
        let double = isDoubleClick(mods == .none ? "thread-\(id)" : "thread-modified")
        let result = Selection.click(
            on: id, in: model.threads.map(\.id), cursor: model.selectedThreadId, marked: model.marked,
            command: mods == .command, shift: mods == .shift)
        model.marked = result.marked
        model.selectedThreadId = result.cursor
        if double, mods == .none, columns == 1 { openReader() }
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
