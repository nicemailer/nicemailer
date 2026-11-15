import Foundation
import Testing
@testable import NiceMailer

/// A long list loaded page by page (`AppModel.loadNextPage`,
/// `prefetchIfNearEnd`, `reloadThreads(keepDepth:)`).
///
/// The bug these tests keep away: in a deep list (All Mail of every
/// account, a few thousand rows), `ctrl-d` stopped at some row for good. A
/// page asked for by the last row was lost when a reload of the list ran
/// at the same time (the core's events: a mail marked read, a sync), and
/// nothing asked again.
@MainActor
struct PagingTests {
    /// 300 threads in the Inbox, served by pages, the cursor being the
    /// offset after a generation number that changes with each list read
    /// from the top (a reload): a page asked with the cursor of an older
    /// generation is stale for the model, as with a list that changed.
    final class Paged: @unchecked Sendable {
        private let lock = NSLock()
        private var generation = 0
        let total = 300

        func answer(_ method: String, _ params: [String: Any]) -> Any {
            guard method == "list_threads" else { return Fixture.answer(method, params) }
            lock.lock()
            let start: Int
            if let cursor = params["cursor"] as? String, let offset = Int(cursor.split(separator: ":")[1]) {
                start = offset
            } else {
                generation += 1
                start = 0
            }
            let gen = generation
            lock.unlock()
            let limit = params["limit"] as? Int ?? 60
            let end = min(total, start + limit)
            let items = (start..<end).map { Fixture.thread($0 + 1) }
            let next: Any = end < total ? "\(gen):\(end)" : NSNull()
            return ["items": items, "next_cursor": next, "as_of": Fixture.position]
        }
    }

    let paged = Paged()
    let core: FakeCore
    let model = AppModel()

    init() async {
        let paged = self.paged
        core = FakeCore { paged.answer($0, $1) }
        await model.attach(core)
        _ = await eventually { model.threads.count == 60 }
    }

    /// A pure function: the next page is needed from half a page before
    /// the end of the loaded rows.
    @Test func theNextPageIsNeededNearTheEnd() {
        #expect(!AppModel.needsNextPage(selected: nil, count: 60))
        #expect(!AppModel.needsNextPage(selected: 0, count: 60))
        #expect(!AppModel.needsNextPage(selected: 60 - AppModel.prefetchMargin - 1, count: 60))
        #expect(AppModel.needsNextPage(selected: 60 - AppModel.prefetchMargin, count: 60))
        #expect(AppModel.needsNextPage(selected: 59, count: 60))
        // An empty list has nothing to select, but asks.
        #expect(AppModel.needsNextPage(selected: 0, count: 0))
    }

    /// Moving the cursor down the list, as `j` or `ctrl-d` do, loads the
    /// rows that follow without any row appearing on screen, up to the end
    /// of the list, each thread once.
    @Test func theCursorLoadsTheNextPages() async {
        #expect(model.threads.count == 60)
        model.selectNext(10)
        try? await Task.sleep(for: .milliseconds(50))
        #expect(model.threads.count == 60, "far from the end: nothing more")
        model.selectNext(25)
        #expect(await eventually { model.threads.count == 120 })
        for _ in 0..<30 {
            model.selectNext(20)
            _ = await eventually { !AppModel.needsNextPage(selected: model.threads.firstIndex { $0.id == model.selectedThreadId }, count: model.threads.count) || model.nextCursor == nil }
        }
        #expect(model.threads.count == paged.total)
        #expect(Set(model.threads.map(\.id)).count == paged.total)
        #expect(model.nextCursor == nil)
    }

    /// A page that arrives while a reload of the list runs: the reload
    /// loads it too instead of putting the list back to its depth of the
    /// start. Before, the list shrank back and the cursor stayed stuck.
    @Test func aReloadKeepsThePageLoadedMeanwhile() async {
        core.delays["list_threads"] = .milliseconds(100)
        // The cursor near the end asks for the next page…
        model.selectNext(50)
        try? await Task.sleep(for: .milliseconds(30))
        // …and a reload starts while it loads, and ends after it.
        await model.reloadThreads(keepDepth: true)
        #expect(model.threads.count == 120)
        try? await Task.sleep(for: .milliseconds(250))
        #expect(model.threads.count >= 120)
    }

    /// A page dropped because a reload replaced the cursor meanwhile is
    /// asked again with the new cursor.
    @Test func aDroppedPageIsAskedAgain() async {
        core.delays["list_threads"] = .milliseconds(100)
        // A reload starts first…
        let reload = Task { await model.reloadThreads(keepDepth: true) }
        try? await Task.sleep(for: .milliseconds(30))
        // …the page asked meanwhile comes back with a stale cursor.
        model.selectNext(50)
        await reload.value
        #expect(await eventually { model.threads.count == 120 })
        #expect(Set(model.threads.map(\.id)).count == 120)
    }
}
