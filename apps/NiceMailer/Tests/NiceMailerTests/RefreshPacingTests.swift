import Foundation
import Testing
@testable import NiceMailer

/// The pacing of the reloads caused by the core's events
/// (`AppModel.scheduleRefresh`): one reload for a burst, at most one every
/// `refreshInterval`, never two at once, and never starved by events that
/// keep coming (the import of an account sends them all the time).
@MainActor
struct RefreshPacingTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()

    init() async {
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 }
    }

    /// A pure function: the burst, or what is left of the interval.
    @Test func theDelayKeepsTheIntervalBetweenTwoReloads() {
        #expect(AppModel.refreshDelay(sinceLastStart: nil) == AppModel.refreshBurst)
        #expect(AppModel.refreshDelay(sinceLastStart: .milliseconds(100)) == .milliseconds(400))
        #expect(AppModel.refreshDelay(sinceLastStart: .milliseconds(490)) == AppModel.refreshBurst)
        #expect(AppModel.refreshDelay(sinceLastStart: .seconds(3)) == AppModel.refreshBurst)
    }

    func changed(_ seq: Int) -> CoreClient.Event {
        CoreClient.Event(method: "ThreadChanged", epoch: "e_test", seq: seq, params: ["thread_id": .int(1), "folder_ids": .ints([1])])
    }

    /// A burst of events: one reload, soon after.
    @Test func aBurstIsOneReload() async {
        let before = model.refreshCount
        for seq in 1...5 { model.handle(changed(seq)) }
        #expect(await eventually { model.refreshCount == before + 1 })
        try? await Task.sleep(for: .milliseconds(150))
        #expect(model.refreshCount == before + 1)
    }

    /// Events every 20 ms for 1.2 s, as during an import: the lists are
    /// reloaded all the same (a debounce would never have reloaded them),
    /// but at most once every `refreshInterval`, and the reloads, which
    /// each make several calls, never overlap.
    @Test func eventsThatKeepComingAreReloadedAtAPace() async {
        let before = model.refreshCount
        let foldersBefore = core.calls("list_folders").count
        let start = ContinuousClock.now
        var seq = 1
        while ContinuousClock.now - start < .milliseconds(1200) {
            model.handle(changed(seq))
            seq += 1
            try? await Task.sleep(for: .milliseconds(20))
        }
        _ = await eventually { model.refreshCount > before }
        let reloads = model.refreshCount - before
        #expect(reloads >= 2, "\(reloads) reloads")
        #expect(reloads <= 4, "\(reloads) reloads for \(seq) events")
        #expect(core.calls("list_folders").count - foldersBefore == reloads)
    }
}
