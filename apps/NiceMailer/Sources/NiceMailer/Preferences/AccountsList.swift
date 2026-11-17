import Foundation

/// The accounts of the Preferences, in display order, and their reordering
/// (`docs/accounts.md`, "In the app").
///
/// The order is the core's (`list_accounts`, by `position`): the one of
/// the account picker `@` and of the folder column. Moving an account
/// sends the whole new order with `set_account_order`, and the list shows
/// it at once; if the core refuses, the list is read again, so that it
/// never shows an order the core does not have.
@Observable
@MainActor
final class AccountsList {
    private(set) var items: [AccountInfo] = []
    private(set) var loaded = false
    private(set) var error: String?
    /// The row highlighted in the Preferences: an account, or the form of
    /// a new one.
    var selection: AccountsSelection?

    var selectedId: Int? {
        if case .account(let id) = selection { return id }
        return nil
    }

    private let client: any CoreCalling

    init(client: any CoreCalling) {
        self.client = client
    }

    func load() async {
        do {
            let r: AccountInfoList = try await client.call("list_accounts")
            items = r.items
            error = nil
        } catch {
            self.error = AccountSetup.message(error)
        }
        loaded = true
    }

    /// Moves the rows at `offsets` before the row `destination`, as a
    /// SwiftUI list's `onMove` gives them (a drag in the list).
    func move(fromOffsets offsets: IndexSet, toOffset destination: Int) async {
        let ids = Self.reordered(items.map(\.id), fromOffsets: offsets, toOffset: destination)
        await apply(ids)
    }

    /// One row up (`-1`) or down (`+1`): the buttons under the list, for
    /// the keyboard and for the UI scripts, which cannot drag.
    func move(_ id: Int, by step: Int) async {
        guard let i = items.firstIndex(where: { $0.id == id }) else { return }
        let j = i + step
        guard items.indices.contains(j) else { return }
        var ids = items.map(\.id)
        ids.swapAt(i, j)
        await apply(ids)
    }

    func canMove(_ id: Int, by step: Int) -> Bool {
        guard let i = items.firstIndex(where: { $0.id == id }) else { return false }
        return items.indices.contains(i + step)
    }

    private func apply(_ ids: [Int]) async {
        guard ids != items.map(\.id) else { return }
        let byId = Dictionary(uniqueKeysWithValues: items.map { ($0.id, $0) })
        items = ids.compactMap { byId[$0] }
        do {
            // Every account, in the new order: `set_account_order` puts the
            // ones named first, so naming them all fixes the whole order.
            let _: IgnoredResult = try await client.call("set_account_order", ["account_ids": ids])
            error = nil
        } catch {
            // Read again first: `load` clears the error it does not meet.
            let message = AccountSetup.message(error)
            await load()
            self.error = message
        }
    }

    /// The ids after a move of `offsets` to `destination`, by the rule of
    /// SwiftUI's `onMove`: `destination` is an index of the list before
    /// the move. Pure, tested apart.
    static func reordered(_ ids: [Int], fromOffsets offsets: IndexSet, toOffset destination: Int) -> [Int] {
        let moving = offsets.sorted().filter { ids.indices.contains($0) }.map { ids[$0] }
        let before = ids.prefix(max(0, min(destination, ids.count))).filter { !moving.contains($0) }
        let after = ids.dropFirst(max(0, min(destination, ids.count))).filter { !moving.contains($0) }
        return before + moving + after
    }
}

/// A result whose content is not read (`CommandResult`, say): only its
/// success counts. Swift note: decoding a type without properties accepts
/// any JSON object.
struct IgnoredResult: Decodable {}
