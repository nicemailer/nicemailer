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
        log.notice("agent status \(service.status.rawValue, privacy: .public)")
        switch service.status {
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

    /// Runs `launchctl`; its exit status (-1 if it could not run).
    @discardableResult
    private static func launchctl(_ arguments: [String]) -> Int32 {
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
