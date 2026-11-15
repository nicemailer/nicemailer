import Foundation
import Testing
@testable import NiceMailer

/// The reload of the conversations that changed instead of the whole list
/// (`ListPatch`, `AppModel.patchThreads`, `evenements.md`, "Rechargement
/// par conversation").
@MainActor
struct ListPatchTests {
    // MARK: - The pure function

    static func row(_ id: Int, date: Int, modified: Int = 0, unread: Int = 0) -> ThreadSummary {
        var t = Fixture.thread(id)
        t["date_last_received"] = date
        t["date_last_modified"] = modified
        t["unread_count"] = unread
        let data = try! JSONSerialization.data(withJSONObject: t)
        return try! CoreClient.decoder.decode(ThreadSummary.self, from: data)
    }

    /// Rows 1 to 5, dates 500 down to 100.
    let rows = (1...5).map { row($0, date: 600 - $0 * 100) }

    /// A row that changed in place (read) keeps its place, with its new
    /// state.
    @Test func aChangedRowKeepsItsPlace() {
        let out = ListPatch.patched(rows, changed: [3], fresh: [Self.row(3, date: 300, unread: 1)], complete: true, sort: "received")
        #expect(out.map(\.id) == [1, 2, 3, 4, 5])
        #expect(out[2].unreadCount == 1)
    }

    /// A row the core no longer gives has left the list.
    @Test func aRowNotGivenBackLeaves() {
        let out = ListPatch.patched(rows, changed: [2, 4], fresh: [Self.row(4, date: 200)], complete: true, sort: "received")
        #expect(out.map(\.id) == [1, 3, 4, 5])
    }

    /// A row whose date changed moves to its new place; a new one comes in
    /// at its place; ties on the date go by id, descending, as in the core.
    @Test func rowsGoToTheirPlaceInTheOrderOfTheList() {
        let fresh = [Self.row(5, date: 450), Self.row(9, date: 300), Self.row(7, date: 50)]
        let out = ListPatch.patched(rows, changed: [5, 9, 7], fresh: fresh, complete: true, sort: "received")
        // 9 and 3 share the date 300: 9 first.
        #expect(out.map(\.id) == [1, 5, 2, 9, 3, 4, 7])
    }

    /// A list not loaded to its end takes no row whose place is after its
    /// last row: that row comes with the next page.
    @Test func aRowBeyondTheLoadedRowsWaitsForItsPage() {
        let fresh = [Self.row(7, date: 50), Self.row(8, date: 150)]
        let out = ListPatch.patched(rows, changed: [7, 8], fresh: fresh, complete: false, sort: "received")
        #expect(out.map(\.id) == [1, 2, 3, 4, 8, 5])
        let all = ListPatch.patched(rows, changed: [7, 8], fresh: fresh, complete: true, sort: "received")
        #expect(all.map(\.id) == [1, 2, 3, 4, 8, 5, 7])
    }

    /// Sorted by modification, the key is that date.
    @Test func theKeyFollowsTheSort() {
        let byModified = (1...3).map { Self.row($0, date: 0, modified: 400 - $0 * 100) }
        let out = ListPatch.patched(byModified, changed: [3], fresh: [Self.row(3, date: 0, modified: 999)], complete: true, sort: "modified")
        #expect(out.map(\.id) == [3, 1, 2])
    }

    /// A JSON boolean of an event is a boolean, not the integer 1.
    @Test func aBooleanOfAnEventStaysABoolean() {
        #expect(JSONValue(true as Any) == .bool(true))
        #expect(JSONValue(NSNumber(value: false)) == .bool(false))
        #expect(JSONValue(1 as Any) == .int(1))
        #expect(JSONValue(true as Any)?.int == nil)
    }

    // MARK: - Through the model

    /// 150 threads in the Inbox, by pages of 60; `unread` and `gone` say
    /// what the core answers now. Thread-safe: the model's requests run
    /// in parallel.
    final class Mailbox: @unchecked Sendable {
        private let lock = NSLock()
        private var unread: Set<Int> = []
        private var gone: Set<Int> = []
        let total = 150

        func set(unread: Set<Int> = [], gone: Set<Int> = []) {
            lock.lock(); defer { lock.unlock() }
            self.unread = unread
            self.gone = gone
        }

        func answer(_ method: String, _ params: [String: Any]) -> Any {
            guard method == "list_threads" else { return Fixture.answer(method, params) }
            lock.lock(); defer { lock.unlock() }
            func thread(_ id: Int) -> [String: Any] {
                var t = Fixture.thread(id)
                t["unread_count"] = unread.contains(id) ? 1 : 0
                return t
            }
            let filter = params["filter"] as? [String: Any] ?? [:]
            if let ids = filter["thread_ids"] as? [Int] {
                // Newest first: the fixture's date goes down with the id.
                let items = ids.sorted().filter { $0 <= total && !gone.contains($0) }.map(thread)
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            }
            let start = (params["cursor"] as? String).flatMap(Int.init) ?? 0
            let limit = params["limit"] as? Int ?? 60
            let all = (1...total).filter { !gone.contains($0) }
            let page = Array(all.dropFirst(start).prefix(limit))
            let end = start + page.count
            return ["items": page.map(thread), "next_cursor": end < all.count ? "\(end)" as Any : NSNull(),
                    "as_of": Fixture.position]
        }
    }

    let mailbox = Mailbox()
    let core: FakeCore
    let model = AppModel()

    init() async {
        let mailbox = self.mailbox
        core = FakeCore { mailbox.answer($0, $1) }
        await model.attach(core)
        _ = await eventually { model.threads.count == 60 }
        // Two pages deep, as a user going down the list.
        model.selectNext(45)
        _ = await eventually { model.threads.count == 120 }
        // A first event sets the position of the events: without one, any
        // event is a gap, and a gap reloads everything. Its own reload
        // ends before the tests start.
        let refreshes = model.refreshCount
        model.handle(CoreClient.Event(method: "AccountChanged", epoch: "e_test", seq: 1, params: [:]))
        _ = await eventually { model.refreshCount > refreshes }
        try? await Task.sleep(for: .milliseconds(100))
    }

    var seq = 1
    mutating func send(_ method: String, _ params: [String: JSONValue]) {
        seq += 1
        model.handle(CoreClient.Event(method: method, epoch: "e_test", seq: seq, params: params))
    }

    /// The events of a thread marked read: its `ThreadChanged`, then the
    /// `FolderChanged` of the Inbox, announced or not.
    mutating func markRead(_ id: Int, announced: Bool) {
        send("ThreadChanged", ["thread_id": .int(id), "folder_ids": .ints([1])])
        var folder: [String: JSONValue] = ["folder_id": .int(1)]
        if announced { folder["threads_announced"] = .bool(true) }
        send("FolderChanged", folder)
    }

    /// The list requests made since `from`: the whole list (no
    /// `thread_ids`) and the patches.
    func listCalls(since from: Int) -> (whole: Int, patches: [[Int]]) {
        let calls = core.calls("list_threads").dropFirst(from)
        let patches = calls.compactMap { ($0["filter"] as? [String: Any])?["thread_ids"] as? [Int] }
        return (calls.count - patches.count, patches)
    }

    /// Announced: the conversation alone is asked for, and takes its new
    /// state in place; the list keeps its depth.
    @Test mutating func anAnnouncedChangeReloadsTheConversationAlone() async {
        let before = core.calls("list_threads").count
        mailbox.set(unread: [10])
        markRead(10, announced: true)
        #expect(await eventually { model.threads.first { $0.id == 10 }?.unreadCount == 1 })
        let calls = listCalls(since: before)
        #expect(calls.whole == 0)
        #expect(calls.patches == [[10]])
        #expect(model.threads.count == 120)
        #expect(model.threads.map(\.id) == Array(1...120))
    }

    /// Not announced (an older core, an import): the whole list, as before.
    @Test mutating func anUnannouncedChangeReloadsTheList() async {
        let before = core.calls("list_threads").count
        mailbox.set(unread: [10])
        markRead(10, announced: false)
        #expect(await eventually { model.threads.first { $0.id == 10 }?.unreadCount == 1 })
        let calls = listCalls(since: before)
        #expect(calls.whole >= 1)
        #expect(calls.patches.isEmpty)
    }

    /// One folder of the batch not announced is enough for the whole list.
    @Test mutating func aBatchWithOneUnannouncedFolderReloadsTheList() async {
        let before = core.calls("list_threads").count
        markRead(10, announced: true)
        send("FolderChanged", ["folder_id": .int(1)])
        _ = await eventually { core.calls("list_threads").count > before }
        try? await Task.sleep(for: .milliseconds(100))
        let calls = listCalls(since: before)
        #expect(calls.whole >= 1)
        #expect(calls.patches.isEmpty)
    }

    /// A conversation that left the list (not given back) leaves it here;
    /// the cursor, elsewhere, does not move.
    @Test mutating func aConversationThatLeftIsRemoved() async {
        let selected = model.selectedThreadId
        mailbox.set(gone: [20])
        markRead(20, announced: true)
        #expect(await eventually { !model.threads.contains { $0.id == 20 } })
        #expect(model.threads.count == 119)
        #expect(model.selectedThreadId == selected)
    }

    /// The cursor's conversation that stopped matching the list while it
    /// still has mails in the listed folder (here: a filter it no longer
    /// meets) stays under the cursor, as with a reload of the whole list
    /// (`keepingSelected`); it leaves once the cursor does.
    @Test mutating func theCursorsConversationStaysUntilLeft() async {
        let selected = model.selectedThreadId!
        _ = await eventually { model.openThread?.id == selected }
        mailbox.set(gone: [selected])
        markRead(selected, announced: true)
        #expect(await eventually { model.stickyThreadId == selected })
        #expect(model.selectedThreadId == selected)
        #expect(model.threads.contains { $0.id == selected })
        model.selectNext(1)
        #expect(!model.threads.contains { $0.id == selected })
    }

    /// Too many conversations at once: the whole list instead.
    @Test mutating func tooManyConversationsReloadTheList() async {
        let before = core.calls("list_threads").count
        for id in 1...(ListPatch.maxThreads + 1) {
            send("ThreadChanged", ["thread_id": .int(id), "folder_ids": .ints([1])])
        }
        send("FolderChanged", ["folder_id": .int(1), "threads_announced": .bool(true)])
        _ = await eventually { core.calls("list_threads").count > before }
        try? await Task.sleep(for: .milliseconds(100))
        let calls = listCalls(since: before)
        #expect(calls.whole >= 1)
        #expect(calls.patches.isEmpty)
    }

    /// A page loaded while the patch was on its way stays.
    @Test mutating func aPageLoadedMeanwhileStays() async {
        core.delays["list_threads"] = .milliseconds(150)
        markRead(10, announced: true)
        // The cursor near the end asks for the third page meanwhile.
        model.selectNext(60)
        #expect(await eventually { model.threads.count == 150 })
        try? await Task.sleep(for: .milliseconds(400))
        #expect(model.threads.count == 150)
        #expect(model.threads.map(\.id) == Array(1...150))
    }
}
