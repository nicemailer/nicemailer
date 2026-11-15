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
}
