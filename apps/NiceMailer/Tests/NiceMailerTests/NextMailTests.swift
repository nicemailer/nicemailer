import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Where the cursor goes when the selected mail leaves the list
/// (`AppModel.successor`): the next mail still listed, even when the mails
/// right after it left in the same change.
///
/// The bug this guards against: in All Mail, moving a mail to Banned put
/// the cursor on the next row; that row came from the same sender, so the
/// core filed it to Banned too (`dossiers.md`, "Cibles proposées"), and
/// the reload that followed sent the cursor back to the top of the list.
@MainActor
struct NextMailTests {
    // MARK: successor, a pure function

    @Test func theNextMailStillListedFollowsTheOneThatLeft() {
        // 3 left alone: 4 follows.
        #expect(AppModel.successor(of: 3, previous: [1, 2, 3, 4, 5], listed: [1, 2, 4, 5]) == 4)
        // 3 left with 4 and 5 (same sender): 6, not the first mail.
        #expect(AppModel.successor(of: 3, previous: [1, 2, 3, 4, 5, 6], listed: [1, 2, 6]) == 6)
    }

    @Test func atTheEndTheCursorGoesBackToTheNearestMailBefore() {
        #expect(AppModel.successor(of: 4, previous: [1, 2, 3, 4, 5], listed: [1, 2]) == 2)
    }

    @Test func withoutAPlaceTheCursorGoesToTheFirstMailOrNowhere() {
        // Not in the previous list (a new list), or nothing selected.
        #expect(AppModel.successor(of: 9, previous: [1, 2], listed: [5, 6]) == 5)
        #expect(AppModel.successor(of: nil, previous: [1, 2], listed: [5, 6]) == 5)
        #expect(AppModel.successor(of: 3, previous: [], listed: [5, 6]) == 5)
        // Every mail left.
        #expect(AppModel.successor(of: 1, previous: [1, 2], listed: []) == nil)
    }

    @Test func newMailsAtTheTopDoNotMoveTheCursorThere() {
        // 7 and 8 arrived above while 3 and 4 left: the cursor stays where
        // the user was in the list, on 5.
        #expect(AppModel.successor(of: 3, previous: [1, 2, 3, 4, 5], listed: [7, 8, 1, 2, 5]) == 5)
    }

    // MARK: In the app, against the fake core

    /// The fake mailbox: six threads in the Inbox, minus those moved away;
    /// read states, for the "unread" filter. Moving thread n also files
    /// thread n + 1, as moving a mail to Banned files its sender's other
    /// mails.
    final class Mailbox: @unchecked Sendable {
        let lock = NSLock()
        var gone = Set<Int>()
        var seen = Set<Int>()
        func isGone(_ id: Int) -> Bool { lock.withLock { gone.contains(id) } }
        func isSeen(_ id: Int) -> Bool { lock.withLock { seen.contains(id) } }
        func remove(_ ids: [Int]) { lock.withLock { gone.formUnion(ids) } }
        func setSeen(_ id: Int) { lock.withLock { _ = seen.insert(id) } }
        /// `get_thread` of `id` waits until `release()` (see `ReadingTests`).
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
    let controller: InterfaceController

    init() async {
        let box = self.box
        core = FakeCore { method, params in
            switch method {
            case "list_threads":
                let unreadOnly = (params["filter"] as? [String: Any])?["seen"] as? Bool == false
                let items = (1...6).filter { !box.isGone($0) && (!unreadOnly || !box.isSeen($0)) }.map { id in
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
                // A thread moved away has its mail in the target folder
                // (Junk, 5): not one to keep in place in the Inbox list.
                if box.isGone(id) { messages[0]["folder_id"] = 5 }
                view["messages"] = messages
                return view
            case "move_threads":
                let ids = (params["threads"] as? [[String: Any]] ?? []).compactMap { $0["thread_id"] as? Int }
                // The sender's other mail, right below, goes with it.
                box.remove(ids + ids.map { $0 + 1 })
                return Fixture.answer(method, params)
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = TestDefaults.make()
        controller = InterfaceController(model: model, defaults: defaults, textSize: TextSize())
        // Long enough that nothing is marked read behind the tests' back.
        model.seenDelay = 60
        await model.attach(core)
        _ = await eventually { model.threads.count == 6 && model.openThread != nil }
    }

    /// What the app does on a change event: the open thread, then the list.
    func refresh() async {
        await model.reloadOpenThread()
        await model.reloadThreads(keepDepth: true)
    }

    @Test func aMoveThatTakesTheNextMailToo() async {
        model.selectedThreadId = 2
        model.moveTargets(to: "junk")
        // At once (optimistic): 2 left, the cursor on the next row, 3.
        #expect(model.selectedThreadId == 3)
        #expect(await eventually { box.isGone(3) })
        await refresh()
        // 3 left with 2: the cursor goes on to 4, not back to 1.
        #expect(model.threads.map(\.id) == [1, 4, 5, 6])
        #expect(model.selectedThreadId == 4)
    }

    @Test func aMoveThatTakesTheLastMailsGoesToTheOneBefore() async {
        // Mails 1 to 5; 4 moved takes 5, the last one, with it.
        box.remove([6])
        await refresh()
        model.selectedThreadId = 4
        model.moveTargets(to: "junk")
        #expect(await eventually { box.isGone(5) })
        await refresh()
        // Nothing after: the nearest mail before, 3, not the first one.
        #expect(model.threads.map(\.id) == [1, 2, 3])
        #expect(model.selectedThreadId == 3)
    }

    @Test func aMailReadWhileTheCursorReachesItIsSkipped() async {
        // Unread mails only. The cursor goes from 2 to 3 while 3's content
        // is still loading, and 3 is read meanwhile (on another device):
        // the list cannot keep 3 in place (its reader shows 2 still), so it
        // leaves; the cursor goes on to 4 rather than back to the top.
        model.attributes.seen = false
        _ = await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["seen"] as? Bool == false }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        box.hold(3)
        model.selectedThreadId = 3
        box.setSeen(3)
        await model.reloadThreads(keepDepth: true)
        #expect(!model.threads.contains { $0.id == 3 })
        #expect(model.selectedThreadId == 4)
        box.release()
    }
}
