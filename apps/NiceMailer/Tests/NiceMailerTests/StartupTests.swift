import Foundation
import Testing
@testable import NiceMailer

/// The launch (`AppModel.StartupPhase`, `StartupTimings`): what the empty
/// list says while the core starts and the list loads, and the log line of
/// its duration.
@MainActor
struct StartupTests {
    /// Each phase but the last has its line; the empty list says "No mail"
    /// only once the launch is over.
    @Test func eachPhaseSaysWhatIsHappening() {
        #expect(AppModel.StartupPhase.connecting.message == L("Connecting to the core…"))
        #expect(AppModel.StartupPhase.startingCore.message == L("Starting the core…"))
        #expect(AppModel.StartupPhase.loading.message == L("Loading the mail…"))
        #expect(AppModel.StartupPhase.ready.message == nil)
    }

    /// The log line: the first list, its rows, the socket, and whether the
    /// app had to start the core.
    @Test func theLogLineGivesTheTimings() {
        let started = StartupTimings(socket: .milliseconds(1840), firstList: .milliseconds(2310), startedCore: true, rows: 60)
        #expect(started.line == "first list after 2310 ms (60 rows; socket after 1840 ms, core started by the app)")
        let running = StartupTimings(socket: .milliseconds(12), firstList: .milliseconds(95), startedCore: false, rows: 0)
        #expect(running.line == "first list after 95 ms (0 rows; socket after 12 ms, core already running)")
        #expect(StartupTimings.milliseconds(.seconds(2) + .milliseconds(5)) == 2005)
    }

    /// A model not connected yet says so; the first list shown ends the
    /// launch and records its timings.
    @Test func theFirstListEndsTheLaunch() async {
        let model = AppModel()
        #expect(model.startupPhase == .connecting)
        #expect(model.startupTimings == nil)
        await model.attach(FakeCore(handler: Fixture.answer))
        #expect(await eventually { model.startupPhase == .ready })
        #expect(model.startupTimings?.rows == 8)
        #expect(model.startupTimings?.startedCore == false)
        // Later reloads change nothing: the launch is timed once.
        let first = model.startupTimings
        await model.reloadThreads(keepDepth: true)
        #expect(model.startupTimings == first)
    }

    /// A mailbox without any folder still ends its launch: the list is
    /// empty, and says "No mail" rather than "Loading" for good.
    @Test func anEmptyMailboxEndsTheLaunchToo() async {
        let model = AppModel()
        await model.attach(FakeCore { method, params in
            method == "list_folders" ? ["items": [], "as_of": Fixture.position] : Fixture.answer(method, params)
        })
        #expect(await eventually { model.startupPhase == .ready })
        #expect(model.startupTimings?.rows == 0)
    }

    /// The settings and the state of the AI, slow to come, do not hold the
    /// first list back: they are asked at the same time. Checked by order,
    /// not by a duration, which a busy machine would make fail: the list is
    /// there while the settings are still on their way (2 s each, when
    /// `eventually` gives up after 1 s).
    @Test func slowSettingsDoNotHoldTheFirstListBack() async {
        let core = FakeCore(handler: Fixture.answer)
        core.delays["config_status"] = .seconds(2)
        core.delays["ai_status"] = .seconds(2)
        let model = AppModel()
        var sessionDone = false
        let session = Task {
            await model.attach(core)
            sessionDone = true
        }
        #expect(await eventually { model.threads.count == 8 })
        #expect(!sessionDone)
        await session.value
        #expect(core.calls("config_status").count == 1)
        #expect(core.calls("ai_status").count == 1)
    }

    /// While the core starts, the app tries its socket at least every
    /// 0.1 s: each longer wait was time lost after the core was ready.
    @Test func theSocketIsTriedOftenWhileTheCoreStarts() {
        #expect(AppModel.nextSocketDelay(0.05) == 0.1)
        #expect(AppModel.nextSocketDelay(0.1) == 0.1)
        var delay = 0.05
        var waited = 0.0
        for _ in 0..<20 {
            waited += delay
            delay = AppModel.nextSocketDelay(delay)
        }
        // Twenty tries cover about two seconds, never more than 0.1 s apart.
        #expect(delay <= AppModel.socketRetryMax)
        #expect(waited < 2.1)
    }
}
