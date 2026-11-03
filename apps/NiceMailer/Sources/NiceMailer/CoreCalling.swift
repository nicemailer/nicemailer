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
protocol CoreCalling: AnyObject {
    /// Sends `method` with `params` and decodes the result as `R`.
    func call<R: Decodable>(_ method: String, _ params: [String: Any]) async throws -> R
}

extension CoreCalling {
    /// Same, without parameters.
    func call<R: Decodable>(_ method: String) async throws -> R {
        try await call(method, [:])
    }
}

extension CoreClient: CoreCalling {}
