import AppKit
import Foundation
import OSLog
import ServiceManagement

/// The core as a launch agent of NiceMailer.app (`architecture.md`,
/// section 6; `docs/app-bundle.md`).
///
/// The app built by `make bundle` carries the core
/// (`Contents/Helpers/nicemailer-core`) and the agent that declares it
/// (`Contents/Library/LaunchAgents/eu.nicemailer.core.plist`). At each
/// launch the app registers the agent if it is not (`SMAppService`); launchd
/// then starts the core at once, at each session opening, and again after
/// a crash. The core keeps running when the app quits, so that mail is
/// received and sorted, and pending sends leave.
///
/// An app started by `swift run` or `make app` has no agent: the core is
/// started by hand (`make core`), as before.
enum CoreAgent {
    static let label = "eu.nicemailer.core"
    static let plistName = "eu.nicemailer.core.plist"
    /// What the app does with the agent, in the system log:
    /// `log show --last 1h --predicate 'subsystem == "eu.nicemailer.app"'`.
    static let log = Logger(subsystem: "eu.nicemailer.app", category: "core-agent")

    /// What the app can do with the agent.
    enum State: Equatable {
        /// Not an app bundle carrying the core: started by hand.
        case notBundled
        /// Registered: launchd starts the core.
        case enabled
        /// Registered, but the user must allow it in System Settings >
        /// General > Login Items (or turned it off there).
        case requiresApproval
        /// The registration failed; the reason.
        case failed(String)
    }

    /// Whether this process is an app bundle that carries the core.
    static var isBundled: Bool {
        let bundle = Bundle.main.bundleURL
        guard bundle.pathExtension == "app" else { return false }
        return FileManager.default.isExecutableFile(atPath: bundle.appendingPathComponent("Contents/Helpers/nicemailer-core").path)
    }

    /// Registers the agent if it is not, and says where it stands.
    static func register() -> State {
        guard isBundled, !UIScript.isActive else { return .notBundled }
        let service = SMAppService.agent(plistName: plistName)
        // Read once: each read of `status` asks the system (a message to
        // another process), and the second read here cost about a second
        // of the app's launch, timed by the log of `connect`.
        let asked = ContinuousClock.now
        let status = service.status
        let took = StartupTimings.milliseconds(ContinuousClock.now - asked)
        log.notice("agent status \(status.rawValue, privacy: .public), read in \(took) ms")
        switch status {
        case .enabled:
            return .enabled
        case .requiresApproval:
            return .requiresApproval
        default:
            do {
                try service.register()
            } catch {
                log.error("register failed: \(error.localizedDescription, privacy: .public)")
                return service.status == .requiresApproval ? .requiresApproval : .failed(error.localizedDescription)
            }
            return service.status == .requiresApproval ? .requiresApproval : .enabled
        }
    }

    /// Asks launchd to start the core (it may have stopped: a stop asked,
    /// another core holding the directory at its start). `restart`: stop
    /// it first, so that it starts again on the directory chosen.
    /// Returns whether launchd accepted (`false`: the agent is not loaded,
    /// it is then registered again).
    @discardableResult
    static func kickstart(restart: Bool = false) -> Bool {
        let status = launchctl(["kickstart"] + (restart ? ["-k"] : []) + [target])
        log.notice("kickstart\(restart ? " -k" : "", privacy: .public): \(status)")
        if status != 0 {
            // Not loaded any more (unloaded by hand, a launchd hiccup):
            // registered again, which loads it, and asked once more.
            let service = SMAppService.agent(plistName: plistName)
            try? service.unregister()
            try? service.register()
            let again = launchctl(["kickstart", target])
            log.notice("registered again, kickstart: \(again)")
            return again == 0
        }
        return true
    }

    /// Registers the agent again when launchd refuses to start its core,
    /// and says whether it did (then asks for the core once more).
    ///
    /// Why: when the app registers the agent, launchd records a launch
    /// constraint made from the core's signature at that moment: its
    /// identifier, its team, and its *validation category* (Apple
    /// Development, Developer ID…). A core signed differently, such as a
    /// release installed over a development build or the reverse, no
    /// longer meets it: launchd refuses to start it ("spawn failed", exit
    /// 78) while `launchctl kickstart` still returns 0, and the core stays
    /// unreachable. Registering again records the constraint of the core
    /// now in the bundle. `docs/app-bundle.md`, "Signature".
    ///
    /// Called only on the slow path (no core answered for 2 s): one more
    /// `launchctl print`, never on a normal launch.
    @discardableResult
    static func repairIfSpawnRefused() -> Bool {
        guard let printed = launchctlOutput(["print", target]), spawnRefused(printed) else { return false }
        log.error("launchd refuses to start the core (spawn failed): registering the agent again")
        let service = SMAppService.agent(plistName: plistName)
        try? service.unregister()
        try? service.register()
        let again = launchctl(["kickstart", target])
        log.notice("registered again after a refused spawn, kickstart: \(again)")
        return true
    }

    /// Whether the output of `launchctl print` says that launchd failed to
    /// start the program, rather than the program having run and left.
    /// A pure function, tested. The core never exits with 78 itself: a
    /// core that ran and stopped shows "job state = exited" instead.
    nonisolated static func spawnRefused(_ printed: String) -> Bool {
        printed.split(separator: "\n").contains { line in
            let parts = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            return parts.count == 2 && parts[0] == "job state" && parts[1] == "spawn failed"
        }
    }

    /// Stops the core: SIGTERM, which it handles by finishing its work and
    /// leaving with code 0, so that launchd does not start it again
    /// (`KeepAlive` restarts only a failure). It starts again at the next
    /// launch of the app, or at the next session.
    static func stop() {
        let status = launchctl(["kill", "SIGTERM", target])
        log.notice("stop: \(status)")
    }

    /// Opens System Settings at the Login Items, where the agent is allowed.
    static func openLoginItemsSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    private static var target: String { "gui/\(getuid())/\(label)" }

    /// Runs `launchctl` and returns what it printed, or nil when it failed
    /// (agent not loaded) or could not run.
    private static func launchctlOutput(_ arguments: [String]) -> String? {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = arguments
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        do {
            try p.run()
        } catch {
            return nil
        }
        // Read before waiting: a full pipe would block launchctl, and
        // waitUntilExit would then never return.
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        guard p.terminationStatus == 0 else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    /// Runs `launchctl`; its exit status (-1 if it could not run).
    @discardableResult
    private static func launchctl(_ arguments: [String]) -> Int32 {
        let started = ContinuousClock.now
        defer {
            log.notice("launchctl \(arguments.first ?? "", privacy: .public) took \(StartupTimings.milliseconds(ContinuousClock.now - started)) ms")
        }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = arguments
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do {
            try p.run()
        } catch {
            return -1
        }
        p.waitUntilExit()
        return p.terminationStatus
    }
}
