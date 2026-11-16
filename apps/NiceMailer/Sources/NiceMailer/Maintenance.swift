import Foundation

/// The core is in maintenance (`specifications/mise-a-jour.md`; the
/// `data` of the error `maintenance`, `nm_api::Maintenance`): it serves no
/// client while NiceMailer is updated or its database is upgraded, and
/// says so to every `hello` instead of not answering. The app shows why,
/// and waits for the end of a maintenance that ends by itself
/// (`AppModel.waitOutMaintenance`) without starting a core.
///
/// Swift note: `Decodable` lets `JSONDecoder` build the value from the
/// JSON; with `CoreClient.decoder`, `since` reads the key `since` and the
/// snake_case keys become camelCase. The raw values of `Reason` are the
/// JSON strings themselves (the conversion applies to keys, not values).
struct CoreMaintenance: Decodable, Equatable, Sendable {
    enum Reason: String, Decodable, Sendable {
        /// An update of NiceMailer is being installed.
        case update
        /// The database is being upgraded to the data format of the core.
        case migration
        /// The database was written by a newer NiceMailer: never touched.
        case newerSchema = "newer_schema"
        /// The upgrade failed; the database was put back as it was.
        case migrationFailed = "migration_failed"
    }

    let reason: Reason
    /// Migration: the data format before, after, and reached so far.
    /// Newer schema: the format of the database (`from`) and the core's.
    var from: Int?
    var to: Int?
    var at: Int?
    /// When it began, in seconds since 1970.
    let since: Int
    /// A failed migration: the core's error (in French, its language).
    var message: String?

    /// The maintenance of an error of the core, if it is one.
    init?(error: Error) {
        guard let e = error as? CoreClient.RPCError, e.code == CoreClient.RPCError.maintenance,
              let data = e.rawData,
              let m = try? CoreClient.decoder.decode(CoreMaintenance.self, from: data) else { return nil }
        self = m
    }

    init(reason: Reason, from: Int? = nil, to: Int? = nil, at: Int? = nil, since: Int, message: String? = nil) {
        (self.reason, self.from, self.to, self.at, self.since, self.message) = (reason, from, to, at, since, message)
    }

    /// Whether it ends by itself: an update or a migration, which the app
    /// waits for. The others need the user (another version, another
    /// mailbox, a new try).
    var isTransient: Bool { reason == .update || reason == .migration }

    /// What the screen says, in the user's language, with the time since
    /// it began. A pure function, tested.
    func text(now: Date = Date()) -> String {
        let elapsed = max(0, Int(now.timeIntervalSince1970) - since)
        let duration = elapsed < 60 ? L("%lld s", elapsed) : L("%lld min %lld s", elapsed / 60, elapsed % 60)
        switch reason {
        case .update:
            return L("NiceMailer is being updated. It opens again as soon as the new version is installed.")
        case .migration:
            return L("The core is upgrading this mailbox to the data format %lld (from %lld, now at %lld), for %@. It is done once, and can take a few minutes on a large mailbox: NiceMailer opens as soon as it ends. Do not stop the core meanwhile.",
                     to ?? 0, from ?? 0, at ?? from ?? 0, duration)
        case .newerSchema:
            return L("The core cannot open this mailbox: it was written by a newer version of NiceMailer (data format %lld; this one reads %lld). Update NiceMailer, or choose another mailbox.",
                     from ?? 0, to ?? 0)
        case .migrationFailed:
            return L("The upgrade of this mailbox failed (%@). It was put back as the previous version of NiceMailer left it: nothing is lost. Try again, or install the previous version of NiceMailer; the details are in core.log of the mailbox.",
                     message ?? "?")
        }
    }

    /// The few words of the status bar of the geek theme.
    var shortText: String {
        switch reason {
        case .update: L("updating NiceMailer…")
        case .migration: L("upgrading the mailbox…")
        case .newerSchema, .migrationFailed: L("mailbox unavailable")
        }
    }
}
