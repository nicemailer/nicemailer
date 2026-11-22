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
            Group {
                if UIScript.isActive {
                    // UI script mode builds its own window (see AppDelegate).
                    EmptyView()
                } else {
                    // Always the geek theme (`ui.md`): a theme saved by the
                    // former Theme menu is ignored, otherwise someone left on
                    // Classic would have no way back.
                    RootView()
                }
            }
            .environment(model)
            // The title bar, hidden or shown as `config.toml` says. Hidden,
            // the content goes up to the top edge: SwiftUI otherwise keeps
            // the height of the bar as a safe area, an empty white strip.
            .ignoresSafeArea(.container, edges: model.titleBarMode == .shown ? [] : .top)
            .background(WindowChrome(mode: model.titleBarMode,
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
            // Back to the app without a core (`nicemailer core stop`, the app
            // reopened before it had quit): it is started again at once.
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                guard !UIScript.isActive, case .failed = model.status,
                      model.agentState != .notBundled else { return }
                Task { await model.connect() }
            }
            // `.task` starts async work when the view appears.
            .task {
                guard !UIScript.isActive else { return }
                // Sparkle, in the background (`Updater`, `docs/updates.md`).
                Updater.shared.start(model: model)
                // The geek theme opens on the unified view "all".
                model.startUnified = true
                // ⌥ held at launch: choose the mailbox first, like Photos.
                if AppDelegate.optionAtLaunch, let url = MailboxChooser.choose() {
                    await model.changeMailbox(to: url)
                    return
                }
                // The app moved since Claude Desktop was connected: its
                // entry follows (`AgentSetup`), in the background.
                Task { await model.repairClaudeDesktop() }
                await model.connect()
            }
        }
        .commands {
            // No File > New Window (⌘N): every window would share the one
            // model above, so a second window only mirrored the first and
            // reconnected to the core (`SingleWindow`, `ui.md`,
            // "Fenêtres"). Swift note: `replacing:` with an empty body
            // removes the standard items of that group.
            CommandGroup(replacing: .newItem) {}
            // The updates, the mailbox and the AI agents, in the app's menu,
            // after "About". The core is stopped and started from the CLI
            // (`nicemailer core`, `docs/app-bundle.md`).
            CommandGroup(after: .appInfo) {
                // Sparkle's check, at once (`Updater`); disabled in a build
                // without updates (`swift run`, no key).
                Button(L("Check for Updates…")) { Updater.shared.checkForUpdates() }
                    .disabled(!Updater.shared.canCheck)
                Divider()
                Button(L("Change Mailbox…")) {
                    if let url = MailboxChooser.choose() { Task { await model.changeMailbox(to: url) } }
                }
                // AI agents reading the mailbox through `nicemailer-mcp`
                // (`AgentSetup`, `docs/mcp.md`); disabled without the
                // helper (`swift run`).
                Menu(L("AI Agents")) {
                    Button(L("Connect to Claude Desktop")) { Task { await model.connectClaudeDesktop() } }
                    Button(L("Copy Command for Claude Code")) { model.copyClaudeCodeCommand() }
                    Button(L("Copy MCP Configuration")) { model.copyMCPConfiguration() }
                }
                .disabled(AgentSetup.find() == nil)
            }
            // Adds a "Reload" item (⌘R) to the View menu.
            CommandGroup(after: .toolbar) {
                // The keys of the items are the keymap's (`[everywhere]`),
                // so that the menu shows what the key really does.
                Button(L("Reload")) { Task { await model.reloadAll() } }
                    .keyboardShortcut(model.keymap.shortcut(.appReload))
                Divider()
                // Text size of the geek theme, like Zed (`TextSize`). In the
                // geek theme the keys are handled by its key monitor first,
                // which also accepts ⌘= for keyboards where `+` needs Shift.
                // A key the monitor does not recognize (some layouts give
                // ⌘+ other characters) reaches these items instead: they
                // go through the model too, which says the new size.
                Button(L("Bigger Text")) { model.changeTextSize(.bigger) }
                    .keyboardShortcut(model.keymap.shortcut(.textBigger))
                Button(L("Smaller Text")) { model.changeTextSize(.smaller) }
                    .keyboardShortcut(model.keymap.shortcut(.textSmaller))
                Button(L("Actual Size")) { model.changeTextSize(.reset) }
                    .keyboardShortcut(model.keymap.shortcut(.textReset))
            }
        }
        // The Preferences (`PreferencesView`): SwiftUI adds "Settings…"
        // (⌘,) to the app menu, named as the system names it ("Réglages…"
        // in French). The same model as the main window: one core, one
        // connection. UI script mode opens its own (`UIScript`).
        Settings {
            if !UIScript.isActive {
                PreferencesView().environment(model)
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
        // No window tabs either, in UI script mode too (`windows.uiscript`
        // checks it): their "+" would open a second window.
        SingleWindow.disableTabs()
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

    /// An update waits to be installed at quit (`Updater`): the quit is
    /// delayed the time to prepare the core (`prepare_update`, at most 5
    /// s), so that it is in maintenance while Sparkle replaces the bundle.
    @MainActor
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard Updater.shared.installsOnQuit, let model = Updater.shared.model, !model.installingUpdate else {
            return .terminateNow
        }
        Task { @MainActor in
            await model.prepareUpdate(limit: .seconds(5))
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
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
