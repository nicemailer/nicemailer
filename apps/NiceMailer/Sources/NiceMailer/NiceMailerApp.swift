import AppKit
import SwiftUI

/// Entry point of the app.
///
/// SwiftUI notes:
/// - `@main` marks the type whose `main` the system calls at launch; for an
///   `App`, SwiftUI provides it.
/// - `@NSApplicationDelegateAdaptor` plugs a classic AppKit delegate into a
///   SwiftUI app, for the few things SwiftUI does not cover.
/// - `@State` owns the model: it is created once and survives redraws.
@main
struct NiceMailerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var model = AppModel()

    /// A `Scene` describes the app's windows and menus.
    var body: some Scene {
        WindowGroup("NiceMailer") {
            ContentView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 560)
                // `.task` starts async work when the view appears.
                .task { await model.connect() }
        }
        // Adds a "Recharger" item (⌘R) to the View menu.
        .commands {
            CommandGroup(after: .toolbar) {
                Button("Recharger") { Task { await model.reloadAll() } }
                    .keyboardShortcut("r", modifiers: .command)
            }
        }
    }
}

/// AppKit delegate, for launch-time setup.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Launched by `swift run`, the app is a bare executable without an
        // .app bundle, so macOS treats it as a background tool. It must be
        // declared a regular app to get a Dock icon and a menu bar, and
        // activated to take the keyboard focus.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Quit when the window is closed (macOS apps otherwise keep running).
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
