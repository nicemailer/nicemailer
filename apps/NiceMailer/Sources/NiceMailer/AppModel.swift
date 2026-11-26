import AppKit
import Foundation
import Observation
import os

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
    /// Connection state, shown by the window (`WindowContent`).
    enum Status: Equatable {
        case connecting
        case connected(coreVersion: String)
        /// The core is in maintenance (`CoreMaintenance`): NiceMailer being
        /// updated, or the mailbox upgraded. The text of the screen and the
        /// few words of the status bar, updated every second while the app
        /// waits (`waitOutMaintenance`).
        case maintenance(text: String, short: String)
        case failed(String)
    }

    // `private(set)`: readable from anywhere, writable only from this class.
    private(set) var status: Status = .connecting

    /// Where the launch is, until the first list is shown: what the app
    /// says while the list is still empty (`StartupPhase.message`), so that
    /// a core that takes a few seconds to start does not look like an
    /// empty mailbox.
    enum StartupPhase: Equatable {
        /// Opening the socket of a core already running.
        case connecting
        /// No core answered: launchd is starting it (`openSocket`).
        case startingCore
        /// Connected: the folders and the first list are loading.
        case loading
        /// The first list is shown.
        case ready

        /// The line shown in the empty list and the status bar; nil once
        /// ready.
        var message: String? {
            switch self {
            case .connecting: L("Connecting to the core…")
            case .startingCore: L("Starting the core…")
            case .loading: L("Loading the mail…")
            case .ready: nil
            }
        }
    }

    private(set) var startupPhase: StartupPhase = .connecting
    /// When the model was made, at the app's launch, to time it
    /// (`StartupTimings`).
    @ObservationIgnored private let created = ContinuousClock.now
    /// How long the launch took, once the first list is shown.
    @ObservationIgnored private(set) var startupTimings: StartupTimings?
    /// When the socket answered, for `startupTimings`.
    @ObservationIgnored private var socketOpened: ContinuousClock.Instant?
    /// The app had to start the core (`openSocket`), for `startupTimings`.
    @ObservationIgnored private var startedCore = false
    private(set) var folders: [FolderView] = []
    /// The contact groups (`list_groups`), in display order: the folder
    /// column lists them under the folders, the Group… attribute offers
    /// them (`ui.md`, "Groupes dans la colonne des dossiers").
    private(set) var groups: [GroupView] = []
    /// Threads of the selected folder, as far as loaded (pages are appended
    /// as the user scrolls).
    // Settable from `AppModel+Actions.swift` (optimistic updates); a
    // `private(set)` setter would only be reachable from this file.
    var threads: [ThreadSummary] = [] {
        didSet {
            // The AI's suggestions follow the rows shown (`AppModel+AI`).
            if threads.map(\.id) != oldValue.map(\.id) { scheduleSuggestions() }
        }
    }
    /// Cursor of the next page of `threads`; nil when everything is loaded.
    private(set) var nextCursor: String?
    /// Thread shown in the reading pane.
    private(set) var openThread: ThreadView?
    private(set) var searchResults: [SearchHit] = []
    /// Error shown in an alert; the view sets it back to nil when dismissed.
    var lastError: String?
    /// Directory where `export` writes, passed to the core; `nil` lets the
    /// core choose ~/Downloads. Set by `NICEMAILER_EXPORT_DIR` in a debug
    /// build (tests must never write into the user's Downloads).
    var exportDirectory: String? = DevEnvironment.value("NICEMAILER_EXPORT_DIR")
    /// Opens a file with its default app, which comes to the front. Replaced
    /// in UI script mode and in unit tests, which must never launch Preview
    /// or Word on the user's screen.
    @ObservationIgnored var openFile: (URL) -> Void = { NSWorkspace.shared.open($0) }
    /// Opens a `tel:` URL with the closure it is given and presses the
    /// confirmation macOS shows before calling (`CallConfirmation`).
    /// Replaced in UI script mode and in unit tests, which must never drive
    /// the user's notifications nor place a call.
    @ObservationIgnored var confirmCall: @MainActor (URL, () -> Void) async -> CallConfirmation.Outcome = { url, open in
        await CallConfirmation.confirm(url, open: open)
    }
    /// Short message shown in the geek status bar (result of an action,
    /// "not available yet"…); cleared by the next key.
    var flash: String? {
        // Swift note: `didSet` runs after each assignment, here to count
        // them; `@Observable` keeps tracking the property as before.
        didSet { flashCount &+= 1 }
    }
    /// How many messages the status bar was given: a message computed
    /// after a wait (the reply screen opening) is shown only if nothing
    /// newer was said meanwhile (`AppModel.sayOpening`).
    @ObservationIgnored private(set) var flashCount = 0
    /// The conversation whose remote images the user asked to load although
    /// its sender's mail is not authenticated (`RemoteImages.automatic`):
    /// while it stays open, until another display is chosen. Forgotten when
    /// another conversation is opened; never stored.
    var remoteImagesOnce: Int?
    /// Where the app puts the files to attach, for the core to take them
    /// (`brouillons.md`, section 7): `incoming.noindex` of the data
    /// directory. Unit tests use a directory of their own.
    @ObservationIgnored var incomingDirectory: URL = DataDirectory.current.appendingPathComponent("incoming.noindex")
    /// Asks the user for files to attach: the open panel. Replaced in UI
    /// script mode (the scenario names the files) and in unit tests.
    @ObservationIgnored var chooseFiles: @MainActor () async -> [URL] = {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        panel.prompt = L("Attach")
        return await panel.begin() == .OK ? panel.urls : []
    }
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
    /// The observer of the address book's changes (`watchAddressBook`).
    @ObservationIgnored var addressBookObserver: NSObjectProtocol?

    /// What `z` undoes, most recent last: moves and decisions of this
    /// session (`AppModel+Workflow.swift`).
    var undoStack: [UndoStep] = []
    /// A move refused until the user confirms it: its sender's mail is
    /// suspected of spoofing (`authentification-expediteur.md`, section 5).
    var suspectMove: SuspectMove?

    /// `ui.title_bar` of `config.toml`, only while the file writes it: it
    /// still wins over the theme's (`themes.md`, section 9).
    var configTitleBar: String?
    /// The title bar of the window (`WindowChrome`): as `ui.title_bar`
    /// says while it is written, else as the theme says
    /// (`window.title_bar`). `merged`, the buttons in the status bar, only
    /// while the status bar is at the top: elsewhere there is nothing to
    /// hold them, and the title bar is shown.
    var titleBarMode: WindowChrome.Mode {
        switch configTitleBar ?? theme.current.window.titleBar {
        case "hidden": .hidden
        case "merged": statusBarAtTop ? .merged : .shown
        default: .shown
        }
    }
    /// The title bar is hidden: the content goes up to the top edge.
    var titleBarHidden: Bool { titleBarMode == .hidden }
    /// The status bar is at the top of the window (`InterfaceController`,
    /// `statusPosition`): where `merged` puts the window's buttons.
    var statusBarAtTop = true
    /// The keys of the geek theme: the defaults until the core answers, then
    /// those of `keymap.toml` (`keymap_get`, `docs/keymap.md`).
    var keymap: Keymap = .defaults

    /// The names of the folders (`FolderNames`): those of `[folder_names]`
    /// in `config.toml`, English by default. Observed: a new name in the
    /// file redraws the sidebar, the status bar and the menus at once.
    var folderNames: FolderNames = .english

    /// The synchronizations in progress, by account and stage, from the
    /// core's `SyncProgress` (`AppModel+Sync.swift`): what the status bar
    /// says while a large account is imported.
    var syncActivity: [String: SyncActivity] = [:]
    /// The task that ends the activities the core stopped speaking of.
    /// Swift note: `@ObservationIgnored` keeps a property out of what
    /// SwiftUI observes; a task is no state the interface shows.
    @ObservationIgnored var syncPrune: Task<Void, Never>?
    /// Why `keymap.toml` is refused, while it is: the previous keys stay,
    /// and the status bar says so until the file is fixed.
    var keymapProblem: KeymapProblem?

    /// The theme in effect (`ThemeStore`, `docs/themes.md`): the shared one
    /// in the app, one of its own in each unit test.
    @ObservationIgnored var theme: ThemeStore = .shared
    /// Why a file of the theme's chain is refused, while it is: the theme
    /// is made of its last valid version, and the status bar says so until
    /// the file is fixed.
    var themeProblem: KeymapProblem?

    /// The text sizes of the window and of its status bar (`TextSize`):
    /// those of `config.toml`, moved by ⌘+ / ⌘−. The shared one in the app,
    /// one of its own in each unit test.
    @ObservationIgnored var textSize: TextSize = .shared

    /// `ui.auto_confirm_calls` of `config.toml`: a number called from the
    /// app is confirmed by the app itself (`CallConfirmation`). Off by
    /// default: it needs the Accessibility permission.
    var autoConfirmCalls = false
    /// `unsubscribe.in_banned` of `config.toml`: the unsubscribe button
    /// shows on the mails of Banned too (`UnsubscribeDisplay.offer`). Off
    /// by default.
    var unsubscribeInBanned = false

    /// The agent of the core, when the app carries it (`CoreAgent`).
    var agentState: CoreAgent.State = .notBundled
    /// The mailbox in use (`DataDirectory`), for the window's title.
    var mailboxName: String { DataDirectory.displayName(DataDirectory.current) }
    /// Checks and upgrades a mailbox before the app switches to it
    /// (`MailboxCheck.swift`): the CLI the app carries; nil without one
    /// (`swift run`), and then the folder is used unchecked, as before.
    @ObservationIgnored var mailboxTool: (any MailboxTool)? = CLIMailboxTool.find()
    /// A mailbox of an earlier data format, waiting for the user's answer:
    /// upgrade it and use it, or keep the current mailbox.
    var pendingUpgrade: PendingUpgrade?
    /// The mailbox being upgraded, while the CLI works.
    var upgradingMailbox: URL?
    /// Replaces the switch itself (recording the choice, restarting the
    /// core) in the unit tests and the UI scenarios, which must never
    /// change the user's mailbox; it receives the folder chosen.
    @ObservationIgnored var switchMailboxOverride: ((URL) async -> Void)?
    /// Where the Preferences send the account creation methods: the core,
    /// unless UI script mode puts its stand-in in between
    /// (`UIScriptSetupCore`), which answers them without any mail server.
    @ObservationIgnored var accountsCoreOverride: (any CoreCalling)?
    /// The core the Preferences talk to.
    var accountsCore: (any CoreCalling)? { accountsCoreOverride ?? core }
    /// Goes up each time the core announces that an account changed
    /// (`AccountChanged`, `AccountError`, `ConfigChanged`): the account list
    /// of the Preferences reads it again.
    private(set) var accountsRevision = 0

    /// The reply being written (`AppModel+Compose.swift`); nil when the
    /// reader shows the mail.
    var composer: Composer?
    /// The editor of the reply's text (`EditorBridge`, `edition.md`): set
    /// by the window at launch, a fake one in the unit tests.
    @ObservationIgnored var editor: MailTextEditor?
    /// Makes the editor (the window sets it at launch): called once the
    /// first list is shown, so that WebKit's start does not slow the
    /// launch down, or at the first reply opened before that.
    @ObservationIgnored var makeEditor: (() -> MailTextEditor)?

    /// The editor, made now if it was not yet (`makeEditor`).
    func ensureEditor() {
        if editor == nil, let make = makeEditor { editor = make() }
    }
    /// The lists of the formatted mode and the mode a draft starts in, from
    /// the core (`compose_formats`, `edition.md`, section 5).
    @ObservationIgnored var composeFormats = ComposeFormats.empty
    /// What is under the editor's cursor, as it last said: the AI palette's
    /// passage, the UI scenarios.
    var editorState = EditorStateView()
    /// ⇧⌘T asked to go back to text only, the formatting to be lost: a
    /// second ⇧⌘T within a few seconds confirms (`toggleComposeFormat`).
    @ObservationIgnored var formatLossArmed: Date?
    /// The AI run shown in the panel of the reply (`AppModel+AI.swift`);
    /// nil when the panel is closed.
    var aiRun: AIRun?
    /// The passage selected in the reply when the AI palette opened (⌃I),
    /// in UTF-16 units of the editor's text; `nil` for the whole text. In
    /// a formatted reply, the AI's text for it goes back there
    /// (`replaceRange`).
    var aiPassage: NSRange?
    /// AI events that came before the answer of `ai_run` told which run is
    /// ours (a fake or quick provider ends before): kept by run id, replayed
    /// once it is known.
    @ObservationIgnored var aiEarly: [Int: [CoreClient.Event]] = [:]
    /// A sorting of several conversations under way (`a` `s`), for the
    /// status bar.
    var aiSort: AISort?
    /// The target the AI suggests for the conversations shown, by thread.
    var aiSuggestions: [Int: AISuggestion] = [:]
    /// The AI is on (`ai_status`): only then are suggestions asked for.
    @ObservationIgnored var aiEnabled = false
    /// The pending load of the suggestions, restarted when the rows change.
    @ObservationIgnored var aiSuggestionsTask: Task<Void, Never>?
    /// The pending save of the reply's text, restarted at each key.
    @ObservationIgnored var composeSaveTask: Task<Void, Never>?
    /// The save in progress, which the next one waits for.
    @ObservationIgnored var composeSaving: Task<Void, Never>?
    /// The writing of a proposed draft in progress (`ensureWritten`): a
    /// second caller waits for it rather than write the draft twice.
    @ObservationIgnored var composeWriting: Task<Bool, Never>?
    /// Mails waiting for their undo delay or for the server, by draft id:
    /// the subject, for the status bar when they leave.
    @ObservationIgnored var sending: [Int: String] = [:]
    /// The addresses a mail can be sent from (`list_senders`), loaded when
    /// a reply opens: the line "From" and ⌥⌘F "Send from".
    var senders: [SenderView] = []

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
                virtual = .everything
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

    /// The virtual folders (`ui.md`, "Dossiers virtuels"): lists of several
    /// folders together, each row with its folder. Shown with `everything`
    /// set (its scope: every account, or one).
    /// - `work`: Inbox, Todo and Waiting, what is still to deal with;
    /// - `allMail`: every folder but Deleted, Junk and Banned, what most
    ///   searches want (`/` lands there);
    /// - `everything`: every folder.
    /// The core reads each in one walk of an index (`list_threads`,
    /// `everything` with `work` or `all_mail`).
    enum Virtual: String, CaseIterable {
        case work, allMail, everything

        /// Its id in `[folder_names]` of `config.toml`, that of its key
        /// (`goto.all_mail`).
        var id: String {
            switch self {
            case .work: "work"
            case .allMail: "all_mail"
            case .everything: "everything"
            }
        }

        /// Its English name. The app shows `FolderNames.name(of:)`, the
        /// name of `config.toml`; this one names it where the English
        /// stays (the targets of the UI scenarios, `KeyAction.label`).
        var name: String {
            switch self {
            case .work: AppModel.workName
            case .allMail: AppModel.allMailName
            case .everything: AppModel.everythingName
            }
        }

        /// The roles whose folders it lists (`nil`: every folder).
        var roles: Set<String>? {
            switch self {
            case .work: ["inbox", "todo", "waiting"]
            case .allMail: nil
            case .everything: nil
            }
        }
    }
    /// Which virtual folder `everything` shows.
    private(set) var virtual: Virtual = .everything
    /// Whether All Mail is shown (`virtual`, with `everything` set).
    var allMail: Bool { everything != nil && virtual == .allMail }

    /// Names of the virtual folders (names, like the folder names, not
    /// translated).
    static let everythingName = "Everything"
    static let allMailName = "All Mail"
    static let workName = "Work"

    /// The roles All Mail leaves out (`FolderRole::OUT_OF_ALL_MAIL`).
    static let outOfAllMail: Set<String> = ["deleted", "junk", "banned"]

    /// Shows "Everything", or "All Mail", of the unified view or of an
    /// account.
    func selectEverything(_ e: Everything, virtual: Virtual = .everything) {
        let changed = everything != e || self.virtual != virtual || selectedFolderId != nil || unifiedRole != nil || listFilter != nil
        everything = e
        self.virtual = virtual
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
        virtual = .everything
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
        // All Mail: an event about Deleted, Junk or Banned changes nothing
        // it shows (the folder a thread left is told by its own event);
        // Work: only its three folders.
        let virtual = everything != nil ? self.virtual : .everything
        let inList = { (f: FolderView) in
            if virtual == .allMail { return !Self.outOfAllMail.contains(f.role ?? "") }
            if let roles = virtual.roles { return roles.contains(f.role ?? "") }
            return true
        }
        switch everything {
        case .all: return folders.filter(inList).map(\.id)
        case .account(let a): return folders.filter { $0.accountId == a && inList($0) }.map(\.id)
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
    static func listScope(everything: Everything?, virtual: Virtual = .everything, folderIds: [Int], listFilter: ListFilter? = nil) -> [String: Any] {
        if listFilter != nil { return ["everything": [String: Any]()] }
        var scope: [String: Any] = switch virtual {
        case .allMail: ["all_mail": true]
        case .work: ["work": true]
        case .everything: [:]
        }
        switch everything {
        case .all: return ["everything": scope]
        case .account(let a):
            scope["account_id"] = a
            return ["everything": scope]
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
        return Self.folderLabels(folders, withAccount: e == .all, names: folderNames)
    }

    /// Folder of the selected thread, in "Everything" only (the reader
    /// shows it next to the subject).
    var selectedThreadFolderLabel: String? {
        guard let e = everything, let t = selectedThread else { return nil }
        return folders.first { $0.id == t.folderId }.map { Self.folderLabel($0, withAccount: e == .all, names: folderNames).text }
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

    /// Label of `f`, with its account when `withAccount` ("all"), its name
    /// taken from `names`.
    static func folderLabel(_ f: FolderView, withAccount: Bool, names: FolderNames = .english) -> FolderLabel {
        FolderLabel(folder: names.name(of: f), account: withAccount ? f.accountName : nil)
    }

    /// `folderLabel` of every folder, by folder id.
    static func folderLabels(_ folders: [FolderView], withAccount: Bool, names: FolderNames = .english) -> [Int: FolderLabel] {
        // Swift note: `uniquingKeysWith` keeps the first value if two
        // folders had the same id (they cannot, but the initializer without
        // it would crash on a duplicate key).
        Dictionary(folders.map { ($0.id, folderLabel($0, withAccount: withAccount, names: names)) },
                   uniquingKeysWith: { first, _ in first })
    }

    /// Name of the folder shown.
    var currentFolderName: String {
        if everything != nil { return folderNames.name(of: virtual) }
        return unifiedRole.map { folderNames.name($0) } ?? selectedFolder.map { folderNames.name(of: $0) } ?? ""
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
            // Going down the list with the keys loads the rows that follow.
            prefetchIfNearEnd()
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
    /// The load of the open thread running (`loadOpenThread`), and the one
    /// wanted next.
    @ObservationIgnored private var threadLoader: Task<Void, Never>?
    @ObservationIgnored private var threadWanted: ThreadLoad?

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
    /// The reload due or running (`scheduleRefresh`): the next one waits
    /// for it, so that two reloads never run at once.
    private var refreshTask: Task<Void, Never>?
    /// A reload is due and has not started: the events received meanwhile
    /// only add to what it reloads.
    private var refreshDue = false
    /// The reload due must reload everything shown (a gap in the events).
    private var refreshFull = false
    /// When the last reload started, for `refreshDelay`.
    private var lastRefreshStart: ContinuousClock.Instant?
    /// How many reloads started: for the tests of the pacing.
    @ObservationIgnored private(set) var refreshCount = 0

    /// Waits until the reload under way, if any, has ended: for the tests
    /// that must start from a quiet list (`ListPatchTests`).
    func refreshSettled() async {
        while let task = refreshTask {
            await task.value
            if refreshTask == task, !refreshDue { return }
        }
    }
    /// How many refreshes reloaded the conversations that changed alone
    /// (`patchThreads`), and how many the whole list: for the UI scripts.
    @ObservationIgnored private(set) var patchCount = 0
    @ObservationIgnored private(set) var wholeReloadCount = 0
    /// The pending debounced search, cancelled when a newer one replaces it.
    private var searchTask: Task<Void, Never>?
    /// Folders and threads invalidated by events since the last refresh.
    private var dirtyFolders = Set<Int>()
    private var dirtyThreads = Set<Int>()
    /// Folders of those whose `FolderChanged` did not say that every
    /// conversation that changed in them was announced (`threads_announced`,
    /// `evenements.md`): a list showing one of them is reloaded whole; the
    /// others only reload their conversations that changed (`patchThreads`).
    private var unannouncedFolders = Set<Int>()

    nonisolated static let pageSize = 60

    /// The last maintenance that left the app without a core
    /// (`newer_schema`, `migration_failed`): "Try Again" restarts the core
    /// after a failed migration (`retry`), and the screen offers its log.
    private(set) var lastMaintenance: CoreMaintenance?
    /// The core was restarted once because its version was not the app's
    /// (`coreNeedsRestart`): never twice in a launch, so that an app run
    /// against another core (development) does not restart it in a loop.
    @ObservationIgnored private var restartedCoreForVersion = false
    /// An update left over by an earlier launch was cancelled once
    /// (`connect`).
    @ObservationIgnored private var cancelledStaleUpdate = false
    /// This app is installing an update (`prepareUpdate`): the maintenance
    /// of the core is its own, waited for rather than cancelled.
    @ObservationIgnored private(set) var installingUpdate = false

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
            socketOpened = socketOpened ?? .now
            if startupPhase != .ready { startupPhase = .loading }
            connection = c
            client = c
            let hello: HelloResult = try await c.call("hello", ["protocol_version": CoreClient.protocolVersion, "client": "app"])
            // The core still runs the version before an update: Sparkle
            // replaced the bundle, and the app, relaunched, restarts the core
            // from it (`mise-a-jour.md`, section 3, step 4). The core also
            // leaves by itself once its program is replaced; this covers a
            // core that did not (an update installed by hand, an app copied
            // over the old one).
            if Self.coreNeedsRestart(coreVersion: hello.coreVersion, appVersion: Self.appVersion,
                                     agent: agentState, alreadyRestarted: restartedCoreForVersion) {
                restartedCoreForVersion = true
                CoreAgent.log.notice("core \(hello.coreVersion, privacy: .public) is not the app's \(Self.appVersion ?? "?", privacy: .public): restarted")
                c.disconnect()
                CoreAgent.kickstart(restart: true)
                await connect()
                return
            }
            // Subscribe *before* loading data (`evenements.md`, section 7):
            // a change happening during the loading will then be announced by
            // an event, instead of being silently missed.
            let sub: SubscribeResult = try await c.call("subscribe", [:])
            lastEvent = Position(epoch: sub.epoch, seq: sub.seq)
            status = .connected(coreVersion: hello.coreVersion)
            // Listen to events in a separate task, which runs as long as the
            // connection lives, while this function goes on.
            Task { await listen(c) }
            await startSession()
        } catch {
            // Inside `catch`, the error is implicitly named `error`. A core
            // in maintenance says why (`CoreMaintenance`): waited for, or
            // shown, never "unreachable" (`mise-a-jour.md`, section 5).
            if let m = CoreMaintenance(error: error) ?? connection?.closedForMaintenance {
                // The maintenance of an update this app is not installing:
                // left by an earlier launch that prepared it, then crashed
                // or saw Sparkle fail (the version is the core's, or the
                // core would have been restarted above). Cancelled once,
                // rather than waited for until `core.update_timeout_secs`.
                if m.reason == .update, !installingUpdate, !cancelledStaleUpdate, let c = connection {
                    cancelledStaleUpdate = true
                    CoreAgent.log.notice("an update prepared by an earlier launch: cancelled")
                    let _: CancelUpdateResult? = try? await c.call("cancel_update", [:])
                    await connect()
                    return
                }
                if await waitOutMaintenance(m) { await connect() }
                return
            }
            // A core that stopped on an error said why in its folder: that
            // reason is more useful than "unreachable".
            status = .failed(Self.noCoreMessage(error.localizedDescription,
                                                report: StartupErrorReport.read(in: DataDirectory.current)))
        }
    }

    /// Sets the status as a UI scenario plays it (`UIScript`,
    /// `maintenance off`): back to the screen before a maintenance it
    /// played, without reconnecting to the scenario's core.
    func showStatusForScript(_ s: Status) {
        guard UIScript.isActive else { return }
        status = s
        lastMaintenance = nil
    }

    /// "Try Again" of the screen without a core. After a failed migration,
    /// the core stays in maintenance until it is restarted: restarted, it
    /// tries the migration again. Only on this button: the app never tries
    /// a migration again by itself (`mise-a-jour.md`, section 4).
    func retry() async {
        if lastMaintenance?.reason == .migrationFailed, agentState == .enabled {
            CoreAgent.kickstart(restart: true)
        }
        await connect()
    }

    /// While the core is in a maintenance that ends by itself (an update,
    /// a migration), shows it and asks again every second (`probe`), without
    /// starting a core: it runs. Returns true once the core serves again,
    /// false when the maintenance needs the user (`newer_schema`,
    /// `migration_failed`), whose reason is then the screen's.
    /// `every` and `probe` are parameters for the tests.
    func waitOutMaintenance(_ first: CoreMaintenance, every: Duration = .seconds(1),
                            probe: (() async -> CoreMaintenance?)? = nil) async -> Bool {
        let probe = probe ?? { await Self.probeMaintenance() }
        var m = first
        lastMaintenance = nil
        while m.isTransient {
            status = .maintenance(text: m.text(), short: m.shortText)
            try? await Task.sleep(for: every)
            guard let next = await probe() else { return true }
            m = next
        }
        lastMaintenance = m
        status = .failed(m.text())
        return false
    }

    /// The maintenance of the core, asked on a connection of its own: the
    /// app's `hello` is answered during an update, and its `subscribe`
    /// refused. Nil when the core serves (or cannot be reached: `connect`
    /// then says what follows).
    nonisolated static func probeMaintenance() async -> CoreMaintenance? {
        guard let c = try? CoreClient() else { return nil }
        defer { c.disconnect() }
        do {
            let _: HelloResult = try await c.call("hello", ["protocol_version": CoreClient.protocolVersion, "client": "app"])
            let _: SubscribeResult = try await c.call("subscribe", [:])
            return nil
        } catch {
            return CoreMaintenance(error: error) ?? c.closedForMaintenance
        }
    }

    /// The version of the app (`CFBundleShortVersionString`), nil outside a
    /// bundle (`swift run`).
    nonisolated static var appVersion: String? {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    }

    /// Whether the core must be restarted because it is not of the app's
    /// version: only the core of the agent, which the app can restart
    /// (`CoreAgent`), and once a launch. A pure function, tested.
    nonisolated static func coreNeedsRestart(coreVersion: String, appVersion: String?,
                                             agent: CoreAgent.State, alreadyRestarted: Bool) -> Bool {
        guard agent == .enabled, !alreadyRestarted, let appVersion else { return false }
        return coreVersion != appVersion
    }

    /// Called by the updater just before Sparkle installs an update
    /// (`Updater`): the core enters maintenance, stops its IMAP workers and
    /// stays alive until it is replaced (`mise-a-jour.md`, section 3, step
    /// 2). Waits at most `limit` for its answer: the core stops its workers
    /// in at most 5 s; a core that does not answer must not hold the update.
    func prepareUpdate(limit: Duration = .seconds(10)) async {
        installingUpdate = true
        guard let client else { return }
        // Swift note: a task group runs both children at once; the first
        // to finish wins, and `cancelAll` stops the other.
        await withTaskGroup(of: Void.self) { group in
            group.addTask {
                do {
                    let r: PrepareUpdateResult = try await client.call("prepare_update", [:])
                    CoreAgent.log.notice("update prepared; the core waits \(r.timeoutSecs) s at most")
                } catch {
                    CoreAgent.log.error("prepare_update: \(error.localizedDescription, privacy: .public)")
                }
            }
            group.addTask { try? await Task.sleep(for: limit) }
            await group.next()
            group.cancelAll()
        }
    }

    /// The update was abandoned after `prepareUpdate` (Sparkle failed to
    /// install it): the core serves again.
    func cancelUpdate() async {
        guard installingUpdate else { return }
        installingUpdate = false
        let _: CancelUpdateResult? = try? await client?.call("cancel_update", [:])
        await connect()
    }

    /// What the screen without a core says: the reason the core left in its
    /// folder (`StartupErrorReport`), else the connection's error.
    nonisolated static func noCoreMessage(_ connectionError: String, report: StartupErrorReport?) -> String {
        report?.userMessage ?? connectionError
    }

    /// Reads the keys in effect from the core (`keymap_get`). An older core
    /// without the method leaves the defaults.
    /// Reads the theme in effect from the core (`theme_get`). An older core
    /// without the method leaves the theme the app has.
    func loadTheme() async {
        guard let client, let view: ThemeView = try? await client.call("theme_get", [:]) else { return }
        let changed = view != theme.current
        theme.apply(view)
        themeProblem = view.error
        // The window is rebuilt in the new theme: the targets of a scenario
        // are recorded again where they now are (`UIScript.locate` waits
        // for them), not clicked where the previous theme drew them.
        if changed, UIScript.isActive {
            UIScript.frames = UIScript.frames.filter { $0.key.hasPrefix("prefs-") }
        }
        // A font the theme names that the Mac does not have: the system
        // font stands in, and the status bar says so once, when the theme
        // arrives (`themes.md`, section 7).
        if changed, let missing = [view.fonts.interface, view.fonts.reading].first(where: { !Fonts.isAvailable($0) }) {
            flash = L("Font “%@” not installed: using the system font", missing)
        }
    }

    func loadKeymap() async {
        guard let client, let view: KeymapView = try? await client.call("keymap_get", [:]) else { return }
        let new = Keymap(contexts: view.contexts, menus: view.menus)
        if new != keymap { keymap = new }
        keymapProblem = view.error
    }

    /// Reads the settings of `config.toml` the app applies itself (`[ui]`,
    /// `[unsubscribe]`, `[folder_names]`). A core that does not give them
    /// leaves the defaults.
    func loadUISettings() async {
        guard let client else { return }
        if let status: ConfigStatusView = try? await client.call("config_status", [:]) {
            if let ui = status.ui {
                // Written, `ui.title_bar` wins over the theme's.
                configTitleBar = ui.titleBar
                autoConfirmCalls = ui.autoConfirmCalls ?? false
                // Applied at once: the window is rebuilt at the new sizes
                // (`WindowContent`, `.id`).
                textSize.configure(size: ui.textSize, statusBar: ui.statusBarTextSize)
            }
            // An older core without `[unsubscribe]` keeps it closed, as
            // that core refuses it.
            unsubscribeInBanned = status.unsubscribe?.inBanned ?? false
            // Assigned only when they changed: an assignment redraws every
            // view that shows a folder, even with the same names.
            let names = FolderNames(byRole: status.folderNames ?? [:])
            if names != folderNames { folderNames = names }
        }
        // The lists of the formatted mode and the mode a draft starts in
        // (`edition.md`, section 5): an older core without them leaves the
        // formatted mode with no font, size or colour to choose.
        if let formats: ComposeFormats = try? await client.call("compose_formats", [:]) {
            composeFormats = formats
        }
        await refreshAIEnabled()
    }

    /// The longest wait between two tries to open the socket of a core
    /// being started (`openSocket`).
    nonisolated static let socketRetryMax = 0.1

    /// The wait before the next try, after a wait of `delay` seconds:
    /// doubled, up to `socketRetryMax`. A pure function, tested.
    nonisolated static func nextSocketDelay(_ delay: Double) -> Double {
        min(delay * 2, socketRetryMax)
    }

    /// Opens the socket of the core. With the agent, a core that does not
    /// answer is started (`launchctl kickstart`), and waited for a few
    /// seconds, the time to open its database (`architecture.md`, section
    /// 6, "Démarrage à la demande").
    private func openSocket() async throws -> CoreClient {
        let deadline = Date().addingTimeInterval(8)
        var lastKick: Date?
        // At most one repair per try: a core that launchd still refuses
        // after it (a damaged bundle) is not registered over and over.
        var repaired = false
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
                    // No core answered: it is being started, which takes
                    // a moment (its database to open). Said as such.
                    if startupPhase == .connecting { startupPhase = .startingCore }
                    startedCore = true
                    // From the second ask on, 2 s without a core: launchd
                    // may be refusing it, because the agent was registered
                    // for a core signed differently (`CoreAgent.
                    // repairIfSpawnRefused`). Checked before asking again,
                    // and never on the first ask, the fast path.
                    if lastKick != nil, !repaired, CoreAgent.repairIfSpawnRefused() {
                        repaired = true
                    } else {
                        CoreAgent.kickstart()
                    }
                    lastKick = Date()
                }
                try? await Task.sleep(for: .seconds(delay))
                // At most 0.1 s between two tries: a try is a connect() on
                // a local socket, which costs nothing, while each step of
                // a longer wait was time the user waited after the core
                // was ready (up to 0.5 s, measured at a launch).
                delay = Self.nextSocketDelay(delay)
            }
        }
    }

    /// "Change Mailbox…": checks the folder first (`MailboxReport.decision`).
    /// A database the core would refuse is refused here, and the current
    /// mailbox stays; one of an earlier data format waits for the user's
    /// answer (`pendingUpgrade`); otherwise the app switches to it.
    func changeMailbox(to url: URL) async {
        // A new choice: what was said about the previous one no longer
        // applies (a refusal left in the status bar beside a new question).
        lastError = nil
        if DataDirectory.environment != nil, switchMailboxOverride == nil {
            lastError = L("NICEMAILER_DATA_DIR is set: it decides the mailbox; launch the app without it to choose one")
            return
        }
        guard let tool = mailboxTool else {
            await switchMailbox(to: url)
            return
        }
        let report: MailboxReport
        do {
            report = try await tool.check(url)
        } catch {
            lastError = L("This folder cannot be checked (%@). The current mailbox is kept.", error.localizedDescription)
            return
        }
        switch report.decision {
        case .use:
            await switchMailbox(to: url)
        case .askUpgrade(let from, let to):
            pendingUpgrade = PendingUpgrade(url: url, from: from, to: to)
        case .refuse(let message):
            lastError = message
        }
    }

    /// The answer "upgrade": the CLI migrates the database, then the app
    /// switches to it. The current mailbox stays in use meanwhile, and if
    /// the upgrade fails. The upgrade is done before the switch rather than
    /// by the core at its start: the app waits a few seconds only for a
    /// core to answer, and a large mailbox takes longer.
    ///
    /// The question is answered at once, before the returned task runs: a
    /// dialog that closes right after its button (SwiftUI's) then finds
    /// nothing left to cancel. The task is returned for the tests.
    @discardableResult
    func confirmMailboxUpgrade() -> Task<Void, Never>? {
        guard let p = pendingUpgrade, let tool = mailboxTool else { return nil }
        pendingUpgrade = nil
        upgradingMailbox = p.url
        return Task { await upgrade(p, with: tool) }
    }

    private func upgrade(_ p: PendingUpgrade, with tool: any MailboxTool) async {
        flash = L("Upgrading %@ to the data format %lld…", DataDirectory.displayName(p.url), p.to)
        do {
            try await tool.upgrade(p.url)
        } catch {
            upgradingMailbox = nil
            flash = nil
            lastError = L("The mailbox could not be upgraded (%@). The current mailbox is kept.", error.localizedDescription)
            return
        }
        upgradingMailbox = nil
        flash = L("%@ upgraded", DataDirectory.displayName(p.url))
        await switchMailbox(to: p.url)
    }

    /// The answer "cancel": nothing changes.
    func cancelMailboxUpgrade() {
        guard let p = pendingUpgrade else { return }
        pendingUpgrade = nil
        flash = L("%@ not upgraded: the current mailbox is kept", DataDirectory.displayName(p.url))
    }

    /// Uses the mailbox in `url` from now on (`DataDirectory`): the choice
    /// is recorded, the core started again on it, and everything reloaded.
    /// Without the agent, the core started by hand must be restarted.
    private func switchMailbox(to url: URL) async {
        if let switchMailboxOverride {
            await switchMailboxOverride(url)
            return
        }
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
        virtual = .everything
        selectedFolderId = nil
        lastEvent = nil
        if CoreAgent.register() == .enabled {
            CoreAgent.kickstart(restart: true)
            await connect()
        } else {
            status = .failed(L("Mailbox changed to %@: restart the core on it (make core)", url.path))
        }
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
    func handle(_ e: CoreClient.Event) {
        let order = AppModel.order(of: e, after: lastEvent)
        switch order {
        case .transient:
            // Progress of a synchronization (said in the status bar), a
            // fragment of an AI answer: nothing to reload, and the position
            // stays that of the last numbered event.
            if e.method == "AiDelta" || e.method == "AiProgress" { aiEvent(e) }
            if e.method == "SyncProgress" { syncEvent(e) }
            return
        case .inOrder(let p), .gap(let p):
            lastEvent = p
        }
        // The end of an AI run is taken whatever its order: dropped with
        // the rest after a gap, it would leave the panel waiting for good.
        if e.method == "AiDone" || e.method == "AiFailed" || e.method == "AiSuggestions" { aiEvent(e) }
        if e.method == "ResyncRequired" || order.isGap {
            scheduleRefresh(full: true)
            // Events were missed: the end of the run among them, maybe, and
            // a change of the settings, the keys or the theme, which have
            // no list to reload: they are read again.
            Task { await aiResync() }
            Task {
                await loadUISettings()
                await loadKeymap()
                await loadTheme()
            }
            return
        }
        switch e.method {
        case "FolderChanged":
            if let f = e.params["folder_id"]?.int {
                dirtyFolders.insert(f)
                if e.params["threads_announced"]?.bool != true { unannouncedFolders.insert(f) }
            }
        case "ThreadChanged":
            if let t = e.params["thread_id"]?.int { dirtyThreads.insert(t) }
            for f in e.params["folder_ids"]?.ints ?? [] { dirtyFolders.insert(f) }
        case "ConfigChanged":
            // `config.toml` applied: its `[ui]` may have changed, and
            // `accounts.toml` its accounts.
            accountsRevision += 1
            Task { await loadUISettings() }
            return
        case "KeymapChanged":
            Task { await loadKeymap() }
            return
        case "ThemeChanged":
            Task { await loadTheme() }
            return
        case "ConfigError" where e.params["file"]?.string?.hasPrefix("themes/") == true:
            // The theme stays the one in effect; `theme_get` says whether
            // the refused file is in its chain.
            Task { await loadTheme() }
            return
        case "ConfigError" where e.params["file"]?.string == "keymap.toml":
            // The keys stay those in effect: only the message changes.
            Task { await loadKeymap() }
            return
        case "AccountChanged", "AccountError":
            // An account added, moved, enabled, refused by its server: the
            // Preferences list it again. Its folders come with their own
            // `FolderChanged`.
            accountsRevision += 1
            return
        case "DraftSent", "SendFailed", "SendPartial":
            draftEvent(e.method, e.params)
            return
        case "AiDone", "AiFailed", "AiSuggestions":
            return
        case "ActionReverted":
            // The server refused a change made here; the core has undone it
            // (`modele de donnees.md`, section 6). Said in the status bar,
            // without stopping the user; the lists follow from the
            // `ThreadChanged` and `FolderChanged` that come with it.
            if case .string(let kind)? = e.params["kind"] {
                flash = AppModel.revertedFlash(kind: kind, why: e.params["why"]?.object ?? [:], message: e.params["message"]?.string ?? "")
            }
            return
        case "MessageChanged":
            // Not always followed by `ThreadChanged`: a body that has just
            // been downloaded (`Inserter::complete`, `docs/ingestion.md`)
            // changes only its message. A mail is listed and opened as soon
            // as its headers are in, so without this the reader kept
            // "(body not downloaded)" until the user left the mail and came
            // back to it.
            guard let t = e.params["thread_id"]?.int, let dirty = messageDirties(thread: t) else { return }
            dirtyThreads.insert(t)
            dirtyFolders.formUnion(dirty)
        default:
            return
        }
        scheduleRefresh(full: false)
    }

    /// What a `MessageChanged` of thread `t` invalidates on screen, or `nil`
    /// when it shows nothing of it. The open thread is reloaded (the reader
    /// shows the body); a listed thread reloads its list, whose row shows
    /// the snippet that comes with the body. A thread neither open nor
    /// listed reloads nothing: while the bodies of a whole account come
    /// in (twenty per batch, one event each), only those of the mails on
    /// screen cost a reload, and `scheduleRefresh` groups them.
    private func messageDirties(thread t: Int) -> Set<Int>? {
        let row = threads.first { $0.id == t }
        guard row != nil || openThread?.id == t else { return nil }
        // An empty set still reloads the open thread (`dirtyThreads`).
        return row.map { [$0.folderId] } ?? []
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
    /// refused, and why: its own answer, cited, or the reason's sentence
    /// (`attempts_failed`).
    nonisolated static func revertedFlash(kind: String, why: [String: String], message: String) -> String {
        let what = kind == "move" ? L("The server refused the move") : L("The server refused the change")
        let detail = why["reason"] == "server_refused_change"
            ? why["server_text"] ?? message
            : CoreReason.text(why: why, message: message) ?? message
        return what + " · " + L("undone: %@", detail)
    }

    nonisolated static func order(of e: CoreClient.Event, after last: Position?) -> EventOrder {
        guard let seq = e.seq else { return .transient }
        let p = Position(epoch: e.epoch, seq: seq)
        if let last, last.epoch == e.epoch, seq == last.seq + 1 { return .inOrder(p) }
        return .gap(p)
    }

    /// The shortest time between the starts of two reloads.
    nonisolated static let refreshInterval: Duration = .milliseconds(500)
    /// The wait that groups the events of a burst (a move sends several
    /// within a few milliseconds) into one reload.
    nonisolated static let refreshBurst: Duration = .milliseconds(40)

    /// How long a reload waits once due: the burst, but never less than
    /// what is left of `refreshInterval` since the previous one started. A
    /// pure function, tested alone.
    nonisolated static func refreshDelay(sinceLastStart: Duration?) -> Duration {
        guard let since = sinceLastStart else { return refreshBurst }
        return max(refreshBurst, refreshInterval - since)
    }

    /// Groups the invalidations of the events into reloads, at most one
    /// every `refreshInterval`, never two at once.
    ///
    /// Why not a debounce (each event restarting the delay), as before:
    /// during the import of an account the core sends events all the time.
    /// Events less than 40 ms apart never let the reload come (the lists
    /// stayed stale); further apart, a reload started at each pause, while
    /// the previous one was still running (cancelling a task does not
    /// cancel the calls it already made), and the reloads piled up in the
    /// core in front of what the user was waiting for. Now the first event
    /// makes a reload due; the next ones only add to what it reloads; it
    /// starts after the burst, at least `refreshInterval` after the last
    /// one started, and once the last one ended. A user's move, in a quiet
    /// mailbox, is still shown 40 ms after its events.
    private func scheduleRefresh(full: Bool) {
        if full {
            dirtyFolders.formUnion(listedFolderIds)
            refreshFull = true
        }
        guard !refreshDue else { return }
        refreshDue = true
        let previous = refreshTask
        let wait = AppModel.refreshDelay(sinceLastStart: lastRefreshStart.map { ContinuousClock.now - $0 })
        refreshTask = Task {
            try? await Task.sleep(for: wait)
            await previous?.value
            // From here, events make the next reload due: this one has
            // taken what they invalidated so far.
            refreshDue = false
            lastRefreshStart = ContinuousClock.now
            refreshCount += 1
            let folders = dirtyFolders, threads = dirtyThreads, full = refreshFull
            let unannounced = unannouncedFolders
            dirtyFolders = []; dirtyThreads = []; refreshFull = false; unannouncedFolders = []
            // Counters are cheap to reload: always refresh the sidebar.
            await reloadFolders()
            // The open thread first: the list refresh reads its new state to
            // keep it in place if it no longer matches (`keepingSelected`).
            if let open = openThread?.id, full || threads.contains(open) {
                await loadOpenThread(markSeen: false)
            }
            if !listedFolderIds.isEmpty, full || !folders.isDisjoint(with: listedFolderIds) {
                // Only the conversations that changed when the core named
                // them all (every listed folder announced), else the whole
                // list, which is always right.
                let announced = !full && unannounced.isDisjoint(with: listedFolderIds)
                // Swift note: `&&` takes its right side as a closure, which
                // cannot hold an `await`; hence the two steps.
                var patched = false
                if announced { patched = await patchThreads(threads) }
                if patched {
                    patchCount += 1
                } else {
                    wholeReloadCount += 1
                    await reloadThreads(keepDepth: true)
                }
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
            // No folder at all (no account yet): no list will load, and
            // the launch is over all the same; the list says "No mail".
            if listedFolderIds.isEmpty && listFilter == nil { firstListShown() }
        } else {
            await reloadThreads(keepDepth: true)
        }
    }

    func reloadFolders() async {
        guard let client else { return }
        // The groups with the folders, so that the folder column shows both,
        // and a group made elsewhere (the CLI, the MCP agent) appears at
        // the next change. Not awaited: the first list never waits for them
        // (a few hundred microseconds in the core, but one more round trip).
        Task { await loadGroups() }
        do {
            let list: FolderList = try await client.call("list_folders")
            folders = list.items
        } catch { report(error) }
    }

    /// Reads the contact groups again (`list_groups`). The core gives no
    /// event for them yet (`groupes.md`, section 6): the app reloads them
    /// after its own changes and with the folders.
    func loadGroups() async {
        guard let client else { return }
        do {
            let list: Items<GroupView> = try await client.call("list_groups", [:])
            if list.items != groups { groups = list.items }
        } catch { report(error) }
    }

    /// Reloads the list. With `keepDepth`, as many rows as before are loaded
    /// (in several pages if needed), so the list does not jump back to the
    /// top on every invalidation.
    func reloadThreads(keepDepth: Bool = false) async {
        let folderIds = listedFolderIds
        let listFilter = self.listFilter
        guard let client, !folderIds.isEmpty || listFilter != nil else {
            threads = []
            if client != nil { firstListShown() }
            return
        }
        let sort = self.sort
        let filter = attributes
        // Read once: every page of this reload lists the same thing.
        let scope = Self.listScope(everything: everything, virtual: virtual, folderIds: folderIds, listFilter: listFilter)
        let filterParameters = Self.filterParameters(filter, listFilter: listFilter)
        // How deep to load. With `keepDepth`, the depth is read again after
        // each page, not once at the start: a deep list takes several pages
        // to reload (a dozen at 2 000 rows), and the next page loaded
        // meanwhile (`loadNextPage`, the user going down the list) makes the
        // list deeper. Taken once, the depth of the start would have thrown
        // that page away when this reload replaced the list, and the list
        // stopped growing there.
        // Swift note: a closure that reads `threads` sees its value at the
        // time it is called: the main actor may have changed it during an
        // `await`.
        let wanted = { keepDepth ? max(self.threads.count, Self.pageSize) : Self.pageSize }
        var items: [ThreadSummary] = []
        var cursor: String?
        do {
            // `repeat { } while` runs the body at least once.
            repeat {
                var params = scope
                params["limit"] = min(200, wanted() - items.count)
                params["sort"] = sort
                if let filterParameters { params["filter"] = filterParameters }
                if let cursor { params["cursor"] = cursor }
                let page: Page<ThreadSummary> = try await client.call("list_threads", params)
                items += page.items
                cursor = page.nextCursor
            } while cursor != nil && items.count < wanted()
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
            // The list as the user last saw it: where a selection that left
            // it goes depends on what followed it there.
            let before = threads
            threads = kept.threads
            stickyThreadId = kept.sticky
            nextCursor = cursor
            Self.listLog.info("list reloaded: \(self.threads.count) rows, \(cursor == nil ? "end of the list" : "more to come", privacy: .public)")
            selectFirstIfNeeded(previous: keepDepth ? before : [])
            firstListShown()
            // A page dropped while this reload ran (its cursor was replaced)
            // is asked again if the cursor still needs it.
            prefetchIfNearEnd()
        } catch { report(error) }
    }

    /// `ui.md`: a list always has a selected mail, so the reading pane
    /// shows something. A selection no longer in the list (moved, or left
    /// out by the attributes) goes to the mail that followed it in
    /// `previous`, the list before the reload, and is still listed
    /// (`successor`); with no such mail, to the first one, or to nothing
    /// when the list is empty, so the reading pane never shows a mail the
    /// list does not. The one exception is a search result opened in an old
    /// part of a folder, not in the pages loaded yet (`keepSelection`).
    private func selectFirstIfNeeded(previous: [ThreadSummary] = []) {
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
            selectedThreadId = Self.successor(of: selectedThreadId, previous: previous.map(\.id),
                                              listed: threads.map(\.id))
        }
    }

    /// Where the cursor goes when the mail `selected` is no longer among
    /// `listed`: the first mail after it in `previous` that is still listed,
    /// else the nearest one before it, else the first mail of `listed` (nil
    /// when it is empty). A pure function, tested alone.
    ///
    /// Why not simply "the row now at the same index": the mail the cursor
    /// went to may leave the list in the same reload. Moving a mail to
    /// Banned (or Newsletter, Paperwork) files every mail of its sender
    /// (`dossiers.md`, "Cibles proposées"): the optimistic update put the
    /// cursor on the next row, and when that row is from the same sender,
    /// the core's reload removes it too. The cursor then fell back to the
    /// top of the list; it now skips the mails gone with it, wherever the
    /// loaded rows shifted to.
    nonisolated static func successor(of selected: Int?, previous: [Int], listed: [Int]) -> Int? {
        let ids = Set(listed)
        if let selected, let index = previous.firstIndex(of: selected) {
            if let after = previous[(index + 1)...].first(where: { ids.contains($0) }) { return after }
            if let before = previous[..<index].last(where: { ids.contains($0) }) { return before }
        }
        return listed.first
    }

    /// Thread to keep selected after the next reload, even if it is not in
    /// the loaded pages (set by `openSearchHit`).
    private var keepSelection: Int?
    /// A deep link received before the core answered (the app launched by
    /// the link itself): opened once connected (`openDeepLink`).
    var pendingDeepLink: URL?
    /// Called once a deep link selected its mail, with where the message
    /// is, so that the interface follows: the geek theme closes its panels
    /// and shows that very message of the conversation, unfolded.
    @ObservationIgnored var onDeepLinkOpened: ((MessageLocation) -> Void)?

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

    /// A tag or group renamed (`new`) or deleted (`new == nil`) on its
    /// screen (`ui.md`, "Écrans Tags et Groupes"): what names it follows.
    /// The attributes of the list (a tag by its name, a group by its id),
    /// and the list of its mails (`;T` then Return) shown under the new
    /// name, or left for the selected folder when it is gone. The list is
    /// read again by the caller: its rows name the tags.
    func labelChanged(_ kind: LabelKind, id: Int?, old: String, new: String?) {
        var a = attributes
        switch kind {
        case .tag: a.renameTag(old, to: new)
        case .group: if let id { a.renameGroup(id, to: new) }
        }
        // Swift note: `didSet` of `attributes` reloads the list only when
        // the value changes, so assigning an unchanged copy costs nothing.
        attributes = a
        switch listFilter {
        case .tag(let t) where kind == .tag && t.name.caseInsensitiveCompare(old) == .orderedSame:
            listFilter = new.map { .tag(TagView(id: t.id, accountId: t.accountId, name: $0, color: t.color, messageCount: t.messageCount)) }
        case .group(let g) where kind == .group && g.id == id:
            listFilter = new.map { name in .group(groups.first { $0.id == g.id } ?? GroupView(
                id: g.id, name: name, color: g.color, memberCount: g.memberCount, dateModified: g.dateModified, mailCount: g.mailCount)) }
        default:
            break
        }
    }

    /// The first list is shown: the launch is over. Its timings go to the
    /// system log once (`StartupTimings`).
    private func firstListShown() {
        guard startupPhase != .ready else { return }
        startupPhase = .ready
        // The editor of the replies' text, loaded now, after the first list,
        // so that `r` opens it without waiting for WebKit.
        Task { @MainActor in self.ensureEditor() }
        let now = ContinuousClock.now
        let timings = StartupTimings(socket: (socketOpened ?? now) - created, firstList: now - created,
                                     startedCore: startedCore, rows: threads.count)
        startupTimings = timings
        StartupTimings.log.notice("\(timings.line, privacy: .public)")
    }

    /// Loads the next page when the last row appears on screen (called by
    /// each row's `.task` in `ThreadRows`): the list
    /// scrolled with the mouse or the trackpad.
    func loadMoreIfNeeded(after thread: ThreadSummary) async {
        guard thread.id == threads.last?.id else { return }
        await loadNextPage()
    }

    /// How close to the end of the loaded rows the cursor may come before
    /// the next page is asked for: half a page, so that the rows are there
    /// before the cursor reaches them.
    nonisolated static let prefetchMargin = pageSize / 2

    /// Whether the cursor at `index`, in a list of `count` loaded rows,
    /// needs the next page (`prefetchMargin`). A pure function, tested alone.
    nonisolated static func needsNextPage(selected index: Int?, count: Int) -> Bool {
        guard let index else { return false }
        return index >= count - prefetchMargin
    }

    /// Asks for the next page when the cursor comes near the end of the
    /// loaded rows: after each move of the cursor, after a reload, after a
    /// page.
    ///
    /// Why not only the last row's `.task`: that task runs once, when the
    /// row appears. When the page it asked for was lost (replaced by a
    /// reload of the list that ran at the same time, `reloadThreads`), the
    /// last row was still the same and still on screen: nothing asked again,
    /// and `ctrl-d` / `j` stopped at that row for good, however much mail
    /// followed. The cursor is what the keys move, so it is what asks.
    func prefetchIfNearEnd() {
        let index = threads.firstIndex { $0.id == selectedThreadId }
        guard Self.needsNextPage(selected: index, count: threads.count) else { return }
        // Near the end and nothing asked: said in the log, since that is
        // where a list that stops growing can be understood afterwards.
        guard nextCursor != nil else { return }
        guard !loadingMore else {
            Self.listLog.info("next page not asked: one is loading (\(self.threads.count) rows)")
            return
        }
        Task { await loadNextPage() }
    }

    /// The loading of the list's pages, in the system log (category
    /// `list`, level info: `log show --info`): each page asked, added,
    /// dropped and why, failed. A list that stops growing is explained
    /// there.
    nonisolated static let listLog = Logger(subsystem: "eu.nicemailer.app", category: "list")

    /// Loads the page after the loaded rows, unless one is loading. A page
    /// whose list changed meanwhile is dropped, then asked again with the
    /// new cursor if the cursor still needs it (`prefetchIfNearEnd`).
    private func loadNextPage() async {
        let folderIds = listedFolderIds
        let listFilter = self.listFilter
        guard let client, !folderIds.isEmpty || listFilter != nil, let cursor = nextCursor,
              !loadingMore else { return }
        let sort = self.sort
        let filter = attributes
        loadingMore = true
        let rowsBefore = threads.count
        Self.listLog.info("next page asked after \(rowsBefore) rows")
        let page: Page<ThreadSummary>
        do {
            var params = Self.listScope(everything: everything, virtual: virtual, folderIds: folderIds, listFilter: listFilter)
            params["cursor"] = cursor
            params["limit"] = Self.pageSize
            params["sort"] = sort
            if let p = Self.filterParameters(filter, listFilter: listFilter) { params["filter"] = p }
            page = try await client.call("list_threads", params)
        } catch {
            // No new attempt on its own: a core that fails would be asked
            // again and again. The next move of the cursor asks again.
            loadingMore = false
            Self.listLog.error("next page failed after \(rowsBefore) rows: \(error.localizedDescription, privacy: .public)")
            report(error)
            return
        }
        loadingMore = false
        // Keep the page only if the folder and the list did not change
        // meanwhile; else it is asked again below, with the new cursor.
        if folderIds == listedFolderIds, cursor == nextCursor, sort == self.sort, filter == attributes,
           listFilter == self.listFilter {
            // A thread already listed is not listed twice: its date may
            // have gone back since the earlier page (its latest mail
            // deleted, All Mail), and two rows with one id confuse
            // `ForEach`.
            let listed = Set(threads.map(\.id))
            threads += page.items.filter { !listed.contains($0.id) }
            nextCursor = page.nextCursor
            Self.listLog.info("next page added: \(self.threads.count) rows, \(page.nextCursor == nil ? "end of the list" : "more to come", privacy: .public)")
        } else {
            let why = cursor != nextCursor ? "the list was reloaded meanwhile" : "another list is shown"
            Self.listLog.info("next page dropped after \(rowsBefore) rows: \(why, privacy: .public)")
        }
        // A cursor that did not move would ask for the same page forever.
        if nextCursor != cursor { prefetchIfNearEnd() }
    }

    /// Reloads the conversations `ids` of the list, and puts them at their
    /// place (`ListPatch.patched`), instead of the whole list: the refresh
    /// of a list after a change the core announced conversation by
    /// conversation. Returns false when it cannot (too many conversations,
    /// an error): the caller reloads the whole list then.
    ///
    /// The list is patched as it is when the answer arrives, not as it was
    /// when the request left: a page loaded meanwhile (`loadNextPage`) is
    /// kept.
    func patchThreads(_ ids: Set<Int>) async -> Bool {
        let folderIds = listedFolderIds
        let listFilter = self.listFilter
        guard let client, !ids.isEmpty, ids.count <= ListPatch.maxThreads,
              !folderIds.isEmpty || listFilter != nil else { return false }
        let sort = self.sort
        let filter = attributes
        var params = Self.listScope(everything: everything, virtual: virtual, folderIds: folderIds, listFilter: listFilter)
        var criteria = Self.filterParameters(filter, listFilter: listFilter) ?? [:]
        criteria["thread_ids"] = ids.sorted()
        params["filter"] = criteria
        params["limit"] = ids.count
        params["sort"] = sort
        let page: Page<ThreadSummary>
        do {
            page = try await client.call("list_threads", params)
        } catch {
            return false
        }
        // Another list meanwhile: it has its own load; nothing to patch.
        guard folderIds == listedFolderIds, sort == self.sort, filter == attributes, listFilter == self.listFilter
        else { return true }
        let patched = ListPatch.patched(threads, changed: ids, fresh: page.items, complete: nextCursor == nil, sort: sort)
        // The selected mail that stopped matching the list stays, as with a
        // reload (`keepingSelected`); one kept so before and not changed
        // now stays kept.
        let kept = Self.keepingSelected(patched, previous: threads, selected: selectedThreadId, open: openThread,
                                        listed: listFilter == nil ? Set(folderIds) : Set(folders.map(\.id)))
        let before = threads
        threads = kept.threads
        Self.listLog.info("list patched: \(ids.count) conversations, \(self.threads.count) rows")
        stickyThreadId = kept.sticky ?? stickyThreadId.flatMap { id in
            !ids.contains(id) && kept.threads.contains { $0.id == id } ? id : nil
        }
        selectFirstIfNeeded(previous: before)
        prefetchIfNearEnd()
        return true
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

    /// Whether the list shows, before each sender's name, an icon telling a
    /// person from a robot (`ui.md`, "Quarantine"): while a Quarantine
    /// folder itself is listed (of an account, or of "all"), as for its
    /// direct targets (`InterfaceController.directTargets`). There the user
    /// judges unknown senders, and a person among the robots is the mail
    /// most likely to deserve the Inbox. Elsewhere the attribute selector's
    /// human / robot filter does that job, and the column keeps its room
    /// for the name.
    var listShowsSenderKind: Bool { listFilter == nil && currentRole == "quarantine" }

    /// Whether the listed thread `id` sits in a Quarantine folder (of any
    /// account: the unified view mixes them, and "Everything" lists them
    /// too).
    ///
    /// Viewing a mail of Quarantine never marks it read (`ui.md`,
    /// "Lecture"): the user is judging an unknown sender, not reading its
    /// mail, and the unread dot keeps saying which mails still wait for that
    /// judgement. Only the automatic marking is concerned (the one-second
    /// rule, Return in one column, leaving the mail): the geek theme also
    /// refuses `u` / `U` while Quarantine is listed (`InterfaceController`), but
    /// the core itself accepts `set_thread_seen`, so a client that asks
    /// explicitly (the CLI) still can.
    func isInQuarantine(_ id: Int) -> Bool {
        guard let folderId = threads.first(where: { $0.id == id })?.folderId else { return false }
        return folders.first { $0.id == folderId }?.role == "quarantine"
    }

    /// Marks a thread read (no change to the list here: the core's events
    /// refresh it).
    private func markSeen(_ id: Int) {
        guard let client, !isInQuarantine(id) else { return }
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

    /// Loads the selected thread into the reading pane, with **at most one
    /// `get_thread` in flight**. Each cursor move asks for the mail under
    /// the cursor; holding `j` asked for every mail passed over, and the
    /// requests queued behind the core's few reader connections: the mail
    /// finally under the cursor came seconds later, and the lists behind
    /// it too (`core.log`, "slow request: get_thread took 9630 ms"). Now a
    /// call made while one is loading only records what it wants, and the
    /// running loop loads the selection of the moment once it is done: the
    /// mails passed over in between are never asked for.
    ///
    /// A caller that awaits (the refresh, which reads the open thread
    /// before the list) returns once that loop has finished, so its own
    /// request has been served.
    private func loadOpenThread(markSeen: Bool) async {
        threadWanted = ThreadLoad.merge(threadWanted, ThreadLoad(id: selectedThreadId, markSeen: markSeen))
        if let running = threadLoader {
            await running.value
            return
        }
        let loader = Task { await runThreadLoads() }
        threadLoader = loader
        await loader.value
    }

    /// The loop of `loadOpenThread`: loads what is wanted until nothing is.
    /// On the main actor, nothing runs between the last check and the end
    /// of the loop: a request comes either before (served) or after (a new
    /// loop).
    private func runThreadLoads() async {
        while let wanted = threadWanted {
            threadWanted = nil
            await fetchOpenThread(markSeen: wanted.markSeen)
        }
        threadLoader = nil
    }

    /// One `get_thread` of the selected thread.
    private func fetchOpenThread(markSeen: Bool) async {
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
        guard let client, readerVisible, openThread?.id == id, !isInQuarantine(id),
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

    /// Opens the mail of a deep link `nicemailer://m/<key>` (`DeepLink`,
    /// `docs/deep-links.md`): asks the core where the message is, by its
    /// key (or its id, for an old link), then shows its thread in its
    /// folder. Before the core answered (the link launched the app), the
    /// link waits for the connection.
    func openDeepLink(_ url: URL) {
        guard let target = DeepLink.target(from: url) else {
            lastError = L("Not a link to a NiceMailer mail: %@", url.absoluteString)
            return
        }
        guard case .connected = status, let client else {
            pendingDeepLink = url
            return
        }
        Task {
            if let place = await reveal(target) { onDeepLinkOpened?(place) }
        }
    }

    /// Shows a mail where it is now, in its folder: a deep link (by key or
    /// id), or a line of the History screen. Returns where the message is,
    /// or nil, with the reason in the status bar, when it cannot be shown.
    func reveal(_ target: DeepLink.Target) async -> MessageLocation? {
        guard let core else { return nil }
        do {
            let place: MessageLocation = try await core.call("locate_message", target.params)
            guard let threadId = place.threadId else {
                lastError = L("This mail is not in a conversation yet")
                return nil
            }
            lastError = nil
            show(threadId: threadId, folderId: place.folderId)
            return place
        } catch let e as CoreClient.RPCError where e.code == CoreClient.RPCError.notFound {
            // An old link by id: deleted since, or the database was rebuilt
            // (ids changed). A key: deleted, or this Mac never had the mail
            // (another account, not synchronized yet), which "no longer"
            // would misstate.
            switch target {
            case .id: lastError = L("This mail is no longer in NiceMailer")
            case .key: lastError = L("This mail is not in NiceMailer on this Mac")
            }
        } catch { report(error) }
        return nil
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
        await startSession()
    }

    /// What follows the connection (`connect`, and `attach` in the tests):
    /// the settings, the folders and the first list, then a deep link
    /// waiting for them.
    ///
    /// The settings of `[ui]` and the state of the AI do not decide what
    /// the first list shows: they are asked at the same time as it, not
    /// before it, which cost the launch two round trips to a core that has
    /// just started.
    private func startSession() async {
        watchAddressBook()
        // Swift note: `async let` starts the call at once, in a child task;
        // `await settings` waits for it where its result is needed (here,
        // only to finish before this function returns).
        async let settings: Void = loadUISettings()
        async let keys: Void = loadKeymap()
        async let themes: Void = loadTheme()
        await reloadAll()
        await settings
        await keys
        await themes
        openPendingDeepLink()
    }

    /// Reloads the open thread (after a change of display preference).
    func reloadOpenThread() async { await loadOpenThread(markSeen: false) }
}

/// A load of the open thread asked for (`AppModel.loadOpenThread`): the
/// selection when it was asked, and whether the mail is then to be marked
/// read after the delay (a mail the user chose, not one the app selected).
struct ThreadLoad: Equatable {
    let id: Int?
    let markSeen: Bool

    /// What to load next when `new` is asked while `pending` waits: always
    /// the latest selection. For the same mail, it is marked read if either
    /// asked for it: a refresh (`markSeen: false`) arriving after the user
    /// chose the mail must not cancel its marking. For another mail, only
    /// the new request counts. A pure function, tested alone.
    static func merge(_ pending: ThreadLoad?, _ new: ThreadLoad) -> ThreadLoad {
        guard let pending, pending.id == new.id else { return new }
        return ThreadLoad(id: new.id, markSeen: pending.markSeen || new.markSeen)
    }
}
