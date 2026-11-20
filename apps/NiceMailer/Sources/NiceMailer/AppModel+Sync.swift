import Foundation

/// The synchronization in progress, said in the status bar (`ui.md`,
/// "Avancement de la synchronisation"; `docs/sync.md`, "Progress").
///
/// The first synchronization of a large account takes minutes: tens of
/// thousands of headers, then as many bodies. Without a word, the app
/// looked frozen. The core says how far it is with `SyncProgress`, a
/// transient event (no `seq`, never replayed) sent at most four times a
/// second: the headers of a folder, from 0 to their total, then the bodies
/// of the account, until none is left.
///
/// The model keeps the last event of each account and stage. An event that
/// reaches its total ends it; one not followed by another for
/// `syncStale` ends it too: a pass cut by the network, or stopped, sends
/// no end.
extension AppModel {
    /// One account's synchronization in progress, from its last
    /// `SyncProgress`.
    struct SyncActivity: Equatable, Sendable {
        enum Stage: String, Sendable {
            case headers, bodies
        }

        var accountId: Int
        var stage: Stage
        /// The folder whose headers are fetched; `nil` for the bodies,
        /// which the core takes from every folder of the account.
        var folderId: Int?
        var done: Int
        var total: Int
        /// When the event came, to end an activity the core no longer
        /// speaks of.
        var at: ContinuousClock.Instant

        /// The key of `syncActivity`: one per account and stage, so that
        /// the bodies of an account (downloaded on a connection of their
        /// own) do not hide its headers, nor the reverse.
        var key: String { "\(accountId)-\(stage.rawValue)" }
    }

    /// Without an event for this long, an activity is taken as ended: the
    /// core sends one every 250 ms (headers) or 500 ms (bodies) while it
    /// works, and a batch of 100 headers rarely takes more than a few
    /// seconds, even on a slow server.
    nonisolated static let syncStale: Duration = .seconds(30)
    /// How often the stale activities are looked for, while there is one.
    nonisolated static let syncPruneEvery: Duration = .seconds(5)

    /// The activity a `SyncProgress` gives (`nil`: not one, or a malformed
    /// one), received at `now`. A pure function, so that it is tested
    /// without a core.
    nonisolated static func syncActivity(_ e: CoreClient.Event, now: ContinuousClock.Instant) -> SyncActivity? {
        guard e.method == "SyncProgress",
              let account = e.params["account_id"]?.int,
              let stage = e.params["stage"]?.string.flatMap(SyncActivity.Stage.init(rawValue:)),
              let done = e.params["done"]?.int, let total = e.params["total"]?.int
        else { return nil }
        return SyncActivity(accountId: account, stage: stage, folderId: e.params["folder_id"]?.int,
                            done: done, total: total, at: now)
    }

    /// The activities once `a` is applied: it replaces the previous one of
    /// its account and stage, and its end (done reaching the total)
    /// removes it.
    nonisolated static func applying(_ a: SyncActivity, to activities: [String: SyncActivity]) -> [String: SyncActivity] {
        var out = activities
        out[a.key] = a.done >= a.total ? nil : a
        return out
    }

    /// The activities still going at `now`.
    nonisolated static func pruned(_ activities: [String: SyncActivity], now: ContinuousClock.Instant,
                                   stale: Duration = syncStale) -> [String: SyncActivity] {
        activities.filter { now - $0.value.at < stale }
    }

    /// A `SyncProgress` received: the status bar follows it.
    func syncEvent(_ e: CoreClient.Event) {
        guard let a = Self.syncActivity(e, now: .now) else { return }
        let next = Self.applying(a, to: syncActivity)
        // Unchanged after a stale end: no redraw of the bar.
        if next != syncActivity { syncActivity = next }
        startSyncPrune()
    }

    /// Looks for the stale activities every `syncPruneEvery`, until there
    /// is none left. One task at a time.
    private func startSyncPrune() {
        guard syncPrune == nil, !syncActivity.isEmpty else { return }
        syncPrune = Task { @MainActor [weak self] in
            while let self, !self.syncActivity.isEmpty {
                try? await Task.sleep(for: Self.syncPruneEvery)
                let next = Self.pruned(self.syncActivity, now: .now)
                if next != self.syncActivity { self.syncActivity = next }
            }
            self?.syncPrune = nil
        }
    }

    /// The line of the status bar, or `nil` when nothing is synchronizing.
    var syncLine: String? {
        Self.syncLine(Array(syncActivity.values), folders: folders, names: folderNames,
                      severalAccounts: hasSeveralAccounts)
    }

    /// What the status bar says of `activities`: the headers first (the
    /// mails are not listed before their headers), the most recent one,
    /// else the bodies. "Importing Inbox: 12,300 of 44,829" says where the
    /// import is and how far it has to go; the bodies say only what is
    /// left ("Downloading mails: 41,429 left"), as the core counts them
    /// again from 0 each time it reads its queue. With several accounts,
    /// the account comes first. A pure function, tested alone.
    nonisolated static func syncLine(_ activities: [SyncActivity], folders: [FolderView], names: FolderNames,
                                     severalAccounts: Bool) -> String? {
        let latest = { (stage: SyncActivity.Stage) in
            activities.filter { $0.stage == stage }.max { $0.at < $1.at }
        }
        guard let a = latest(.headers) ?? latest(.bodies) else { return nil }
        let text: String
        switch a.stage {
        case .headers:
            if let folder = folders.first(where: { $0.id == a.folderId }) {
                text = L("Importing %@: %@ of %@", names.name(of: folder), count(a.done), count(a.total))
            } else {
                // A folder the app has not loaded yet (a new account).
                text = L("Importing mails: %@ of %@", count(a.done), count(a.total))
            }
        case .bodies:
            text = L("Downloading mails: %@ left", count(a.total - a.done))
        }
        guard severalAccounts, let account = folders.first(where: { $0.accountId == a.accountId })?.accountName else {
            return text
        }
        return account + " · " + text
    }

    /// A number as the app's locale writes it (the language of the app,
    /// the region of the Mac): 44,829 or 44 829.
    nonisolated static func count(_ n: Int, locale: Locale = uiLocale) -> String {
        n.formatted(.number.locale(locale))
    }
}
