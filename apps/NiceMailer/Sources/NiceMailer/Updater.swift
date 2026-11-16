import AppKit
import os
import Sparkle

/// The updates of NiceMailer, by Sparkle (`docs/updates.md`,
/// `specifications/mise-a-jour.md`).
///
/// Sparkle reads the feed of the releases (`SUFeedURL` of Info.plist, an
/// `appcast.xml` signed with the EdDSA key whose public half is
/// `SUPublicEDKey`), offers a newer version, downloads it, checks its
/// signature, and replaces the bundle. What NiceMailer adds is the core:
/// it runs on its own (a launch agent), from the bundle being replaced,
/// and serves other clients (the CLI, an AI agent). So just before Sparkle
/// installs, the app calls `prepare_update` (`AppModel.prepareUpdate`): the
/// core stops serving and syncing, and stays alive in maintenance until it
/// sees its program replaced, then leaves for launchd to start the new one.
///
/// Two moments Sparkle installs at, two hooks:
/// - "Install and Relaunch": Sparkle asks whether to postpone the relaunch
///   (`shouldPostponeRelaunchForUpdate`); the app prepares the core, then
///   lets Sparkle go on.
/// - "Install on quit" (an update downloaded in the background, installed
///   when the app quits): Sparkle says so in advance
///   (`willInstallUpdateOnQuit`); at quit, the app delays its termination
///   the time to prepare the core (`AppDelegate.applicationShouldTerminate`).
///
/// An update that fails after the preparation (`didAbortWithError`):
/// `cancel_update`, and the core serves again. If the app dies in between,
/// the core leaves the maintenance after `core.update_timeout_secs`, or at
/// the next launch of the app (`AppModel.connect`).
///
/// Swift note: Sparkle calls its delegate on the main thread, from
/// Objective-C. The class is not `@MainActor` (the protocol's methods are
/// not), so each method hops to the main actor explicitly to reach the
/// model.
final class Updater: NSObject, SPUUpdaterDelegate {
    /// The one updater of the app.
    @MainActor static let shared = Updater()

    /// The model, to prepare the core; set by `start`.
    @MainActor private(set) weak var model: AppModel?
    @MainActor private var controller: SPUStandardUpdaterController?
    /// Sparkle will install the update downloaded when the app quits.
    @MainActor private(set) var installsOnQuit = false

    /// Whether "Check for Updates…" can be used: Sparkle started.
    @MainActor var canCheck: Bool { controller != nil }

    /// Whether this app can update itself: an app bundle (not `swift run`)
    /// whose Info.plist names a feed and the public key that signs it, and
    /// not a UI scenario, which must never touch the network or replace the
    /// app. A development build made without the key has no updates
    /// (`scripts/make-bundle.sh`). A pure function, tested.
    nonisolated static func isConfigured(info: [String: Any]?, bundleURL: URL, scripted: Bool) -> Bool {
        guard !scripted, bundleURL.pathExtension == "app" else { return false }
        let feed = info?["SUFeedURL"] as? String ?? ""
        let key = info?["SUPublicEDKey"] as? String ?? ""
        return !feed.isEmpty && !key.isEmpty
    }

    /// Starts Sparkle once: it checks for updates in the background, as
    /// the user allowed it to (it asks at the second launch).
    @MainActor func start(model: AppModel) {
        self.model = model
        guard controller == nil,
              Self.isConfigured(info: Bundle.main.infoDictionary, bundleURL: Bundle.main.bundleURL, scripted: UIScript.isActive)
        else { return }
        controller = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil)
    }

    /// The menu "Check for Updates…".
    @MainActor func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }

    // MARK: - SPUUpdaterDelegate

    /// "Install and Relaunch": the core is prepared before Sparkle replaces
    /// the bundle and relaunches the app. Returning true postpones the
    /// relaunch until `installHandler` is called.
    func updater(_ updater: SPUUpdater, shouldPostponeRelaunchForUpdate item: SUAppcastItem,
                 untilInvokingBlock installHandler: @escaping () -> Void) -> Bool {
        Task { @MainActor in
            await self.model?.prepareUpdate()
            installHandler()
        }
        return true
    }

    /// An update will be installed when the app quits: noted, for
    /// `AppDelegate.applicationShouldTerminate`. False: Sparkle installs it
    /// at quit by itself.
    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem,
                 immediateInstallationBlock immediateInstallHandler: @escaping () -> Void) -> Bool {
        Task { @MainActor in self.installsOnQuit = true }
        return false
    }

    /// The update failed (download, signature, installation): if the core
    /// was prepared, it serves again.
    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        Task { @MainActor in
            UpdaterLog.log.error("update aborted: \(error.localizedDescription, privacy: .public)")
            await self.model?.cancelUpdate()
        }
    }
}

/// The log of the updates, in the system log (`subsystem ==
/// "eu.nicemailer.app" AND category == "update"`).
enum UpdaterLog {
    static let log = Logger(subsystem: "eu.nicemailer.app", category: "update")
}
