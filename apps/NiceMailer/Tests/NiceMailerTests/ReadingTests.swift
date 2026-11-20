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
        /// `get_thread` of `id` waits until `release()`. The fake core
        /// answers off the main actor, so waiting there blocks only that
        /// request.
        private var held: (id: Int, gate: DispatchSemaphore)?
        func hold(_ id: Int) { lock.withLock { held = (id, DispatchSemaphore(value: 0)) } }
        func release() { lock.withLock { held?.gate.signal() } }
        func waitIfHeld(_ id: Int) {
            let gate = lock.withLock { held?.id == id ? held?.gate : nil }
            gate?.wait()
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
                box.waitIfHeld(id)
                var view = Fixture.threadView(id)
                var messages = view["messages"] as! [[String: Any]]
                messages[0]["is_seen"] = box.isSeen(id)
                view["messages"] = messages
                return view
            case "set_thread_seen":
                // One thread (`thread_id`, the reader) or several
                // (`thread_ids`, a grouped `u` / `U`).
                let ids = (params["thread_ids"] as? [Int]) ?? [params["thread_id"] as! Int]
                for id in ids { box.setSeen(id, params["seen"] as! Bool) }
                return ["changed": ids.count]
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = TestDefaults.make()
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize())
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

    /// The threads of every `set_thread_seen` sent, in order, whether by
    /// `thread_id` or in a batch (`thread_ids`).
    var seenCalls: [Int] { core.calls("set_thread_seen").flatMap { ($0["thread_ids"] as? [Int]) ?? [$0["thread_id"] as? Int].compactMap { $0 } } }

    /// Shows the unread mails only (attribute selector).
    func filterUnread() async {
        model.attributes.seen = false
        _ = await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["seen"] as? Bool == false }
        _ = await eventually { model.threads.count == 6 }
    }

    func pressEscape() {
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: KeyCode.escape)!
        _ = controller.handle(e)
    }

    func pressReturn() {
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: KeyCode.returnKey)!
        _ = controller.handle(e)
    }

    // MARK: Opening a mail in one column

    @Test func returnInOneColumnMarksTheMailReadAtOnce() async {
        controller.columns = 1
        // The first mail was selected by the app: alone, it stays unread.
        pressReturn()
        #expect(controller.reading)
        #expect(await eventually { seenCalls == [1] })
        // Leaving it afterwards does not mark it a second time.
        press("j")
        await pause()
        #expect(seenCalls.filter { $0 == 1 }.count == 1)
    }

    @Test func aDoubleClickInOneColumnMarksTheMailReadAtOnce() async {
        controller.columns = 1
        controller.clickThread(3, .none)
        controller.clickThread(3, .none)
        #expect(controller.reading)
        #expect(await eventually { seenCalls.contains(3) })
    }

    @Test func returnOnAReadMailOrAgainInTheReaderMarksNothing() async {
        controller.columns = 1
        box.setSeen(1, true)
        await refresh()
        pressReturn()
        // Return again while reading: already open, nothing to do.
        pressReturn()
        await pause()
        #expect(controller.reading && seenCalls.isEmpty)
    }

    @Test func theOneColumnListMarksNothingWhileTheMailIsHidden() async {
        controller.columns = 1
        press("j")
        await pause()
        // Mail 2 was selected by hand and kept, but only its row was shown.
        #expect(model.selectedThreadId == 2 && seenCalls.isEmpty)
        // Leaving the app's first choice by hand marks nothing either.
        press("j")
        await pause()
        #expect(seenCalls.isEmpty)
    }

    @Test func aMailSelectedByHandIsMarkedOnceInTheReaderOnly() async {
        controller.columns = 1
        press("j")
        pressReturn()
        #expect(await eventually { seenCalls == [2] })
        // In the reader, j / k follow the usual delay.
        press("j")
        #expect(await eventually { seenCalls == [2, 3] })
        // Back in the list: staying on a row marks nothing.
        pressEscape()
        press("j")
        await pause()
        #expect(seenCalls == [2, 3])
    }

    @Test func goingBackToTwoColumnsStartsTheDelay() async {
        controller.columns = 1
        press("j")
        await pause()
        #expect(seenCalls.isEmpty)
        controller.columns = 2
        #expect(await eventually { seenCalls == [2] })
    }

    @Test func aMailWhoseContentArrivesAfterTheReaderCameBackIsMarkedOnce() async {
        // The order a loaded machine produced: the content of mail 2 comes
        // back only once the reader is visible again. The opening and the
        // return of the reader each started a delay, and both marked it.
        controller.columns = 1
        box.hold(2)
        press("j")
        await pause()
        box.release()
        // Blocks the main actor while the fake core answers on its thread:
        // the answer is queued before the delay of the reader starts.
        Thread.sleep(forTimeInterval: 0.05)
        controller.columns = 2
        #expect(await eventually { seenCalls == [2] })
        await pause()
        #expect(seenCalls == [2], "marked once")
    }

    @Test func hidingTheReaderDuringTheDelayMarksNothing() async {
        model.seenDelay = 0.2
        controller.columns = 1
        press("j")
        pressReturn()
        // Return marks mail 2 at once; mail 3 is shown, then hidden at once.
        press("j")
        controller.reading = false
        try? await Task.sleep(for: .milliseconds(400))
        #expect(seenCalls == [2])
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
        // A delay well above the time the test takes to press j (50 ms
        // could pass on a busy machine, making the test flaky).
        model.seenDelay = 0.5
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

    // MARK: Marking read with U

    /// `(thread_id, seen)` of every `set_thread_seen` sent so far.
    var seenRequests: [(Int, Bool)] {
        core.calls("set_thread_seen").flatMap { c -> [(Int, Bool)] in
            let ids = (c["thread_ids"] as? [Int]) ?? [c["thread_id"] as? Int].compactMap { $0 }
            return ids.map { ($0, c["seen"] as? Bool ?? false) }
        }
    }

    @Test func capitalUMarksReadAndNeverUnread() async {
        // The first mail, selected by the app, is unread: U marks it read.
        press("U")
        #expect(await eventually { seenRequests.map(\.0) == [1] && seenRequests[0].1 })
        await refresh()
        #expect(model.selectedThread?.unreadCount == 0)
        // Read already: U again sends nothing, where u would make it unread.
        press("U")
        await pause()
        #expect(seenRequests.count == 1 && model.selectedThread?.unreadCount == 0)
        press("u")
        #expect(await eventually { seenRequests.count == 2 && !seenRequests[1].1 })
        await refresh()
        // Unread again: U marks it read once more.
        press("U")
        #expect(await eventually { seenRequests.count == 3 && seenRequests[2].1 })
    }

    @Test func capitalUMarksEveryMarkedMailReadAndSkipsTheReadOnes() async {
        box.setSeen(2, true)
        await refresh()
        model.marked = [1, 2, 3]
        press("U")
        #expect(await eventually { Set(seenRequests.map(\.0)) == [1, 3] })
        await pause()
        // No request for mail 2, already read, and none to make one unread.
        #expect(seenRequests.count == 2 && seenRequests.allSatisfy(\.1))
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
