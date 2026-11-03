import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Reading rules (`ui.md`, "Lecture"): which mails are marked read, and the
/// selected mail kept in a filtered list after it stops matching.
///
/// The fake core here keeps the read state of six threads (all unread at
/// first, one message each in the Inbox) and honours the "unread" filter,
/// like the real core. Tests have no event stream: `refresh()` plays what
/// the app does when the core reports a change (open thread, then list).
@MainActor
struct ReadingTests {
    /// Read state of the fake mailbox, shared with the fake core's handler.
    /// Swift note: a `final class` is a reference type, so the closure and
    /// the test see the same instance (a struct would be copied).
    final class Mailbox: @unchecked Sendable {
        let lock = NSLock()
        var seen = Set<Int>()
        func isSeen(_ id: Int) -> Bool { lock.withLock { seen.contains(id) } }
        func setSeen(_ id: Int, _ value: Bool) {
            lock.withLock { if value { seen.insert(id) } else { seen.remove(id) } }
        }
    }

    let box = Mailbox()
    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async {
        let box = self.box
        core = FakeCore { method, params in
            switch method {
            case "list_threads":
                let filter = params["filter"] as? [String: Any]
                let unreadOnly = filter?["seen"] as? Bool == false
                let items = (1...6).filter { !unreadOnly || !box.isSeen($0) }.map { id -> [String: Any] in
                    var t = Fixture.thread(id)
                    t["unread_count"] = box.isSeen(id) ? 0 : 1
                    return t
                }
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            case "get_thread":
                let id = params["thread_id"] as? Int ?? 1
                var view = Fixture.threadView(id)
                var messages = view["messages"] as! [[String: Any]]
                messages[0]["is_seen"] = box.isSeen(id)
                view["messages"] = messages
                return view
            case "set_thread_seen":
                box.setSeen(params["thread_id"] as! Int, params["seen"] as! Bool)
                return ["changed": 1]
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        model.seenDelay = 0.05
        await model.attach(core)
        _ = await eventually { model.threads.count == 6 && model.openThread != nil }
    }

    func press(_ chars: String) {
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: 0)!
        _ = controller.handle(e)
    }

    /// What the app does on a change event: the open thread, then the list.
    func refresh() async {
        await model.reloadOpenThread()
        await model.reloadThreads(keepDepth: true)
    }

    /// Waits longer than `seenDelay`, for the negative checks.
    func pause() async { try? await Task.sleep(for: .milliseconds(250)) }

    var seenCalls: [Int] { core.calls("set_thread_seen").compactMap { $0["thread_id"] as? Int } }

    /// Shows the unread mails only (attribute selector).
    func filterUnread() async {
        model.attributes.seen = false
        _ = await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["seen"] as? Bool == false }
        _ = await eventually { model.threads.count == 6 }
    }

    // MARK: Automatic selection (proposal 3)

    @Test func theMailSelectedWhenTheListOpensIsNotMarkedRead() async {
        #expect(model.selectedThreadId == 1)
        await pause()
        #expect(seenCalls.isEmpty)
    }

    @Test func aListOfUnreadMailsNoLongerEmptiesItself() async {
        await filterUnread()
        // Before the fix: 1 marked read, gone, 2 selected and marked… Now
        // nothing is marked without the user.
        await pause()
        await refresh()
        await pause()
        await refresh()
        #expect(seenCalls.isEmpty)
        #expect(model.threads.count == 6 && model.selectedThreadId == 1)
    }

    @Test func leavingTheAutomaticMailByHandAfterTheDelayMarksItRead() async {
        await pause()
        press("j")
        // 1 (shown long enough, left by hand), then 2 (selected by hand).
        #expect(await eventually { seenCalls.contains(1) })
        #expect(await eventually { seenCalls.contains(2) })
    }

    @Test func leavingTheAutomaticMailAtOnceDoesNotMarkIt() async {
        press("j")
        #expect(await eventually { seenCalls.contains(2) })
        #expect(!seenCalls.contains(1))
    }

    @Test func movingTheAutomaticMailDoesNotMarkIt() async {
        await pause()
        press("d")
        // The next mail is shown because of the user's action: marked.
        #expect(await eventually { seenCalls.contains(2) })
        #expect(!seenCalls.contains(1))
    }

    // MARK: The selected mail stays in the list (proposal 1)

    /// Ids listed, without the first mail: it was selected automatically,
    /// then left by hand after the delay, so it is read too and may have
    /// left the unread list (rule above).
    var listedButFirst: [Int] { model.threads.map(\.id).filter { $0 != 1 } }

    @Test func aMailReadInAListOfUnreadMailsStaysUntilTheUserLeavesIt() async {
        await filterUnread()
        press("j")
        #expect(await eventually { seenCalls.contains(2) })
        await refresh()
        // Still listed, under the cursor, shown as read.
        #expect(listedButFirst == [2, 3, 4, 5, 6])
        #expect(model.stickyThreadId == 2 && model.selectedThreadId == 2)
        #expect(model.threads.first { $0.id == 2 }?.unreadCount == 0)
        // Leaving it removes it; the cursor is on the next mail.
        press("j")
        #expect(listedButFirst == [3, 4, 5, 6])
        #expect(model.selectedThreadId == 3 && model.stickyThreadId == nil)
    }

    @Test func clickingAnotherMailRemovesTheKeptOne() async {
        await filterUnread()
        press("j")
        #expect(await eventually { seenCalls.contains(2) })
        await refresh()
        controller.clickThread(5, .none)
        #expect(listedButFirst == [3, 4, 5, 6] && model.selectedThreadId == 5)
    }

    @Test func aNewListDoesNotKeepTheMail() async {
        await filterUnread()
        press("j")
        #expect(await eventually { seenCalls.contains(2) })
        // Another list (attributes changed): no reason to keep it.
        model.attributes.flagged = false
        #expect(await eventually { listedButFirst == [3, 4, 5, 6] })
        #expect(model.stickyThreadId == nil)
    }

    // MARK: keepingSelected

    func row(_ id: Int) -> ThreadSummary {
        let data = try! JSONSerialization.data(withJSONObject: Fixture.thread(id))
        let d = JSONDecoder(); d.keyDecodingStrategy = .convertFromSnakeCase
        return try! d.decode(ThreadSummary.self, from: data)
    }

    func view(_ id: Int, folder: Int = 1, seen: Bool = true, flagged: Bool = false) -> ThreadView {
        var v = Fixture.threadView(id)
        var m = v["messages"] as! [[String: Any]]
        m[0]["folder_id"] = folder; m[0]["is_seen"] = seen; m[0]["is_flagged"] = flagged
        v["messages"] = m
        let d = JSONDecoder(); d.keyDecodingStrategy = .convertFromSnakeCase
        return try! d.decode(ThreadView.self, from: JSONSerialization.data(withJSONObject: v))
    }

    @Test func theKeptRowGoesBackToItsPlaceWithItsNewState() {
        let previous = [1, 2, 3, 4].map(row)
        let kept = AppModel.keepingSelected(
            [1, 3, 4].map(row), previous: previous, selected: 2, open: view(2, flagged: true), listed: [1])
        #expect(kept.threads.map(\.id) == [1, 2, 3, 4] && kept.sticky == 2)
        #expect(kept.threads[1].unreadCount == 0 && kept.threads[1].isFlagged)
        // First row, or after rows that left too: at the top / after the
        // nearest row still listed.
        #expect(AppModel.keepingSelected([2, 3].map(row), previous: previous, selected: 1, open: view(1), listed: [1])
            .threads.map(\.id) == [1, 2, 3])
        #expect(AppModel.keepingSelected([1, 4].map(row), previous: previous, selected: 3, open: view(3), listed: [1])
            .threads.map(\.id) == [1, 3, 4])
    }

    @Test func aRowIsNotKeptWhenItMovedOrIsStillListed() {
        let previous = [1, 2, 3].map(row)
        // Moved to another folder: no message left in the listed ones.
        let moved = AppModel.keepingSelected([1, 3].map(row), previous: previous, selected: 2, open: view(2, folder: 3), listed: [1])
        #expect(moved.threads.map(\.id) == [1, 3] && moved.sticky == nil)
        // Still in the list: nothing to keep.
        #expect(AppModel.keepingSelected(previous, previous: previous, selected: 2, open: view(2), listed: [1]).sticky == nil)
        // The open thread is another one (not reloaded yet): not kept.
        #expect(AppModel.keepingSelected([1, 3].map(row), previous: previous, selected: 2, open: view(1), listed: [1]).sticky == nil)
        // Nothing selected.
        #expect(AppModel.keepingSelected([1, 3].map(row), previous: previous, selected: nil, open: nil, listed: [1]).sticky == nil)
    }
}
