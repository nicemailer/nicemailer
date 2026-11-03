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
    /// Short message shown in the geek status bar (result of an action,
    /// "not available yet"…); cleared by the next key.
    var flash: String?

    /// Selected folder. `didSet` is a property observer: it runs after each
    /// assignment, with the previous value in `oldValue`. Changing folder
    /// clears the thread selection and loads the new list.
    var selectedFolderId: Int? {
        didSet {
            guard selectedFolderId != oldValue else { return }
            selectedThreadId = nil
            marked = []
            // Choosing a folder leaves a tag, group or contact list.
            listFilter = nil
            // `Task { ... }` starts asynchronous work from synchronous code.
            Task { await reloadThreads() }
        }
    }

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

    /// Threads marked for a grouped action (J/K in the geek theme). The
    /// thread under the cursor is `selectedThreadId`.
    var marked = Set<Int>()

    /// Selected thread: selecting one opens it in the reading pane.
    var selectedThreadId: Int? {
        didSet {
            guard selectedThreadId != oldValue else { return }
            Task { await loadOpenThread(markSeen: true) }
        }
    }

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
        do {
            let c = try CoreClient()
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
            await reloadAll()
        } catch {
            // Inside `catch`, the error is implicitly named `error`.
            status = .failed(error.localizedDescription)
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
        if connection === c { status = .failed("Connexion au noyau perdue.") }
    }

    /// Records what an event invalidates and schedules a refresh. If an
    /// event is missing (seq not consecutive, or another epoch), the model
    /// can no longer know what changed, so it reloads everything it shows.
    private func handle(_ e: CoreClient.Event) {
        let consecutive = lastEvent.map { $0.epoch == e.epoch && e.seq == $0.seq + 1 } ?? false
        lastEvent = Position(epoch: e.epoch, seq: e.seq)
        if e.method == "ResyncRequired" || !consecutive {
            scheduleRefresh(full: true)
            return
        }
        switch e.method {
        case "FolderChanged":
            if let f = e.params["folder_id"]?.int { dirtyFolders.insert(f) }
        case "ThreadChanged":
            if let t = e.params["thread_id"]?.int { dirtyThreads.insert(t) }
            for f in e.params["folder_ids"]?.ints ?? [] { dirtyFolders.insert(f) }
        default:
            // MessageChanged is always followed by ThreadChanged, which is
            // enough for what the app displays.
            return
        }
        scheduleRefresh(full: false)
    }

    /// Groups invalidations received in a burst into a single reload: a move
    /// produces several events within a few milliseconds, and reloading
    /// after each one would be wasteful. Each new event cancels the pending
    /// task and restarts the 40 ms delay ("debounce").
    private func scheduleRefresh(full: Bool) {
        if full, let id = selectedFolderId { dirtyFolders.insert(id) }
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
            if let id = selectedFolderId, full || folders.contains(id) {
                await reloadThreads(keepDepth: true)
            }
            if let open = openThread?.id, full || threads.contains(open) {
                await loadOpenThread(markSeen: false)
            }
        }
    }

    // MARK: - Reads

    /// Reloads the folders, then the thread list (selecting the Inbox the
    /// first time).
    func reloadAll() async {
        await reloadFolders()
        if selectedFolderId == nil {
            // Setting the selection triggers `didSet`, which loads the list.
            selectedFolderId = folders.first { $0.role == "inbox" }?.id ?? folders.first?.id
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
        if let filter = listFilter {
            await reloadFiltered(filter)
            return
        }
        guard let client, let folderId = selectedFolderId else { threads = []; return }
        let sort = self.sort
        let wanted = keepDepth ? max(threads.count, Self.pageSize) : Self.pageSize
        var items: [ThreadSummary] = []
        var cursor: String?
        do {
            // `repeat { } while` runs the body at least once.
            repeat {
                var params: [String: Any] = ["folder_id": folderId, "limit": min(200, wanted - items.count), "sort": sort]
                if let cursor { params["cursor"] = cursor }
                let page: Page<ThreadSummary> = try await client.call("list_threads", params)
                items += page.items
                cursor = page.nextCursor
            } while cursor != nil && items.count < wanted
            // The user may have changed folder while we were loading: then
            // this result is stale and must not replace the new list.
            guard folderId == selectedFolderId, sort == self.sort, listFilter == nil else { return }
            threads = items
            nextCursor = cursor
            selectFirstIfNeeded()
        } catch { report(error) }
    }

    /// `ui.md`: a list always has a selected mail, so the reading pane
    /// shows something. A selection is kept even when the thread is not in
    /// the pages loaded so far (a search result opened in an old part of
    /// the folder): only an empty selection is replaced.
    private func selectFirstIfNeeded() {
        marked = marked.filter { id in threads.contains { $0.id == id } }
        if selectedThreadId == nil {
            selectedThreadId = threads.first?.id
        }
    }

    /// Shows the threads of a tag, a group or a contact (across folders).
    func showThreads(of filter: ListFilter) {
        listFilter = filter
        selectedThreadId = nil
        marked = []
        Task { await reloadThreads() }
    }

    private func reloadFiltered(_ filter: ListFilter) async {
        guard let client else { return }
        var params: [String: Any] = ["limit": 200]
        switch filter {
        case .tag(let t): params["tag_id"] = t.id
        case .group(let g): params["group_id"] = g.id
        case .contact(let c): params["address_id"] = c.id
        }
        do {
            let list: Items<ThreadSummary> = try await client.call("list_threads_by", params)
            guard filter == listFilter else { return }
            threads = list.items
            nextCursor = nil
            selectFirstIfNeeded()
        } catch { report(error) }
    }

    /// Loads the next page when the last row appears on screen (called by
    /// each row's `.task` in `ThreadList`).
    func loadMoreIfNeeded(after thread: ThreadSummary) async {
        guard let client, let folderId = selectedFolderId, let cursor = nextCursor, listFilter == nil,
              !loadingMore, thread.id == threads.last?.id else { return }
        let sort = self.sort
        loadingMore = true
        // `defer` runs when the function exits, whatever the path.
        defer { loadingMore = false }
        do {
            let page: Page<ThreadSummary> = try await client.call(
                "list_threads", ["folder_id": folderId, "cursor": cursor, "limit": Self.pageSize, "sort": sort])
            // Drop the page if the folder or the list changed meanwhile.
            guard folderId == selectedFolderId, cursor == nextCursor, sort == self.sort else { return }
            threads += page.items
            nextCursor = page.nextCursor
        } catch { report(error) }
    }

    /// Loads the selected thread into the reading pane.
    private func loadOpenThread(markSeen: Bool) async {
        guard let client, let id = selectedThreadId else { openThread = nil; return }
        do {
            let view: ThreadView = try await client.call("get_thread", ["thread_id": id])
            guard id == selectedThreadId else { return }
            openThread = view
            // UX hypothesis to validate: a thread displayed for 1 s is marked
            // as read. The delay avoids marking everything while browsing the
            // list with j/k.
            if markSeen, view.messages.contains(where: { !$0.isSeen }) {
                try await Task.sleep(for: .seconds(1))
                guard id == selectedThreadId else { return }
                // `let _: CommandResult` decodes the result only to check it;
                // the value itself is not needed.
                let _: CommandResult = try await client.call("set_thread_seen", ["thread_id": id, "seen": true])
            }
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
        if selectedFolderId != hit.folderId {
            selectedFolderId = hit.folderId
        }
        // Set after the folder, because changing folder clears the selection.
        selectedThreadId = threadId
        searchText = ""
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
    }

    /// Reloads the open thread (after a change of display preference).
    func reloadOpenThread() async { await loadOpenThread(markSeen: false) }
}
