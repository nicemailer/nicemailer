import Foundation

/// JSON-RPC 2.0 client of the core, over a Unix socket, one JSON object per
/// line. It contains no business logic: the app only sends requests and
/// relays events (`architecture.md`, section 5).
///
/// How it works:
/// - `call` writes a request and suspends until the matching response
///   arrives. Each pending request is stored by id, with a *continuation*,
///   the object that resumes the suspended caller.
/// - A dedicated thread (`readLoop`) reads the socket line by line. A line
///   with an `id` is a response: it resumes the matching continuation. A
///   line without an `id` is an event: it is pushed into the `events` stream.
///
/// The socket is used through the POSIX C API (`socket`, `connect`, `read`,
/// `write`), which Swift can call directly. Foundation has no simple
/// high-level API for Unix sockets.
///
/// Swift notes:
/// - `final class`: a reference type that cannot be subclassed. A class
///   (not a struct) is needed because the client is shared and has a
///   lifetime (an open socket closed in `deinit`).
/// - `@unchecked Sendable` tells the compiler the object may be used from
///   several threads, and that we take responsibility for safety: shared
///   mutable state (`nextId`, `pending`, `closed`) is only touched while
///   holding `lock`.
final class CoreClient: @unchecked Sendable {
    static let protocolVersion = 1

    /// Error returned by the core in a response's `error` member.
    /// Conforming to `LocalizedError` provides the text shown in alerts:
    /// the sentence of its reason in the user's language (`CoreReason`).
    struct RPCError: LocalizedError {
        let code: Int
        /// The core's English sentence, for the logs and a generic error.
        let message: String
        /// The error's structured details (`data`), as text: its `reason`
        /// and the values of its sentence (a number written in digits, a
        /// list joined by ", ").
        var data: [String: String] = [:]
        /// The whole `data`, as JSON: for `maintenance`, a `CoreMaintenance`.
        var rawData: Data?
        /// Why it failed (`nm_api::reason`): `tag_exists`, `imap_auth_refused`…
        var reason: String? { data["reason"] }
        var errorDescription: String? { CoreReason.text(self) }

        /// A favorable decision on a sender suspected of spoofing waits for
        /// the user's confirmation (`nm_api::error_code`).
        static let confirmationRequired = 1005
        /// What was asked for does not exist (a deep link to a deleted mail).
        static let notFound = 1001
        /// The core is in maintenance (`CoreMaintenance`): an update, a
        /// migration of the database, or a database it cannot open.
        static let maintenance = 1003
    }

    /// Socket-level failures, in the app's language: the app shows them
    /// as they are (`ConnectionErrorView`).
    enum ConnectionError: LocalizedError {
        case socket(String)
        case closed
        var errorDescription: String? {
            switch self {
            case .socket(let m): return m
            case .closed: return L("The connection to the core was closed.")
            }
        }
    }

    /// Event pushed by the core. `Sendable` because events cross from the
    /// reader thread to the main actor.
    ///
    /// `seq` is `nil` for a transient event (`SyncProgress`,
    /// `evenements.md`): it has no place in the sequence and is never
    /// replayed, so it must not be taken for a gap.
    struct Event: Sendable {
        let method: String
        let epoch: String
        let seq: Int?
        let params: [String: JSONValue]
    }

    /// Stream of events, consumed with `for await event in client.events`.
    /// An `AsyncStream` is fed through its *continuation*.
    let events: AsyncStream<Event>
    private let eventContinuation: AsyncStream<Event>.Continuation
    /// File descriptor of the socket (a small integer identifying it to the OS).
    private let fd: Int32
    private let lock = NSLock()
    private var nextId = 1
    /// Requests awaiting their response, by id.
    private var pending: [Int: CheckedContinuation<Data, Error>] = [:]
    private var closed = false
    /// The maintenance that made the core close this connection, if any:
    /// its last line, without an id (`mise-a-jour.md`, section 2).
    private var closingMaintenance: CoreMaintenance?

    /// Why the core closed the connection, when a maintenance did.
    var closedForMaintenance: CoreMaintenance? {
        lock.lock()
        defer { lock.unlock() }
        return closingMaintenance
    }

    /// `<data dir>/core.sock`, with the same data directory as the core
    /// (`DataDirectory`: `NICEMAILER_DATA_DIR`, else the mailbox chosen in
    /// the app, else `~/Library/Application Support/NiceMailer`).
    static var defaultSocketPath: String {
        DataDirectory.current.appendingPathComponent("core.sock").path
    }

    /// Connects to the socket and starts the reader thread. `throws`: the
    /// initializer fails if the core is not reachable.
    init(socketPath: String = CoreClient.defaultSocketPath) throws {
        let sock = socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else { throw ConnectionError.socket(L("The socket to the core could not be created.")) }
        // `sockaddr_un` is the C struct holding a Unix socket address. Its
        // `sun_path` field is a fixed-size C char array (104 bytes on macOS),
        // which is why socket paths must stay short.
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path)
        // `guard ... else` exits early when the condition is false; the code
        // after it can rely on the condition.
        guard socketPath.utf8.count < capacity else {
            close(sock)
            throw ConnectionError.socket(L("The socket path is too long: %@", socketPath))
        }
        // Swift imports the C array as a tuple, so it is reinterpreted as a
        // pointer to `CChar` to copy the path into it. These `withUnsafe...`
        // calls give temporary raw pointers, valid only inside the closure.
        _ = withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: capacity) { strncpy($0, socketPath, capacity) }
        }
        // `connect` expects a generic `sockaddr*`, hence the second rebind.
        let result = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(sock, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard result == 0 else {
            close(sock)
            throw ConnectionError.socket(L("The core cannot be reached at %@. Start nicemailer-core.", socketPath))
        }
        // Stored properties are assigned only now: Swift forbids using `self`
        // (even in a closure) before every stored property is initialized,
        // which is why the socket is built in the local `sock` first.
        fd = sock
        // Keep at most the 1 000 newest events if the app falls behind.
        (events, eventContinuation) = AsyncStream.makeStream(bufferingPolicy: .bufferingNewest(1000))
        // `[weak self]` avoids a retain cycle: the thread must not keep the
        // client alive on its own.
        let thread = Thread { [weak self] in self?.readLoop() }
        thread.name = "nicemailer.core.reader"
        thread.start()
    }

    /// Called when the last reference to the client goes away.
    deinit { disconnect() }

    /// Closes the connection, fails every pending request and ends the event
    /// stream. Safe to call several times.
    func disconnect() {
        lock.lock()
        let wasClosed = closed
        closed = true
        let waiting = pending
        pending = [:]
        lock.unlock()
        guard !wasClosed else { return }
        // `shutdown` wakes up the reader thread blocked in `read`.
        shutdown(fd, SHUT_RDWR)
        close(fd)
        // Continuations are resumed outside the lock, so the resumed code
        // cannot deadlock by calling back into the client.
        for (_, c) in waiting { c.resume(throwing: ConnectionError.closed) }
        eventContinuation.finish()
    }

    // MARK: - Requests

    /// Sends a request and decodes its result as `R`.
    ///
    /// Swift notes: `<R: Decodable>` is a generic parameter; the caller picks
    /// `R` through the type of the variable receiving the result, e.g.
    /// `let list: FolderList = try await client.call("list_folders")`.
    /// `async throws`: the call suspends (without blocking the thread) and
    /// may throw; callers write `try await`.
    func call<R: Decodable>(_ method: String, _ params: [String: Any]) async throws -> R {
        let data = try await raw(method, params)
        return try Self.decoder.decode(R.self, from: data)
    }

    /// Decoder shared by all calls, converting snake_case keys to camelCase.
    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    /// Sends a request and returns the raw JSON of its `result`.
    ///
    /// `withCheckedThrowingContinuation` suspends the caller and hands us a
    /// continuation `c`. The caller resumes when someone calls
    /// `c.resume(returning:)` or `c.resume(throwing:)`, here the reader
    /// thread when the response arrives. "Checked" means Swift verifies at
    /// runtime that it is resumed exactly once.
    private func raw(_ method: String, _ params: [String: Any]) async throws -> Data {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data, Error>) in
            lock.lock()
            if closed {
                lock.unlock()
                c.resume(throwing: ConnectionError.closed)
                return
            }
            let id = nextId
            nextId += 1
            // Registered *before* writing, so the response cannot arrive
            // before we are ready to receive it.
            pending[id] = c
            lock.unlock()
            do {
                let body: [String: Any] = ["jsonrpc": "2.0", "id": id, "method": method, "params": params]
                var line = try JSONSerialization.data(withJSONObject: body)
                line.append(0x0A) // "\n": one request per line
                try write(line)
            } catch {
                // Writing failed: remove the request and fail it, unless
                // `disconnect` already did (`removeValue` then returns nil).
                lock.lock()
                let c = pending.removeValue(forKey: id)
                lock.unlock()
                c?.resume(throwing: error)
            }
        }
    }

    /// Writes all the bytes: `write` may write fewer bytes than asked, so it
    /// is called in a loop until everything is sent.
    private func write(_ data: Data) throws {
        try data.withUnsafeBytes { (buf: UnsafeRawBufferPointer) in
            var offset = 0
            while offset < buf.count {
                // `Darwin.write` is the C function, named explicitly to avoid
                // confusion with this method, also called `write`.
                let n = Darwin.write(fd, buf.baseAddress! + offset, buf.count - offset)
                if n <= 0 { throw ConnectionError.closed }
                offset += n
            }
        }
    }

    // MARK: - Reading

    /// Runs on the reader thread: reads chunks from the socket, splits them
    /// into lines and handles each line. A chunk may contain several lines,
    /// or only part of one, so bytes accumulate in `buffer` until a newline.
    private func readLoop() {
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
            // Blocks until data arrives. 0 means the core closed the
            // connection, a negative value an error.
            let n = read(fd, &chunk, chunk.count)
            if n <= 0 { break }
            buffer.append(contentsOf: chunk[0..<n])
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                if !line.isEmpty { handle(Data(line)) }
            }
        }
        disconnect()
    }

    /// Dispatches one line: response to a pending request, or event.
    private func handle(_ line: Data) {
        // `try?` turns an error into nil; `as? [String: Any]` checks that the
        // JSON is an object. Invalid lines are ignored.
        guard let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { return }
        // An error without an id (`"id": null`) answers no request: the
        // core says why it closes the connection.
        if obj["id"] is NSNull, let err = obj["error"] as? [String: Any] {
            let error = Self.rpcError(err)
            lock.lock()
            closingMaintenance = CoreMaintenance(error: error)
            lock.unlock()
            return
        }
        if let id = obj["id"] as? Int {
            lock.lock()
            let c = pending.removeValue(forKey: id)
            lock.unlock()
            guard let c else { return }
            if let err = obj["error"] as? [String: Any] {
                c.resume(throwing: Self.rpcError(err))
            } else {
                // Re-encode `result` alone, so that `call` can decode it with
                // `JSONDecoder` into the expected struct. `.fragmentsAllowed`
                // accepts a result that is not an object (e.g. "pong").
                let result = obj["result"] ?? NSNull()
                let data = (try? JSONSerialization.data(withJSONObject: result, options: .fragmentsAllowed)) ?? Data("null".utf8)
                c.resume(returning: data)
            }
        } else if let method = obj["method"] as? String {
            let params = (obj["params"] as? [String: Any]) ?? [:]
            eventContinuation.yield(Event(
                method: method,
                epoch: params["epoch"] as? String ?? "",
                seq: params["seq"] as? Int,
                params: params.compactMapValues(JSONValue.init)
            ))
        }
    }
}

extension CoreClient {
    /// The error of a response's `error` member: its code, message, the
    /// values of `data` as text, and the whole `data` as JSON.
    static func rpcError(_ err: [String: Any]) -> RPCError {
        let data = err["data"]
        return RPCError(code: err["code"] as? Int ?? 0, message: err["message"] as? String ?? "",
                        data: (data as? [String: Any])?.compactMapValues(text) ?? [:],
                        rawData: data.flatMap { try? JSONSerialization.data(withJSONObject: $0) })
    }

    /// A value of `data` as text: a string as it is, a number in digits, a
    /// list of strings joined by ", "; nil for anything else.
    static func text(_ value: Any) -> String? {
        switch value {
        case let s as String: s
        case let n as NSNumber: n.stringValue
        case let list as [Any]: list.compactMap { $0 as? String }.joined(separator: ", ")
        default: nil
        }
    }
}

/// Simplified JSON value, for event params. `[String: Any]` is not
/// `Sendable` (`Any` could be anything), so params are converted into this
/// small enum, which only keeps what the app uses.
///
/// Swift note: enum cases can carry *associated values*, like Rust enums.
enum JSONValue: Sendable, Equatable {
    case bool(Bool)
    case int(Int)
    case string(String)
    case ints([Int])
    case strings([String])
    /// An object, its values as text (`CoreClient.text`): the `why` of an
    /// event, a reason and its values.
    case object([String: String])
    case other

    /// Failable initializer (`init?`): may return nil. `case let i as Int`
    /// matches when the value can be cast to `Int`.
    init?(_ any: Any) {
        // A JSON boolean arrives as an `NSNumber`, which `as Int` would
        // also accept (true as 1): told apart by its Core Foundation type
        // first.
        if let n = any as? NSNumber, CFGetTypeID(n) == CFBooleanGetTypeID() {
            self = .bool(n.boolValue)
            return
        }
        switch any {
        case let i as Int: self = .int(i)
        case let s as String: self = .string(s)
        case let a as [Int]: self = .ints(a)
        case let a as [String]: self = .strings(a)
        case let d as [String: Any]: self = .object(d.compactMapValues(CoreClient.text))
        default: self = .other
        }
    }

    /// The boolean, if this value is one.
    var bool: Bool? { if case .bool(let b) = self { return b } else { return nil } }
    /// The integer, if this value is one.
    var int: Int? { if case .int(let i) = self { return i } else { return nil } }
    /// The integer list, or an empty list.
    var ints: [Int] { if case .ints(let a) = self { return a } else { return [] } }
    /// The text, if this value is one.
    var string: String? { if case .string(let s) = self { return s } else { return nil } }
    /// The list of texts, or an empty list.
    var strings: [String] { if case .strings(let a) = self { return a } else { return [] } }
    /// The object's values as text, if this value is one.
    var object: [String: String]? { if case .object(let d) = self { return d } else { return nil } }
}
