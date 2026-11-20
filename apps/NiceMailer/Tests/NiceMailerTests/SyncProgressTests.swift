import Foundation
import Testing
@testable import NiceMailer

/// The synchronization in progress, said in the status bar
/// (`AppModel+Sync.swift`, `docs/sync.md`, "Progress").
@MainActor
struct SyncProgressTests {
    typealias Activity = AppModel.SyncActivity

    /// A `SyncProgress` as the core sends it: transient, without `seq`.
    static func event(account: Int = 1, folder: Int? = 1, stage: String = "headers", done: Int, total: Int) -> CoreClient.Event {
        var params: [String: JSONValue] = [
            "account_id": .int(account), "stage": .string(stage), "done": .int(done), "total": .int(total),
        ]
        params["folder_id"] = folder.map { .int($0) } ?? .other
        return CoreClient.Event(method: "SyncProgress", epoch: "e", seq: nil, params: params)
    }

    static func folder(_ id: Int, role: String, account: Int = 1, accountName: String = "perso") throws -> FolderView {
        let json = #"{"id": \#(id), "role": "\#(role)", "name": "\#(role)", "letter": null, "category": null, "total_count": 0, "unread_count": 0, "move_targets": [], "account_id": \#(account), "account_name": "\#(accountName)", "account_kind": "imap"}"#
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(FolderView.self, from: Data(json.utf8))
    }

    let t0 = ContinuousClock.now

    func activity(_ stage: Activity.Stage, folder: Int? = 1, done: Int, total: Int, account: Int = 1, after s: Int = 0) -> Activity {
        Activity(accountId: account, stage: stage, folderId: folder, done: done, total: total, at: t0 + .seconds(s))
    }

    // MARK: Reading the events

    @Test func anEventGivesItsActivity() {
        let a = AppModel.syncActivity(Self.event(done: 3, total: 10), now: t0)
        #expect(a == activity(.headers, done: 3, total: 10))
        // The bodies come without a folder.
        let b = AppModel.syncActivity(Self.event(folder: nil, stage: "bodies", done: 1, total: 2), now: t0)
        #expect(b?.stage == .bodies && b?.folderId == nil)
        // Another event, an unknown stage, a missing number: none.
        #expect(AppModel.syncActivity(CoreClient.Event(method: "AiProgress", epoch: "e", seq: nil, params: [:]), now: t0) == nil)
        #expect(AppModel.syncActivity(Self.event(stage: "flags", done: 1, total: 2), now: t0) == nil)
        var partial = Self.event(done: 1, total: 2)
        partial = CoreClient.Event(method: partial.method, epoch: "e", seq: nil, params: partial.params.filter { $0.key != "total" })
        #expect(AppModel.syncActivity(partial, now: t0) == nil)
    }

    /// An activity replaces the previous one of its account and stage, and
    /// its end removes it; the stages of an account are kept apart.
    @Test func anActivityEndsWhenItReachesItsTotal() {
        var all: [String: Activity] = [:]
        all = AppModel.applying(activity(.headers, done: 0, total: 10), to: all)
        all = AppModel.applying(activity(.bodies, folder: nil, done: 1, total: 5), to: all)
        all = AppModel.applying(activity(.headers, done: 4, total: 10), to: all)
        #expect(all.count == 2)
        #expect(all["1-headers"]?.done == 4)
        all = AppModel.applying(activity(.headers, done: 10, total: 10), to: all)
        #expect(all.keys.sorted() == ["1-bodies"])
        all = AppModel.applying(activity(.bodies, folder: nil, done: 5, total: 5), to: all)
        #expect(all.isEmpty)
    }

    /// The core sends no end when a pass is cut: an activity it no longer
    /// speaks of ends after `syncStale`.
    @Test func aSilentActivityEnds() {
        let all = ["1-headers": activity(.headers, done: 4, total: 10), "2-bodies": activity(.bodies, folder: nil, done: 1, total: 9, account: 2, after: 20)]
        #expect(AppModel.pruned(all, now: t0 + .seconds(29)).count == 2)
        #expect(AppModel.pruned(all, now: t0 + .seconds(31)).keys.sorted() == ["2-bodies"])
        #expect(AppModel.pruned(all, now: t0 + .seconds(51)).isEmpty)
    }

    // MARK: The line of the status bar

    @Test func theLineSaysTheHeadersFirstThenWhatIsLeftOfTheBodies() throws {
        let folders = [try Self.folder(1, role: "inbox")]
        let line = { (a: [Activity], names: FolderNames) in
            AppModel.syncLine(a, folders: folders, names: names, severalAccounts: false)
        }
        #expect(line([], .english) == nil)
        let headers = activity(.headers, done: 300, total: 829)
        let bodies = activity(.bodies, folder: nil, done: 100, total: 829, after: 1)
        #expect(line([headers], .english) == L("Importing %@: %@ of %@", "Inbox", AppModel.count(300), AppModel.count(829)))
        // The folder as `[folder_names]` names it.
        #expect(line([headers], FolderNames(byRole: ["inbox": "Réception"]))?.contains("Réception") == true)
        // Headers and bodies together: the headers, even older.
        #expect(line([bodies, headers], .english) == line([headers], .english))
        #expect(line([bodies], .english) == L("Downloading mails: %@ left", AppModel.count(729)))
        // A folder not listed yet.
        #expect(line([activity(.headers, folder: 99, done: 5, total: 10)], .english) == L("Importing mails: %@ of %@", "5", "10"))
    }

    @Test func withSeveralAccountsTheAccountComesFirst() throws {
        let folders = [try Self.folder(1, role: "inbox"), try Self.folder(13, role: "inbox", account: 2, accountName: "pro")]
        let a = activity(.headers, folder: 13, done: 1, total: 2, account: 2)
        #expect(AppModel.syncLine([a], folders: folders, names: .english, severalAccounts: true)?.hasPrefix("pro · ") == true)
        #expect(AppModel.syncLine([a], folders: folders, names: .english, severalAccounts: false)?.hasPrefix("pro") == false)
    }

    /// The numbers follow the region: grouped as the reader writes them.
    @Test func theNumbersAreGroupedAsTheRegionWritesThem() {
        #expect(AppModel.count(44829, locale: Locale(identifier: "en_US")) == "44,829")
        // French groups with a narrow no-break space.
        let fr = AppModel.count(44829, locale: Locale(identifier: "fr_FR"))
        #expect(fr.filter(\.isNumber) == "44829" && fr.count == 6 && !fr.contains(","))
    }

    // MARK: Through the model

    /// The events reach the model as the stream gives them: transient,
    /// they reload nothing and take no place in the sequence.
    @Test func theModelFollowsTheEvents() {
        let model = AppModel()
        model.handle(Self.event(done: 0, total: 829))
        #expect(model.syncActivity["1-headers"]?.done == 0)
        #expect(model.syncLine?.contains("829") == true)
        model.handle(Self.event(done: 829, total: 829))
        #expect(model.syncActivity.isEmpty && model.syncLine == nil)
    }
}
