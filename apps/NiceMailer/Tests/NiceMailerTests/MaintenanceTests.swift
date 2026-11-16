import Foundation
import Testing
@testable import NiceMailer

/// The core in maintenance (`CoreMaintenance`, `docs/updates.md`): the
/// error read as the core sends it, what the app says, and the wait.
@MainActor
struct MaintenanceTests {
    /// An error line of the core, as `CoreClient` reads it.
    func error(_ data: [String: Any], code: Int = 1003) -> CoreClient.RPCError {
        CoreClient.rpcError(["code": code, "message": "m", "data": data])
    }

    @Test func theErrorIsReadAsTheCoreSendsIt() throws {
        // As `nm_api::Maintenance` serializes it.
        let m = try #require(CoreMaintenance(error: error(["reason": "migration", "from": 9, "to": 13, "at": 11, "since": 1000])))
        #expect(m == CoreMaintenance(reason: .migration, from: 9, to: 13, at: 11, since: 1000))
        #expect(m.isTransient)
        #expect(CoreMaintenance(error: error(["reason": "newer_schema", "from": 14, "to": 13, "since": 1]))?.reason == .newerSchema)
        let failed = try #require(CoreMaintenance(error: error(["reason": "migration_failed", "since": 1, "message": "disque plein"])))
        #expect(failed.reason == .migrationFailed && !failed.isTransient && failed.message == "disque plein")
        // Another error, or a maintenance without its data: not one.
        #expect(CoreMaintenance(error: error(["reason": "update", "since": 1], code: 1001)) == nil)
        #expect(CoreMaintenance(error: CoreClient.rpcError(["code": 1003, "message": "m"])) == nil)
        #expect(CoreMaintenance(error: CoreClient.ConnectionError.closed) == nil)
    }

    @Test func eachMaintenanceSaysWhatHappensAndWhatToDo() {
        let now = Date(timeIntervalSince1970: 1075)
        let migration = CoreMaintenance(reason: .migration, from: 9, to: 13, at: 11, since: 1000)
        let text = migration.text(now: now)
        #expect(text.contains("13") && text.contains("9") && text.contains("11"))
        #expect(text.contains(L("%lld min %lld s", 1, 15)))
        #expect(CoreMaintenance(reason: .migration, from: 9, to: 13, since: 1042).text(now: now).contains(L("%lld s", 33)))
        #expect(CoreMaintenance(reason: .update, since: 1).text(now: now) == L("NiceMailer is being updated. It opens again as soon as the new version is installed."))
        #expect(CoreMaintenance(reason: .newerSchema, from: 14, to: 13, since: 1).text().contains("14"))
        #expect(CoreMaintenance(reason: .migrationFailed, since: 1, message: "disque plein").text().contains("disque plein"))
        #expect(CoreMaintenance(reason: .update, since: 1).shortText == L("updating NiceMailer…"))
        #expect(migration.shortText == L("upgrading the mailbox…"))
    }

    /// An update, then a migration (the new core upgrading the mailbox),
    /// then served: the screen follows each answer of the probe, then the
    /// wait ends.
    @Test func theAppWaitsWhileTheMaintenanceEndsByItself() async {
        let model = AppModel()
        var answers: [CoreMaintenance?] = [CoreMaintenance(reason: .migration, from: 9, to: 13, at: 9, since: 1), nil]
        var shown: [String] = []
        let served = await model.waitOutMaintenance(CoreMaintenance(reason: .update, since: 1), every: .milliseconds(1)) {
            if case .maintenance(_, let short) = model.status { shown.append(short) }
            return answers.removeFirst()
        }
        #expect(served)
        #expect(shown == [L("updating NiceMailer…"), L("upgrading the mailbox…")])
        #expect(model.lastMaintenance == nil)
    }

    /// A maintenance only the user can end: not waited for, its reason
    /// shown, kept for "Try Again" and the core's log.
    @Test func aMaintenanceOnlyTheUserCanEndIsShown() async {
        let model = AppModel()
        var probes = 0
        let failed = CoreMaintenance(reason: .migrationFailed, from: 9, to: 13, since: 1, message: "disque plein")
        let served = await model.waitOutMaintenance(CoreMaintenance(reason: .migration, from: 9, to: 13, since: 1),
                                                    every: .milliseconds(1)) {
            probes += 1
            return failed
        }
        #expect(!served)
        #expect(probes == 1)
        #expect(model.status == .failed(failed.text()))
        #expect(model.lastMaintenance == failed)
    }

    /// The core of another version than the app (the one launched before
    /// the update) is restarted, once, and only the agent's.
    @Test func aCoreOfAnotherVersionIsRestartedOnce() {
        let restart = AppModel.coreNeedsRestart
        #expect(restart("0.1.0", "0.2.0", .enabled, false))
        #expect(!restart("0.2.0", "0.2.0", .enabled, false))
        #expect(!restart("0.1.0", "0.2.0", .enabled, true), "once a launch")
        #expect(!restart("0.1.0", "0.2.0", .notBundled, false), "a core started by hand")
        #expect(!restart("0.1.0", nil, .enabled, false), "no version: swift run")
    }

    /// Sparkle starts only in a bundle that names its feed and key, never
    /// in a UI scenario.
    @Test func updatesNeedABundleAFeedAndAKey() {
        let app = URL(fileURLWithPath: "/Applications/NiceMailer.app")
        let info: [String: Any] = ["SUFeedURL": "https://example.org/appcast.xml", "SUPublicEDKey": "abc="]
        #expect(Updater.isConfigured(info: info, bundleURL: app, scripted: false))
        #expect(!Updater.isConfigured(info: info, bundleURL: app, scripted: true))
        #expect(!Updater.isConfigured(info: info, bundleURL: URL(fileURLWithPath: "/tmp/debug"), scripted: false))
        #expect(!Updater.isConfigured(info: ["SUFeedURL": "https://example.org/appcast.xml"], bundleURL: app, scripted: false))
        #expect(!Updater.isConfigured(info: ["SUFeedURL": "", "SUPublicEDKey": "abc="], bundleURL: app, scripted: false))
        #expect(!Updater.isConfigured(info: nil, bundleURL: app, scripted: false))
    }

    /// A model without a core: preparing an update does not wait for an
    /// answer that cannot come, and cancelling only acts after preparing.
    @Test func preparingWithoutACoreGoesOn() async {
        let model = AppModel()
        await model.prepareUpdate(limit: .milliseconds(10))
        #expect(model.installingUpdate)
        let fresh = AppModel()
        await fresh.cancelUpdate()
        #expect(!fresh.installingUpdate)
    }
}
