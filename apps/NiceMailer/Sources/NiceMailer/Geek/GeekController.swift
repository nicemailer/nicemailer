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
/// key and shows a menu of what it can be. Which key does what comes from
/// the keymap (`Keymap`, `keymap.toml`, `docs/keymap.md`): the keys named
/// in the comments are the defaults.
@MainActor
@Observable
final class GeekController {
    // MARK: - Types

    /// A menu opened by a key, a *prefix* as in Helix: the four of the app
    /// (`m`, `;`, `s`, `v` by default) and those of the user (`[menu.<name>]`
    /// of `keymap.toml`, `raccourcis.md`, section 7).
    ///
    /// Swift note: an enum case may carry a value (`user(String)`); such an
    /// enum cannot have raw values, so the names are a property.
    enum Prefix: Hashable {
        case move, goto, save, view
        /// A menu of the user, by its name.
        case user(String)

        /// The menus of the app, which have a block in the status bar.
        static let builtins: [Prefix] = [.move, .goto, .save, .view]

        /// Its name in the keymap: `[menu.save]`, opened by `menu.save`.
        var name: String {
            switch self {
            case .move: "move"
            case .goto: "goto"
            case .save: "save"
            case .view: "view"
            case .user(let n): n
            }
        }

        /// The menu of a name of the keymap.
        init(name: String) {
            self = Self.builtins.first { $0.name == name } ?? .user(name)
        }

        /// The action that opens it, for the built-in menus.
        var action: KeyAction? { KeyAction(rawValue: "menu." + name) }

        /// Id of its block in the UI scripts (`status-m`): the default key,
        /// fixed whatever the keymap says, so that a scenario stays valid.
        var uiID: String {
            switch self {
            case .move: "m"
            case .goto: ";"
            case .save: "s"
            case .view: "v"
            case .user(let n): n
            }
        }

        /// Icon of its block in the status bar (SF Symbols).
        var icon: String {
            switch self {
            case .move: "arrowshape.turn.up.right"
            case .goto: "folder"
            case .save: "square.and.arrow.down"
            case .view: "eye"
            case .user: "list.bullet"
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
        /// Text…, Subject…, Date…).
        case attributeValue(AttributeField)
        /// `/`: words searched anywhere (addresses included), applied to
        /// "Everything" (`ui.md`).
        case search
        /// `l`: open a link of the mail.
        case links
        /// `t`: tag the threads (existing tag, or a new one typed).
        case tag
        /// `g`: add the senders to a group (existing, or new).
        case group
        /// `a`, or ⌘I in a reply: the AI prompts of the screen, as a
        /// command palette (`ia.md`, section 11).
        case ai
        /// `r` on the Tags or Groups screen: the new name of the tag or
        /// group `name` (`id`: the group's), typed over the current one
        /// (`ui.md`, "Écrans Tags et Groupes").
        case rename(LabelKind, name: String, id: Int?)
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
        /// `o` on the contacts screen: whether the address is a person or a
        /// robot (`expediteurs.md`, section 9). `current`: the shown verdict,
        /// `forced` when the user chose it.
        case senderKind(addressId: Int, addr: String, current: String?, forced: Bool)
        /// "Change Mailbox…" chose a mailbox of an earlier data format:
        /// upgrade it, or keep the current one (`AppModel.pendingUpgrade`).
        case mailboxUpgrade(PendingUpgrade)
        /// ⇧⌘↩ in the reply: send it, then file the conversation where the
        /// key says (`reponse.md`, section 8). Escape goes back to the text.
        case sendThen
        /// ⌥⌘F in the reply, or a click on "From": the address, and so the
        /// account, it is sent from (`reponse.md`, section 4 ter).
        case sendFrom
        /// `d` on the Tags or Groups screen: deleting cannot be undone (a
        /// tag leaves every mail, on the server too), so it is asked
        /// first. `count`: the mails of the tag, the members of the group.
        case deleteLabel(LabelKind, name: String, id: Int?, count: Int)
        /// `f` on the History screen: whose actions it shows.
        case historySource
    }

    /// One row of a menu or a picker.
    struct Entry: Identifiable {
        var key: String
        let label: String
        /// Grayed out: shown for discoverability, not available yet.
        var disabled = false
        /// Starts a new block: the menu leaves a gap above the row.
        var gapBefore = false
        /// Shown at the right end of the row (a folder's unread count).
        var detail: String? = nil
        /// How the detail is shown: a count stands out, an information
        /// (where a link goes, how a date was read) stays in the background.
        var detailStyle: DetailStyle = .count
        /// SF Symbol drawn before the label (pickers: link, file type).
        var icon: String? = nil
        /// What ⌘-Return does, when it differs from Return (a file of `l`:
        /// save, then open).
        var commandAction: (@MainActor () -> Void)? = nil
        /// In a menu, a second key that runs `commandAction`: `T` for "as
        /// text file, then open", next to `t` on the same row (`ui.md`,
        /// menu `s`). The row says so in its detail. Shown; `altChord` is
        /// its spelling, compared with the key pressed.
        var altKey: String? = nil
        var altChord: String? = nil
        /// Hidden words that also find the row when typed (`cc` for "To…"),
        /// never shown (`PickerMatch`).
        var keywords: [String] = []
        /// A virtual folder in the menu `;`: drawn apart from the real
        /// folders (`ui.md`, "Dossiers virtuels").
        var virtual = false
        /// The spelling of the row's key in a menu (`ctrl-x`, `T`), compared
        /// with the key pressed (`KeyChord`); `nil`: `key` itself (the
        /// answers of a question, a letter or `-`).
        var chord: String? = nil
        let action: @MainActor () -> Void
        var id: String { key + label }
    }

    /// How the detail of a row is shown (`Entry.detailStyle`).
    enum DetailStyle {
        /// Something to notice (unread mails, a warning): the unread colour.
        case count
        /// Secondary information (a domain, a host, a date as read): muted.
        case info
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
    /// Text typed in the open picker, with its cursor (`LineEdit`), and
    /// the highlighted row.
    var pickerLine = LineEdit()
    var pickerIndex = 0
    /// The text typed in the open picker. Setting it puts the cursor at
    /// the end (a new panel, a cleared query).
    var pickerQuery: String {
        get { pickerLine.text }
        set { pickerLine = LineEdit(newValue) }
    }
    /// Where ⌘V reads from. The general pasteboard, except in tests and in
    /// UI script mode, which use a private one so as never to read or
    /// overwrite what the user copied.
    @ObservationIgnored var pasteboard: NSPasteboard = .general
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
        var virtual: AppModel.Virtual = .everything
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
    /// The contacts filter typed after `/`, with its cursor.
    var contactsLine = LineEdit()
    var contactsQuery: String {
        get { contactsLine.text }
        set { contactsLine = LineEdit(newValue) }
    }
    /// The rows of the History screen read so far, and where the next page
    /// starts.
    var history: [HistoryGroup] = []
    var historyCursor: String?
    /// The lines of the groups unfolded, by the group's `lastId`.
    var historyUnfolded: [Int: [HistoryGroup]] = [:]
    /// The mails marked as read are shown too (`history.toggle_read`).
    var historyIncludesRead = false
    /// Whose actions are shown (`history.filter_source`). Kept when the
    /// screen is left, like `historyIncludesRead`: the chip of the status
    /// bar says it is on (`historySourceChip`).
    var historySource: HistoryDisplay.SourceFilter = .everyone
    /// A mail was opened from the History screen: Escape goes back to it.
    var returnsToHistory = false
    /// A page of the screen is being read: scrolling asks for the next one
    /// at each row that appears, and it must be read only once.
    private var loadingMoreRows = false
    /// The latest read of the history from its top: the answer of an earlier
    /// one, overtaken by `u`, is dropped.
    private var historyRequest = 0
    var tags: [TagView] = []
    /// The contact groups, kept by the model: the folder column, the Group…
    /// panel, the `g` picker and the groups screen show the same list.
    var groups: [GroupView] { model.groups }
    /// The groups of the mails the `g` picker aims at, once the core said
    /// (`heldGroups`); `nil` while it has not.
    var pickerGroupLabels: [ThreadLabels]?
    /// Row under the cursor on the current screen.
    var screenIndex = 0
    /// The rows that fit in the list of the screen: `ctrl-d` and `ctrl-u`
    /// move by half of them, as in the list of mails.
    var screenVisibleRows = 20

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

    /// Puts the window in full screen or takes it out (⌃⌘F). Replaced in
    /// the tests and in UI script mode, whose window must never fill the
    /// user's screen.
    @ObservationIgnored var toggleFullScreen: () -> Void = {
        (NSApp.keyWindow ?? NSApp.mainWindow)?.toggleFullScreen(nil)
    }

    /// The view of the reply's text: the editor's web view
    /// (`EditorBridge`), or an empty view with a fake editor (the tests).
    var editorView: NSView {
        if let bridge = model.editor as? EditorBridge { return bridge.webView }
        return placeholderEditorView
    }
    @ObservationIgnored private lazy var placeholderEditorView = NSView()

    /// The editor of the reply's text says what is under its cursor: a
    /// click in the text, while another field had the keys, gives them to
    /// the text.
    /// `⇧⇥` outside a list in the text (the editor's `leave`): the keys go
    /// to the field before the text, as ⇧⇥ does from any field.
    func leaveText() {
        guard let c = model.composer else { return }
        focusCompose(c.field(after: .body, backward: true))
    }

    func editorStateChanged(_ s: EditorStateView) {
        model.editorState = s
        if s.focused, let c = model.composer, c.focus != .body { focusCompose(.body) }
    }

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
        let key = keymap.mention(.readerToggleAll, in: Keymap.mailChain)
        model.flash = s.showAll ? L("Whole conversation shown (%@)", key) : L("Simplified conversation (%@)", key)
    }

    /// The message the reader scrolls to once laid out (`stepMessage`);
    /// the reader clears it.
    @ObservationIgnored var pendingMessageScroll: Int?
    /// Top of each message's header in the text reader, by message id, set
    /// after layout.
    @ObservationIgnored var readerHeaders: [Int: Double] = [:]

    /// ⌘A: every mail of the list marked. The status bar shows how many,
    /// as for `J` / `K`: no message on top of it.
    func markAll() {
        model.marked = Set(model.threads.map(\.id))
        model.flash = model.marked.isEmpty ? L("No mail") : nil
    }

    /// `ctrl-k` / `ctrl-j`: up to the previous message of the conversation,
    /// or down to the next (`ThreadDisplay.step`); the reader scrolls to
    /// its header. Stops at either end, and says so.
    func stepMessage(_ delta: Int) {
        guard let thread = model.openThread else { return }
        guard thread.messages.count > 1 else {
            model.flash = L("A single mail: no other message")
            return
        }
        guard let (state, message) = ThreadDisplay.step(thread, readerState, by: delta) else {
            model.flash = delta < 0 ? L("First message of the conversation") : L("Last message of the conversation")
            return
        }
        readerState = state
        // Scrolled now if the reader is laid out already (`T`: nothing
        // folds), and again once laid out anew.
        pendingMessageScroll = message
        scrollToMessage(message)
    }

    /// Scrolls the reader to the header of `message`.
    func scrollToMessage(_ message: Int) {
        if model.bodyFormat != "text", let web = readerWeb {
            web.evaluateJavaScript("document.getElementById('nm-message-\(message)')?.scrollIntoView()")
            return
        }
        guard let y = readerHeaders[message], let scroll = readerScroll, let doc = scroll.documentView else { return }
        let clip = scroll.contentView
        let maxY = max(0, doc.frame.height - clip.bounds.height)
        clip.scroll(to: NSPoint(x: 0, y: min(maxY, max(0, CGFloat(y) - 8))))
        scroll.reflectScrolledClipView(clip)
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
    func readerLink(_ url: URL, command: Bool? = nil) -> Bool {
        guard url.scheme == "nm" else { return false }
        // ⌘ held: given by the HTML reader (its navigation action knows), or
        // read from the event being handled (the text reader; no app,
        // hence no event, in unit tests).
        let command = command ?? (NSApp?.currentEvent?.modifierFlags.contains(.command) == true)
        let parts = url.pathComponents.filter { $0 != "/" }
        var s = readerState
        switch url.host {
        case "fold":
            if let id = parts.first.flatMap(Int.init),
               let m = model.openThread?.messages.first(where: { $0.id == id }) {
                let unfolded = s.folding[id] ?? m.expanded
                s.folding[id] = !unfolded
                // The message clicked is where ctrl-k / ctrl-j go on from.
                s.current = id
            }
        case "reveal":
            if parts.count == 2 { s.revealed.insert("\(parts[0]):\(parts[1])") }
        case "attachment":
            // A tile: saved to Downloads, and opened with ⌘ (the click's own
            // modifiers, read from the event being handled; no app, hence
            // no event, in unit tests).
            if let id = parts.first.flatMap(Int.init) {
                model.saveAttachment(id, command ? .saveAndOpen : .save)
            }
            return true
        case "sender":
            if let id = parts.first.flatMap(Int.init),
               let address = model.openThread?.messages.first(where: { $0.id == id })?.fromAddr {
                senderClicked(address, command: command)
            }
            return true
        case "book":
            // The address book's icon: what `ctrl-c` does, for that message.
            if let id = parts.first.flatMap(Int.init),
               let m = model.openThread?.messages.first(where: { $0.id == id }) {
                Task { await cardOrCopy(m) }
            }
            return true
        default:
            return false
        }
        readerState = s
        return true
    }

    /// A click on the sender's address in the reader (`ui.md`,
    /// "Expéditeur"): copied to the pasteboard; with ⌘, the list filtered
    /// on this sender (a "from:" chip, as `F` would add).
    func senderClicked(_ address: String, command: Bool) {
        if command {
            model.attributes.add(.from, address)
            model.flash = L("Mails from %@", address)
        } else {
            pasteboard.clearContents()
            pasteboard.setString(address, forType: .string)
            model.flash = L("Copied: %@", address)
        }
    }

    /// The message `ctrl-c` and `ctrl-C` act on (`ThreadDisplay.senderMessage`).
    private var senderMessage: MessageView? {
        model.openThread.flatMap { ThreadDisplay.senderMessage($0, readerState) }
    }

    /// `ctrl-c` (or a click on the address book's icon): opens the card of
    /// the sender of `message` in Contacts when the address book holds it;
    /// otherwise copies the address, to create the card or write to it
    /// (`ui.md`, "Expéditeur"). An address book macOS has not asked about
    /// yet is asked now, as `l` would: the answer decides between the two.
    /// A book that cannot be read copies.
    func cardOrCopy(_ message: MessageView? = nil) async {
        guard let m = message ?? senderMessage, let address = m.fromAddr, !address.isEmpty else {
            model.flash = L("No sender to look up")
            return
        }
        if model.addressBook.access == .notDetermined { await model.loadSenderCards(asking: true) }
        if let card = model.senderCard(address), let url = ContactLinks.cardURL(card.id) {
            model.openFile(url)
            model.flash = L("Card of %@ opened in Contacts", card.name)
        } else {
            pasteboard.clearContents()
            pasteboard.setString(address, forType: .string)
            model.flash = L("Copied: %@", address)
        }
    }

    /// `ctrl-C`: the mails of the sender of the current message, from every
    /// folder, as Return on the contacts screen lists them (a "contact:"
    /// list, Escape goes back). The core knows every sender of a mail it
    /// holds: the contact is the row of `list_contacts` with this very
    /// address (the query also matches longer addresses and names).
    func listSenderMails() async {
        guard let m = senderMessage, let address = m.fromAddr?.lowercased(), !address.isEmpty,
              let client = model.core else {
            model.flash = L("No sender to look up")
            return
        }
        do {
            let page: Page<ContactView> = try await client.call("list_contacts", ["query": address, "limit": 50])
            guard let contact = page.items.first(where: { $0.addr.lowercased() == address }) else {
                model.flash = L("No contact %@", address)
                return
            }
            // One column: back to the list, which now shows these mails.
            reading = false
            model.showThreads(of: .contact(contact))
        } catch {
            model.report(error)
        }
    }

    /// Last row clicked and when, to detect double-clicks (see `clickSearch`).
    @ObservationIgnored var lastClick: (tag: String, time: TimeInterval) = ("", 0)

    /// The text sizes ⌘+ / ⌘− / ⌘0 change: the model's (a separate one in
    /// tests).
    var textSize: TextSize { model.textSize }

    init(model: AppModel, defaults: UserDefaults = AppDefaults.store, textSize: TextSize? = nil) {
        self.model = model
        self.defaults = defaults
        if let textSize { model.textSize = textSize }
        columns = defaults.object(forKey: "geek.columns") as? Int ?? 2
        // At the top by default, under the title bar; with the title bar
        // hidden (`ui.title_bar`), it takes its place, and moves the window.
        statusPosition = StatusPosition(rawValue: defaults.string(forKey: "geek.status") ?? "") ?? .top
        listFraction = defaults.object(forKey: "geek.listFraction") as? Double ?? 0.32
        // Swift note: `didSet` does not run for assignments made in `init`,
        // so the model is told here about the remembered layout.
        model.readerVisible = readerVisible
        // Swift note: `[weak self]` keeps the model from holding the
        // controller alive (the controller already holds the model).
        model.onDeepLinkOpened = { [weak self] in self?.showDeepLinkedMail() }
    }

    // MARK: - Labels

    /// Short mode name for the status bar, like Helix's "NOR".
    var modeLabel: String {
        // A suspect move, or a mailbox to upgrade, asks without an
        // overlay of its own.
        if model.suspectMove != nil || model.pendingUpgrade != nil { return "ASK" }
        // The "edition" context of `ui.md`: the keys are text.
        if model.composer != nil, screen == .mail, overlay == .none { return "EDT" }
        return switch overlay {
        case .none, .help: "NOR"
        case .menu(let p): menuKeyLabel(p)
        case .picker: "PCK"
        case .contactFilter: "/"
        case .choice: "ASK"
        }
    }

    /// Whether the status bar shows the app's icon in place of the mode:
    /// in the normal mode (NOR), which needs no reminder. The other modes
    /// (EDT, a prefix waiting for its key, PCK, ASK) stay written: they say
    /// what the next key will do.
    var showsAppIcon: Bool { modeLabel == "NOR" }

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

    /// The keys in effect: the defaults with `keymap.toml` applied
    /// (`AppModel.keymap`, `docs/keymap.md`).
    var keymap: Keymap { model.keymap }

    /// The contexts a key is looked for in, most precise first, for what
    /// has the keys now (`raccourcis.md`, section 5.1). The first context
    /// that has the key decides; `everywhere` comes last in every chain.
    ///
    /// A line of a reply (the subject, a recipient line) looks in
    /// `line_edit`, then in `compose`: ⌘V pastes on one line there, and
    /// ⌘↩ still sends.
    var keyChain: [KeyContext] {
        if activeChoice != nil { return [.choice, .everywhere] }
        if let c = model.composer, screen == .mail, overlay == .none {
            if model.aiRun != nil { return [.aiPanel, .everywhere] }
            switch c.focus {
            case .body, .attachments: return [.compose, .everywhere]
            case .subject, .to, .cc, .bcc: return [.lineEdit, .compose, .everywhere]
            }
        }
        if let run = model.aiRun, run.composer == nil, overlay == .none { return [.aiPanel, .everywhere] }
        switch overlay {
        case .menu: return [.menuNavigation, .everywhere]
        case .help: return [.everywhere]
        case .picker: return [.picker, .lineEdit, .everywhere]
        case .contactFilter: return [.contactFilter, .lineEdit, .everywhere]
        case .choice, .none: break
        }
        switch screen {
        case .mail: return quarantineListed ? [.quarantine, .mail, .normal, .everywhere] : [.mail, .normal, .everywhere]
        case .contacts: return [.contacts, .screen, .normal, .everywhere]
        case .tags: return [.tags, .screen, .normal, .everywhere]
        case .groups: return [.groups, .screen, .normal, .everywhere]
        case .history: return [.history, .screen, .normal, .everywhere]
        }
    }

    /// The chain of the mail screen in normal mode, for the reminders of
    /// the status bar and the help, whatever has the keys now.
    var mailChain: [KeyContext] { quarantineListed ? [.quarantine, .mail, .normal, .everywhere] : [.mail, .normal, .everywhere] }

    /// The key of `action` on the mail screen, as shown (`m`, `⌃I`), or
    /// `""` when no key runs it: the reminders of the status bar.
    func keyLabel(_ action: KeyAction) -> String {
        let chain: [KeyContext] = action == .aiComposeMenu ? [.compose, .everywhere] : mailChain
        return keymap.label(action, in: chain)
    }

    /// Handles one key press. Returns `true` if the key was used (it then
    /// goes no further), `false` to let AppKit process it (the ⌘ keys of
    /// the menus, the keys of the reply's editor).
    ///
    /// The key is spelled as the keymap spells it (`KeyChord`), looked for
    /// in the contexts of what has the keys (`keyChain`), and its action
    /// run (`perform`). What is not an action stays with each panel: the
    /// letters typed in a picker, the answers of a question, the rows of a
    /// menu, and Escape, which always goes back (`raccourcis.md`, 4.4).
    func handle(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let key = KeyChord.of(event)
        let chain = keyChain
        // The reply being written takes every key (`reponse.md`, section
        // 6): the letters are text, not commands.
        if model.composer != nil, screen == .mail, overlay == .none, activeChoice == nil {
            return handleComposeKey(event, key: key, chain: chain, mods: mods)
        }
        let action = keymap.action(key, in: chain)
        // A menu of the user (`[menu.<name>]`), opened by its own key.
        let userMenu = action == nil && overlay == .none && activeChoice == nil ? keymap.userMenu(key, in: chain) : nil
        // A ⌘ key no context takes goes on to the menus of the app (⌘Q,
        // ⌘,…), and leaves the message on screen; but for a move of a line
        // with Shift added (⇧⌘→), which selects.
        if action == nil, userMenu == nil, mods.contains(.command), shiftedLineMove(key, in: chain) == nil { return false }

        // Any key clears the previous message, as in Helix.
        model.flash = nil
        model.lastError = nil

        // A question waits for its answer before anything else.
        if let choice = activeChoice {
            handleChoiceKey(choice, key: key, action: action, code: event.keyCode)
            return true
        }
        // The AI panel of a mail takes every key while it is open, as the
        // one of a reply does (`AI.action`).
        if let run = model.aiRun, run.composer == nil, overlay == .none {
            handleAIKey(event, key: key, action: action, run: run)
            return true
        }
        switch overlay {
        case .menu(let prefix):
            handleMenuKey(prefix, key: key, action: action, code: event.keyCode)
            return true
        case .help:
            // A key of `everywhere` (the text size, full screen) keeps the
            // list open; any other closes it.
            if let action, perform(action) { return true }
            overlay = .none
            return true
        case .picker(let kind):
            handlePickerKey(kind, event: event, key: key, action: action, mods: mods)
            return true
        case .contactFilter:
            handleContactFilterInput(event: event, key: key, action: action, mods: mods)
            return true
        case .choice, .none:
            break
        }

        if let action, perform(action) { return true }
        if let userMenu {
            overlay = .menu(.user(userMenu))
            return true
        }
        if event.keyCode == KeyCode.escape {
            if screen == .mail, returnsToHistory {
                // Back to the row left, to go on correcting (`ui.md`).
                returnsToHistory = false
                reading = false
                screen = .history
            } else if screen != .mail {
                screen = .mail
            } else {
                mailEscape()
            }
            return true
        }
        // A ⌃ key without an action: AppKit's (a beep), as before.
        if mods.contains(.control) { return false }
        if screen == .mail, !key.isEmpty { model.flash = L("No command %@", KeyChord.display(key)) }
        return true
    }

    /// Escape on the mail screen: undoes the last thing started or shown,
    /// one step per press (`ui.md`). Not configurable: Escape always goes
    /// back.
    private func mailEscape() {
        // A sorting under way stops first: it is the last thing started.
        if model.aiSort != nil {
            Task { await model.stopSort() }
        } else if reading {
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
    }

    /// Runs an action of the keymap. Returns `false` when it does nothing
    /// here, so the key goes on as if it had no action: a ⌘ key then
    /// reaches the menus.
    ///
    /// The actions of the panels (a picker, a menu, a question, the AI
    /// panel, a line, a reply) are run by their own handlers, which know
    /// the panel's state; reaching them here means the panel is not open.
    @discardableResult
    func perform(_ action: KeyAction) -> Bool {
        // In Quarantine, the only decision is where the mail goes: the
        // actions that change it otherwise say why they do nothing
        // (`quarantineRefused`).
        if quarantineListed, screen == .mail, overlay == .none, action.refusedInQuarantine {
            model.flash = L("In Quarantine, a mail can only be moved or flagged")
            return true
        }
        if let role = action.moveRole {
            moveTo(role)
            return true
        }
        if action.gotoRole != nil || action.gotoVirtual != nil {
            goTo(action)
            return true
        }
        if let s = action.gotoScreen {
            open(s)
            return true
        }
        switch action {
        // everywhere
        case .windowFullScreen:
            toggleFullScreen()
        case .textBigger:
            model.changeTextSize(.bigger)
        case .textSmaller:
            model.changeTextSize(.smaller)
        case .textReset:
            model.changeTextSize(.reset)
        case .appReload:
            Task { await model.reloadAll() }
        // normal
        case .menuGoto:
            overlay = .menu(.goto)
        case .helpShow:
            overlay = .help
        case .undoLast:
            // Undo the last move or decision, from any screen.
            Task {
                await model.undoLast()
                if screen == .contacts { await loadContacts(reset: false, keep: true) }
            }
        // mail: the list
        case .listDown: moveCursor(1)
        case .listUp: moveCursor(-1)
        case .listMarkDown: model.markAndMove(1)
        case .listMarkUp: model.markAndMove(-1)
        case .listMarkAll:
            // Every mail of the list as shown now (the rows loaded so far,
            // as far as one can scroll), for a grouped action (`ui.md`,
            // "Liste"); Escape unmarks.
            markAll()
        // As in Zed and vim, ctrl-d / ctrl-u move by half a page in the
        // list the cursor belongs to (j / k move it too).
        case .listHalfPageDown: moveCursor(halfPage)
        case .listHalfPageUp: moveCursor(-halfPage)
        case .messageNext: stepMessage(1)
        case .messagePrevious: stepMessage(-1)
        case .mailOpen:
            // A conversation the AI suggested a target for: Return moves it
            // there (`ia.md`, section 11, "Quarantine").
            if model.acceptSuggestion() { return true }
            // In Drafts, Return reopens the draft, as `r` does.
            if model.currentRole == "drafts" {
                startReply()
                return true
            }
            // One column shows the list only: Return reads the mail under
            // the cursor in the whole window (`ui.md`, `v1`). With two or
            // three columns the mail is already shown: nothing to do.
            if columns == 1 { openReader() }
        case .readerOpenOrPageDown:
            // One column, on the list: Space opens the mail, as Return
            // does. With the mail on screen, it scrolls it.
            if columns == 1, !reading { openReader() } else { scrollReader(pages: 1) }
        case .readerPageDown: scrollReader(pages: 1)
        case .readerPageUp: scrollReader(pages: -1)
        case .readerToggleAll: toggleShowAll()
        case .readerSkipQuoted: skipQuoted()
        case .pickerLinks: openPicker(.links)
        // The sender's card in Contacts, or their address copied when they
        // are not in it; the sender's mails (`ui.md`, "Expéditeur").
        case .senderCardOrCopy: Task { await cardOrCopy() }
        case .senderMails: Task { await listSenderMails() }
        case .mailArchive: model.moveTargets(to: "archive")
        case .menuMove:
            if quarantineListed {
                // The menu would only repeat what the status bar shows:
                // say which keys to press instead.
                let letters = directTargets.map { keyLabel(KeyAction(rawValue: "move." + $0) ?? .menuMove) }
                model.flash = L("In Quarantine, press the folder's key: %@", letters.joined(separator: " "))
            } else {
                overlay = .menu(.move)
            }
        case .mailDelete: model.moveTargets(to: "deleted")
        case .mailToggleSeen: model.toggleSeenTargets()
        case .mailMarkSeen: model.markTargetsSeen()
        case .mailToggleFlag: model.toggleFlagTargets()
        case .pickerTag: openPicker(.tag)
        case .pickerGroup: openPicker(.group)
        case .menuSave: overlay = .menu(.save)
        case .mailUnsubscribe:
            // ⌘U by default (`ui.md`, "Désabonnement"): it cannot be undone,
            // and `u` alone toggles read / unread.
            model.unsubscribe()
        // A reply to all (`reponse.md`); to the sender alone; a forward; a
        // new mail.
        case .composeReplyAll: startReply()
        case .composeReplySender: startReply(kind: .replySender)
        case .composeForward: startReply(kind: .forward)
        case .composeNew:
            Task {
                await model.startNewMail()
                if model.composer != nil, columns == 1 { reading = true }
            }
        case .aiMenu:
            // The AI menu of the list, or of the mail read (`ia.md`,
            // section 11); in the reply, `ai.compose_menu`.
            openAI()
        case .pickerSearch: openPicker(.search)
        case .pickerAttributes: openPicker(.attributes)
        case .pickerAccounts: openPicker(.accounts)
        case .menuView: overlay = .menu(.view)
        // The rows of the menus `s` and `v`, also bound to keys of their
        // own when the user wants.
        case .saveClipboard: model.copyText(to: pasteboard, current: readerState.current)
        case .saveEml: export("eml")
        case .saveText: export("txt")
        case .saveTextOpen: export("txt", open: true)
        case .saveMarkdown: export("md")
        case .saveMarkdownOpen: export("md", open: true)
        case .saveAttachments: model.saveAttachmentsOfTargets(includingGraphics: true, current: readerState.current)
        case .saveAttachmentsNoGraphics: model.saveAttachmentsOfTargets(includingGraphics: false, current: readerState.current)
        case .savePdf: exportPDF(.plain)
        case .savePdfOpen: exportPDF(.plain, open: true)
        case .savePdfEvidence: exportPDF(.evidence)
        case .savePdfEvidenceOpen: exportPDF(.evidence, open: true)
        case .viewOneColumn: columns = 1
        case .viewTwoColumns: columns = 2
        case .viewThreeColumns: columns = 3
        case .viewSortReceived: model.sort = "received"
        case .viewSortModified: model.sort = "modified"
        case .viewToggleHtml: model.toggleBodyFormat()
        case .viewToggleRemoteImages: model.toggleRemoteImages()
        case .viewCycleStatusBar: cycleStatusBar()
        // The screens
        case .screenDown:
            screenIndex = min(screenCount - 1, screenIndex + 1)
            loadMoreIfNeeded()
        case .screenUp:
            screenIndex = max(0, screenIndex - 1)
        case .screenHalfPageDown:
            screenIndex = max(0, min(screenCount - 1, screenIndex + Self.halfPage(screenVisibleRows)))
            loadMoreIfNeeded()
        case .screenHalfPageUp:
            screenIndex = max(0, screenIndex - Self.halfPage(screenVisibleRows))
        case .screenOpen:
            activateScreenRow()
        case .historyToggleRead:
            historyIncludesRead.toggle()
            screenIndex = 0
            Task { await loadHistory(reset: true) }
        case .historyFilterSource:
            choiceIndex = nil
            overlay = .choice(.historySource)
        case .contactsFilter:
            overlay = .contactFilter
        case .contactsDecideAddress, .contactsDecideDomain:
            // The decision on the contact's address, or on its domain.
            guard contacts.indices.contains(screenIndex) else { return true }
            let c = contacts[screenIndex]
            let domain = action == .contactsDecideDomain
            guard let name = domain ? Self.domain(of: c.addr) : c.addr else { return true }
            choiceIndex = nil
            overlay = .choice(.decision(name: name, domain: domain, current: domain ? c.domainTarget : c.screeningTarget))
        case .contactsSenderKind:
            // A person or a robot.
            guard contacts.indices.contains(screenIndex) else { return true }
            let c = contacts[screenIndex]
            choiceIndex = nil
            overlay = .choice(.senderKind(addressId: c.id, addr: c.addr, current: c.senderKind, forced: c.forced))
        case .labelRename, .labelDelete:
            // Rename, or delete, the tag or group under the cursor (`ui.md`,
            // "Écrans Tags et Groupes"); nothing on the other screens.
            guard let label = screenLabel else { return true }
            if action == .labelRename {
                openRename(label.kind, name: label.name, id: label.id)
            } else {
                choiceIndex = nil
                overlay = .choice(.deleteLabel(label.kind, name: label.name, id: label.id, count: label.count))
            }
        // The panels and the reply run their own actions (see above).
        default:
            return false
        }
        return true
    }

    /// `move.<role>`, from a key of `[mail]`, `[quarantine]` or a menu: the
    /// marked mails (or the one under the cursor) go there, if the core
    /// allows it from their folder (`move_targets`, `dossiers.md`).
    func moveTo(_ role: String) {
        let folderId = model.selectedThread?.folderId ?? model.selectedFolderId
        let targets = model.folders.first { $0.id == folderId }?.moveTargets ?? []
        guard targets.contains(role) else {
            model.flash = L("Cannot move to %@ from this folder", model.folderNames.name(role))
            return
        }
        model.moveTargets(to: role)
    }

    /// `goto.<role>` or a virtual folder: the folder of the view (`;`), if
    /// it has one.
    private func goTo(_ action: KeyAction) {
        let target = scopeFolders.first { f in
            if let v = action.gotoVirtual { return f.virtual == v }
            return f.virtual == nil && f.role == action.gotoRole
        }
        guard let target else {
            model.flash = L("No such folder in this view: %@", label(of: action))
            return
        }
        open(target)
    }

    /// The menu of `prefix` has the keys (`ui.md`): the arrows highlight a
    /// row without closing it, Return chooses it, a row's key runs it at
    /// once.
    private func handleMenuKey(_ prefix: Prefix, key: String, action: KeyAction?, code: UInt16) {
        let entries = menuEntries(prefix)
        let count = entries.count
        switch action {
        case .menuNext?:
            // Down starts on the first row, up on the last, and both turn
            // around (`stepDown`, `stepUp`).
            menuIndex = menuIndex.map { Self.stepDown($0, count: count) } ?? 0
            return
        case .menuPrevious?:
            menuIndex = menuIndex.map { Self.stepUp($0, count: count) } ?? max(0, count - 1)
            return
        case .menuChoose?:
            if let i = menuIndex { chooseMenuEntry(i) } else { overlay = .none }
            return
        case .menuChooseAlt?:
            // The highlighted row's second action (`sT`: save, then open),
            // or its usual one.
            if let i = menuIndex { chooseMenuEntry(i, command: true) } else { overlay = .none }
            return
        default:
            break
        }
        if code == KeyCode.escape {
            overlay = .none
            return
        }
        if let i = entries.firstIndex(where: { ($0.chord ?? $0.key) == key }) {
            chooseMenuEntry(i)
            return
        }
        if let entry = entries.first(where: { $0.altChord == key }), let second = entry.commandAction {
            overlay = .none
            second()
            return
        }
        // A key of `everywhere` (the text size, full screen): the menu
        // stays open.
        if let action, perform(action) { return }
        overlay = .none
        model.flash = L("No command %@%@", menuKeyLabel(prefix), KeyChord.display(key))
    }

    // MARK: - Quarantine

    /// The move targets offered as single keys, without the `m` prefix:
    /// those of Quarantine (`ui.md`, "Quarantine"), empty elsewhere.
    ///
    /// In Quarantine the only decision to take on a mail is where it goes
    /// (`dossiers.md`, "Cibles proposées": Inbox, Newsletter, Paperwork,
    /// Junk, Banned, Deleted), so the status bar shows these targets at its
    /// right end, in place of the `m` and `s` blocks, and `i` does what `mi`
    /// does elsewhere: one key less for the most repeated gesture of the
    /// folder. Their keys are those of `[quarantine]` in the keymap (by
    /// default `i n p x b d`: free in the normal mode, except `d`, which
    /// archives elsewhere; Archive is not a target from Quarantine, so `d`
    /// takes the meaning `md` has there, Deleted, which `z` undoes).
    ///
    /// Only while a Quarantine folder itself is listed (of an account, or of
    /// "all"): not in "Everything" nor in a tag, group or contact list,
    /// where the rows come from several folders and `d` must keep archiving.
    /// The targets are the core's (`move_targets`) for the folder of the mail
    /// under the cursor, as for the `m` menu: no list is written here twice.
    var directTargets: [String] {
        guard screen == .mail, quarantineListed else { return [] }
        let folderId = model.selectedThread?.folderId ?? model.selectedFolderId
        return model.folders.first { $0.id == folderId }?.moveTargets ?? []
    }

    /// Whether a Quarantine folder itself is listed: `[quarantine]` of the
    /// keymap applies (`raccourcis.md`, section 5.1).
    var quarantineListed: Bool { model.listFilter == nil && model.currentRole == "quarantine" }

    /// The actions refused in Quarantine: those that change the mail other
    /// than by moving it or flagging it (`KeyAction.refusedInQuarantine`).
    /// Reading state (viewing never marks read there, so marking by hand
    /// would only blur which mails still wait for a decision), tags and
    /// groups (the sender is not accepted yet), the replies (writing to the
    /// sender would accept it, `dossiers.md`, "Écrire à quelqu'un vaut
    /// décision Inbox", outside the workflow of the folder), the forward (a
    /// mail not sorted yet is not passed on; move it first), and saving (its
    /// block leaves the status bar, the mail is to be sorted, not kept).
    /// What stays: navigation, display, links, the flag, undo. Refused by
    /// action, not by key: a user who moves `u` to another key finds it
    /// refused there too.
    static var quarantineRefused: Set<KeyAction> { Set(KeyAction.allCases.filter(\.refusedInQuarantine)) }

    /// A click on a target of the status bar in Quarantine: the same as its
    /// key.
    func moveFromStatusBar(_ role: String) {
        model.flash = nil
        overlay = .none
        model.moveTargets(to: role)
    }

    /// Copies the link of the open mail (`DeepLink`): as a Markdown link
    /// `[subject](nicemailer://m/<id>)`, ready to paste in a note, or as the
    /// bare URL. The status bar says what was copied.
    func copyMailLink(markdown: Bool) {
        guard let id = model.deepLinkMessageId else { return }
        let text = markdown
            ? DeepLink.markdown(subject: model.openThread?.subject ?? "", messageId: id)
            : DeepLink.url(messageId: id).absoluteString
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        model.flash = markdown ? L("Markdown link copied: %@", text) : L("Link copied: %@", text)
    }

    /// Copies a verification code (the first row of `l`), to paste it in
    /// the site or the app that asked for it.
    func copyVerificationCode(_ code: String) {
        pasteboard.clearContents()
        pasteboard.setString(code, forType: .string)
        model.flash = L("Code %@ copied", code)
    }

    /// A deep link was opened (`AppModel.openDeepLink`): the mail is
    /// selected in its folder; the panels close and, in one column, the
    /// mail fills the window, as after Return.
    func showDeepLinkedMail() {
        // A question waiting (a suspect move) is dropped as by Escape: the
        // mail it was about stays where it is.
        overlay = .none
        screen = .mail
        if columns == 1 { reading = true }
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
        if model.bodyFormat != "text", let web = readerWeb, web.window != nil {
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

    /// Entries of a menu (`ui.md`), from the keymap: its rows in their
    /// order, each with its key and the label of its action
    /// (`raccourcis.md`, section 7).
    func menuEntries(_ prefix: Prefix) -> [Entry] {
        let rows = keymap.menu(prefix.name)?.entries ?? []
        switch prefix {
        case .move:
            // Only the targets allowed from the current thread's folder; the
            // core computes them (`move_targets`). A target without a key
            // in the menu is not offered.
            let folderId = model.selectedThread?.folderId ?? model.selectedFolderId
            let targets = model.folders.first { $0.id == folderId }?.moveTargets ?? []
            return rows.compactMap { row -> Entry? in
                guard let action = KeyAction(rawValue: row.action) else { return nil }
                if let role = action.moveRole, !targets.contains(role) { return nil }
                return entry(row, action)
            }
        case .goto:
            // Folders of the current view ("all" or one account) in blocks
            // (`ui.md`, `folderGroups`), Everything, then the screens:
            // Contacts, Tags, Groups, and History on its own. The folders
            // keep the order of the view; the menu gives their keys.
            var entries: [Entry] = []
            var group: Int?
            var shown: Set<String> = []
            for f in scopeFolders {
                guard let action = KeyAction.goto(role: f.role, virtual: f.virtual),
                      let key = keymap.menuKey("goto", action) else { continue }
                shown.insert(action.rawValue)
                entries.append(Entry(
                    key: KeyChord.display(key), label: f.name,
                    gapBefore: group != nil && group != f.group, detail: f.unread > 0 ? "\(f.unread)" : nil,
                    virtual: f.virtual != nil, chord: key
                ) { [weak self] in self?.open(f) })
                group = f.group
            }
            // The screens, then whatever else the user put in the menu.
            var gap = true
            for row in rows {
                guard let action = KeyAction(rawValue: row.action), !shown.contains(row.action) else { continue }
                // A folder the view does not have is not offered.
                if action.gotoRole != nil || action.gotoVirtual != nil { continue }
                var e = entry(row, action)
                e.gapBefore = gap || action == .gotoHistory
                gap = false
                entries.append(e)
            }
            return entries
        case .save, .view, .user:
            // A `save.*` row and its `_open` variant make one row: the
            // variant's key on its right ("T: and open"), and ⌘-Return on
            // the row runs it (`ui.md`, menu `s`).
            let actions = Set(rows.map(\.action))
            return rows.compactMap { row -> Entry? in
                guard let action = KeyAction(rawValue: row.action) else { return nil }
                if action.isOpenVariant, let base = KeyAction(rawValue: String(row.action.dropLast(5))),
                   actions.contains(base.rawValue) {
                    return nil
                }
                var e = entry(row, action)
                if let variant = action.openVariant, let alt = rows.first(where: { $0.action == variant.rawValue }) {
                    e.detail = L("%@: and open", KeyChord.display(alt.key))
                    e.detailStyle = .info
                    e.commandAction = { [weak self] in self?.perform(variant) }
                    e.altKey = KeyChord.display(alt.key)
                    e.altChord = alt.key
                }
                return e
            }
        }
    }

    /// The row of a menu for a binding of the keymap.
    private func entry(_ row: Keymap.Binding, _ action: KeyAction) -> Entry {
        Entry(key: KeyChord.display(row.key), label: label(of: action), chord: row.key) { [weak self] in
            self?.perform(action)
        }
    }

    /// What a menu row says for `action`: the folder's name for a move or
    /// a "go to" (`FolderNames`, those of `config.toml`), else the action's
    /// own label (`KeyAction.label`, which knows only the English names).
    func label(of action: KeyAction) -> String {
        if let role = action.moveRole ?? action.gotoRole { return model.folderNames.name(role) }
        if let v = action.gotoVirtual { return model.folderNames.name(of: v) }
        return action.label
    }

    /// `se`, `st`, `sm`: the file of the message the reader is on in the
    /// open conversation, as `exportPDF`.
    func export(_ format: String, open: Bool = false) {
        model.export(format, open: open, current: readerState.current)
    }

    /// `sp`, `si`: the PDF of the message the reader is on in the open
    /// conversation (set by `ctrl-k` / `ctrl-j` or a click on a header),
    /// as `r` answers it; the latest when none was chosen.
    func exportPDF(_ kind: MailPDF.Kind, open: Bool = false) {
        model.exportPDF(kind, open: open, current: readerState.current)
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
        /// A virtual folder (Work, All Mail, Everything), shown apart in
        /// the lists (`ui.md`, "Dossiers virtuels").
        var virtual: AppModel.Virtual? = nil
        /// Block of the folder in the lists (`folderGroup`): a virtual
        /// folder ends the block of what it gathers.
        var group: Int {
            switch virtual {
            case .work: 0
            case .allMail: 4
            case .everything: 6
            case nil: GeekController.folderGroup(role)
            }
        }
        var id: String { virtual.map { $0.rawValue } ?? folderId.map { "f\($0)" } ?? "r\(role ?? "")" }
        /// Its key in the menu `;`: the folder's letter in lower case.
        var key: String { letter.lowercased() }
    }

    /// Folders of the current view, in display order. In the unified view,
    /// one entry per role, with the unread counts of every account added up;
    /// folders without a role (user folders) stay per account and are not
    /// listed there.
    var scopeFolders: [ScopeFolder] {
        let names = model.folderNames
        return Self.ordered(realScopeFolders + AppModel.Virtual.allCases.map { Self.virtualFolder($0, names: names) })
    }

    /// Blocks of the folder lists (`ui.md`, "Thème geek"), in order: what
    /// is to deal with, what is filed, what was written, what was thrown
    /// away. Folders of other roles, and the user's own folders, form a
    /// block of their own after the filed ones.
    ///
    /// Swift note: `nonisolated` lets code outside the main actor (here the
    /// `group` property of `ScopeFolder`) use these constant rules.
    /// Quarantine is with the filed folders: what it holds waits for a
    /// decision on its sender, not for work. Each block ends with the
    /// virtual folder that gathers it or more (`virtualFolder`).
    nonisolated static let folderGroups: [[String]] = [
        ["inbox", "todo", "waiting"],
        ["archive", "newsletter", "paperwork", "quarantine"],
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
            // A virtual folder after the folders of its block.
            if f.virtual != nil { return Int.max }
            return f.role.flatMap { r in folderGroups.lazy.compactMap { $0.firstIndex(of: r) }.first } ?? 0
        }
        // Swift note: `enumerated()` pairs each element with its index; the
        // index breaks ties, which makes the sort stable.
        return folders.enumerated().sorted { a, b in
            (a.element.group, rank(a.element), a.offset) < (b.element.group, rank(b.element), b.offset)
        }.map(\.element)
    }

    /// A virtual folder of the view (`ui.md`, "Dossiers virtuels"), at the
    /// end of the block of what it gathers: Work (`o`, Inbox, Todo and
    /// Waiting) after Waiting, All Mail (`m`, all but Deleted, Junk and
    /// Banned, where `/` searches) after Drafts, Everything (`e`) after
    /// Banned. Their letters are free in the menu `;`.
    nonisolated static func virtualFolder(_ v: AppModel.Virtual, names: FolderNames = .english) -> ScopeFolder {
        let letter = switch v {
        case .work: "o"
        case .allMail: "m"
        case .everything: "e"
        }
        return ScopeFolder(folderId: nil, role: nil, name: names.name(of: v), letter: letter, unread: 0, virtual: v)
    }

    private var realScopeFolders: [ScopeFolder] {
        let names = model.folderNames
        if let account = model.scopeAccountId {
            return model.folders.filter { $0.accountId == account }.map {
                ScopeFolder(folderId: $0.id, role: $0.role, name: names.name(of: $0), letter: $0.letter ?? "", unread: $0.unreadCount)
            }
        }
        var out: [ScopeFolder] = []
        for f in model.folders {
            guard let role = f.role else { continue }
            if let i = out.firstIndex(where: { $0.role == role }) {
                let o = out[i]
                out[i] = ScopeFolder(folderId: nil, role: role, name: o.name, letter: o.letter, unread: o.unread + f.unreadCount)
            } else {
                out.append(ScopeFolder(folderId: nil, role: role, name: names.name(role), letter: f.letter ?? "", unread: f.unreadCount))
            }
        }
        return out
    }

    /// Whether `f` is the folder shown.
    func isCurrent(_ f: ScopeFolder) -> Bool {
        guard model.listFilter == nil else { return false }
        if let v = f.virtual { return model.everything != nil && model.virtual == v }
        if model.everything != nil { return false }
        if let id = f.folderId { return id == model.selectedFolderId }
        return f.role == model.unifiedRole
    }

    /// Opens a folder of the current view.
    func open(_ f: ScopeFolder) {
        if let v = f.virtual {
            screen = .mail
            searchOrigin = nil
            model.selectEverything(model.scopeAccountId.map { .account($0) } ?? .all, virtual: v)
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

    // MARK: - Pickers

    private func openPicker(_ kind: PickerKind) {
        // With a single account there is nothing to choose: `@` and a click
        // on the title of the folder column do nothing (`hasSeveralAccounts`).
        if kind == .accounts, !hasSeveralAccounts { return }
        pickerQuery = ""
        pickerIndex = 0
        overlay = .picker(kind)
        switch kind {
        case .tag:
            Task { await loadTags() }
        case .group:
            pickerGroupLabels = nil
            let ids = model.targetThreads.map(\.id)
            Task {
                await loadGroups()
                let labels = await model.threadLabels(ids)
                // Still the picker of the same mails.
                if overlay == .picker(.group), model.targetThreads.map(\.id) == ids { pickerGroupLabels = labels }
            }
        case .links:
            // The senders' cards join the links; on the first `l`, macOS
            // asks whether the app may read Contacts. The rows appear
            // when the cards are read (the panel follows the model).
            Task { await model.loadSenderCards(asking: true) }
        case .accounts, .attributes, .ai, .rename:
            break
        case .attributeValue, .search:
            valueSuggestions = []
        }
    }

    /// Contacts suggested in the From… and To… panels, for the text typed.
    var valueSuggestions: [ContactView] = []

    /// Loads the contacts matching the text typed in a From… or To… panel.
    private func loadSuggestions() {
        // Without the "-" of an exclusion: the contacts are the same.
        let query = ThreadFilter.parse(pickerQuery).value
        guard query.count >= 2, let client = model.core else { valueSuggestions = []; return }
        Task {
            do {
                let page: Page<ContactView> = try await client.call("list_contacts", ["query": query, "limit": 20])
                if query == ThreadFilter.parse(self.pickerQuery).value { valueSuggestions = page.items }
            } catch { model.report(error) }
        }
    }

    /// Opens a prefix menu from a click on its block in the status bar:
    /// the folder (`;`: folders of the view, "Everything", screens), or the
    /// `m`, `s`, `v` icons.
    /// A click on the AI block of the status bar: the AI palette, as `a`
    /// (or ⌘I in a reply) opens it, for the same target (`openAI`). While a
    /// result is shown, its panel keeps the keys and the click does
    /// nothing: a new palette over it would hide what is being read.
    func openAIFromStatusBar() {
        guard model.aiRun == nil, activeChoice == nil else { return }
        model.flash = nil
        overlay = .none
        openAI()
    }

    /// The key the AI block of the status bar reminds of: that of
    /// `ai.compose_menu` while a reply is written (⌃I: a letter alone would
    /// be typed in it), of `ai.menu` elsewhere (`a`).
    func aiKeyLabel(composing: Bool) -> String { keyLabel(composing ? .aiComposeMenu : .aiMenu) }

    /// The key of `action` on the current screen (Contacts, Tags, Groups),
    /// for the hint under its title.
    func screenKey(_ action: KeyAction) -> String {
        let chain: [KeyContext] = switch screen {
        case .contacts: [.contacts, .screen, .normal, .everywhere]
        case .tags: [.tags, .screen, .normal, .everywhere]
        case .groups: [.groups, .screen, .normal, .everywhere]
        case .history: [.history, .screen, .normal, .everywhere]
        default: [.screen, .normal, .everywhere]
        }
        return keymap.mention(action, in: chain)
    }

    /// The key of a folder in the menu `;` (`goto.<role>`), as the folder
    /// column shows it: `i` for Inbox; `" "` for a folder without one.
    func gotoKeyLabel(_ f: ScopeFolder) -> String {
        guard let action = KeyAction.goto(role: f.role, virtual: f.virtual),
              let key = keymap.menuKey("goto", action) else { return " " }
        return KeyChord.display(key)
    }

    /// The key that opens the menu `prefix`, as shown (`m`), or its name
    /// when no key opens it (its block of the status bar still does).
    func menuKeyLabel(_ prefix: Prefix) -> String {
        let label = keymap.label(raw: "menu." + prefix.name, in: mailChain)
        return label.isEmpty ? prefix.name : label
    }

    func openMenuFromStatusBar(_ prefix: Prefix) {
        model.flash = nil
        overlay = .menu(prefix)
    }

    /// A click on a row of the open prefix menu: same as its key; with
    /// `command` (⌘-Return), its second action if it has one.
    func chooseMenuEntry(_ index: Int, command: Bool = false) {
        guard case .menu(let prefix) = overlay else { return }
        overlay = .none
        let entries = menuEntries(prefix)
        guard entries.indices.contains(index) else { return }
        if entries[index].disabled {
            model.flash = L("Not available yet: %@", entries[index].label)
        } else if command, let second = entries[index].commandAction {
            second()
        } else {
            entries[index].action()
        }
    }

    /// Opens a picker from a click in the status bar (`@`, `F`).
    /// A click on the `?` block at the right end of the status bar: opens
    /// the list of keys, or closes it when it is open (the status bar is
    /// not under the click catcher, so the same block closes what it
    /// opened). A question waiting for its answer (a suspect move) keeps
    /// the screen: the help would hide it.
    func toggleHelpFromStatusBar() {
        guard activeChoice == nil else { return }
        model.flash = nil
        overlay = overlay == .help ? .none : .help
    }

    func openPickerFromStatusBar(_ kind: PickerKind) {
        model.flash = nil
        if screen != .mail { screen = .mail }
        openPicker(kind)
    }

    /// More than one account in the mailbox. Only then do the account
    /// picker (`@`), the account block of the status bar and its line in
    /// the help appear: with a single account, choosing one is noise.
    /// Stops at the second account found, so it costs nothing for a mailbox
    /// of many folders (the status bar reads it on every redraw).
    var hasSeveralAccounts: Bool {
        guard let first = model.folders.first?.accountId else { return false }
        return model.folders.contains { $0.accountId != first }
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
        // From "Everything" (or "All Mail"), stay there in the chosen view.
        if model.everything != nil {
            screen = .mail
            searchOrigin = nil
            model.selectEverything(accountId.map { .account($0) } ?? .all, virtual: model.virtual)
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

    /// What can be typed in a value panel, shown in the input line while
    /// nothing is typed (`ui.md`, "Sélecteur d'attributs"): the Date…
    /// panel opens on its presets, so a hint shown only when the panel is
    /// empty was never seen, and typed dates went unnoticed. `nil` for the
    /// pickers whose rows say it all.
    func pickerHint(_ kind: PickerKind) -> String? {
        let ok = keymap.mention(.pickerChoose, in: Keymap.pickerChain)
        return switch kind {
        case .attributeValue(.date), .attributeValue(.mailDate):
            L("or type a date (2026-10-01, %@) or a duration (10d, 3w, 6m, 1y)", DateChoice.Order.ui.examples)
        case .attributeValue(.text): L("words of the subject or body; -word to exclude")
        case .attributeValue(.subject): L("words of the subject; -word to exclude")
        case .attributeValue(.group): L("%@ chooses or removes a group, several allowed; -name to exclude; esc closes", ok)
        case .attributeValue(.tag): L("%@ chooses or removes a tag, several allowed; -name to exclude; esc closes", ok)
        case .attributeValue: L("a name, an address or @domain; -value to exclude")
        case .search: L("words: subject, sender, recipients or body")
        case .ai: L("type to filter, or a prompt's letters · %@ runs the first row", ok)
        case .rename: L("the new name · %@ renames · esc cancels", ok)
        default: nil
        }
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
            // The typed words, searched anywhere in "Everything".
            let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
            guard !typed.isEmpty else { return [] }
            return [Entry(key: "+", label: typed) { [weak self] in self?.searchEverywhere(typed) }]
        case .links:
            // The links of the mail and its files (`ui.md`), after the
            // link to the mail and the senders' cards, in four groups: links with a text, phone numbers (`AppModel.Link.merge`),
            // the attached files, then the links without a text (mostly an
            // image leading to a social network: the least wanted, after
            // the files). Return opens a link in the browser, calls a
            // number with the phone app (macOS opens `tel:` URLs with it;
            // `AppModel.openLink` may also press the confirmation macOS
            // shows first), or saves a file; ⌘-Return saves the file and opens it at once.
            // A link shows its text and, on the right, where it goes; its
            // whole address is a hidden keyword, so typing part of it finds
            // the row too.
            let links = model.links
            // First of all, the verification code of a code mail
            // (`ui.md`, "Codes de vérification"): what the user opened the
            // mail for. Return copies it, ⌘-Return too.
            let codeRow = model.verificationCode.map { code in
                Entry(key: "", label: code, detail: L("Verification code"), detailStyle: .info, icon: "lock.fill",
                      commandAction: { [weak self] in self?.copyVerificationCode(code) },
                      keywords: ["code", "otp", L("Verification code")]) { [weak self] in
                    self?.copyVerificationCode(code)
                }
            }
            // Then the link to this mail itself, for a note app
            // (`DeepLink`): Return copies it as a Markdown link, ⌘-Return as
            // a bare URL.
            let selfRow = model.deepLinkMessageId.map { id in
                Entry(key: "", label: L("Link to this mail"), detail: DeepLink.url(messageId: id).absoluteString,
                      detailStyle: .info, icon: "doc.on.doc",
                      commandAction: { [weak self] in self?.copyMailLink(markdown: false) },
                      keywords: ["nicemailer", "obsidian", "deeplink", "markdown"]) { [weak self] in
                    self?.copyMailLink(markdown: true)
                }
            }
            // Then the senders' numbers and cards, from the address book
            // (`ContactLinks`): who wrote, before what the mail offers.
            let contactRows = model.contactLinks.map { link in
                Entry(key: "", label: link.title, detail: link.destination, detailStyle: .info, icon: link.icon,
                      keywords: [link.url.absoluteString, link.shownNumber ?? "", "contacts"]) { [model] in
                    model.openLink(link.url)
                }
            }
            let linkRows = links.map { link in
                // Typing any of its texts, its address or where it leads
                // finds the row.
                Entry(key: "", label: link.title, detail: link.destination, detailStyle: .info, icon: link.icon,
                      keywords: [link.url.absoluteString] + [link.target].compactMap { $0 } + link.otherTexts) { [model] in
                    model.openLink(link.url)
                }
            }
            let fileRows = model.attachments.map { a in
                // The name on the left, the size in the right half, as the
                // address of a link.
                Entry(key: "", label: AttachmentDisplay.name(a), detail: AttachmentDisplay.size(a.size), detailStyle: .info,
                      icon: AttachmentDisplay.symbol(mimeType: a.mimeType, filename: a.filename),
                      commandAction: { [model] in model.saveAttachment(a.id, .saveAndOpen) }) { [model] in
                    model.saveAttachment(a.id, .save)
                }
            }
            // Which link rows are bare (no text, not a number); each group
            // keeps the order `merge` gave it.
            let bare = links.map { $0.text == nil && !$0.isPhone }
            let ordered = [codeRow, selfRow].compactMap { $0 } + contactRows + zip(linkRows, bare).filter { !$0.1 }.map(\.0) + fileRows
                + zip(linkRows, bare).filter { $0.1 }.map(\.0)
            // Numbered once ordered: the number is the row's place.
            all = ordered.enumerated().map { i, e in
                var e = e
                e.key = "\(i + 1)"
                return e
            }
        case .tag:
            // The tags of every account: a tag belongs to one account
            // (an IMAP keyword of its mailbox), but the user thinks of it
            // by its name, as `;T` does when it lists the mails of every
            // tag of that name. Choosing a tag another account has creates
            // it in the mail's own account (`tag_thread`). Before (November
            // 2026), only the tags of the current mail's account were
            // offered: with several accounts, the list was often empty.
            // A name several accounts share is one row (`LabelChoices`).
            let names = tags.map(\.name)
            all = labelEntries(names, held: model.targetThreads.map(\.tags)) { [model] name, remove in
                model.tagTargets(name, remove: remove)
            }
        case .group:
            all = labelEntries(groups.map(\.name), held: heldGroups) { [model] name, remove in
                model.setSendersGroup(name, remove: remove)
            }
        case .ai:
            all = aiEntries()
        case .rename(let kind, let old, let id):
            return renameEntries(kind, old: old, id: id)
        }
        // Best matches first, so that Return takes the row the user meant
        // (`PickerMatch`); the "+ typed text" row always stays. Once
        // something is typed, the rows are ranked, no longer in their
        // pairs: no line between them.
        let ranked = PickerMatch.filter(all, query: pickerQuery, label: \.label,
                                        keywords: \.keywords, pinned: { $0.key == "+" })
        return pickerQuery.isEmpty ? ranked : ranked.map { var e = $0; e.gapBefore = false; return e }
    }

    /// Rows of the attribute selector with "clear" (`attributeEntries`):
    /// the panel is that tall (`PickerPopup.height`).
    static var attributeRowCount: Int { ThreadFilter.selectorLayout.joined().count + 1 }

    /// Lines between its pairs of rows, "clear" included: the panel is
    /// that much taller (`PickerPopup.gapHeight`).
    static var attributeGapCount: Int { ThreadFilter.selectorLayout.count }

    /// The attribute selector, in the pairs of `ThreadFilter.selectorLayout`,
    /// a line between two pairs (`gapBefore`): each value of a yes / no
    /// attribute (an active one is marked "•", choosing it again removes
    /// it), and the attributes with a value, which expand the panel; then
    /// "clear". Every row starts with a capital.
    private var attributeEntries: [Entry] {
        let a = model.attributes
        var entries: [Entry] = []
        for (n, pair) in ThreadFilter.selectorLayout.enumerated() {
            for (i, row) in pair.enumerated() {
                var e = attributeEntry(row, a)
                e.gapBefore = n > 0 && i == 0
                entries.append(e)
            }
        }
        if !a.isEmpty {
            entries.append(Entry(key: "×", label: ThreadFilter.capitalizedFirst(L("clear all attributes")), gapBefore: true) {
                [model] in model.attributes = ThreadFilter()
            })
        }
        return entries
    }

    /// One row of the attribute selector.
    private func attributeEntry(_ row: ThreadFilter.SelectorRow, _ a: ThreadFilter) -> Entry {
        switch row {
        case .flag(let flag, let value):
            return Entry(key: a.value(flag) == value ? "•" : " ",
                         label: ThreadFilter.capitalizedFirst(ThreadFilter.label(flag, value)),
                         keywords: ThreadFilter.keywords(flag, value)) {
                [model] in model.attributes.toggle(flag, value)
            }
        case .field(let field):
            return Entry(key: a.isActive(field) ? "•" : " ", label: field.title, keywords: field.keywords) { [weak self] in
                // The panel grows at once to choose the value (`ui.md`).
                // Text… and Subject… open on their current words, selected:
                // typing replaces them (`ThreadFilter.add`).
                self?.pickerLine = LineEdit(a.words(of: field) ?? "", selected: true)
                self?.pickerIndex = 0
                self?.valueSuggestions = []
                self?.overlay = .picker(.attributeValue(field))
                // The groups may have changed elsewhere (the CLI, the MCP
                // agent): the panel shows them as loaded, then as read.
                if field == .group { Task { await self?.loadGroups() } }
                if field == .tag { Task { await self?.loadTags() } }
            }
        }
    }

    /// Rows of the expanded panel of `field` (`ui.md`, "Négation"): the
    /// typed value to add ("+"), then the same value to exclude ("−",
    /// one arrow down), or only the exclusion when the value was typed
    /// with a leading `-`. From / To add the contacts suggested for the
    /// value, wanted or excluded the same way.
    private func valueEntries(_ field: AttributeField) -> [Entry] {
        let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
        let (value, excluded) = ThreadFilter.parse(typed)
        // A value starting with "@" is a domain: say so on the row.
        let detail = value.hasPrefix("@") && value.count > 1 ? L("domain") : nil
        func rows(_ label: String, _ value: String, key wanted: String = "+", detail: String? = nil) -> [Entry] {
            let add = Entry(key: wanted, label: label, detail: detail, detailStyle: .info) { [model] in
                model.attributes.add(field, value)
            }
            let exclude = Entry(key: "−", label: L("not %@", label), detail: detail, detailStyle: .info) { [model] in
                model.attributes.add(field, value, excluded: true)
            }
            return excluded ? [exclude] : [add, exclude]
        }
        switch field {
        case .group:
            return groupEntries()
        case .tag:
            return tagEntries()
        case .from, .to:
            var entries: [Entry] = []
            if !value.isEmpty { entries += rows(value, value, detail: detail) }
            for c in valueSuggestions {
                let label = c.name.map { "\($0) <\(c.addr)>" } ?? c.addr
                // One row per contact: wanted, or excluded when "-" was
                // typed (the first of its rows).
                entries.append(rows(label, c.addr, key: " ")[0])
            }
            return entries
        case .text, .subject:
            // The words erased: Return removes the criterion.
            if value.isEmpty, let current = model.attributes.chips.first(where: { $0.id == field.rawValue }) {
                return [Entry(key: "×", label: L("remove %@", current.label)) { [model] in
                    model.attributes.clearWords(of: field)
                }]
            }
            guard !value.isEmpty else { return [] }
            // The words as typed: a "-word" among them stays an exclusion.
            return rows(value, value)
        case .date, .mailDate:
            // A typed date or duration first, then the presets (all of
            // them below a typed date, otherwise those whose label
            // matches).
            let fromTyped = DateChoice.typed(typed)
            let presets = fromTyped + DateChoice.presets().filter {
                typed.isEmpty || !fromTyped.isEmpty
                    || $0.label.range(of: typed, options: [.caseInsensitive, .diacriticInsensitive]) != nil
            }
            // Each row sets one bound and keeps the other (an interval).
            // A row that would leave no day between them says so in its
            // detail instead of how it was read: the list would be empty.
            return presets.map { p in
                // How the date was read is information; an empty interval
                // is a warning, shown as such.
                let empty = model.attributes.dateIsEmpty(with: p, on: field)
                return Entry(key: " ", label: p.label, detail: empty ? L("nothing in between") : p.detail,
                             detailStyle: empty ? .count : .info) { [model] in model.attributes.setDate(p, on: field) }
            }
        }
    }

    /// Rows of the Group… panel (`ui.md`, "Sélecteur d'attributs"): every
    /// group, those already chosen first ("•" wanted, "−" excluded), then
    /// the others, most relevant first (`GroupRanking.ranked`), each with
    /// its number of members. Typing narrows them by name; a leading `-`
    /// turns the rows into exclusions ("not VIP"), as in From….
    ///
    /// Return chooses the group, or unchooses it, and the panel **stays
    /// open** (`chooseGroup`): one filters on one group or several, and the
    /// list behind it follows each choice. Escape closes it.
    private func groupEntries() -> [Entry] {
        let (value, excluded) = ThreadFilter.parse(pickerQuery)
        guard !groups.isEmpty else {
            return [Entry(key: " ", label: L("No group yet: g adds the sender of a mail to one"), disabled: true) {}]
        }
        let a = model.attributes
        let ranked = GroupRanking.ranked(groups)
        let ordered = ranked.filter { a.groupChoice($0.id) != nil } + ranked.filter { a.groupChoice($0.id) == nil }
        let entries = ordered.map { g in
            let ref = ThreadFilter.GroupRef(id: g.id, name: g.name)
            let key = switch a.groupChoice(g.id) {
            case true?: "•"
            case false?: "−"
            case nil: " "
            }
            return Entry(key: key, label: excluded ? L("not %@", g.name) : g.name,
                         detail: L("%lld members", g.memberCount), detailStyle: .info,
                         keywords: [g.name]) { [weak self] in self?.chooseGroup(ref, excluded: excluded) }
        }
        // "without group": last, so that Return on the first row still
        // takes the most relevant group; first once chosen, as the chosen
        // groups. Not among the exclusions ("-" typed): it is one already.
        let none = Entry(key: a.noGroup ? "•" : " ", label: L("without group"), keywords: L("none no").split(separator: " ").map(String.init)) {
            [weak self] in self?.chooseNone(.group)
        }
        let rows = excluded ? entries : a.noGroup ? [none] + entries : entries + [none]
        return PickerMatch.filter(rows, query: value, label: \.label, keywords: \.keywords)
    }

    /// "without group" or "without tag" chosen in its panel: as a group or
    /// a tag, the panel opens again on its row.
    private func chooseNone(_ field: AttributeField) {
        if field == .group { model.attributes.toggleNoGroup() } else { model.attributes.toggleNoTag() }
        pickerLine = LineEdit("")
        overlay = .picker(.attributeValue(field))
        let label = field == .group ? L("without group") : L("without tag")
        pickerIndex = pickerEntries(.attributeValue(field)).firstIndex { $0.label == label } ?? 0
    }

    /// A row of the Group… panel chosen: the group is added to the
    /// attributes, or removed, and the panel opens again, its text cleared
    /// and the cursor on that group, ready for the next one.
    private func chooseGroup(_ group: ThreadFilter.GroupRef, excluded: Bool) {
        model.attributes.toggleGroup(group, excluded: excluded)
        pickerLine = LineEdit("")
        overlay = .picker(.attributeValue(.group))
        pickerIndex = pickerEntries(.attributeValue(.group)).firstIndex { $0.keywords == [group.name] } ?? 0
    }

    /// Rows of the Tag… panel, as those of Group…: every tag, one row per
    /// name (the tags of that name in every account are one criterion),
    /// those chosen first ("•" wanted, "−" excluded), then the others,
    /// most used first, each with its number of mails. Typing narrows them;
    /// a leading `-` turns them into exclusions. Return chooses or
    /// unchooses one and the panel stays open (`chooseTag`).
    private func tagEntries() -> [Entry] {
        let (value, excluded) = ThreadFilter.parse(pickerQuery)
        let a = model.attributes
        // One row per name, its mails counted in every account; the names
        // chosen stay listed even when no tag has them any more, so that
        // they can be removed here too.
        var counts: [String: Int] = [:]
        var names: [String] = []
        for t in tags {
            let key = t.name.lowercased()
            if counts[key] == nil { names.append(t.name) }
            counts[key, default: 0] += t.messageCount
        }
        for n in a.tags + a.notTags where counts[n.lowercased()] == nil {
            counts[n.lowercased()] = 0
            names.append(n)
        }
        guard !names.isEmpty else {
            return [Entry(key: " ", label: L("No tag yet: t tags the mail under the cursor"), disabled: true) {}]
        }
        // Swift note: `sorted(by:)` is stable, so equal counts keep the
        // order of `list_tags`.
        let ranked = names.sorted { counts[$0.lowercased()]! > counts[$1.lowercased()]! }
        let ordered = ranked.filter { a.tagChoice($0) != nil } + ranked.filter { a.tagChoice($0) == nil }
        let entries = ordered.map { name in
            let key = switch a.tagChoice(name) {
            case true?: "•"
            case false?: "−"
            case nil: " "
            }
            let n = counts[name.lowercased()] ?? 0
            return Entry(key: key, label: excluded ? L("not %@", name) : name,
                         detail: L("%lld mails", n), detailStyle: .info,
                         keywords: [name]) { [weak self] in self?.chooseTag(name, excluded: excluded) }
        }
        let none = Entry(key: a.noTag ? "•" : " ", label: L("without tag"), keywords: L("none no").split(separator: " ").map(String.init)) {
            [weak self] in self?.chooseNone(.tag)
        }
        // Placed as "without group" (`groupEntries`).
        let rows = excluded ? entries : a.noTag ? [none] + entries : entries + [none]
        return PickerMatch.filter(rows, query: value, label: \.label, keywords: \.keywords)
    }

    /// A row of the Tag… panel chosen: as `chooseGroup`, the panel opens
    /// again on that tag, ready for the next one.
    private func chooseTag(_ name: String, excluded: Bool) {
        model.attributes.toggleTag(name, excluded: excluded)
        pickerLine = LineEdit("")
        overlay = .picker(.attributeValue(.tag))
        pickerIndex = pickerEntries(.attributeValue(.tag)).firstIndex { $0.keywords == [name] } ?? 0
    }

    /// A click on a group of the folder column (`ui.md`, "Groupes dans la
    /// colonne des dossiers"): the current list is narrowed to the mails of
    /// its members, or no longer, the same attribute as Group… (its chip in
    /// the status bar, removed by a click or Escape). ⌘-click excludes the
    /// group instead.
    func clickGroup(_ g: GroupView, _ mods: ClickModifiers) {
        model.flash = nil
        overlay = .none
        model.attributes.toggleGroup(ThreadFilter.GroupRef(id: g.id, name: g.name), excluded: mods == .command)
    }

    /// The Group… panel opened directly: the "Groups" title of the folder
    /// column, or its "N more…" row when some groups had no room.
    func openGroupPanel() {
        model.flash = nil
        if screen != .mail { screen = .mail }
        pickerLine = LineEdit("")
        pickerIndex = 0
        valueSuggestions = []
        overlay = .picker(.attributeValue(.group))
        Task { await loadGroups() }
    }

    /// What the folder column shows under the folders: the most relevant
    /// groups (`GroupRanking.shown`), those filtering the list always
    /// among them, and how many are left out.
    /// Whether a group of the folder column has the grey background of the
    /// current folder (`ui.md`, "Groupes dans la colonne des dossiers"): it
    /// filters the list, wanted or excluded, whether chosen by a click or
    /// through the Group… panel of `f`. Both kinds shape what the list
    /// shows, so both stand out like the folder does; the marker and the
    /// strikethrough still tell them apart.
    func isHighlighted(_ g: GroupView) -> Bool {
        model.attributes.groupChoice(g.id) != nil
    }

    var columnGroups: (rows: [GroupView], hidden: Int) {
        let a = model.attributes
        return GroupRanking.shown(groups, active: Set((a.groups + a.notGroups).map(\.id)))
    }

    /// The rows of `t` and `g` (`LabelChoices`): "remove X" for the names
    /// some aimed mails have, "X" for the names to add, plus "+ <typed
    /// text>" to create a new one when the text matches no existing name.
    /// With several mails, a row says how many of them have its name.
    private func labelEntries(_ names: [String], held: [[String]],
                              _ apply: @escaping @MainActor (String, Bool) -> Void) -> [Entry] {
        let several = held.count > 1
        var entries = LabelChoices.rows(names: names, held: held).map { row in
            Entry(key: row.remove ? "−" : " ",
                  label: row.remove ? L("remove %@", row.name) : row.name,
                  detail: several && row.held > 0 ? L("%lld of %lld", row.held, held.count) : nil,
                  detailStyle: .info,
                  // Typing the name finds both rows, in any language.
                  keywords: [row.name]) { apply(row.name, row.remove) }
        }
        let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
        let all = names + held.flatMap { $0 }
        if !typed.isEmpty, !all.contains(where: { $0.compare(typed, options: [.caseInsensitive]) == .orderedSame }) {
            entries.insert(Entry(key: "+", label: typed) { apply(typed, false) }, at: 0)
        }
        return entries
    }

    /// The groups of the person of each mail `g` aims at: read from the
    /// core when the picker opens (`thread_labels`). Until it answers, the
    /// open thread's header gives them when it is the only mail aimed at,
    /// so that its removal rows show at once.
    var heldGroups: [[String]] {
        let targets = model.targetThreads
        if let labels = pickerGroupLabels {
            let byThread = Dictionary(labels.map { ($0.threadId, $0.groups) }) { first, _ in first }
            return targets.map { byThread[$0.id] ?? [] }
        }
        if targets.count == 1, let open = model.openThread, open.id == targets[0].id {
            return [open.labels.groups]
        }
        return targets.map { _ in [] }
    }

    private func handlePickerKey(_ kind: PickerKind, event: NSEvent, key: String, action: KeyAction?,
                                 mods: NSEvent.ModifierFlags) {
        let entries = pickerEntries(kind)
        // Moving in the text and deleting (`LineEdit`, `editLine`).
        if let change = editLine(&pickerLine, event: event, key: key) {
            if change == .text { pickerTextChanged(kind) }
            return
        }
        switch action {
        // Helix's picker keys by default: ↓ ↑, ctrl-n / ctrl-p (and
        // ctrl-j / ctrl-k).
        case .pickerNext?:
            pickerIndex = Self.stepDown(pickerIndex, count: entries.count)
            return
        case .pickerPrevious?:
            pickerIndex = Self.stepUp(pickerIndex, count: entries.count)
            return
        case .pickerChoose?:
            overlay = .none
            if entries.indices.contains(pickerIndex) { entries[pickerIndex].action() }
            return
        case .pickerChooseAlt?:
            // The row's command action (a file of `l`: saved and opened),
            // or its usual one.
            overlay = .none
            if entries.indices.contains(pickerIndex) {
                let e = entries[pickerIndex]
                (e.commandAction ?? e.action)()
            }
            return
        case let a?:
            // A key of `everywhere` (the text size, full screen).
            if perform(a) { return }
        case nil:
            break
        }
        if event.keyCode == KeyCode.escape {
            overlay = .none
            return
        }
        guard !mods.contains(.control), !mods.contains(.command), let text = event.characters,
              LineEdit.isTypable(text) else { return }
        pickerLine.insert(text)
        pickerTextChanged(kind)
    }

    /// After the text of a picker changed: the best match is on top again,
    /// and the From… and To… panels look for contacts.
    private func pickerTextChanged(_ kind: PickerKind) {
        pickerIndex = 0
        if kind == .attributeValue(.from) || kind == .attributeValue(.to) { loadSuggestions() }
    }

    // MARK: - Reply

    /// `r`: the reply to all to the mail under the reader's cursor. One
    /// column shows it in the whole window, as the mail.
    func startReply(kind: DraftKind = .replyAll) {
        guard model.openThread != nil else {
            model.flash = kind == .forward ? L("No mail to forward") : L("No mail to answer")
            return
        }
        let current = readerState.current
        Task {
            await model.startReply(current: current, kind: kind)
            if model.composer != nil, columns == 1 { reading = true }
        }
    }

    /// A key while the reply is written (`reponse.md`, section 6): the
    /// actions of `[compose]` (⌘↩ sends, ⌘⎋ abandons the draft…), Escape
    /// goes back to the mail and keeps the draft; the other keys edit the
    /// text, with the shortcuts of every macOS text view. Returns `false`
    /// for the keys the editor of the text takes, and for the ⌘ shortcuts
    /// of the menus (⌘Q…), which go on as usual.
    private func handleComposeKey(_ event: NSEvent, key: String, chain: [KeyContext],
                                  mods: NSEvent.ModifierFlags) -> Bool {
        let code = event.keyCode
        model.flash = nil
        model.lastError = nil
        let action = keymap.action(key, in: chain)
        // The AI panel takes every key while it is open (`AI.action`).
        if let run = model.aiRun {
            handleAIKey(event, key: key, action: action, run: run)
            return true
        }
        guard let c = model.composer else { return true }
        switch action {
        case .aiComposeMenu?:
            // The AI menu of the reply (`ia.md`, section 11; `edition.md`,
            // section 7): ⌃I by default, ⌘I being the Mac's italic.
            openAI()
            return true
        case .composeAbandon?:
            // ⌘⎋ by default: ⌘⌫ is the Mac's "delete to the start of the
            // line", in the editor; ⌥⎋ is its "Speak selection".
            Task { await model.discardComposer() }
            return true
        case .composeTogglePlain?:
            // ⇧⌘T: text only ↔ formatted, as "Make Plain Text" in Mail.
            Task { await model.toggleComposeFormat() }
            return true
        case .composeFileAndSend?:
            // Choose where the conversation goes, then send. Only when
            // there is something to file: a conversation outside the Inbox,
            // Todo and Waiting stays where it is whatever the choice, and a
            // forward files nothing. A new mail files itself (`Compose
            // .offersFiling`, `reponse.md`, section 8).
            let send = keyLabel(.composeSend, in: chain)
            if !Compose.offersFiling(c.draft) {
                model.flash = c.draft.kind == "forward"
                    ? L("A forward files nothing: %@ to send", send)
                    : L("This conversation stays where it is: nothing to choose · %@ to send", send)
            } else {
                choiceIndex = nil
                overlay = .choice(.sendThen)
            }
            return true
        case .composeSend?:
            Task { await model.sendComposer() }
            return true
        case .composeAttach?:
            // ⇧⌘A: attach files (as in Mail).
            Task { await model.chooseAttachments() }
            return true
        case .composeFrom?:
            // ⌥⌘F: the address, and the account, the mail is sent from.
            chooseSender()
            return true
        case .composeBcc?:
            // ⌥⌘B: the Bcc line, shown and focused (as in Mail).
            model.composer?.showsBcc = true
            focusCompose(.bcc)
            return true
        case .composeNextField?, .composePreviousField?:
            // Tab and ⇧Tab go from field to field (`Composer.fields`); in
            // the text, they are the editor's: an indent in a list, ⇧⇥
            // elsewhere back to the subject (the editor's `leave`).
            if c.focus != .body {
                focusCompose(c.field(after: c.focus, backward: action == .composePreviousField))
                return true
            }
        case .composePaste?, .composePastePlain?:
            guard c.focus == .body else { return true }
            // The editor's page still loading: the key waits for it.
            if model.editor?.keepKeyUntilReady(event) == true { return true }
            return pasteInBody(plain: action == .composePastePlain)
        case let a?:
            // A key of `everywhere`: the text size of the whole interface
            // (never WebKit's zoom), full screen.
            if perform(a) { return true }
        case nil:
            break
        }
        if code == KeyCode.escape {
            // A dialog of the editor (the link, a menu of the bar) has
            // Escape.
            if c.focus == .body, model.editorState.modal { return false }
            // Suggestions shown: Escape closes them, and only them.
            if c.suggestions.isEmpty == false {
                model.composer?.suggestions = []
                model.composer?.suggestionIndex = nil
                return true
            }
            model.leaveComposer()
            return true
        }
        switch c.focus {
        case .subject:
            handleSubjectKey(event, key: key, mods: mods)
            return true
        case .attachments:
            handleAttachmentKey(event)
            return true
        case .to, .cc, .bcc:
            handleRecipientKey(event, key: key, action: action, mods: mods)
            return true
        case .body:
            // The editor's page still loading (just after the launch): its
            // key, kept until it can take it, never lost.
            if model.editor?.keepKeyUntilReady(event) == true { return true }
            return false
        }
    }

    /// `compose.paste` (⌘V) and `compose.paste_plain` (⌥⇧⌘V) in the text:
    /// files copied in the Finder are attached, not pasted as their names;
    /// otherwise the text alone at the style of the cursor, or, for ⌘V,
    /// the editor's own paste (`false`: the key goes to it).
    private func pasteInBody(plain: Bool) -> Bool {
        let files = (pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
        if !files.isEmpty {
            Task { await model.attach(files) }
            return true
        }
        guard plain else { return false }
        model.pastePlain(pasteboard.string(forType: .string) ?? "")
        return true
    }

    /// The key of `action` in `chain`, as shown, for a message.
    private func keyLabel(_ action: KeyAction, in chain: [KeyContext]) -> String {
        keymap.label(action, in: chain)
    }

    /// Moves the keys to another field of the reply. The text typed in the
    /// recipient line left becomes addresses (`AppModel.commitRecipients`).
    func focusCompose(_ field: ComposeField) {
        guard var c = model.composer, c.focus != field else { return }
        let pending = model.takePendingLine()
        c = model.composer ?? c
        c.focus = field
        c.line = LineEdit()
        c.suggestions = []
        c.suggestionIndex = nil
        if field == .bcc { c.showsBcc = true }
        if field == .subject { c.subjectLine = LineEdit(c.headers.subject) }
        let leftText = c.focus != .body && model.composer?.focus == .body
        model.composer = c
        if field == .body { model.editor?.focus() } else if leftText { model.editor?.blur() }
        if let pending { Task { await model.commitRecipients(pending.text, to: pending.field) } }
    }

    /// The button at the end of the Cc line: shows the Bcc line and gives
    /// it the keys, or hides it again when it is empty (`Composer.canHideBcc`).
    /// The keys leaving a line that disappears go to Cc, where the button is.
    func toggleBcc() {
        guard let c = model.composer else { return }
        guard c.showsBcc else {
            focusCompose(.bcc)
            return
        }
        guard c.canHideBcc else { return }
        if c.focus == .bcc { focusCompose(.cc) }
        model.composer?.showsBcc = false
    }

    /// A key in a recipient line (`reponse.md`, section 6): the text typed
    /// with the editing keys of a text field; Return, `,` or `;` make it an
    /// address; ↑ ↓ choose among the suggestions, Return or Tab takes one;
    /// ⌫ on an empty line takes the last address back, selected: a
    /// second ⌫ removes it, typing replaces it, an arrow corrects it. These
    /// are the behaviour of the field, not actions of the keymap
    /// (`raccourcis.md`, section 5.3); the line's actions are
    /// (`[line_edit]`).
    private func handleRecipientKey(_ event: NSEvent, key: String, action: KeyAction?, mods: NSEvent.ModifierFlags) {
        guard var c = model.composer else { return }
        let code = event.keyCode
        let command = mods.contains(.command)
        switch code {
        case KeyCode.down where !c.suggestions.isEmpty && mods.isDisjoint(with: [.command, .option, .control]):
            c.suggestionIndex = c.suggestionIndex.map { Self.stepDown($0, count: c.suggestions.count) } ?? 0
            model.composer = c
            return
        case KeyCode.up where !c.suggestions.isEmpty && mods.isDisjoint(with: [.command, .option, .control]):
            c.suggestionIndex = c.suggestionIndex.map { Self.stepUp($0, count: c.suggestions.count) } ?? c.suggestions.count - 1
            model.composer = c
            return
        case KeyCode.returnKey where !command:
            if let i = c.suggestionIndex {
                model.chooseSuggestion(i)
            } else if c.line.text.trimmingCharacters(in: .whitespaces).isEmpty {
                // Nothing typed: on to the next field.
                focusCompose(c.field(after: c.focus, backward: false))
            } else {
                Task { await model.commitRecipientLine() }
            }
            return
        case KeyCode.delete where !command && c.line.text.isEmpty:
            let field = c.focus
            if c.takeBackLast(of: field) {
                model.composer = c
                model.composerChanged()
                model.suggestRecipients()
            }
            return
        default:
            break
        }
        if action == .linePaste {
            // A pasted list becomes addresses at once.
            c.line.insert(LineEdit.pasted(pasteboard.string(forType: .string) ?? ""))
            model.composer = c
            Task { await model.commitRecipientLine() }
            return
        }
        if let change = editLine(&c.line, event: event, key: key) {
            model.composer = c
            if change == .text { model.suggestRecipients() }
            return
        }
        guard !command, !mods.contains(.control), let text = event.characters, LineEdit.isTypable(text) else { return }
        if Compose.endsAddress(text) {
            Task { await model.commitRecipientLine() }
            return
        }
        c.line.insert(text)
        model.composer = c
        model.suggestRecipients()
    }

    /// A key in the line of files: ← → select one, ⌫ removes it, Return
    /// goes to the text.
    private func handleAttachmentKey(_ event: NSEvent) {
        guard var c = model.composer, !c.attachments.isEmpty else { return }
        switch event.keyCode {
        case KeyCode.left:
            c.attachmentIndex = max(0, c.attachmentIndex - 1)
        case KeyCode.right:
            c.attachmentIndex = min(c.attachments.count - 1, c.attachmentIndex + 1)
        case KeyCode.delete, KeyCode.forwardDelete:
            let id = c.attachments[min(c.attachmentIndex, c.attachments.count - 1)].id
            Task { await model.removeAttachment(id) }
            return
        case KeyCode.returnKey:
            focusCompose(.body)
            return
        default:
            return
        }
        model.composer = c
    }

    /// A click on an address of a recipient line (`reponse.md`, section 6,
    /// "Corriger"): the line takes the keys and that address comes back
    /// into it, selected, exactly as ⌫ brings back the last one. ⌫ then
    /// removes it, typing replaces it, ← or → corrects it letter by
    /// letter. Any of the three lines' addresses can be reached this way,
    /// not only the last.
    ///
    /// What was being typed becomes an address first, as a click
    /// elsewhere does, so nothing typed is lost; it is added at the end
    /// of its line, which leaves `index` pointing at the same address.
    func clickRecipient(_ field: ComposeField, _ index: Int) {
        guard model.composer != nil else { return }
        let pending = model.takePendingLine()
        Task {
            if let pending { await model.commitRecipients(pending.text, to: pending.field) }
            guard var c = model.composer, c.takeBack(index, of: field) else { return }
            c.focus = field
            c.suggestions = []
            c.suggestionIndex = nil
            model.composer = c
            model.composerChanged()
        }
    }

    /// A click on a file of the reply: the line of files takes the keys,
    /// with that file selected.
    func clickAttachment(_ index: Int) {
        focusCompose(.attachments)
        model.composer?.attachmentIndex = index
    }

    /// A key in the subject: a one-line text field; Return goes to the text.
    private func handleSubjectKey(_ event: NSEvent, key: String, mods: NSEvent.ModifierFlags) {
        guard var c = model.composer else { return }
        let command = mods.contains(.command)
        if event.keyCode == KeyCode.returnKey, !command {
            focusCompose(.body)
            return
        }
        if editLine(&c.subjectLine, event: event, key: key) == nil {
            guard !command, !mods.contains(.control), let text = event.characters, LineEdit.isTypable(text) else { return }
            c.subjectLine.insert(text)
        }
        guard c.subjectLine.text != c.headers.subject else {
            model.composer = c
            return
        }
        c.headers.subject = c.subjectLine.text
        model.composer = c
        model.composerChanged()
    }

    // MARK: - Line editing

    /// What an editing key changed.
    enum LineChange { case text, cursor }

    /// A key of a line being typed (a picker, the contacts filter, the
    /// subject or a recipient line of a reply), with the shortcuts of
    /// every macOS text field: the fixed keys (`editFixed`), then the
    /// actions of `[line_edit]` (`applyLine`). Shift added to the key of a
    /// move selects instead (⇧⌥→ a word, ⇧⌘← to the start…), without a
    /// binding of its own (`raccourcis.md`, section 5.2). Returns `nil`
    /// for any other key.
    ///
    /// Swift note: `inout` lets the function change the caller's variable
    /// (passed with `&`), here the picker's or the contacts filter's line.
    func editLine(_ line: inout LineEdit, event: NSEvent, key: String) -> LineChange? {
        if let change = Self.editFixed(&line, code: event.keyCode, mods: event.modifierFlags) { return change }
        let chain = keyChain
        if let a = keymap.action(key, in: chain) { return applyLine(a, to: &line, extend: false) }
        if let a = shiftedLineMove(key, in: chain) { return applyLine(a, to: &line, extend: true) }
        return nil
    }

    /// The move a Shift turns into a selection: `alt-shift-left` when only
    /// `alt-left` is bound (to `line.word_left`). `nil` otherwise.
    func shiftedLineMove(_ key: String, in chain: [KeyContext]) -> KeyAction? {
        guard chain.contains(.lineEdit), keymap.action(key, in: chain) == nil, let plain = KeyChord.withoutShift(key),
              let a = keymap.action(plain, in: chain), a.isLineMove else { return nil }
        return a
    }

    /// The keys of a line that are not in the keymap (`raccourcis.md`,
    /// section 5.3): ← → (Shift selects), ⌫ and ⌦, without ⌘ ⌥ ⌃.
    static func editFixed(_ line: inout LineEdit, code: UInt16, mods: NSEvent.ModifierFlags) -> LineChange? {
        let mods = mods.intersection(.deviceIndependentFlagsMask)
        guard mods.isDisjoint(with: [.command, .option, .control]) else { return nil }
        let shift = mods.contains(.shift)
        switch code {
        case KeyCode.left:
            if shift { line.extend { $0.moveLeft() } } else { line.moveLeft() }
            return .cursor
        case KeyCode.right:
            if shift { line.extend { $0.moveRight() } } else { line.moveRight() }
            return .cursor
        case KeyCode.delete:
            line.deleteBackward()
            return .text
        case KeyCode.forwardDelete:
            line.deleteForward()
            return .text
        default:
            return nil
        }
    }

    /// An action of `[line_edit]` applied to `line`: the ends (⌘← ⌘→, Home,
    /// End, `ctrl-a`, `ctrl-e` by default), by word (⌥← ⌥→), ⌥⌫ a word, ⌘⌫
    /// everything before the cursor, and ⌘A ⌘C ⌘X ⌘V: select all, copy or
    /// cut the selection (nothing selected: nothing copied), paste on one
    /// line (`LineEdit.pasted`). `extend`: the move selects. `nil` for an
    /// action that is not a line's.
    func applyLine(_ action: KeyAction, to line: inout LineEdit, extend: Bool) -> LineChange? {
        func move(_ m: (inout LineEdit) -> Void) -> LineChange {
            if extend { line.extend(m) } else { m(&line) }
            return .cursor
        }
        switch action {
        case .lineStart: return move { $0.moveToStart() }
        case .lineEnd: return move { $0.moveToEnd() }
        case .lineWordLeft: return move { $0.moveWordLeft() }
        case .lineWordRight: return move { $0.moveWordRight() }
        case .lineDeleteWord:
            line.deleteWordBackward()
            return .text
        case .lineDeleteToStart:
            line.deleteToStart()
            return .text
        case .lineSelectAll:
            line.selectAll()
            return .cursor
        case .lineCopy, .lineCut:
            let selected = line.selectedText
            guard !selected.isEmpty else { return .cursor }
            pasteboard.clearContents()
            pasteboard.setString(selected, forType: .string)
            guard action == .lineCut else { return .cursor }
            line.deleteBackward()
            return .text
        case .linePaste:
            let text = LineEdit.pasted(pasteboard.string(forType: .string) ?? "")
            guard !text.isEmpty else { return .cursor }
            line.insert(text)
            return .text
        default:
            return nil
        }
    }

    // MARK: - Search

    /// `/` then Return: the words become the "anywhere" criterion of
    /// "All Mail" of the current view (every folder but Deleted, Junk and
    /// Banned, every account in "all"), shown like any filtered list
    /// (`ui.md`): what a search mostly wants. Searched from Everything, it
    /// stays in Everything. The place `/` was pressed from is kept for
    /// Escape; a second `/` keeps the first one.
    func searchEverywhere(_ words: String) {
        if searchOrigin == nil {
            searchOrigin = SearchOrigin(
                folderId: model.selectedFolderId, unifiedRole: model.unifiedRole, everything: model.everything,
                virtual: model.virtual)
        }
        screen = .mail
        model.attributes.contains = words
        let inEverything = model.everything != nil && model.virtual == .everything
        model.selectEverything(model.scopeAccountId.map { .account($0) } ?? .all, virtual: inEverything ? .everything : .allMail)
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
            model.selectEverything(e, virtual: origin.virtual)
        }
        searchOrigin = nil
    }

    // MARK: - Screens

    func open(_ s: Screen) {
        screen = s
        screenIndex = 0
        returnsToHistory = false
        switch s {
        case .contacts: Task { await loadContacts(reset: true) }
        case .history: Task { await loadHistory(reset: true) }
        case .tags: Task { await loadTags() }
        case .groups: Task { await loadGroups() }
        case .mail: break
        }
    }

    /// The rows of the Tags screen: one per name, its mails summed over
    /// the accounts, in the order of `list_tags` (by name). A tag belongs
    /// to one account (an IMAP keyword of its mailbox), but the user thinks
    /// of it by its name, and renaming or deleting acts on every account
    /// that has it (`rename_tag`, `delete_tag`). Before (November 2026), a
    /// name two accounts shared was two rows.
    var screenTags: [TagView] {
        var out: [TagView] = []
        var index: [String: Int] = [:]
        for t in tags {
            let key = t.name.lowercased()
            if let i = index[key] {
                let o = out[i]
                out[i] = TagView(id: o.id, accountId: o.accountId, name: o.name, color: o.color,
                                 messageCount: o.messageCount + t.messageCount)
            } else {
                index[key] = out.count
                out.append(t)
            }
        }
        return out
    }

    private var screenCount: Int {
        switch screen {
        case .contacts: contacts.count
        case .history: historyRows.count
        case .tags: screenTags.count
        case .groups: groups.count
        case .mail: 0
        }
    }

    /// The tag or group under the cursor of the Tags or Groups screen:
    /// its kind, name, id (a group's) and count (mails, members).
    private var screenLabel: (kind: LabelKind, name: String, id: Int?, count: Int)? {
        switch screen {
        case .tags where screenTags.indices.contains(screenIndex):
            let t = screenTags[screenIndex]
            return (.tag, t.name, nil, t.messageCount)
        case .groups where groups.indices.contains(screenIndex):
            let g = groups[screenIndex]
            return (.group, g.name, g.id, g.memberCount)
        default:
            return nil
        }
    }

    /// The rename panel, on the current name, selected: typing replaces
    /// it, an arrow key keeps it to edit (`LineEdit`).
    private func openRename(_ kind: LabelKind, name: String, id: Int?) {
        pickerLine = LineEdit(name, selected: true)
        pickerIndex = 0
        overlay = .picker(.rename(kind, name: name, id: id))
    }

    /// Rows of the rename panel: the rename of the name typed, or why it
    /// cannot be (the name of another tag or group: the core refuses it,
    /// merging two is not a rename). Nothing while the name is empty or
    /// unchanged. Not filtered by the text typed: it is the new name.
    private func renameEntries(_ kind: LabelKind, old: String, id: Int?) -> [Entry] {
        let typed = pickerQuery.trimmingCharacters(in: .whitespaces)
        guard !typed.isEmpty, typed != old else { return [] }
        let names = kind == .tag ? screenTags.map(\.name) : groups.map(\.name)
        func same(_ a: String, _ b: String) -> Bool { a.caseInsensitiveCompare(b) == .orderedSame }
        if names.contains(where: { same($0, typed) && !same($0, old) }) {
            return [Entry(key: " ", label: L("%@ already exists", typed), disabled: true) {}]
        }
        return [Entry(key: "↩", label: L("rename %@ to %@", old, typed)) { [weak self] in
            Task {
                await self?.model.renameLabel(kind, old, id: id, to: typed)
                await self?.reloadLabels(kind)
            }
        }]
    }

    /// The Tags or Groups screen read again after a change, its cursor
    /// kept in the list.
    private func reloadLabels(_ kind: LabelKind) async {
        if kind == .tag { await loadTags() } else { await loadGroups() }
        screenIndex = max(0, min(screenIndex, screenCount - 1))
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
        // Asked by a menu, not by a key: like a suspect move, it comes
        // before anything the keys opened.
        if let u = model.pendingUpgrade { return .mailboxUpgrade(u) }
        if case .choice(let c) = overlay { return c }
        return nil
    }

    func choiceTitle(_ c: Choice) -> String {
        switch c {
        case .suspect(let s): L("Probable spoofing of %@ (verdict %@)", s.addr, s.verdict)
        case .decision(let name, let domain, _): domain ? L("Decision on the domain %@", name) : L("Decision on %@", name)
        case .senderKind(_, let addr, _, _): L("Who writes from %@", addr)
        case .mailboxUpgrade(let u):
            L("The mailbox %@ uses an older data format (%lld; this NiceMailer uses %lld)", DataDirectory.displayName(u.url), u.from, u.to)
        case .sendThen:
            // A new mail has no conversation yet: what is filed is the
            // mail written (`reponse.md`, section 8, "Nouveau mail").
            model.composer.map { Compose.filesItself($0.draft) } == true
                ? L("Send, then file this mail in")
                : L("Send, then file the conversation in")
        case .sendFrom: L("Send from")
        case .historySource: L("Show the actions of")
        case .deleteLabel(.tag, let name, _, let count):
            L("Delete the tag %@ (taken off its %lld mails)?", name, count)
        case .deleteLabel(.group, let name, _, let count):
            L("Delete the group %@ and its %lld members? Their mails stay", name, count)
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
                Entry(key: Role.letter(role).lowercased(), label: model.folderNames.name(role), detail: current == role ? "•" : nil) {
                    [weak self] in self?.decide(name, domain: domain, target: role)
                }
            }
            entries.append(Entry(key: "-", label: L("no decision"), detail: current == nil ? "•" : nil) { [weak self] in
                self?.decide(name, domain: domain, target: nil)
            })
            return entries
        case .senderKind(let id, _, let current, let forced):
            // The forced value is marked; "automatic" when the core decides.
            return Self.senderKindChoices.map { choice in
                let marked = choice.kind == nil ? !forced : forced && current == choice.kind
                return Entry(key: choice.key, label: choice.label, detail: marked ? "•" : nil) { [weak self] in
                    self?.setSenderKind(id, choice.kind)
                }
            }
        case .historySource:
            // The current filter is marked, as the current decision is.
            return HistoryDisplay.SourceFilter.allCases.map { filter in
                Entry(key: filter.key, label: filter.label, detail: filter == historySource ? "•" : nil) { [weak self] in
                    self?.filterHistory(filter)
                }
            }
        case .deleteLabel(let kind, let name, let id, _):
            return [
                Entry(key: "y", label: kind == .tag ? L("delete the tag") : L("delete the group")) { [weak self] in
                    Task {
                        await self?.model.deleteLabel(kind, name, id: id)
                        await self?.reloadLabels(kind)
                    }
                },
                Entry(key: "esc", label: L("cancel: keep it")) {},
            ]
        case .mailboxUpgrade:
            return [
                Entry(key: "y", label: L("upgrade it and use it (older versions of NiceMailer will no longer open it: copy the folder first to keep them able to)")) {
                    [model] in model.confirmMailboxUpgrade()
                },
                Entry(key: "esc", label: L("cancel: keep the current mailbox")) { [model] in model.cancelMailboxUpgrade() },
            ]
        case .sendThen:
            guard let d = model.composer?.draft else { return [] }
            var entries = Compose.sendChoices(d, names: model.folderNames).map { choice in
                Entry(key: choice.key, label: choice.label, detail: choice.current ? "•" : nil) { [model] in
                    Task { await model.sendComposer(afterSend: choice.afterSend) }
                }
            }
            // The trial option apart, after a gap: it is not one of the
            // filings of the workflow (`dossiers.md`).
            if let i = entries.firstIndex(where: { $0.key == "s" }) { entries[i].gapBefore = true }
            entries.append(Entry(key: "esc", label: L("cancel: back to the reply")) {})
            return entries
        case .sendFrom:
            guard let h = model.composer?.headers else { return [] }
            var entries = Compose.senderChoices(model.senders, current: h).map { choice in
                Entry(key: choice.key, label: choice.label, detail: choice.current ? "•" : nil) { [model] in
                    model.chooseSender(choice.sender)
                }
            }
            entries.append(Entry(key: "esc", label: L("cancel: back to the reply")) {})
            return entries
        }
    }

    /// The rows of the AI palette: a prompt per row, and a row per variant
    /// of a prompt that has some ("Rewrite · Shorter"), so that typing finds
    /// any of them at once, as in the command palette of an editor. The
    /// prompts' order is the core's, the most used first: the first row,
    /// highlighted, is the one Return runs (⌘I then Return proofreads). The
    /// letters of a prompt (`c`, `rc` for a variant) are its search words:
    /// typed, they bring it first. A prompt this version cannot run yet
    /// says so on the right.
    func aiEntries() -> [Entry] {
        aiPrompts.flatMap { p -> [Entry] in
            let notYet = p.available ? nil : L("not yet")
            // A prompt of an attachment: a row per file the AI can read,
            // with its icon, found by typing its name.
            if p.scope == "attachment" {
                return aiAttachments.map { pair in
                    let a = pair.file
                    let name = AttachmentDisplay.name(a)
                    return Entry(key: p.key, label: "\(p.label) · \(name)", disabled: !p.available, detail: notYet,
                                 detailStyle: .info, icon: AttachmentDisplay.symbol(mimeType: a.mimeType, filename: a.filename),
                                 keywords: [p.key, name]) { [weak self] in
                        self?.chooseAttachmentAI(p, pair.file, message: pair.message)
                    }
                }
            }
            guard !p.variants.isEmpty else {
                return [Entry(key: p.key, label: p.label, disabled: !p.available, detail: notYet, detailStyle: .info,
                              keywords: [p.key]) { [weak self] in self?.chooseAI(p, variant: nil) }]
            }
            return p.variants.map { v in
                Entry(key: p.key + v.key, label: AI.title(p, variant: v), disabled: !p.available, detail: notYet,
                      detailStyle: .info, keywords: [p.key + v.key]) { [weak self] in self?.chooseAI(p, variant: v) }
            }
        }
    }

    /// The prompts of the AI palette open (`openAI`).
    var aiPrompts: [AIPromptView] = []
    /// The files of the conversation the palette works on that the AI can
    /// read, each with its mail: the rows of the prompts of an attachment.
    var aiAttachments: [(message: Int, file: AttachmentView)] = []

    private func chooseAttachmentAI(_ p: AIPromptView, _ file: AttachmentView, message: Int) {
        guard p.available else {
            model.flash = L("Not available yet: %@", p.label)
            return
        }
        guard let thread = model.openThread?.id else { return }
        Task { await model.startMailAI(p, variant: nil, thread: thread, message: message, attachment: file) }
    }

    private func chooseAI(_ p: AIPromptView, variant: AIVariantView?) {
        guard p.available else {
            model.flash = L("Not available yet: %@", p.label)
            return
        }
        runAI(p, variant: variant)
    }

    /// Starts a prompt chosen in the AI menu: on the reply being written,
    /// or on the conversation open, the mail under the reader's cursor for
    /// a prompt of one mail.
    private func runAI(_ p: AIPromptView, variant: AIVariantView?) {
        // Several conversations: sorted in the background, the keys free.
        if p.scope == "selection" {
            Task { await model.startSort(p) }
            return
        }
        if model.composer != nil {
            Task { await model.startAI(p, variant: variant, passage: model.aiPassage) }
            return
        }
        guard let thread = Self.aiThread(marked: model.marked, open: model.openThread?.id, cursor: model.selectedThreadId) else {
            model.flash = L("No mail to work on")
            return
        }
        // The mail under the reader's cursor, if the conversation is the
        // one shown; else the core takes its latest mail received.
        let current = thread == model.openThread?.id ? readerState.current : nil
        Task { await model.startMailAI(p, variant: variant, thread: thread, message: current) }
    }

    /// `a` outside the text, ⌘I in the reply: the AI menu of the screen,
    /// once the core has said which prompts it offers there. In the reply,
    /// the prompts work on the text; elsewhere, on the mails, for a later
    /// version of the core (`available: false` for now).
    func openAI() {
        let compose = model.composer != nil && screen == .mail
        guard compose || screen == .mail else { return }
        // In the reply, the text selected decides: its prompts (other
        // wordings…) work on it alone; none, the whole text. The
        // passage is kept as it was when ⌃I was pressed, in the units of
        // the editor's text (a line per text block when formatted: the
        // core measures it in the same text, `ia.md`, section 5).
        let passage: NSRange? = compose
            ? model.editorState.textRange.flatMap { $0.end > $0.start ? NSRange(location: $0.start, length: $0.end - $0.start) : nil }
            : nil
        let name = passage != nil ? "compose-selection" : Self.aiScreen(compose: compose, marked: model.marked.count)
        model.aiPassage = passage
        let folder = compose ? nil : model.currentRole
        // One mail of the conversation shown: its files the AI can read
        // join the palette, after the mail's prompts. Not for a row under
        // the cursor that is not shown: its files are not loaded.
        let thread = Self.aiThread(marked: model.marked, open: model.openThread?.id, cursor: model.selectedThreadId)
        let files = name == "mail" && thread != nil && thread == model.openThread?.id
            ? AI.readableAttachments(model.openThread?.messages ?? []) : []
        Task {
            guard var prompts = await model.aiPrompts(screen: name, folder: folder) else { return }
            if !files.isEmpty, let more = await model.aiPrompts(screen: "attachment", folder: folder) {
                prompts += more
            }
            aiAttachments = files
            guard !prompts.isEmpty else {
                model.flash = L("No AI prompt here")
                return
            }
            // The first row highlighted: the most used prompt comes first
            // (`Catalog::menu`), so ⌘I then Return proofreads a reply, `a`
            // then Return summarizes a mail or sorts the Quarantine.
            aiPrompts = prompts
            openPicker(.ai)
        }
    }

    /// Where the AI shows: `palette` for its menu (`a` or ⌘I), a picker
    /// over the rest of the window; `panel` for the result of a prompt, in
    /// the palette's place, for a reply as for a mail; `none` when nothing
    /// of the AI is open.
    var aiPlacement: String {
        if overlay == .picker(.ai) { return "palette" }
        if model.aiRun != nil { return "panel" }
        return "none"
    }

    /// The screen whose AI palette `a` (or ⌘I) opens (`ia.md`, section 11,
    /// "Contexte"): what it works on, not the layout of the window. The
    /// reply while one is written; several mails when two or more are
    /// marked (filing them); else one mail. The folder plays its part in
    /// the core: in Quarantine, the sorting of the whole folder joins the
    /// mail's prompts. A pure function, tested alone.
    nonisolated static func aiScreen(compose: Bool, marked: Int) -> String {
        if compose { return "compose" }
        return marked >= 2 ? "selection" : "mail"
    }

    /// The conversation a prompt of one mail works on: the one marked
    /// alone, else the one shown, else the row under the cursor (a summary
    /// of a conversation not opened yet is a way to sort it). A pure
    /// function, tested alone.
    nonisolated static func aiThread(marked: Set<Int>, open: Int?, cursor: Int?) -> Int? {
        if marked.count == 1 { return marked.first }
        return open ?? cursor
    }

    /// A key while the AI panel of the reply is open: the answer it gives
    /// (`AI.action`), and nothing reaches the text.
    private func handleAIKey(_ event: NSEvent, key: String, action: KeyAction?, run: AIRun) {
        // The actions of `[ai_panel]` become what `AI.action` reads (a pure
        // function, tested on its own): the letters it knows (`y`, `n`,
        // `c`), Return, ⌘-Return, the arrows. A digit chooses a row, and
        // Escape refuses or stops: these are not in the keymap.
        var letter = ""
        var isReturn = false, isCommand = false, arrow = 0
        switch action {
        case .aiAccept?: letter = "y"
        case .aiRefuse?: letter = "n"
        case .aiCopy?: letter = "c"
        case .aiConfirm?: isReturn = true
        case .aiOpen?: isReturn = true; isCommand = true
        case .aiNext?: arrow = 1
        case .aiPrevious?: arrow = -1
        case let a?:
            // A key of `everywhere` (the text size, full screen).
            if perform(a) { return }
        case nil:
            if key.count == 1, key.first?.isASCII == true, key.first?.isNumber == true { letter = key }
        }
        let result = AI.action(run.phase, key: letter, isReturn: isReturn, isEscape: event.keyCode == KeyCode.escape,
                               arrow: arrow, choice: run.choice, isCommand: isCommand)
        // A move is applied at once, not in a `Task`: the next arrow
        // (a key held down repeats it) must read the choice it left, or
        // two quick ↓ would move by one.
        if case .move(let i) = result {
            model.aiRun?.choice = i
            return
        }
        if result == .copy {
            copyAIResult(run)
            return
        }
        Task { await model.answerAI(result) }
    }

    /// `c` in the AI panel: the whole text of the result to the clipboard
    /// (`AI.copyText`), to paste it in a note or another app. The panel
    /// stays open: a proposal can still be accepted after it was copied.
    func copyAIResult(_ run: AIRun) {
        guard case .proposal(let p) = run.phase, let text = AI.copyText(p) else { return }
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        model.flash = L("Result copied to the clipboard")
    }

    /// ⌥⌘F or a click on "From": the choice of the address to send from,
    /// among those of every account that can send. With a single one,
    /// there is nothing to choose, and the status bar says so.
    func chooseSender() {
        guard model.composer != nil else { return }
        guard model.canChooseSender else {
            model.flash = model.senders.isEmpty
                ? L("No account can send: add an [smtp] server to one in accounts.toml")
                : L("Only one address can send: %@", model.senders.map(Compose.senderLabel).joined())
            return
        }
        choiceIndex = nil
        overlay = .choice(.sendFrom)
    }

    /// The answers of `o` on the contacts screen: `h` human, `r` robot,
    /// `-` automatic (the core's verdict takes over again).
    static var senderKindChoices: [(key: String, label: String, kind: String?)] {
        [("h", L("human"), "person"), ("r", L("robot"), "robot"), ("-", L("automatic (computed from the mails)"), nil)]
    }

    private func setSenderKind(_ addressId: Int, _ kind: String?) {
        Task {
            await model.setSenderKind(addressId, kind: kind)
            await loadContacts(reset: false, keep: true)
        }
    }

    private func decide(_ name: String, domain: Bool, target: String?) {
        Task {
            await model.setScreening(name, domain: domain, target: target)
            await loadContacts(reset: false, keep: true)
        }
    }

    private func handleChoiceKey(_ c: Choice, key: String, action: KeyAction?, code: UInt16) {
        let entries = choiceEntries(c)
        switch action {
        case .choiceNext?:
            choiceIndex = choiceIndex.map { Self.stepDown($0, count: entries.count) } ?? 0
            return
        case .choicePrevious?:
            choiceIndex = choiceIndex.map { Self.stepUp($0, count: entries.count) } ?? max(0, entries.count - 1)
            return
        case .choiceChoose?:
            if let i = choiceIndex { chooseChoiceEntry(i) }
            return
        default:
            break
        }
        if code == KeyCode.escape {
            closeChoice()
            if case .suspect = c { model.cancelSuspect() }
            if case .mailboxUpgrade = c { model.cancelMailboxUpgrade() }
            return
        }
        // The answers have their letters, shown with the question: not in
        // the keymap (`raccourcis.md`, section 5.3).
        if let i = entries.firstIndex(where: { ($0.chord ?? $0.key) == key }) {
            chooseChoiceEntry(i)
        } else if let action, perform(action) {
            // A key of `everywhere` (full screen): the question stays.
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
        case .tags where screenTags.indices.contains(screenIndex):
            screen = .mail
            model.showThreads(of: .tag(screenTags[screenIndex]))
        case .groups where groups.indices.contains(screenIndex):
            screen = .mail
            model.showThreads(of: .group(groups[screenIndex]))
        case .history:
            Task { await openHistoryRow() }
        default:
            break
        }
    }

    /// The rows of the History screen, the lines of the unfolded groups
    /// under their group.
    var historyRows: [HistoryDisplay.Row] {
        HistoryDisplay.rows(history, unfolded: historyUnfolded)
    }

    /// Return on a row of the History screen: a group folds or unfolds; a
    /// mail opens where it is now, to correct what was done to it; a sender
    /// lists their mails. The other rows have nothing to open.
    func openHistoryRow() async {
        let rows = historyRows
        guard rows.indices.contains(screenIndex) else { return }
        let group = rows[screenIndex].group
        if group.count > 1 {
            await toggleHistoryGroup(group)
            return
        }
        switch group.entity {
        case "message":
            guard await model.reveal(messageId: group.entityId) else { return }
            returnsToHistory = true
            showDeepLinkedMail()
        case "address":
            guard let addr = group.title, let client = model.core else { return }
            do {
                let found: Page<ContactView> = try await client.call("list_contacts", ["query": addr, "limit": 20])
                guard let contact = found.items.first(where: { $0.addr.caseInsensitiveCompare(addr) == .orderedSame }) else { return }
                returnsToHistory = true
                screen = .mail
                model.showThreads(of: .contact(contact))
            } catch { model.report(error) }
        default:
            break
        }
    }

    /// Folds a group, or reads its lines and shows them under it. All of
    /// them at once: there are as many as the action touched, and a page cut
    /// in the middle of a group would need a row of its own to go on.
    private func toggleHistoryGroup(_ group: HistoryGroup) async {
        if historyUnfolded.removeValue(forKey: group.lastId) != nil { return }
        guard let client = model.core else { return }
        var lines: [HistoryGroup] = []
        var cursor: String?
        do {
            repeat {
                var params: [String: Any] = ["first_id": group.firstId, "last_id": group.lastId, "limit": 200]
                if let cursor { params["cursor"] = cursor }
                let page: Page<HistoryGroup> = try await client.call("list_history_groups", params)
                lines += page.items
                cursor = page.nextCursor
            } while cursor != nil
            historyUnfolded[group.lastId] = lines
        } catch { model.report(error) }
    }

    /// The list of the screen has this height.
    func screenHeightChanged(_ height: CGFloat) {
        screenVisibleRows = Self.visibleRows(height: height, rowHeight: scaled(20))
    }

    /// A row of a long screen came into view: the wheel and the scroll bar
    /// reach the end of what is loaded without moving the cursor.
    func screenRowAppeared(_ index: Int) {
        loadMoreIfNeeded(near: index)
    }

    private func handleContactFilterInput(event: NSEvent, key: String, action: KeyAction?, mods: NSEvent.ModifierFlags) {
        if let change = editLine(&contactsLine, event: event, key: key) {
            if change == .text { contactsFilterChanged() }
            return
        }
        if action == .contactFilterClose {
            // The line closes, the filter stays (its chip says so).
            overlay = .none
            return
        }
        if let action, perform(action) { return }
        if event.keyCode == KeyCode.escape {
            overlay = .none
            contactsQuery = ""
        } else {
            guard !mods.contains(.control), !mods.contains(.command), let text = event.characters,
                  LineEdit.isTypable(text) else { return }
            contactsLine.insert(text)
        }
        contactsFilterChanged()
    }

    /// The contacts filter to show as a chip in the status bar (`/gazette ×`),
    /// or `nil`. Return closes the input line but keeps the filter, and the
    /// screen keeps it when the user leaves and comes back (to find the
    /// search again after listing a contact's mails): without the chip the
    /// list would be narrowed, or empty, with nothing on screen to say why.
    /// While the line is open, it shows the text itself, with its cursor.
    var contactsFilterChip: String? {
        guard screen == .contacts, !contactsQuery.isEmpty else { return nil }
        // Swift note: `if case` tests one case of an enum; here, "the input
        // line is open".
        if case .contactFilter = overlay { return nil }
        return contactsQuery
    }

    /// A click on the chip of the contacts filter: the filter goes, and the
    /// whole list comes back.
    func clearContactsFilter() {
        contactsQuery = ""
        contactsFilterChanged()
    }

    /// The contacts filter changed: the list is loaded again from its top.
    private func contactsFilterChanged() {
        screenIndex = 0
        Task { await loadContacts(reset: true) }
    }

    private func loadMoreIfNeeded(near index: Int? = nil) {
        guard (index ?? screenIndex) >= screenCount - 10, !loadingMoreRows else { return }
        let more = screen == .contacts ? contactsCursor != nil : screen == .history && historyCursor != nil
        guard more else { return }
        loadingMoreRows = true
        let current = screen
        Task {
            if current == .contacts { await loadContacts(reset: false) } else { await loadHistory(reset: false) }
            loadingMoreRows = false
        }
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

    /// Shows the actions of `filter` only, from the top of the log: the
    /// rows read so far were of another filter.
    func filterHistory(_ filter: HistoryDisplay.SourceFilter) {
        historySource = filter
        screenIndex = 0
        Task { await loadHistory(reset: true) }
    }

    /// The filter on who did it, as a chip of the status bar (`Me ×`), or
    /// `nil` when every action is shown. Without it, a list of a few rows
    /// would not say that the others are hidden.
    var historySourceChip: String? {
        guard screen == .history, historySource != .everyone else { return nil }
        return historySource.label
    }

    func loadHistory(reset: Bool) async {
        guard let client = model.core else { return }
        var params: [String: Any] = ["limit": 100, "include_read": historyIncludesRead]
        if !historySource.sources.isEmpty { params["sources"] = historySource.sources }
        if !reset, let c = historyCursor { params["cursor"] = c }
        if reset { historyRequest += 1 }
        let request = historyRequest
        do {
            let page: Page<HistoryGroup> = try await client.call("list_history_groups", params)
            guard request == historyRequest else { return }
            if reset { historyUnfolded = [:] }
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
        await model.loadGroups()
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

    /// A click outside the open menu, picker or help: closes it, like
    /// Escape, as a popover does. The click does nothing else (it does not
    /// also select the mail under it), so that closing a panel never
    /// changes the list by surprise. A question (`activeChoice`) stays: it
    /// waits for its answer.
    func clickOutsideOverlay() {
        guard activeChoice == nil else { return }
        switch overlay {
        case .menu, .help, .picker:
            model.flash = nil
            overlay = .none
        case .none, .contactFilter, .choice:
            break
        }
    }

    /// Whether a click outside the panel should close it (`clickOutsideOverlay`).
    var closesOnClickOutside: Bool {
        guard activeChoice == nil else { return false }
        switch overlay {
        case .menu, .help, .picker: return true
        case .none, .contactFilter, .choice: return false
        }
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
    static let home: UInt16 = 115
    static let end: UInt16 = 119
    static let left: UInt16 = 123
    static let right: UInt16 = 124
    static let down: UInt16 = 125
    static let up: UInt16 = 126
    static let tab: UInt16 = 48
}
