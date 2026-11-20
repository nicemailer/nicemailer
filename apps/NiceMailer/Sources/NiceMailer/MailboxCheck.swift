import Foundation

// What the app knows of a mailbox folder before it uses it, and why a core
// could not run on one (`docs/app-bundle.md`, "Choosing the mailbox" and
// "When the core cannot start").
//
// The app never opens a database itself (`docs/migrations.md`): it asks the
// CLI it carries, `nicemailer --data-dir <folder> mailbox check --json`,
// which reads it without changing it, and `nicemailer --data-dir <folder>
// mailbox upgrade`, which migrates it. The folder must be passed with
// `--data-dir`: without it the CLI works on the mailbox in use, not on the
// one being chosen. A core that stops on an error writes why to
// `core-error.json` of its folder, which the app shows instead of a plain
// "unreachable".

/// What `nicemailer mailbox check --json` prints (`nm_store::inspect`).
struct MailboxReport: Decodable, Equatable {
    enum State: String, Decodable {
        case new, current, upgradable, outdated, unreadable
        case tooNew = "too_new"
    }

    let state: State
    /// `upgradable`: the schema version of the database, and this one's.
    var from: Int?
    var to: Int?
    /// `outdated`: what the database lacks.
    var missing: String?
    /// `too_new`: the database's version, and the one this NiceMailer reads.
    var found: Int?
    var expected: Int?
    /// `unreadable`: why.
    var message: String?
    /// A core holds the folder (it runs on it).
    var inUse: Bool = false

    /// What the app does with the folder: use it, ask before upgrading it,
    /// or refuse it and keep the current mailbox. A pure function, so the
    /// rule is tested without any file.
    var decision: MailboxDecision {
        switch state {
        case .new, .current:
            return .use
        case .upgradable:
            return .askUpgrade(from: from ?? 0, to: to ?? 0)
        case .outdated:
            return .refuse(L("This mailbox was created by an early development version of NiceMailer (%@ missing): it cannot be upgraded. The current mailbox is kept.", missing ?? "?"))
        case .tooNew:
            return .refuse(L("This mailbox was written by a newer version of NiceMailer (data format %lld; this one reads %lld): update NiceMailer to open it. The current mailbox is kept.", found ?? 0, expected ?? 0))
        case .unreadable:
            return .refuse(L("The database of this folder cannot be read (%@). The current mailbox is kept.", message ?? "?"))
        }
    }
}

enum MailboxDecision: Equatable {
    case use
    case askUpgrade(from: Int, to: Int)
    case refuse(String)
}

/// A mailbox waiting for the user's answer: upgrade it, or keep the
/// current one.
struct PendingUpgrade: Equatable {
    let url: URL
    let from: Int
    let to: Int
}

/// Checks and upgrades mailbox folders. A protocol so that the tests give
/// answers of their own, without a CLI.
///
/// Swift note: `Sendable` says a value may cross from one concurrent task
/// to another safely; the calls below run off the main thread.
protocol MailboxTool: Sendable {
    func check(_ dir: URL) async throws -> MailboxReport
    func upgrade(_ dir: URL) async throws
}

/// An error of the CLI: what it wrote on its standard error.
struct MailboxToolError: LocalizedError, Equatable {
    let message: String
    var errorDescription: String? { message }
}

/// The CLI the app carries: `Contents/Helpers/nicemailer` of NiceMailer.app
/// (`make bundle`), or the one `NICEMAILER_CLI` names (development, UI
/// tests). Without either (`swift run`), there is no check: the folder is
/// used as before.
struct CLIMailboxTool: MailboxTool {
    let executable: URL

    static func find() -> CLIMailboxTool? {
        let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/nicemailer")
        if FileManager.default.isExecutableFile(atPath: bundled.path) { return CLIMailboxTool(executable: bundled) }
        if let path = ProcessInfo.processInfo.environment["NICEMAILER_CLI"], !path.isEmpty,
           FileManager.default.isExecutableFile(atPath: path) {
            return CLIMailboxTool(executable: URL(fileURLWithPath: path))
        }
        return nil
    }

    func check(_ dir: URL) async throws -> MailboxReport {
        let out = try await run(Self.checkArguments(dir))
        return try CoreClient.decoder.decode(MailboxReport.self, from: out)
    }

    func upgrade(_ dir: URL) async throws {
        _ = try await run(Self.upgradeArguments(dir))
    }

    /// The arguments of `check` and `upgrade` on `dir`, as pure functions
    /// so a test can see that the folder goes through `--data-dir`.
    static func checkArguments(_ dir: URL) -> [String] {
        ["--data-dir", dir.path, "mailbox", "check", "--json"]
    }

    static func upgradeArguments(_ dir: URL) -> [String] {
        ["--data-dir", dir.path, "mailbox", "upgrade"]
    }

    /// Runs the CLI with `arguments`; its standard output, or its error.
    ///
    /// Swift note: `withCheckedThrowingContinuation` turns a callback into
    /// an `await`: the closure starts the work, and `resume` hands back its
    /// result once, from whichever thread. The process runs on a queue of
    /// its own, not on the threads of Swift's concurrency, which an upgrade
    /// of a large mailbox (a minute) must not hold.
    private func run(_ arguments: [String]) async throws -> Data {
        let executable = self.executable
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let p = Process()
                p.executableURL = executable
                p.arguments = arguments
                let out = Pipe(), err = Pipe()
                p.standardOutput = out
                p.standardError = err
                do {
                    try p.run()
                } catch {
                    continuation.resume(throwing: error)
                    return
                }
                // Read before waiting: a full pipe would block the CLI, and
                // waiting first would then never end.
                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                p.waitUntilExit()
                if p.terminationStatus == 0 {
                    continuation.resume(returning: stdout)
                } else {
                    continuation.resume(throwing: MailboxToolError(message: Self.errorMessage(stderr)))
                }
            }
        }
    }

    /// The message of the CLI's error output, without `anyhow`'s "Error: ".
    static func errorMessage(_ data: Data) -> String {
        let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.hasPrefix("Error: ") ? String(text.dropFirst(7)) : text
    }
}

/// Why the core could not run on a folder: `core-error.json`, written by
/// the core (`nm_api::StartupError`), removed when one starts there.
struct StartupErrorReport: Decodable, Equatable {
    let kind: String
    let message: String
    var missing: String?
    var found: Int?
    var expected: Int?

    /// The report left in `dir`, if any.
    static func read(in dir: URL) -> StartupErrorReport? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("core-error.json")) else { return nil }
        return try? CoreClient.decoder.decode(StartupErrorReport.self, from: data)
    }

    /// What the screen without a core says, in the user's language. The
    /// core's own message (French) is shown only for the other errors,
    /// which have no translation.
    var userMessage: String {
        switch kind {
        case "outdated":
            return L("The core cannot open this mailbox: it was created by an early development version of NiceMailer (%@ missing) and cannot be upgraded. Choose another mailbox.", missing ?? "?")
        case "too_new":
            return L("The core cannot open this mailbox: it was written by a newer version of NiceMailer (data format %lld; this one reads %lld). Update NiceMailer, or choose another mailbox.", found ?? 0, expected ?? 0)
        default:
            return L("The core stopped on this error: %@", message)
        }
    }
}
