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

    /// The name of the file naming the chosen directory
    /// (`nm_api::paths::CHOICE_FILE`), in the default directory.
    static let choiceName = "data-dir"

    /// The variable, if set (and not empty).
    static var environment: String? {
        ProcessInfo.processInfo.environment["NICEMAILER_DATA_DIR"].flatMap { $0.isEmpty ? nil : $0 }
    }

    /// The directory in use.
    static var current: URL {
        URL(fileURLWithPath: resolve(env: environment, choice: readChoice(in: defaultURL), defaultPath: defaultURL.path))
    }

    /// The text of the choice file in `directory`, when only the current
    /// user can have written it — the rule of the core
    /// (`nm_api::paths::choice`, `architecture.md`, section 9): a regular
    /// file, never a symbolic link, owned by the user and writable by her
    /// alone, in a directory writable by her alone. Otherwise `nil`, as
    /// the core then serves the default mailbox: the app must look for the
    /// core where it is.
    ///
    /// The file is opened first (`O_NOFOLLOW`), then checked on the open
    /// descriptor (`fstat`), so that it cannot be swapped between the
    /// check and the read.
    static func readChoice(in directory: URL) -> String? {
        var dir = stat()
        // Swift note: the C functions of Darwin are called as they are;
        // `stat()` is the zeroed C struct they fill.
        guard lstat(directory.path, &dir) == 0, dir.st_mode & S_IFMT == S_IFDIR,
              untrusted(owner: dir.st_uid, mode: dir.st_mode) == nil else { return nil }
        let fd = open(directory.appendingPathComponent(choiceName).path, O_RDONLY | O_NOFOLLOW)
        guard fd >= 0 else { return nil }
        // `defer` runs when the function returns, whatever the path.
        defer { close(fd) }
        var st = stat()
        guard fstat(fd, &st) == 0, st.st_mode & S_IFMT == S_IFREG,
              untrusted(owner: st.st_uid, mode: st.st_mode) == nil,
              st.st_size <= maxChoiceBytes else { return nil }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: false)
        guard let data = try? handle.readToEnd() else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// The longest choice file read, as the core (`nm_api::paths`).
    static let maxChoiceBytes = 4096

    /// Why a file of `owner` and `mode` could have been written by someone
    /// else than `user` (the current one by default): owned by another, or
    /// writable by the group or the others. `nil`: only the user can.
    static func untrusted(owner: uid_t, mode: mode_t, user: uid_t = geteuid()) -> String? {
        if owner != user { return "owned by another user" }
        if mode & 0o022 != 0 { return "writable by the group or the others" }
        return nil
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
    ///
    /// The file is written `0600`, then moved into place: a file others
    /// could write would be ignored by the core and by `readChoice`
    /// (`String.write(atomically:)` gave `0644`, which was fine to read but
    /// is now written private, as the data it points to).
    static func choose(_ url: URL, in directory: URL = defaultURL) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let file = directory.appendingPathComponent(choiceName)
        if url.standardizedFileURL.path == directory.standardizedFileURL.path {
            try? fm.removeItem(at: file)
            return
        }
        let temporary = directory.appendingPathComponent(".\(choiceName).\(getpid())")
        let text = Data((url.standardizedFileURL.path + "\n").utf8)
        guard fm.createFile(atPath: temporary.path, contents: text, attributes: [.posixPermissions: 0o600]) else {
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: temporary.path])
        }
        // `rename` replaces the old file in one step: a reader sees the old
        // choice or the new one, never half of it.
        guard rename(temporary.path, file.path) == 0 else {
            try? fm.removeItem(at: temporary)
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: file.path])
        }
    }

    /// The name shown for a directory: its last component, "NiceMailer"
    /// for the default one.
    static func displayName(_ url: URL) -> String {
        url.lastPathComponent
    }
}
