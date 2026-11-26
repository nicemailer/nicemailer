import Foundation

/// The environment variables that drive the app for its tests
/// (`NICEMAILER_UI_SCRIPT`, `NICEMAILER_UI_OUT`, `NICEMAILER_CLI`,
/// `NICEMAILER_MCP`, `NICEMAILER_EXPORT_DIR`), read **in a debug build
/// only**.
///
/// Why: the core recognizes the app by its code signature and gives it
/// rights no other client has (`architecture.md`, section 9). Any program
/// of the user can launch the signed app with variables of its choosing:
/// if a release build obeyed `NICEMAILER_UI_SCRIPT`, that program could
/// play keys and clicks in it — confirm a send, change a protected setting
/// — without the Accessibility permission macOS asks for synthetic events.
/// The UI tests run a debug build (`scripts/ui-test.sh`, `swift build`), so
/// they lose nothing. `NICEMAILER_DATA_DIR` is not here: it is a documented
/// setting of every build (`docs/app-bundle.md`), and the app verifies the
/// core it finds there (`CoreSignature`).
enum DevEnvironment {
    /// Whether this is a debug build, fixed at compile time.
    /// Swift note: `#if DEBUG` keeps or drops code when compiling; `swift
    /// build` defines `DEBUG`, `swift build -c release` does not.
    static let isDebugBuild: Bool = {
        #if DEBUG
        return true
        #else
        return false
        #endif
    }()

    /// The value of the variable `name`, when this is a debug build and it
    /// is set and not empty.
    static func value(_ name: String) -> String? {
        value(name, in: ProcessInfo.processInfo.environment, debug: isDebugBuild)
    }

    /// The rule of `value`, as a pure function (tested).
    static func value(_ name: String, in environment: [String: String], debug: Bool) -> String? {
        guard debug, let v = environment[name], !v.isEmpty else { return nil }
        return v
    }
}
