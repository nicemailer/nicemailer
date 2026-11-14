import Foundation

/// Preferences of their own for the tests, which leave nothing behind.
///
/// Each test wants empty preferences that no other test sees, as tests run
/// in parallel. A suite named `NiceMailerTests-<UUID>` gives that, but
/// macOS keeps it for ever as `~/Library/Preferences/<name>.plist`: one
/// file per test and per run, thousands in `defaults domains`. Removing the
/// domain at the end is not enough either: `removePersistentDomain` leaves
/// the file, emptied, and cfprefsd (the daemon that writes preferences,
/// later and on its own) writes it again if it is deleted.
///
/// A suite name that is an absolute path is a file at that path (plus
/// `.plist`), not a domain of `~/Library/Preferences`. So every suite of a
/// test run lives in one temporary folder, removed when the process exits.
/// If a run crashes, the folder stays in the temporary directory, which
/// macOS purges.
enum TestDefaults {
    /// A new, empty `UserDefaults`, distinct from every other.
    static func make() -> UserDefaults {
        // Swift note: `UserDefaults(suiteName:)` returns an optional (nil
        // only for the app's own bundle identifier); `!` unwraps it, and
        // would stop the test run loudly if it ever were nil.
        UserDefaults(suiteName: newPath())!
    }

    /// A new suite name: a path in this run's folder, without `.plist`.
    static func newPath() -> String {
        directory.appendingPathComponent(UUID().uuidString).path
    }

    /// The folder of this run's suites, created on first use.
    ///
    /// Swift note: a `static let` is initialized lazily, the first time it
    /// is read, and only once even when several tests read it at the same
    /// time: no lock needed.
    private static let directory: URL = {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("NiceMailerTests-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // Swift note: `atexit` takes a C function, so the closure cannot
        // capture anything; it reads `directory` again, already set by then.
        atexit { try? FileManager.default.removeItem(at: TestDefaults.directory) }
        return dir
    }()
}
