import AppKit
import Foundation
import Observation

/// State displayed by the app, and the actions the views can trigger.
/// Everything that is a business rule (move targets, preselection,
/// counters) comes from the core; this class only loads, caches and
/// forwards.
///
/// Swift notes:
/// - `@MainActor`: every property and method runs on the main thread, the
///   only one allowed to touch the UI. Async work started here comes back to
///   the main thread after each `await`, so there are no data races on the
///   state.
/// - `@Observable` (macOS 14+): SwiftUI tracks which properties each view
///   reads, and redraws exactly those views when the properties change. No
///   `@Published` needed.
@MainActor
@Observable
final class AppModel {
    /// Connection state, shown by `ContentView`.
    enum Status: Equatable {
        case connecting
        case connected(coreVersion: String)
        case failed(String)
    }

    // `private(set)`: readable from anywhere, writable only from this class.
    private(set) var status: Status = .connecting
    private(set) var folders: [FolderView] = []
    /// Threads of the selected folder, as far as loaded (pages are appended
    /// as the user scrolls).
    // Settable from `AppModel+Actions.swift` (optimistic updates); a
    // `private(set)` setter would only be reachable from this file.
    var threads: [ThreadSummary] = []
    /// Cursor of the next page of `threads`; nil when everything is loaded.
    private(set) var nextCursor: String?
    /// Thread shown in the reading pane.
    private(set) var openThread: ThreadView?
    private(set) var searchResults: [SearchHit] = []
    /// Error shown in an alert; the view sets it back to nil when dismissed.
    var lastError: String?
    /// Directory where `export` writes, passed to the core; `nil` lets the
    /// core choose ~/Downloads. Set by `NICEMAILER_EXPORT_DIR` (tests must
    /// never write into the user's Downloads).
    var exportDirectory: String? = ProcessInfo.processInfo.environment["NICEMAILER_EXPORT_DIR"]
    /// Opens a file with its default app, which comes to the front. Replaced
    /// in UI script mode and in unit tests, which must never launch Preview
    /// or Word on the user's screen.
    @ObservationIgnored var openFile: (URL) -> Void = { NSWorkspace.shared.open($0) }
    /// Short message shown in the geek status bar (result of an action,
    /// "not available yet"…); cleared by the next key.
    var flash: String?
    /// The conversation whose remote images the user asked to load although
    /// its sender's mail is not authenticated (`RemoteImages.automatic`):
    /// while it stays open, until another display is chosen. Forgotten when
    /// another conversation is opened; never stored.
    var remoteImagesOnce: Int?
    /// The Mac's address book (`AddressBook.swift`). The UI scenarios get
    /// a book of their own: the user's cards are never read in script mode.
    @ObservationIgnored var addressBook: any AddressBook =
        UIScript.isActive ? FixtureAddressBook.sample() : SystemAddressBook()
    /// The cards of the senders of the open thread (`loadSenderCards`),
    /// and the thread they belong to.
    var senderCards: [ContactCard] = []
    var senderCardsThreadId: Int?
    /// Whether the status bar already said, this session, that the
    /// address book cannot be read: said once, not at each `l`.
    @ObservationIgnored var addressBookRefusalShown = false

    /// What `z` undoes, most recent last: moves and decisions of this
    /// session (`AppModel+Workflow.swift`).
    var undoStack: [UndoStep] = []
    /// A move refused until the user confirms it: its sender's mail is
    /// suspected of spoofing (`authentification-expediteur.md`, section 5).
    var suspectMove: SuspectMove?

    /// `ui.title_bar` of `config.toml`: the title bar of the window is
    /// hidden (the default), or shown (`WindowChrome`).
    var titleBarHidden = true

    /// The agent of the core, when the app carries it (`CoreAgent`).
    var agentState: CoreAgent.State = .notBundled
    /// The core was stopped from the menu in this session: coming back to
    /// the app does not start it again; a new launch, or "Try Again", does.
    var coreStoppedByUser = false
    /// The mailbox in use (`DataDirectory`), for the window's title.
    var mailboxName: String { DataDirectory.displayName(DataDirectory.current) }

    /// The reply being written (`AppModel+Compose.swift`); nil when the
    /// reader shows the mail.
    var composer: Composer?
    /// The pending save of the reply's text, restarted at each key.
    @ObservationIgnored var composeSaveTask: Task<Void, Never>?
    /// The save in progress, which the next one waits for.
    @ObservationIgnored var composeSaving: Task<Void, Never>?
    /// Mails waiting for their undo delay or for the server, by draft id:
    /// the subject, for the status bar when they leave.
    @ObservationIgnored var sending: [Int: String] = [:]

    /// Selected folder. `didSet` is a property observer: it runs after each
    /// assignment, with the previous value in `oldValue`. Changing folder
    /// clears the thread selection and loads the new list.
    var selectedFolderId: Int? {
        didSet {
            guard selectedFolderId != oldValue else { return }
            // Choosing a folder leaves the unified view and "Everything".
            if selectedFolderId != nil {
                unifiedRole = nil
                everything = nil
            }
            selectedThreadId = nil
            marked = []
            // Choosing a folder leaves a tag, group or contact list.
            listFilter = nil
            // `Task { ... }` starts asynchronous work from synchronous code.
            Task { await reloadThreads() }
        }
    }

    /// Unified view, the account "all" (`ui.md`): the folders of this role
    /// in every account, listed together. `nil` when a single folder of one
    /// account is shown (`selectedFolderId`).
    private(set) var unifiedRole: String?

    /// The virtual folder "Everything": every folder of the unified view
    /// (`.all`) or of one account, listed together to search everywhere
    /// with the attributes.
    enum Everything: Equatable {
        case all
        case account(Int)
    }
    private(set) var everything: Everything?

    /// Name of the virtual folder (a name, like the folder names, not
    /// translated).
    static let everythingName = "Everything"

    /// Shows "Everything" of the unified view or of an account.
    func selectEverything(_ e: Everything) {
        let changed = everything != e || selectedFolderId != nil || unifiedRole != nil || listFilter != nil
        everything = e
        unifiedRole = nil
        guard changed else { return }
        if selectedFolderId != nil {
            selectedFolderId = nil
        } else {
            selectedThreadId = nil
            marked = []
            listFilter = nil
            Task { await reloadThreads() }
        }
    }

    /// Start in the unified view when there are several accounts (the geek
    /// theme does; the classic one keeps its folder sidebar).
    var startUnified = false

    /// Name of the unified account.
    static let unifiedName = "all"

    /// Shows the unified view of `role` (Inbox, Archive…).
    func selectUnified(_ role: String) {
        let changed = unifiedRole != role || selectedFolderId != nil || listFilter != nil || everything != nil
        unifiedRole = role
        everything = nil
        guard changed else { return }
        if selectedFolderId != nil {
            // `didSet` clears the selection and reloads the list.
            selectedFolderId = nil
        } else {
            selectedThreadId = nil
            marked = []
            listFilter = nil
            Task { await reloadThreads() }
        }
    }

    /// The folders whose threads the list shows: one folder, or every
    /// folder of the unified role.
    var listedFolderIds: [Int] {
        switch everything {
        case .all: return folders.map(\.id)
        case .account(let a): return folders.filter { $0.accountId == a }.map(\.id)
        case nil: break
        }
        if let role = unifiedRole { return folders.filter { $0.role == role }.map(\.id) }
        return selectedFolderId.map { [$0] } ?? []
    }

    /// What `list_threads` lists, as its parameters: Everything by its scope
    /// (the core then reads whole threads, one query whatever the number of
    /// folders), anything else by its folders. The threads of a tag, a group
    /// or a contact are Everything of every account, narrowed by the
    /// criterion (`listFilterParameters`).
    ///
    /// A pure function of its inputs, so it is tested without a core.
    static func listScope(everything: Everything?, folderIds: [Int], listFilter: ListFilter? = nil) -> [String: Any] {
        if listFilter != nil { return ["everything": [String: Any]()] }
        switch everything {
        case .all: return ["everything": [String: Any]()]
        case .account(let a): return ["everything": ["account_id": a]]
        case nil: return ["folder_ids": folderIds]
        }
    }

    /// The `filter` of `list_threads`: the attributes, plus the criterion of
    /// a tag, group or contact list. A tag goes by its name: the core lists
    /// the tags of that name in every account (each account has its own
    /// "Clients"). `nil` when there is nothing to filter.
    static func filterParameters(_ attributes: ThreadFilter, listFilter: ListFilter?) -> [String: Any]? {
        var p = attributes.parameters
        switch listFilter {
        case .tag(let t): p["tag"] = t.name
        case .group(let g): p["group_id"] = g.id
        case .contact(let c): p["address_id"] = c.id
        case nil: break
        }
        return p.isEmpty ? nil : p
    }

    /// Account of the view: `nil` for the unified one.
    var scopeAccountId: Int? {
        switch everything {
        case .all: return nil
        case .account(let a): return a
        case nil: return unifiedRole == nil ? selectedFolder?.accountId : nil
        }
    }
    /// "all", or the name of the account.
    var scopeName: String {
        guard let a = scopeAccountId else { return Self.unifiedName }
        return folders.first { $0.accountId == a }?.accountName ?? ""
    }
    /// Role of the folder shown (Inbox, Archive…), in either view; `nil`
    /// for "Everything".
    var currentRole: String? { everything != nil ? nil : unifiedRole ?? selectedFolder?.role }
    /// In "Everything" only, the folder of each listed thread, by folder
    /// id: the list and the reader show it, since the list mixes every
    /// folder (`ui.md`, "Sélecteur d'attributs"). `nil` elsewhere: the
    /// folder is the one in the status bar. Built once per list redraw (a
    /// dictionary) rather than looked up row by row.
    var everythingFolderLabels: [Int: FolderLabel]? {
        guard let e = everything else { return nil }
        return Self.folderLabels(folders, withAccount: e == .all)
    }

    /// Folder of the selected thread, in "Everything" only (the reader
    /// shows it next to the subject).
    var selectedThreadFolderLabel: String? {
        guard let e = everything, let t = selectedThread else { return nil }
        return folders.first { $0.id == t.folderId }.map { Self.folderLabel($0, withAccount: e == .all).text }
    }

    /// Folder of a mail in "Everything": its name, and its account in the
    /// unified view (each account has its own Inbox, Archive…). Kept apart
    /// so that a narrow list can show the folder alone.
    struct FolderLabel: Equatable {
        let folder: String
        let account: String?
        /// "Todo", or "Todo · perso".
        var text: String { account.map { "\(folder) · \($0)" } ?? folder }
    }

    /// Label of `f`, with its account when `withAccount` ("all").
    static func folderLabel(_ f: FolderView, withAccount: Bool) -> FolderLabel {
        FolderLabel(folder: f.name, account: withAccount ? f.accountName : nil)
    }

    /// `folderLabel` of every folder, by folder id.
    static func folderLabels(_ folders: [FolderView], withAccount: Bool) -> [Int: FolderLabel] {
        // Swift note: `uniquingKeysWith` keeps the first value if two
        // folders had the same id (they cannot, but the initializer without
        // it would crash on a duplicate key).
        Dictionary(folders.map { ($0.id, folderLabel($0, withAccount: withAccount)) },
                   uniquingKeysWith: { first, _ in first })
    }

    /// Name of the folder shown.
    var currentFolderName: String {
        if everything != nil { return Self.everythingName }
        return unifiedRole.map { Role.name($0) } ?? selectedFolder?.name ?? ""
    }
    /// Whether there are several accounts (the unified view is offered).
    var hasSeveralAccounts: Bool { Set(folders.map(\.accountId)).count > 1 }

    /// A list across folders shown instead of the selected folder: the
    /// threads of a tag, of a group's members, or of a contact.
    enum ListFilter: Equatable {
        case tag(TagView)
        case group(GroupView)
        case contact(ContactView)
    }

    /// Set by `showThreads(of:)`; `nil` shows the selected folder.
    private(set) var listFilter: ListFilter?

    /// Sort of the folder lists: "received" or "modified" (`ui.md`, vc/vm).
    var sort: String = "received" {
        didSet {
            guard sort != oldValue else { return }
            Task { await reloadThreads() }
        }
    }

    /// Attributes narrowing the list (`F`, `ThreadFilter`), kept when the
    /// folder or the account changes.
    var attributes = ThreadFilter() {
        didSet {
            guard attributes != oldValue else { return }
            Task { await reloadThreads() }
        }
    }

    /// Threads marked for a grouped action (J/K in the geek theme). The
    /// thread under the cursor is `selectedThreadId`.
    var marked = Set<Int>()

    /// Selected thread: selecting one opens it in the reading pane.
    ///
    /// Reading rules (`ui.md`, "Lecture"):
    /// - a mail the user selects (keys, click, the next one after a move)
    ///   is marked read once shown for `seenDelay`;
    /// - a mail selected automatically (first mail of a list, replacement
    ///   of a mail that left the list) is not: it is marked read only when
    ///   the user moves from it to another mail of the list after
    ///   `seenDelay`. Otherwise a list filtered on "unread" emptied itself:
    ///   the mail marked read left the list, the next one was selected
    ///   automatically, marked read a second later, and so on;
    /// - leaving a mail kept in the list although it no longer matches
    ///   (`stickyThreadId`) removes it from the list;
    /// - a mail is "shown" only while the reader is visible
    ///   (`readerVisible`): in the geek theme's single column, the list alone
    ///   shows no content, so staying on a row marks nothing.
    var selectedThreadId: Int? {
        didSet {
            guard selectedThreadId != oldValue else { return }
            let auto = selectingAutomatically
            if let old = oldValue {
                // Moving by hand from an automatically selected mail (still
                // in the list: a moved mail is not marked) to another one.
                if !auto, readerVisible, selectedThreadId != nil, let shown = autoShown, shown.id == old,
                   Date().timeIntervalSince(shown.since) >= seenDelay,
                   threads.first(where: { $0.id == old }).map({ $0.unreadCount > 0 }) == true {
                    markSeen(old)
                }
                if old == stickyThreadId {
                    threads.removeAll { $0.id == old }
                    stickyThreadId = nil
                }
            }
            autoShown = auto ? selectedThreadId.map { ($0, Date()) } : nil
            Task { await loadOpenThread(markSeen: !auto) }
        }
    }

    /// Whether the reading pane is on screen. The geek theme's single column
    /// (`ui.md`, `v1`) shows the list only, until Return opens the mail: a
    /// mail whose content is not visible has not been read, so the delay
    /// rule waits. Becoming visible (back to two columns) starts the delay
    /// for the mail under the cursor, as if it had just been selected.
    var readerVisible = true {
        didSet {
            guard readerVisible, !oldValue, let id = selectedThreadId, autoShown == nil else { return }
            scheduleSeen(id)
        }
    }

    /// The one delayed mark in progress: the mail just opened, or the one
    /// under the cursor when the reader became visible (`readerVisible`).
    /// One task for both, so that a mail is never marked twice: when its
    /// content arrived late, after the reader came back, both used to wait
    /// their delay, then both marked it.
    @ObservationIgnored private var seenTask: Task<Void, Never>?

    /// Starts the delay of `id`, replacing the one in progress. Swift note:
    /// cancelling a task makes its `Task.sleep` throw `CancellationError`,
    /// so the replaced delay ends without marking anything.
    private func scheduleSeen(_ id: Int) {
        seenTask?.cancel()
        seenTask = Task { await markSeenAfterDelay(id) }
    }

    /// Delay after which a mail shown is considered read (seconds). A
    /// variable so that the tests can shorten it.
    @ObservationIgnored var seenDelay: TimeInterval = 1
    /// Set while the model itself chooses the selection (`selectFirstIfNeeded`).
    @ObservationIgnored private var selectingAutomatically = false
    /// The automatically selected mail, and since when it is shown.
    @ObservationIgnored private var autoShown: (id: Int, since: Date)?
    /// The selected mail kept in the list after a refresh although it no
    /// longer matches it (read in a list of unread mails, unflagged in a
    /// list of flagged ones…): it stays under the cursor, shown with its new
    /// state, until the user leaves it. As in Gmail's "unread" view.
    private(set) var stickyThreadId: Int?

    /// Text of the search field; each change schedules a search.
    var searchText: String = "" {
        didSet { scheduleSearch() }
    }

    // Computed properties: looked up from the ids on each access.
    var selectedFolder: FolderView? { folders.first { $0.id == selectedFolderId } }
    var selectedThread: ThreadSummary? { threads.first { $0.id == selectedThreadId } }

    /// Where requests go: the socket connection, or a fake in tests.
    private var client: (any CoreCalling)?
    /// The socket connection itself, for its event stream and to close it.
    private var connection: CoreClient?
    /// Position of the last event received, to detect missed events.
    private var lastEvent: Position?
    private var loadingMore = false
    /// Pending debounced tasks, cancelled when a newer one replaces them.
    private var refreshTask: Task<Void, Never>?
    private var searchTask: Task<Void, Never>?
    /// Folders and threads invalidated by events since the last refresh.
    private var dirtyFolders = Set<Int>()
    private var dirtyThreads = Set<Int>()

    private static let pageSize = 60

    // MARK: - Connection

    /// Connects to the core, performs the handshake, subscribes to events
    /// and loads the initial data. Called at launch and by "Réessayer".
    func connect() async {
        status = .connecting
        // `?.` calls the method only if `client` is not nil.
        connection?.disconnect()
        // NiceMailer.app carries the core: its agent is registered, and
        // launchd starts it (`CoreAgent`).
        agentState = CoreAgent.register()
        if agentState == .requiresApproval {
            status = .failed(L("NiceMailer needs your permission to run its core: allow it in System Settings > General > Login Items"))
            return
        }
        do {
            let c = try await openSocket()
            connection = c
            client = c
            let hello: HelloResult = try await c.call("hello", ["protocol_version": CoreClient.protocolVersion, "client": "app"])
            // Subscribe *before* loading data (`evenements.md`, section 7):
            // a change happening during the loading will then be announced by
            // an event, instead of being silently missed.
            let sub: SubscribeResult = try await c.call("subscribe", [:])
            lastEvent = Position(epoch: sub.epoch, seq: sub.seq)
            status = .connected(coreVersion: hello.coreVersion)
            // Listen to events in a separate task, which runs as long as the
            // connection lives, while this function goes on.
            Task { await listen(c) }
            await loadUISettings()
            await reloadAll()
            openPendingDeepLink()
        } catch {
            // Inside `catch`, the error is implicitly named `error`.
            status = .failed(error.localizedDescription)
        }
    }

    /// Reads the settings of `[ui]` the app applies itself. A core that
    /// does not give them leaves the defaults.
    func loadUISettings() async {
        guard let client else { return }
        if let status: ConfigStatusView = try? await client.call("config_status", [:]), let ui = status.ui {
            titleBarHidden = ui.titleBar != "shown"
        }
    }

    /// Opens the socket of the core. With the agent, a core that does not
    /// answer is started (`launchctl kickstart`), and waited for a few
    /// seconds, the time to open its database (`architecture.md`, section
    /// 6, "Démarrage à la demande").
    private func openSocket() async throws -> CoreClient {
        let deadline = Date().addingTimeInterval(8)
        var lastKick: Date?
        var delay = 0.05
        while true {
            do {
                return try CoreClient()
            } catch {
                guard agentState == .enabled, Date() < deadline else {
                    CoreAgent.log.error("no core: \(error.localizedDescription, privacy: .public)")
                    throw error
                }
                // Asked again every 2 s: a request lost, or refused while
                // launchd was still noting the previous core's exit, is
                // not the end of it.
                if lastKick.map({ Date().timeIntervalSince($0) > 2 }) ?? true {
                    CoreAgent.kickstart()
                    lastKick = Date()
                }
                try? await Task.sleep(for: .seconds(delay))
                delay = min(delay * 2, 0.5)
            }
        }
    }

    /// Uses the mailbox in `url` from now on (`DataDirectory`): the choice
    /// is recorded, the core started again on it, and everything reloaded.
    /// Without the agent, the core started by hand must be restarted.
    func changeMailbox(to url: URL) async {
        if DataDirectory.environment != nil {
            lastError = L("NICEMAILER_DATA_DIR is set: it decides the mailbox; launch the app without it to choose one")
            return
        }
        do {
            try DataDirectory.choose(url)
        } catch {
            report(error)
            return
        }
        connection?.disconnect()
        connection = nil
        client = nil
        // Nothing of the previous mailbox is kept: its ids mean nothing in
        // the next one.
        composer = nil
        undoStack = []
        sending = [:]
        threads = []
        folders = []
        openThread = nil
        unifiedRole = nil
        everything = nil
        selectedFolderId = nil
        lastEvent = nil
        if CoreAgent.register() == .enabled {
            CoreAgent.kickstart(restart: true)
            await connect()
        } else {
            status = .failed(L("Mailbox changed to %@: restart the core on it (make core)", url.path))
        }
    }

    /// The menu "Stop the Core": the core finishes its work and leaves; it
    /// starts again at the next launch of the app.
    func stopCore() {
        guard CoreAgent.register() == .enabled else {
            flash = L("The core was started by hand: stop it where it runs (Ctrl-C)")
            return
        }
        coreStoppedByUser = true
        CoreAgent.stop()
    }

    /// Consumes the event stream until the connection closes.
    private func listen(_ c: CoreClient) async {
        for await event in c.events {
            handle(event)
        }
        // The stream ended: the connection is lost, unless the user already
        // reconnected (then `client` is a newer object; `===` compares
        // object identity).
        if connection === c { status = .failed(L("Connection to the core lost")) }
    }

    /// Records what an event invalidates and schedules a refresh. If an
    /// event is missing (seq not consecutive, or another epoch), the model
    /// can no longer know what changed, so it reloads everything it shows.
    private func handle(_ e: CoreClient.Event) {
        let order = AppModel.order(of: e, after: lastEvent)
        switch order {
        case .transient:
            // Progress of a synchronization: nothing to reload, and the
            // position stays that of the last numbered event.
            return
        case .inOrder(let p), .gap(let p):
            lastEvent = p
        }
        if e.method == "ResyncRequired" || order.isGap {
            scheduleRefresh(full: true)
            return
        }
        switch e.method {
        case "FolderChanged":
            if let f = e.params["folder_id"]?.int { dirtyFolders.insert(f) }
        case "ThreadChanged":
            if let t = e.params["thread_id"]?.int { dirtyThreads.insert(t) }
            for f in e.params["folder_ids"]?.ints ?? [] { dirtyFolders.insert(f) }
        case "ConfigChanged":
            // `config.toml` applied: its `[ui]` may have changed.
            Task { await loadUISettings() }
            return
        case "DraftSent", "SendFailed", "SendPartial":
            draftEvent(e.method, e.params)
            return
        case "ActionReverted":
            // The server refused a change made here; the core has undone it
            // (`modele de donnees.md`, section 6). Said in the status bar,
            // without stopping the user; the lists follow from the
            // `ThreadChanged` and `FolderChanged` that come with it.
            if case .string(let kind)? = e.params["kind"], case .string(let reason)? = e.params["reason"] {
                flash = AppModel.revertedFlash(kind: kind, reason: reason)
            }
            return
        default:
            // MessageChanged is always followed by ThreadChanged, which is
            // enough for what the app displays.
            return
        }
        scheduleRefresh(full: false)
    }

    /// Where an event stands in the stream (`evenements.md`): transient
    /// (no `seq`: outside the sequence), next in order, or after a gap (an
    /// event missed, or another epoch: the core restarted). A pure function
    /// of the event and the last position, so it is tested alone.
    enum EventOrder: Equatable {
        case transient
        case inOrder(Position)
        case gap(Position)

        nonisolated var isGap: Bool { if case .gap = self { return true } else { return false } }
    }

    /// The status bar message of an `ActionReverted` event: what the server
    /// refused, and its answer.
    nonisolated static func revertedFlash(kind: String, reason: String) -> String {
        let what = kind == "move" ? L("The server refused the move") : L("The server refused the change")
        return what + " · " + L("undone: %@", reason)
    }

    nonisolated static func order(of e: CoreClient.Event, after last: Position?) -> EventOrder {
        guard let seq = e.seq else { return .transient }
        let p = Position(epoch: e.epoch, seq: seq)
        if let last, last.epoch == e.epoch, seq == last.seq + 1 { return .inOrder(p) }
        return .gap(p)
    }

    /// Groups invalidations received in a burst into a single reload: a move
    /// produces several events within a few milliseconds, and reloading
    /// after each one would be wasteful. Each new event cancels the pending
    /// task and restarts the 40 ms delay ("debounce").
    private func scheduleRefresh(full: Bool) {
        if full { dirtyFolders.formUnion(listedFolderIds) }
        refreshTask?.cancel()
        refreshTask = Task {
            // `try?` ignores the error thrown when the sleep is cancelled; the
            // cancellation is then detected just below.
            try? await Task.sleep(for: .milliseconds(40))
            guard !Task.isCancelled else { return }
            let folders = dirtyFolders, threads = dirtyThreads
            dirtyFolders = []; dirtyThreads = []
            // Counters are cheap to reload: always refresh the sidebar.
            await reloadFolders()
            // The open thread first: the list refresh reads its new state to
            // keep it in place if it no longer matches (`keepingSelected`).
            if let open = openThread?.id, full || threads.contains(open) {
                await loadOpenThread(markSeen: false)
            }
            if !listedFolderIds.isEmpty, full || !folders.isDisjoint(with: listedFolderIds) {
                await reloadThreads(keepDepth: true)
            }
        }
    }

    // MARK: - Reads

    /// Reloads the folders, then the thread list (selecting the Inbox the
    /// first time).
    func reloadAll() async {
        await reloadFolders()
        if selectedFolderId == nil && unifiedRole == nil && everything == nil {
            if startUnified && hasSeveralAccounts {
                selectUnified("inbox")
            } else {
                // Setting the selection triggers `didSet`, which loads the list.
                selectedFolderId = folders.first { $0.role == "inbox" }?.id ?? folders.first?.id
            }
        } else {
            await reloadThreads(keepDepth: true)
        }
    }

    func reloadFolders() async {
        guard let client else { return }
        do {
            let list: FolderList = try await client.call("list_folders")
            folders = list.items
        } catch { report(error) }
    }

    /// Reloads the list. With `keepDepth`, as many rows as before are loaded
    /// (in several pages if needed), so the list does not jump back to the
    /// top on every invalidation.
    func reloadThreads(keepDepth: Bool = false) async {
        let folderIds = listedFolderIds
        let listFilter = self.listFilter
        guard let client, !folderIds.isEmpty || listFilter != nil else { threads = []; return }
        let sort = self.sort
        let filter = attributes
        // Read once: every page of this reload lists the same thing.
        let scope = Self.listScope(everything: everything, folderIds: folderIds, listFilter: listFilter)
        let filterParameters = Self.filterParameters(filter, listFilter: listFilter)
        let wanted = keepDepth ? max(threads.count, Self.pageSize) : Self.pageSize
        var items: [ThreadSummary] = []
        var cursor: String?
        do {
            // `repeat { } while` runs the body at least once.
            repeat {
                var params = scope
                params["limit"] = min(200, wanted - items.count)
                params["sort"] = sort
                if let filterParameters { params["filter"] = filterParameters }
                if let cursor { params["cursor"] = cursor }
                let page: Page<ThreadSummary> = try await client.call("list_threads", params)
                items += page.items
                cursor = page.nextCursor
            } while cursor != nil && items.count < wanted
            // The user may have changed folder while we were loading: then
            // this result is stale and must not replace the new list.
            guard folderIds == listedFolderIds, sort == self.sort, filter == attributes, listFilter == self.listFilter else { return }
            // A refresh (`keepDepth`) keeps the selected mail if it only
            // stopped matching the attributes; a new list (other folder,
            // other attributes) starts afresh.
            let kept = keepDepth
                ? Self.keepingSelected(items, previous: threads, selected: selectedThreadId,
                                       open: openThread,
                                       listed: listFilter == nil ? Set(folderIds) : Set(folders.map(\.id)))
                : (threads: items, sticky: nil)
            threads = kept.threads
            stickyThreadId = kept.sticky
            nextCursor = cursor
            selectFirstIfNeeded()
        } catch { report(error) }
    }

    /// `ui.md`: a list always has a selected mail, so the reading pane
    /// shows something. A selection no longer in the list (moved, or left
    /// out by the attributes) goes to the first mail, or to nothing when the
    /// list is empty, so the reading pane never shows a mail the list does
    /// not. The one exception is a search result opened in an old part of
    /// a folder, not in the pages loaded yet (`keepSelection`).
    private func selectFirstIfNeeded() {
        marked = marked.filter { id in threads.contains { $0.id == id } }
        if let keep = keepSelection {
            keepSelection = nil
            selectedThreadId = keep
            return
        }
        if selectedThreadId == nil || !threads.contains(where: { $0.id == selectedThreadId }) {
            // Chosen by the model, not the user: not marked read (see
            // `selectedThreadId`).
            selectingAutomatically = true
            defer { selectingAutomatically = false }
            selectedThreadId = threads.first?.id
        }
    }

    /// Thread to keep selected after the next reload, even if it is not in
    /// the loaded pages (set by `openSearchHit`).
    private var keepSelection: Int?
    /// A deep link received before the core answered (the app launched by
    /// the link itself): opened once connected (`openDeepLink`).
    var pendingDeepLink: URL?
    /// Called once a deep link selected its mail, so that the interface
    /// follows (the geek theme closes its panels and shows the mail).
    @ObservationIgnored var onDeepLinkOpened: (() -> Void)?

    /// Leaves a tag, group or contact list for the folder (or unified view)
    /// shown before.
    func leaveFilter() {
        guard listFilter != nil else { return }
        listFilter = nil
        selectedThreadId = nil
        marked = []
        Task { await reloadThreads() }
    }

    /// Shows the threads of a tag, a group or a contact (across folders).
    func showThreads(of filter: ListFilter) {
        listFilter = filter
        selectedThreadId = nil
        marked = []
        Task { await reloadThreads() }
    }

    /// Loads the next page when the last row appears on screen (called by
    /// each row's `.task` in `ThreadList`).
    func loadMoreIfNeeded(after thread: ThreadSummary) async {
        let folderIds = listedFolderIds
        let listFilter = self.listFilter
        guard let client, !folderIds.isEmpty || listFilter != nil, let cursor = nextCursor,
              !loadingMore, thread.id == threads.last?.id else { return }
        let sort = self.sort
        let filter = attributes
        loadingMore = true
        // `defer` runs when the function exits, whatever the path.
        defer { loadingMore = false }
        do {
            var params = Self.listScope(everything: everything, folderIds: folderIds, listFilter: listFilter)
            params["cursor"] = cursor
            params["limit"] = Self.pageSize
            params["sort"] = sort
            if let p = Self.filterParameters(filter, listFilter: listFilter) { params["filter"] = p }
            let page: Page<ThreadSummary> = try await client.call("list_threads", params)
            // Drop the page if the folder or the list changed meanwhile.
            guard folderIds == listedFolderIds, cursor == nextCursor, sort == self.sort, filter == attributes,
                  listFilter == self.listFilter else { return }
            threads += page.items
            nextCursor = page.nextCursor
        } catch { report(error) }
    }

    /// `items` (a refreshed list), plus the selected thread if it left the
    /// list while it still has messages in the listed folders: it changed
    /// state (read, unflagged…) rather than moved. It goes back to its place
    /// (after the row that preceded it), with its unread count and flag
    /// recomputed from `open`, the thread as just reloaded. A thread that
    /// moved away (no message left in `listed`) is dropped as usual.
    static func keepingSelected(
        _ items: [ThreadSummary], previous: [ThreadSummary], selected: Int?, open: ThreadView?, listed: Set<Int>
    ) -> (threads: [ThreadSummary], sticky: Int?) {
        guard let selected, !items.contains(where: { $0.id == selected }),
              let index = previous.firstIndex(where: { $0.id == selected }),
              let open, open.id == selected else { return (items, nil) }
        let here = open.messages.filter { listed.contains($0.folderId) }
        guard !here.isEmpty else { return (items, nil) }
        let row = previous[index].with(unreadCount: here.filter { !$0.isSeen }.count,
                                       isFlagged: here.contains { $0.isFlagged })
        // After the nearest earlier row still listed, or first.
        let ids = Set(items.map(\.id))
        let before = previous[..<index].last { ids.contains($0.id) }
        let at = before.flatMap { b in items.firstIndex { $0.id == b.id } }.map { $0 + 1 } ?? 0
        var out = items
        out.insert(row, at: at)
        return (out, selected)
    }

    /// Marks a thread read (no change to the list here: the core's events
    /// refresh it).
    private func markSeen(_ id: Int) {
        guard let client else { return }
        Task {
            do {
                let _: CommandResult = try await client.call("set_thread_seen", ["thread_id": id, "seen": true])
            } catch { report(error) }
        }
    }

    /// Marks the selected thread read at once, without `seenDelay`: the user
    /// asked to read it (Return or a double-click in one column, `ui.md`,
    /// `v1`), so there is nothing to guess. Even a mail the app selected by
    /// itself is marked: opening it is the user's choice.
    func markSelectedSeen() {
        guard let id = selectedThreadId,
              threads.first(where: { $0.id == id }).map({ $0.unreadCount > 0 }) == true
        else { return }
        // Its leaving no longer needs to mark it (see `selectedThreadId`),
        // and the delay started by the reader appearing is now pointless.
        if autoShown?.id == id { autoShown = nil }
        seenTask?.cancel()
        markSeen(id)
    }

    /// Loads the selected thread into the reading pane.
    private func loadOpenThread(markSeen: Bool) async {
        guard let client, let id = selectedThreadId else { openThread = nil; return }
        do {
            let view: ThreadView = try await client.call("get_thread", ["thread_id": id])
            guard id == selectedThreadId else { return }
            if remoteImagesOnce != id { remoteImagesOnce = nil }
            openThread = view
            // The senders' cards, if the book may already be read: macOS's
            // question is only asked from `l` (`linksOpened`).
            Task { await loadSenderCards(asking: false) }
            if markSeen { scheduleSeen(id) }
        } catch { report(error) }
    }

    /// A thread the user selected and kept on screen for `seenDelay` is
    /// marked read. The delay avoids marking everything while browsing the
    /// list with j/k. The reader must be visible when the delay starts and
    /// when it ends: hidden in between (Escape back to the single-column
    /// list), the mail was not read.
    private func markSeenAfterDelay(_ id: Int) async {
        guard let client, readerVisible, openThread?.id == id,
              openThread?.messages.contains(where: { !$0.isSeen }) == true
        else { return }
        do {
            try await Task.sleep(for: .seconds(seenDelay))
            guard id == selectedThreadId, readerVisible else { return }
            // `let _: CommandResult` decodes the result only to check it;
            // the value itself is not needed.
            let _: CommandResult = try await client.call("set_thread_seen", ["thread_id": id, "seen": true])
        } catch is CancellationError {
            // Swift note: cancelling a task makes `Task.sleep` throw; it is
            // not an error to report.
        } catch { report(error) }
    }

    /// Searches 120 ms after the last keystroke, so that typing "facture"
    /// does not send seven queries.
    private func scheduleSearch() {
        searchTask?.cancel()
        let query = searchText
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { searchResults = []; return }
        searchTask = Task {
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled, let client else { return }
            do {
                let result: SearchResult = try await client.call("search", ["query": query, "limit": 100])
                // Ignore results of a query the user has since changed.
                if query == searchText { searchResults = result.items }
            } catch { report(error) }
        }
    }

    /// Opens a search result in its folder, so that commands (move...) have
    /// their context, then closes the search.
    func openSearchHit(_ hit: SearchHit) {
        guard let threadId = hit.threadId else { return }
        show(threadId: threadId, folderId: hit.folderId)
        searchText = ""
    }

    /// Shows the thread `threadId` in the folder `folderId`, selected and
    /// open: a search result, a deep link (`openDeepLink`).
    func show(threadId: Int, folderId: Int) {
        // In the unified view, stay unified: show the folder's role for all
        // accounts. Otherwise, go to the folder. When the list is
        // reloaded, the thread stays selected even if it is not in the
        // first pages (`keepSelection`).
        if unifiedRole != nil, let role = folders.first(where: { $0.id == folderId })?.role {
            if role != unifiedRole || listFilter != nil { keepSelection = threadId }
            selectUnified(role)
        } else if selectedFolderId != folderId {
            keepSelection = threadId
            selectedFolderId = folderId
        }
        // Set after the folder, because changing folder clears the selection.
        selectedThreadId = threadId
    }

    // MARK: - Commands

    /// Moves the selected thread and selects the next one.
    ///
    /// The thread is removed from the list immediately ("optimistic update"),
    /// without waiting for the core, so the interface feels instant. The
    /// core's events then reconcile the list; if the command fails, the list
    /// is reloaded to put the thread back.
    func move(to target: String) {
        guard let client, let thread = selectedThread, let folder = selectedFolder,
              folder.moveTargets.contains(target) else { return }
        let index = threads.firstIndex(of: thread) ?? 0
        threads.removeAll { $0.id == thread.id }
        selectedThreadId = threads.isEmpty ? nil : threads[min(index, threads.count - 1)].id
        Task {
            do {
                let _: CommandResult = try await client.call(
                    "move_thread", ["thread_id": thread.id, "target": target, "from_folder_id": folder.id])
            } catch {
                report(error)
                await reloadThreads(keepDepth: true)
            }
        }
    }

    /// Toggles read/unread. Not optimistic: the list updates when the core's
    /// event arrives, a few milliseconds later.
    func toggleSeen() {
        guard let client, let thread = selectedThread else { return }
        let seen = thread.unreadCount > 0
        Task {
            do {
                let _: CommandResult = try await client.call("set_thread_seen", ["thread_id": thread.id, "seen": seen])
            } catch { report(error) }
        }
    }

    /// Toggles the flag.
    func toggleFlag() {
        guard let client, let thread = selectedThread else { return }
        Task {
            do {
                let _: CommandResult = try await client.call(
                    "set_thread_flagged", ["thread_id": thread.id, "flagged": !thread.isFlagged])
            } catch { report(error) }
        }
    }

    /// Moves the selection by `delta` rows (j = +1, k = -1), staying within
    /// the list. With no selection, j selects the first row.
    func selectNext(_ delta: Int) {
        guard !threads.isEmpty else { return }
        let current = threads.firstIndex { $0.id == selectedThreadId } ?? -1
        let next = max(0, min(threads.count - 1, current + delta))
        selectedThreadId = threads[next].id
    }

    func report(_ error: Error) {
        lastError = error.localizedDescription
    }

    // MARK: - Deep links

    /// Opens the mail of a deep link `nicemailer://m/<id>` (`DeepLink`,
    /// `docs/deep-links.md`): asks the core where the message is, then shows
    /// its thread in its folder. Before the core answered (the link launched
    /// the app), the link waits for the connection.
    func openDeepLink(_ url: URL) {
        guard let id = DeepLink.messageId(from: url) else {
            lastError = L("Not a link to a NiceMailer mail: %@", url.absoluteString)
            return
        }
        guard case .connected = status, let client else {
            pendingDeepLink = url
            return
        }
        Task {
            do {
                let place: MessageLocation = try await client.call("locate_message", ["message_id": id])
                guard let threadId = place.threadId else {
                    lastError = L("This mail is not in a conversation yet")
                    return
                }
                lastError = nil
                show(threadId: threadId, folderId: place.folderId)
                onDeepLinkOpened?()
            } catch let e as CoreClient.RPCError where e.code == CoreClient.RPCError.notFound {
                // Deleted since, or the database was rebuilt (ids changed).
                lastError = L("This mail is no longer in NiceMailer")
            } catch { report(error) }
        }
    }

    private func openPendingDeepLink() {
        guard let url = pendingDeepLink else { return }
        pendingDeepLink = nil
        openDeepLink(url)
    }

    /// The connected client, for the actions defined in other files
    /// (`AppModel+Actions.swift`).
    var core: (any CoreCalling)? { client }

    /// Uses `core` instead of the socket and loads the data: used by the
    /// unit tests, with a fake core. No event stream in that case.
    func attach(_ core: any CoreCalling) async {
        connection?.disconnect()
        connection = nil
        client = core
        status = .connected(coreVersion: "test")
        await reloadAll()
        openPendingDeepLink()
    }

    /// Reloads the open thread (after a change of display preference).
    func reloadOpenThread() async { await loadOpenThread(markSeen: false) }
}
