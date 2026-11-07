import Foundation

/// Where the core keeps its data, and so where its socket is: the same
/// rule as the core and the CLI (`nm_api::paths::data_dir`,
/// `docs/app-bundle.md`):
///
/// 1. `NICEMAILER_DATA_DIR`, when set (development, tests): it always wins;
/// 2. the directory chosen in the app ("Change Mailbox…", or ⌥ held at
///    launch, like Photos' libraries), named in the file `data-dir` of the
///    default directory;
/// 3. the default directory, `~/Library/Application Support/NiceMailer`.
///
/// The choice is a file rather than a preference of the app because the
/// core reads it too: started by launchd, it gets no variable from the app.
enum DataDirectory {
    /// `~/Library/Application Support/NiceMailer`.
    static var defaultURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/NiceMailer")
    }

    /// The file naming the chosen directory (`nm_api::paths::CHOICE_FILE`).
    static var choiceFile: URL { defaultURL.appendingPathComponent("data-dir") }

    /// The variable, if set (and not empty).
    static var environment: String? {
        ProcessInfo.processInfo.environment["NICEMAILER_DATA_DIR"].flatMap { $0.isEmpty ? nil : $0 }
    }

    /// The directory in use.
    static var current: URL {
        URL(fileURLWithPath: resolve(env: environment, choice: try? String(contentsOf: choiceFile, encoding: .utf8),
                                     defaultPath: defaultURL.path))
    }

    /// The rule above, as a pure function: the variable, else the choice
    /// (an absolute path, spaces and line breaks around it ignored), else
    /// the default. An empty or relative choice is ignored.
    static func resolve(env: String?, choice: String?, defaultPath: String) -> String {
        if let env, !env.isEmpty { return env }
        if let c = choice?.trimmingCharacters(in: .whitespacesAndNewlines), c.hasPrefix("/") { return c }
        return defaultPath
    }

    /// Records `url` as the chosen directory; the default one removes the
    /// choice. The core uses it at its next start.
    static func choose(_ url: URL) throws {
        try FileManager.default.createDirectory(at: defaultURL, withIntermediateDirectories: true)
        if url.standardizedFileURL.path == defaultURL.standardizedFileURL.path {
            try? FileManager.default.removeItem(at: choiceFile)
        } else {
            try (url.standardizedFileURL.path + "\n").write(to: choiceFile, atomically: true, encoding: .utf8)
        }
    }

    /// The name shown for a directory: its last component, "NiceMailer"
    /// for the default one.
    static func displayName(_ url: URL) -> String {
        url.lastPathComponent
    }
}
