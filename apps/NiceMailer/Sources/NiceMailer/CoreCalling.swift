import Foundation

/// What the app needs from the core: send a request, get a decoded result.
///
/// The real implementation is `CoreClient` (Unix socket). Tests use a fake
/// that records the requests and answers with canned JSON, so the model and
/// the geek controller can be tested without a running core
/// (`Tests/NiceMailerTests`).
///
/// Swift note: a *protocol* is an interface (like a Rust trait). Code typed
/// against `any CoreCalling` accepts any value conforming to it; `any` marks
/// an existential, a box holding some conforming type chosen at runtime.
/// An event of the core, for a waiter of [`CoreCalling.waitEvent`]: its
/// method and its params (a `CoreClient.Event` without the sequence
/// bookkeeping of the app's own stream).
struct CoreEvent: Sendable {
    let method: String
    let params: [String: JSONValue]
}

/// The waiters of `waitEvent`, shared by `CoreClient` and the fake core of
/// the tests so that both hand the events out the same way.
enum EventWaiters {
    /// Removes from `waiters` those waiting for `method`, and returns them
    /// to be resumed; the others keep waiting for their own event.
    ///
    /// Each waiter is resumed exactly once. Swift note: a
    /// `CheckedContinuation` resumed twice is a fatal error (the app
    /// stops), and one never resumed leaves its task suspended forever. A
    /// waiter left in the list after being resumed, or dropped without
    /// being resumed, is therefore a crash or a sign-in that never ends.
    /// Generic over the continuation (`C`) so that it is tested on plain
    /// values.
    static func take<C>(_ waiters: inout [(method: String, continuation: C)], for method: String) -> [C] {
        let taken = waiters.filter { $0.method == method }.map(\.continuation)
        waiters.removeAll { $0.method == method }
        return taken
    }
}

protocol CoreCalling: AnyObject {
    /// Sends `method` with `params` and decodes the result as `R`.
    func call<R: Decodable>(_ method: String, _ params: [String: Any]) async throws -> R
    /// The next event of the core named `method`, or `nil` when the
    /// connection ends first — the OAuth2 sign-in waits for
    /// `OAuthFinished` this way (`specifications/oauth2.md`, section 8).
    /// The app reads its single event stream in one place
    /// (`AppModel.listen`), so the client fans the events out to the
    /// waiters; a fake core in tests pushes them (`FakeCore.pushEvent`).
    func waitEvent(_ method: String) async -> CoreEvent?
}

extension CoreCalling {
    /// Same, without parameters.
    func call<R: Decodable>(_ method: String) async throws -> R {
        try await call(method, [:])
    }
}

extension CoreClient: CoreCalling {
    /// Registers a waiter, resumed by `notifyWaiters` on the next event of
    /// `method`, with `nil` when the connection closes first.
    func waitEvent(_ method: String) async -> CoreEvent? {
        await withCheckedContinuation { cont in
            lock.lock()
            eventWaiters.append((method, cont))
            lock.unlock()
        }
    }
}
