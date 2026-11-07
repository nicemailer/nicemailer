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
            // The title bar, hidden or shown as `config.toml` says. Hidden,
            // the content goes up to the top edge: SwiftUI otherwise keeps
            // the height of the bar as a safe area, an empty white strip.
            .ignoresSafeArea(.container, edges: model.titleBarHidden ? .top : [])
            .background(WindowChrome(hidden: model.titleBarHidden,
                                     title: model.mailboxName == "NiceMailer" ? "NiceMailer" : "NiceMailer — \(model.mailboxName)"))
            .frame(minWidth: 700, minHeight: 420)
            // A deep link `nicemailer://m/<id>` (`DeepLink`), clicked in a
            // note: macOS gives it to the app (its bundle declares the
            // scheme, `make bundle`), which opens the mail.
            .onOpenURL { url in
                NSApp.activate(ignoringOtherApps: true)
                model.openDeepLink(url)
            }
            // Swift note: by default a `WindowGroup` opens a new window for
            // each URL it receives; this sends every URL to the window
            // already open instead.
            .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
            // Back to the app without a core (stopped by the menu, the app
            // reopened before it had quit): it is started again at once.
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                guard !UIScript.isActive, !model.coreStoppedByUser, case .failed = model.status,
                      model.agentState != .notBundled else { return }
                Task { await model.connect() }
            }
            // `.task` starts async work when the view appears.
            .task {
                guard !UIScript.isActive else { return }
                // The geek theme opens on the unified view "all".
                model.startUnified = theme == .geek
                // ⌥ held at launch: choose the mailbox first, like Photos.
                if AppDelegate.optionAtLaunch, let url = MailboxChooser.choose() {
                    await model.changeMailbox(to: url)
                    return
                }
                await model.connect()
            }
        }
        .commands {
            // The mailbox and the core (`docs/app-bundle.md`), in the app's
            // menu, after "About".
            CommandGroup(after: .appInfo) {
                Button(L("Change Mailbox…")) {
                    if let url = MailboxChooser.choose() { Task { await model.changeMailbox(to: url) } }
                }
                Button(L("Stop the Core")) { model.stopCore() }
            }
            // Adds a "Reload" item (⌘R) to the View menu.
            CommandGroup(after: .toolbar) {
                Button(L("Reload")) { Task { await model.reloadAll() } }
                    .keyboardShortcut("r", modifiers: .command)
                Divider()
                // Text size of the geek theme, like Zed (`TextSize`). In the
                // geek theme the keys are handled by its key monitor first,
                // which also accepts ⌘= for keyboards where `+` needs Shift.
                Button(L("Bigger Text")) { TextSize.shared.bigger() }
                    .keyboardShortcut("+", modifiers: .command)
                Button(L("Smaller Text")) { TextSize.shared.smaller() }
                    .keyboardShortcut("-", modifiers: .command)
                Button(L("Actual Size")) { TextSize.shared.reset() }
                    .keyboardShortcut("0", modifiers: .command)
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
    /// ⌥ was held when the app opened: the mailbox is chosen first.
    @MainActor static var optionAtLaunch = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        Fonts.register()
        AppDelegate.optionAtLaunch = NSEvent.modifierFlags.contains(.option)
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

/// The panel that chooses a mailbox: a folder holding the data of NiceMailer
/// (`nicemailer.db`), or an empty one for a new mailbox.
@MainActor
enum MailboxChooser {
    static func choose() -> URL? {
        let panel = NSOpenPanel()
        panel.title = L("Choose a Mailbox")
        panel.message = L("A folder of NiceMailer data (with nicemailer.db), or an empty folder for a new mailbox")
        panel.prompt = L("Use This Mailbox")
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = DataDirectory.current.deletingLastPathComponent()
        NSApp.activate(ignoringOtherApps: true)
        return panel.runModal() == .OK ? panel.url : nil
    }
}
