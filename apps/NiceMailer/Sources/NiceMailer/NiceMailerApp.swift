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
    /// Current theme (`ui.md`). Swift note: `@AppStorage` is a property
    /// stored in UserDefaults; changing it redraws the views that read it.
    @AppStorage(UITheme.defaultsKey, store: AppDefaults.store) private var theme: UITheme = .geek

    /// A `Scene` describes the app's windows and menus.
    var body: some Scene {
        WindowGroup("NiceMailer") {
            Group {
                if UIScript.isActive {
                    // UI script mode builds its own window (see AppDelegate).
                    EmptyView()
                } else {
                    switch theme {
                    case .geek: GeekRootView()
                    // Design does not exist yet: it shows the classic interface.
                    case .classic, .design: ContentView()
                    }
                }
            }
            .environment(model)
            .frame(minWidth: 700, minHeight: 420)
            // `.task` starts async work when the view appears.
            .task { if !UIScript.isActive { await model.connect() } }
        }
        .commands {
            // Adds a "Reload" item (⌘R) to the View menu.
            CommandGroup(after: .toolbar) {
                Button(L("Reload")) { Task { await model.reloadAll() } }
                    .keyboardShortcut("r", modifiers: .command)
            }
            // Theme menu, during development; it will become a setting.
            CommandMenu(L("Theme")) {
                Picker(L("Theme"), selection: $theme) {
                    ForEach(UITheme.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                .pickerStyle(.inline)
            }
        }
    }
}

/// AppKit delegate, for launch-time setup.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        Fonts.register()
        // UI script mode (automated tests): never take the keyboard or the
        // screen from the user. `.accessory` = no Dock icon, not activated.
        if UIScript.isActive {
            NSApp.setActivationPolicy(.accessory)
            UIScript.openWindow()
            return
        }
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
