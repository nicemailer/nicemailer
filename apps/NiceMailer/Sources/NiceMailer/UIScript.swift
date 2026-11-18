import AppKit
import WebKit
import SwiftUI

/// Scripted UI testing of the geek theme (see `UITests/README.md`).
///
/// When `NICEMAILER_UI_SCRIPT` names a script file, the app plays it: it
/// sends simulated keys and mouse clicks to its own window and checks the
/// resulting state, then quits with status 0 (all checks passed) or 1.
///
/// The app then stays out of the user's way:
/// - it never becomes the active app (no Dock icon, no keyboard focus), so
///   the user's typing keeps going to their own app;
/// - its window is transparent and moved off screen;
/// - it uses its own, emptied preferences domain (`AppDefaults`);
/// - exports go to `NICEMAILER_EXPORT_DIR`, never to ~/Downloads.
///
/// Events are delivered with `NSApp.sendEvent` / `NSWindow.sendEvent`: they
/// go through the same path as real ones (the key monitor, SwiftUI's
/// gestures), which is what the script is meant to check.
@MainActor
enum UIScript {
    /// The script file, if UI script mode is on.
    /// Swift note: `nonisolated` lets any thread read these constants;
    /// the rest of the type is tied to the main thread (`@MainActor`).
    nonisolated static let path: String? = ProcessInfo.processInfo.environment["NICEMAILER_UI_SCRIPT"]
    nonisolated static var isActive: Bool { path != nil }
    /// Where results and snapshots go.
    nonisolated static let outDir: URL = URL(fileURLWithPath: ProcessInfo.processInfo.environment["NICEMAILER_UI_OUT"]
        ?? NSTemporaryDirectory() + "nicemailer-ui")

    /// Frames of the clickable rows, in window coordinates (bottom-left
    /// origin), recorded by `.uiTarget(_:)` while the script runs.
    static var frames: [String: CGRect] = [:]
    /// What the HTML reader shows (text, font loaded, font family), set
    /// when a page finishes loading.
    static var html: [String: String] = [:]
    /// Text shown by the text reader, and its number of `nm:` links.
    static var readerText = ""
    static var readerLinks = 0
    /// Attachment tiles drawn by the text reader.
    static var readerAttachments = 0
    /// Remote images the current HTML page may load (`HTMLReader`).
    static var remoteImagesAllowed = 0
    /// Remote images handed to the web view since launch
    /// (`RemoteImageLoader`).
    static var remoteImagesShown = 0
    /// Embedded images (`cid:`) served to the HTML reader.
    static var inlineImagesShown = 0
    /// Dark Reader in the HTML page (`DarkMode`): `on`, `off` (light
    /// theme), `absent` (not needed); and the requests it asked for.
    static var darkReader = ""
    static var darkReaderFetches = ""
    /// Requests that reached the "network" since launch: in UI script mode
    /// a local stand-in that answers every request with a small PNG
    /// (`UIScriptImageProtocol`); nothing leaves the machine.
    static var remoteRequests = 0
    /// Name of the last attachment the app would have opened: in UI script
    /// mode nothing is opened, so as not to launch Preview or Word on the
    /// user's screen.
    static var openedFile = ""
    /// The `tel:` URLs whose call the app would have confirmed
    /// (`CallConfirmation`): in UI script mode no notification is touched
    /// and no call is placed.
    static var confirmedCalls: [String] = []
    /// What the open panel of ⇧⌘A answers (`files`).
    static var chosenFiles: [URL] = []
    /// The maintenance the core of the scenario plays (`maintenance`), what
    /// the app's probe answers; and the status before it, given back by
    /// `maintenance off` without reconnecting.
    static var scriptedMaintenance: CoreMaintenance?
    static var statusBeforeMaintenance: AppModel.Status?
    /// Lines of the reader's title line, 1 or 2 (`ReaderTitleBar`).
    static var titleLines = 0
    /// Titles of the help's groups, column by column (`HelpPanel`).
    static var helpColumns: [[String]] = []
    /// Width of the open picker panel, in points (`PickerPopup.width`).
    static var pickerWidth: CGFloat = 0
    /// Rows of the open picker that fit without scrolling (`PickerPopup`).
    static var pickerVisibleRows = 0
    /// Times ⌃⌘F asked for full screen (never applied in UI script mode).
    static var fullScreenToggles = 0
    /// The mailbox folders "Change Mailbox…" switched to, in order
    /// (`mailbox`): the switch itself is never made in UI script mode, which
    /// must not change the user's mailbox nor restart a core.
    static var mailboxesSwitched: [String] = []
    /// Measures again where the text reader's links and tiles are, in the
    /// window (`TextReader`): before a click on one of them.
    static var remeasureReader: (() -> Void)?

    private static var started = false
    /// The test window and its model, kept alive while the script runs.
    private static var window: NSWindow?
    private static var model: AppModel?

    /// Creates the window of UI script mode: the geek interface in a plain
    /// AppKit window that is never made key nor brought to the front, out of
    /// sight.
    static func openWindow() {
        // The SwiftUI WindowGroup still opens an (empty) window: close it.
        for w in NSApp.windows { w.close() }
        let model = AppModel()
        model.startUnified = true
        let host = NSHostingView(rootView: GeekRootView().environment(model))
        // Borderless: the content fills the window (no title bar to offset
        // coordinates), and macOS lets it sit entirely off screen.
        let w = ScriptWindow(contentRect: NSRect(x: -30_000, y: -30_000, width: 1400, height: 900),
                             styleMask: [.borderless], backing: .buffered, defer: false)
        w.contentView = host
        w.isReleasedWhenClosed = false
        w.alphaValue = 0
        w.orderFrontRegardless()
        self.window = w
        self.model = model
        Task { await model.connect() }
    }

    /// Starts the script once the geek window exists.
    static func start(model: AppModel, controller: GeekController) {
        guard let path, !started else { return }
        started = true
        // Opening a file would launch an app on the user's screen: only
        // record its name.
        model.openFile = { openedFile = $0.isFileURL ? $0.lastPathComponent : $0.absoluteString }
        // Confirming a call would drive the user's notifications and call
        // through their iPhone: the URL is opened (recorded, above) and the
        // confirmation only recorded.
        model.confirmCall = { url, open in
            open()
            confirmedCalls.append(url.absoluteString)
            return .confirmed
        }
        // ⇧⌘A would show the open panel: the scenario names the files
        // (`files`), taken from `UITests/fixtures/files`.
        model.chooseFiles = { UIScript.chosenFiles }
        // ⌘V reads a private pasteboard filled by `clipboard`: the user's
        // clipboard is never read nor overwritten.
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailer-UIScript-\(ProcessInfo.processInfo.processIdentifier)"))
        // Full screen would take the user's screen: only counted.
        controller.toggleFullScreen = { fullScreenToggles += 1 }
        // Switching the mailbox would write the user's choice file and
        // restart a core: only recorded, by folder name.
        model.switchMailboxOverride = { url in mailboxesSwitched.append(url.lastPathComponent) }
        Task { @MainActor in
            let runner = Runner(model: model, controller: controller)
            let failures = await runner.run(path: path)
            exit(failures == 0 ? 0 : 1)
        }
    }

    static var testWindow: NSWindow? { window }

    // MARK: Preferences

    /// The Preferences window, while a scenario has it open (`preferences`):
    /// a second off-screen window, like the first. Keys, clicks, `snap` and
    /// `resize` go to it while it is open.
    static private(set) var preferencesWindow: NSWindow?
    /// The objects of its accounts pane, once it appeared
    /// (`preferencesOpened`), for `Runner.state()`.
    static private(set) weak var preferencesList: AccountsList?
    static private(set) weak var preferencesSetup: AccountSetup?
    /// The stand-in that answers the creation methods (`UIScriptSetupCore`).
    static private(set) var setupCore: UIScriptSetupCore?

    /// Called by `AccountsPane` when it appears (does nothing outside UI
    /// script mode).
    static func preferencesOpened(list: AccountsList, setup: AccountSetup) {
        guard isActive else { return }
        preferencesList = list
        preferencesSetup = setup
    }

    /// The organizing of an account only looked at, once asked for.
    static private(set) weak var preferencesActivation: AccountActivation?

    /// Called by `AccountsPane` on "Organize…".
    static func preferencesActivationOpened(_ a: AccountActivation) {
        guard isActive else { return }
        preferencesActivation = a
    }

    /// Opens the Preferences as the app menu's "Settings…" would, on the
    /// script's model, with the creation methods answered by
    /// `UIScriptSetupCore`: no mail server, no DNS, nothing written.
    static func openPreferences() {
        guard preferencesWindow == nil, let model, let core = model.core else { return }
        let fake = UIScriptSetupCore(real: core)
        setupCore = fake
        model.accountsCoreOverride = fake
        let host = NSHostingView(rootView: PreferencesView().environment(model))
        let w = ScriptWindow(contentRect: NSRect(x: -30_000, y: -32_000, width: 720, height: 540),
                             styleMask: [.borderless], backing: .buffered, defer: false)
        w.contentView = host
        w.isReleasedWhenClosed = false
        w.alphaValue = 0
        w.orderFrontRegardless()
        preferencesWindow = w
    }

    static func closePreferences() {
        preferencesWindow?.close()
        preferencesWindow = nil
        frames = frames.filter { !$0.key.hasPrefix("prefs-") }
    }

    /// Records the frame of a clickable row (see `View.uiTarget`). The
    /// rows of the Preferences (`prefs-…`) are in their own window.
    static func record(_ id: String, _ globalFrame: CGRect) {
        let w = id.hasPrefix("prefs-") ? preferencesWindow : window
        guard let content = w?.contentView else { return }
        // SwiftUI's global space has its origin at the top-left of the
        // window content; AppKit events use the bottom-left corner.
        frames[id] = CGRect(
            x: globalFrame.minX, y: content.bounds.height - globalFrame.maxY,
            width: globalFrame.width, height: globalFrame.height)
    }
}

extension View {
    /// Makes the view clickable by the UI script under `id`. Does nothing
    /// outside UI script mode, so normal use pays no cost.
    @ViewBuilder func uiTarget(_ id: String) -> some View {
        if UIScript.isActive {
            background(GeometryReader { geo in
                Color.clear
                    .onAppear { UIScript.record(id, geo.frame(in: .global)) }
                    .onChange(of: geo.frame(in: .global)) { _, f in UIScript.record(id, f) }
                    // A section of the Preferences that goes away takes
                    // its targets with it: a frame left behind would be
                    // found under the field now at its place (`prefs_focus`).
                    // The main window keeps its frames as before (a list's
                    // rows leave and come back as it scrolls).
                    .onDisappear { if id.hasPrefix("prefs-") { UIScript.frames[id] = nil } }
            })
        } else {
            self
        }
    }
}

/// Plays a script. One command per line; `#` starts a comment.
///
/// | Command | Effect |
/// | --- | --- |
/// | `key K` | one key: a character, or `esc` `ret` `del` `up` `down` `space` `shift-space` `shift-up` `shift-down` `cmd-ret` `shift-cmd-ret`, or `ctrl-X` / `cmd-X` |
/// | `type TEXT` | each character of TEXT as a key |
/// | `agents ACTION` | an item of the menu "AI Agents": `claude-desktop`, `claude-code`, `config`, or `repair` (the launch) |
/// | `clipboard TEXT` | puts TEXT (`\n` for a line break) on the pasteboard ⌘V reads |
/// | `click TARGET [cmd] [shift]` | a mouse click on a row |
/// | `dblclick TARGET` | two clicks |
/// | `resize WxH` | window size |
/// | `snap NAME` | PNG of the window in the output directory |
/// | `wait MS` | pause |
/// | `expect FIELD OP VALUE` | check the state (`=`, `!=`, `~` contains, `!~` does not contain, `>`, `<`), waiting up to 5 s |
///
/// Targets: `thread:N` (N-th row of the list), `picker:N`,
/// `screen:N`, `folder:NAME` (folder of the current view), `attachment:N` (N-th attachment tile of the text reader), `reader-link:N` (N-th header or
/// "···" line of the reader), `reader-book:N` (N-th address book icon of the
/// text reader).
@MainActor
private final class Runner {
    let model: AppModel
    let controller: GeekController
    var failures = 0
    var log: [String] = []

    init(model: AppModel, controller: GeekController) {
        self.model = model
        self.controller = controller
    }

    /// Where keys, clicks and snapshots go: the Preferences while they are
    /// open, else the main window.
    var window: NSWindow? { UIScript.preferencesWindow ?? UIScript.testWindow }

    func run(path: String) async -> Int {
        try? FileManager.default.createDirectory(at: UIScript.outDir, withIntermediateDirectories: true)
        // Wait for the first list to load.
        _ = await waitUntil(seconds: 10) { self.model.threads.count > 0 }
        let lines = (try? String(contentsOfFile: path, encoding: .utf8))?.components(separatedBy: .newlines) ?? []
        for (n, raw) in lines.enumerated() {
            let line = raw.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first
                .map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
            if line.isEmpty { continue }
            await execute(line, lineNumber: n + 1)
            // Let SwiftUI apply the change before the next command.
            try? await Task.sleep(for: .milliseconds(60))
        }
        log.append(failures == 0 ? "OK" : "FAILED: \(failures)")
        try? log.joined(separator: "\n").write(to: UIScript.outDir.appendingPathComponent("result.txt"),
                                               atomically: true, encoding: .utf8)
        FileHandle.standardError.write((log.joined(separator: "\n") + "\n").data(using: .utf8)!)
        return failures
    }

    func execute(_ line: String, lineNumber: Int) async {
        let parts = line.split(separator: " ", maxSplits: 1).map(String.init)
        let command = parts[0]
        let arg = parts.count > 1 ? parts[1] : ""
        switch command {
        case "key":
            key(arg)
        case "type":
            for c in arg { key(String(c)) }
        case "openurl":
            // A deep link, as macOS would give it to the app. `clipboard`
            // opens the first `nicemailer://` URL of the private pasteboard
            // (a link copied with `l`).
            var text = arg
            if arg == "clipboard" {
                let copied = controller.pasteboard.string(forType: .string) ?? ""
                text = copied.range(of: #"nicemailer://[^\s)]+"#, options: .regularExpression).map { String(copied[$0]) } ?? copied
            }
            if let url = URL(string: text) { model.openDeepLink(url) }
        case "appearance":
            // Light or dark, for this app only: scenarios and snapshots do
            // not depend on the Mac's setting (`dark`, `light`, `system`).
            NSApp.appearance = arg == "dark" ? NSAppearance(named: .darkAqua)
                : arg == "light" ? NSAppearance(named: .aqua) : nil
        case "addressbook":
            // What macOS would answer for Contacts: `denied`, `unavailable`
            // (a bare executable), `authorized`, `undetermined`. The book
            // is always the scenario's own (`FixtureAddressBook.sample`).
            let access: [String: AddressBookAccess] = [
                "denied": .denied, "unavailable": .unavailable, "authorized": .authorized, "undetermined": .notDetermined,
            ]
            if let book = model.addressBook as? FixtureAddressBook, let value = access[arg] { book.setAccess(value) }
        case "maintenance":
            // `maintenance migration 9 10 9`, `maintenance update`,
            // `maintenance newer_schema 14 13`, `maintenance
            // migration_failed 9 13`: the core of the scenario plays a core
            // in maintenance, as its probe would answer it
            // (`AppModel.waitOutMaintenance`), begun 75 s ago. Changed while
            // the app waits, the next probe shows it. `maintenance off`:
            // the core serves again; the status before it comes back.
            if arg == "off" {
                UIScript.scriptedMaintenance = nil
                try? await Task.sleep(for: .milliseconds(300))
                if let before = UIScript.statusBeforeMaintenance { model.showStatusForScript(before) }
                UIScript.statusBeforeMaintenance = nil
            } else {
                let words = arg.split(separator: " ").map(String.init)
                let n = words.dropFirst().compactMap { Int($0) }
                let m = CoreMaintenance(reason: CoreMaintenance.Reason(rawValue: words.first ?? "") ?? .update,
                                        from: n.first, to: n.dropFirst().first, at: n.dropFirst(2).first,
                                        since: Int(Date().timeIntervalSince1970) - 75,
                                        message: "disque plein")
                let waiting = UIScript.scriptedMaintenance?.isTransient == true
                UIScript.scriptedMaintenance = m
                if UIScript.statusBeforeMaintenance == nil { UIScript.statusBeforeMaintenance = model.status }
                if !waiting {
                    Task { _ = await model.waitOutMaintenance(m, every: .milliseconds(100)) { UIScript.scriptedMaintenance } }
                }
                try? await Task.sleep(for: .milliseconds(300))
            }
        case "mailbox":
            // "Change Mailbox…" with this folder, as the panel would give
            // it: a path relative to the scenario's data directory
            // (`scripts/ui-test.sh` makes `mailboxes/old`, `early`…).
            let base = DataDirectory.environment.map { URL(fileURLWithPath: $0) } ?? FileManager.default.temporaryDirectory
            await model.changeMailbox(to: base.appendingPathComponent(arg))
        case "files", "clipfiles":
            // Files of `UITests/fixtures/files`, by name, comma-separated:
            // what the open panel answers next (`files`), or file URLs on
            // the private pasteboard, as the Finder's ⌘C puts them.
            let dir = URL(fileURLWithPath: UIScript.path ?? "").deletingLastPathComponent().appendingPathComponent("fixtures/files")
            let urls = arg.split(separator: ",").map { dir.appendingPathComponent(String($0).trimmingCharacters(in: .whitespaces)) }
            if command == "files" {
                UIScript.chosenFiles = urls
            } else {
                controller.pasteboard.clearContents()
                controller.pasteboard.writeObjects(urls as [NSURL])
            }
        case "agents":
            // The items of the menu "AI Agents" (`AgentSetup`), as a click
            // on them would run them: `agents claude-desktop`,
            // `agents claude-code`, `agents config`, `agents repair` (what
            // the launch does). Claude Desktop's configuration is a file of
            // the scenario's data directory, never the user's.
            switch arg {
            case "claude-desktop": await model.connectClaudeDesktop()
            case "claude-code": model.copyClaudeCodeCommand(to: controller.pasteboard)
            case "config": model.copyMCPConfiguration(to: controller.pasteboard)
            case "repair": await model.repairClaudeDesktop()
            default: fail(lineNumber, line, "unknown agents action")
            }
        case "preferences":
            // `preferences` opens them, `preferences close` closes them.
            if arg == "close" {
                UIScript.closePreferences()
            } else {
                UIScript.openPreferences()
                if !(await waitUntil(seconds: 5, { UIScript.preferencesList?.loaded == true })) {
                    fail(lineNumber, line, "the Preferences did not load their accounts")
                }
            }
        case "clipboard":
            controller.pasteboard.clearContents()
            controller.pasteboard.setString(arg.replacingOccurrences(of: "\\n", with: "\n"), forType: .string)
        case "click", "dblclick":
            let words = arg.split(separator: " ").map(String.init)
            var mods: NSEvent.ModifierFlags = []
            if words.contains("cmd") { mods.insert(.command) }
            if words.contains("shift") { mods.insert(.shift) }
            guard let target = words.first, var point = await locate(target) else {
                fail(lineNumber, line, "target not found")
                return
            }
            // `right`: near the right edge rather than in the middle, where
            // a row of a settings form has its control (a checkbox).
            if words.contains("right"), let f = frame(target) { point.x = f.maxX - 10 }
            click(at: point, modifiers: mods, count: 1)
            if command == "dblclick" {
                try? await Task.sleep(for: .milliseconds(50))
                click(at: point, modifiers: mods, count: 2)
            }
        case "resize":
            let wh = arg.split(separator: "x").compactMap { Double($0) }
            if wh.count == 2, let w = window {
                var f = w.frame
                f.size = NSSize(width: wh[0], height: wh[1])
                w.setFrame(f, display: true)
            }
        case "snap":
            snap(arg)
        case "snapeditor":
            // The editor's page, which a snapshot of the window cannot see
            // (WebKit draws it in another process): WebKit's own snapshot.
            await snapEditor(arg)
        case "wait":
            try? await Task.sleep(for: .milliseconds(Int(arg) ?? 100))
        case "expect":
            await expect(arg, line: line, lineNumber: lineNumber)
        default:
            fail(lineNumber, line, "unknown command")
        }
    }

    // MARK: Events

    func key(_ name: String) {
        guard let w = window else { return }
        var chars = name
        var code: UInt16 = 0
        var mods: NSEvent.ModifierFlags = []
        switch name {
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        case "ret": chars = "\r"; code = KeyCode.returnKey
        case "del": chars = "\u{7f}"; code = KeyCode.delete
        case "up": chars = "\u{F700}"; code = KeyCode.up
        case "down": chars = "\u{F701}"; code = KeyCode.down
        case "space": chars = " "
        case "shift-space": chars = " "; mods = .shift
        case "shift-up": chars = "\u{F700}"; code = KeyCode.up; mods = .shift
        case "shift-down": chars = "\u{F701}"; code = KeyCode.down; mods = .shift
        case "cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = .command
        case "shift-cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = [.command, .shift]
        case "tab": chars = "\t"; code = KeyCode.tab
        case "shift-tab": chars = "\u{19}"; code = KeyCode.tab; mods = .shift
        case "alt-cmd-b": chars = "b"; code = 11; mods = [.command, .option]
        case "alt-cmd-f": chars = "f"; code = 3; mods = [.command, .option]
        case "shift-cmd-a": chars = "A"; code = 0; mods = [.command, .shift]
        // Editing keys of a typed line (`LineEdit`).
        case "left": chars = "\u{F702}"; code = KeyCode.left; mods = .function
        case "right": chars = "\u{F703}"; code = KeyCode.right; mods = .function
        case "alt-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .option]
        case "alt-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .option]
        case "cmd-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .command]
        case "cmd-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .command]
        case "home": chars = "\u{F729}"; code = KeyCode.home; mods = .function
        case "end": chars = "\u{F72B}"; code = KeyCode.end; mods = .function
        case "fdel": chars = "\u{F728}"; code = KeyCode.forwardDelete; mods = .function
        case "alt-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .option
        case "cmd-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .command
        // The reply's commands of the editor (`edition.md`, section 7).
        case "cmd-esc": chars = "\u{1b}"; code = KeyCode.escape; mods = .command
        case "shift-cmd-t": chars = "T"; code = 17; mods = [.command, .shift]
        case "shift-alt-cmd-v": chars = "v"; code = 9; mods = [.command, .shift, .option]
        case "shift-ret": chars = "\r"; code = KeyCode.returnKey; mods = .shift
        // `#` starts a comment in a script line.
        case "hash": chars = "#"
        // Selecting in the text of a reply: Shift with the moves.
        case "shift-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift]
        case "shift-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .shift]
        case "shift-alt-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift, .option]
        case "shift-alt-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .shift, .option]
        case "shift-cmd-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift, .command]
        case "shift-cmd-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .shift, .command]
        case "shift-cmd-up": chars = "\u{F700}"; code = KeyCode.up; mods = [.function, .shift, .command]
        case "shift-alt-up": chars = "\u{F700}"; code = KeyCode.up; mods = [.function, .shift, .option]
        case "shift-alt-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .shift, .option]
        case "shift-cmd-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .shift, .command]
        case "cmd-up": chars = "\u{F700}"; code = KeyCode.up; mods = [.function, .command]
        case "cmd-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .command]
        // Full screen: AppKit gives ⌃F as the control character 0x06.
        case "ctrl-cmd-f": chars = "\u{6}"; code = 3; mods = [.control, .command]
        default:
            if name.hasPrefix("ctrl-") {
                mods = .control
                chars = String(name.dropFirst(5))
                // `ctrl-C`: Shift is held, as on the keyboard (AppKit keeps
                // the capital in the characters without modifiers).
                if chars.count == 1, chars.uppercased() == chars, chars.lowercased() != chars { mods.insert(.shift) }
            } else if name.hasPrefix("cmd-") {
                mods = .command
                chars = String(name.dropFirst(4))
            }
        }
        guard let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: w.windowNumber, context: nil, characters: chars, charactersIgnoringModifiers: chars,
            isARepeat: false, keyCode: code)
        else { return }
        // The reply's text has the keys: what the app does not take goes to
        // the editor's web view. Delivered to it directly, as the clicks
        // are (`click`): the app is never active, and AppKit would give a
        // key nowhere in a window that is not the key window.
        // The Preferences: the window gives the key to its first responder,
        // the field being edited; Return, to the default button.
        if w === UIScript.preferencesWindow {
            // The monitor of the app sees every key first, as with a real
            // keyboard: it must leave this one to the Preferences.
            guard controller.route(e, mainWindow: UIScript.testWindow) != nil else { return }
            if !(w.firstResponder?.performKeyEquivalent(with: e) ?? false), !w.performKeyEquivalent(with: e) {
                w.sendEvent(e)
            }
            return
        }
        if let c = model.composer, c.focus == .body, controller.screen == .mail, controller.overlay == .none {
            if !controller.handle(e) {
                let web = controller.editorView
                if web.window?.firstResponder !== web { web.window?.makeFirstResponder(web) }
                if !(mods.contains(.command) && web.performKeyEquivalent(with: e)) {
                    web.keyDown(with: e)
                    if let up = NSEvent.keyEvent(with: .keyUp, location: .zero, modifierFlags: mods, timestamp: e.timestamp,
                                                 windowNumber: w.windowNumber, context: nil, characters: chars,
                                                 charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code) {
                        web.keyUp(with: up)
                    }
                }
            }
            return
        }
        NSApp.sendEvent(e)
    }

    /// Sends a mouse down and up to the view under `point`.
    ///
    /// The events go straight to that view rather than through the window:
    /// the app is never active, and a window of an inactive app uses a
    /// plain click only to activate itself. Calling the view's `mouseDown` /
    /// `mouseUp` is what the window would do for an active app, so SwiftUI's
    /// gestures (and their modifier filters) see a normal click.
    func click(at point: CGPoint, modifiers: NSEvent.ModifierFlags, count: Int) {
        guard let w = window, let content = w.contentView else { return }
        let target = content.hitTest(point) ?? content
        // The controls of the Preferences follow the click the same way: a
        // text field starts editing, a button waits to be released, a
        // table selects its row, each in a loop; from the view hit or any
        // view around it.
        let tracks = sequence(first: target, next: { $0.superview })
            .contains { $0 is NSTextView || $0 is NSControl || $0 is NSTableView }
        if tracks {
            // A text view follows a click in a tracking loop that waits for
            // the mouse up: queue it first, then send the mouse down.
            guard let down = NSEvent.mouseEvent(
                with: .leftMouseDown, location: point, modifierFlags: modifiers,
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: w.windowNumber, context: nil,
                eventNumber: 0, clickCount: count, pressure: 1),
                let up = NSEvent.mouseEvent(
                    with: .leftMouseUp, location: point, modifierFlags: modifiers,
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: w.windowNumber, context: nil,
                    eventNumber: 0, clickCount: count, pressure: 0)
            else { return }
            NSApp.postEvent(up, atStart: false)
            target.mouseDown(with: down)
            return
        }
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            guard let e = NSEvent.mouseEvent(
                with: type, location: point, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: w.windowNumber, context: nil, eventNumber: 0, clickCount: count, pressure: 1)
            else { continue }
            if type == .leftMouseDown { target.mouseDown(with: e) } else { target.mouseUp(with: e) }
        }
    }

    /// Frame of a target already located (`locate`), by the same id.
    func frame(_ target: String) -> CGRect? {
        let parts = target.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        return UIScript.frames["\(parts[0])-\(parts[1])"]
    }

    /// Center of a target row, waiting for it to be laid out.
    func locate(_ target: String) async -> CGPoint? {
        let parts = target.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        if ["attachment", "reader-link", "reader-sender", "reader-book"].contains(parts[0]) {
            UIScript.remeasureReader?()
        }
        let id: String
        switch parts[0] {
        case "thread":
            guard let i = Int(parts[1]), model.threads.indices.contains(i) else { return nil }
            id = "thread-\(model.threads[i].id)"
        case "folder":
            // A folder of the current view, by name.
            id = "folder-\(parts[1])"
        default:
            id = "\(parts[0])-\(parts[1])"
        }
        guard await waitUntil(seconds: 3, { UIScript.frames[id] != nil }), let f = UIScript.frames[id] else { return nil }
        return CGPoint(x: f.midX, y: f.midY)
    }

    func snapEditor(_ name: String) async {
        guard let web = controller.editorView as? WKWebView else { return }
        let image: NSImage? = await withCheckedContinuation { c in
            web.takeSnapshot(with: nil) { image, _ in c.resume(returning: image) }
        }
        guard let tiff = image?.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff) else { return }
        try? rep.representation(using: .png, properties: [:])?
            .write(to: UIScript.outDir.appendingPathComponent("\(name).png"))
    }

    func snap(_ name: String) {
        guard let v = window?.contentView, let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds) else { return }
        v.cacheDisplay(in: v.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?
            .write(to: UIScript.outDir.appendingPathComponent("\(name).png"))
    }

    // MARK: Checks

    func expect(_ arg: String, line: String, lineNumber: Int) async {
        let parts = arg.split(separator: " ", maxSplits: 2).map(String.init)
        guard parts.count >= 2 else { fail(lineNumber, line, "bad expect"); return }
        let (field, op) = (parts[0], parts[1])
        let wanted = parts.count == 3 ? parts[2] : ""
        let ok = await waitUntil(seconds: 5) { Self.compare(self.state()[field] ?? "<unknown field>", op, wanted) }
        if ok {
            log.append("ok   \(lineNumber): \(line)")
        } else {
            fail(lineNumber, line, "got \"\(state()[field] ?? "<unknown field>")\"")
        }
    }

    static func compare(_ got: String, _ op: String, _ wanted: String) -> Bool {
        switch op {
        case "=": return got == wanted
        case "!=": return got != wanted
        case "~": return got.contains(wanted)
        case "!~": return !got.contains(wanted)
        case ">": return (Double(got) ?? .nan) > (Double(wanted) ?? .nan)
        case "<": return (Double(got) ?? .nan) < (Double(wanted) ?? .nan)
        default: return false
        }
    }

    func fail(_ lineNumber: Int, _ line: String, _ why: String) {
        failures += 1
        log.append("FAIL \(lineNumber): \(line) — \(why)")
        snap("fail-line-\(lineNumber)")
    }

    func waitUntil(seconds: Double, _ condition: @escaping @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(40))
        }
        return condition()
    }

    /// The observable state, as text, for `expect`.
    func state() -> [String: String] {
        let m = model, c = controller
        let index = m.threads.firstIndex { $0.id == m.selectedThreadId }
        let markedIdx = m.threads.indices.filter { m.marked.contains(m.threads[$0].id) }
        let sel = index.map { m.threads[$0] }
        var exports = "0"
        if let dir = m.exportDirectory {
            exports = "\((try? FileManager.default.contentsOfDirectory(atPath: dir))?.count ?? 0)"
        }
        let pickerRows: [String] = {
            if case .picker(let k) = c.overlay { return c.pickerEntries(k).map(\.label) }
            return []
        }()
        let main: [String: String] = [
            "screen": "\(c.screen)",
            // The rows of the Tags or Groups screen, by name: `Urgent, Calme`.
            "screen_rows": c.screen == .tags ? c.screenTags.map(\.name).joined(separator: ", ")
                : c.screen == .groups ? c.groups.map(\.name).joined(separator: ", ") : "",
            "overlay": "\(c.overlay)",
            // `Move,Read | Select,Act on mails | …`: the help's columns.
            "help_columns": c.overlay == .help
                ? UIScript.helpColumns.map { $0.joined(separator: ",") }.joined(separator: " | ") : "",
            "mode": c.modeLabel,
            // Move targets shown in the status bar in Quarantine, in place
            // of the `m` block: `inbox,newsletter,…`, empty elsewhere.
            "status_targets": c.directTargets.joined(separator: ","),
            // The keys the status bar reminds of (`keymap.toml`):
            // `account=@ folder=; attributes=f v=v m=m s=s ai=a help=?`.
            "status_keys": [
                "account=" + c.keyLabel(.pickerAccounts), "folder=" + c.menuKeyLabel(.goto),
                "attributes=" + c.keyLabel(.pickerAttributes), "v=" + c.menuKeyLabel(.view),
                "m=" + c.menuKeyLabel(.move), "s=" + c.menuKeyLabel(.save),
                "ai=" + c.aiKeyLabel(composing: m.composer != nil), "help=" + c.keyLabel(.helpShow),
            ].joined(separator: " "),
            // The lines of the help, `key label`, ` | ` between (while it
            // is open).
            "help_rows": c.overlay == .help
                ? Help.groups(c.keymap).flatMap(\.entries).map { "\($0.key) \($0.label)" }.joined(separator: " | ") : "",
            // Why keymap.toml is refused, empty when it is applied.
            "keymap_problem": m.keymapProblem?.text ?? "",
            // The keys of the folders in the folder column (`;` menu), as
            // `i Inbox, t Todo…`.
            "folder_keys": c.scopeFolders.map { "\(c.gotoKeyLabel($0)) \($0.name)" }.joined(separator: ", "),
            "mode_icon": "\(c.showsAppIcon && AppIcon.statusImage != nil)",
            "location": c.locationLabel,
            "flash": m.flash ?? "",
            "error": m.lastError ?? "",
            "thread_count": "\(m.threads.count)",
            "selected": index.map(String.init) ?? "-1",
            "selected_subject": sel?.subject ?? "",
            // The sender column of the selected row, as drawn: "→ Alice"
            // when the user wrote the latest mail (`ui.md`, "Liste").
            "selected_sender": sel.map(GeekSender.plain) ?? "",
            // The date column of the selected row, as drawn, in the
            // interface's locale (`docs/localization.md`).
            "selected_date": sel.map { Format.geekListDate($0.dateLastReceived) } ?? "",
            // The icon before that name, Quarantine only (`ui.md`,
            // "Quarantine"): `person`, `robot`, `none` (no verdict, a blank
            // place), or empty when the list draws no icon.
            "selected_sender_kind": m.listShowsSenderKind ? sel.map { $0.correspondentKind ?? "none" } ?? "" : "",
            "selected_unread": "\(sel?.unreadCount ?? 0)",
            "selected_flagged": "\(sel?.isFlagged ?? false)",
            "selected_tags": sel?.tags.joined(separator: ",") ?? "",
            // The labels of the open thread's header (`ReaderTitleBar`).
            "header_tags": m.openThread?.labels.tags.joined(separator: ",") ?? "",
            "header_groups": m.openThread?.labels.groups.joined(separator: ",") ?? "",
            "marked": markedIdx.map(String.init).joined(separator: ","),
            "marked_count": "\(m.marked.count)",
            // Marked rows still unread, and flagged: grouped actions
            // (`ui.md`, "Actions groupées") change them all at once.
            "marked_unread": "\(m.threads.filter { m.marked.contains($0.id) && $0.unreadCount > 0 }.count)",
            "marked_flagged": "\(m.threads.filter { m.marked.contains($0.id) && $0.isFlagged }.count)",
            "folder": m.currentFolderName,
            "account": m.scopeName,
            "search_origin": c.searchOrigin.map { o in
                o.folderId.map { "folder:\($0)" } ?? o.unifiedRole.map { "all:\($0)" } ?? "everything"
            } ?? "",
            "picker_query": c.pickerQuery,
            // Where the cursor of the typed text is, in characters.
            "picker_cursor": "\(c.pickerLine.cursor)",
            // Whether the typed text is selected (typing replaces it).
            "picker_selected": "\(c.pickerLine.selectedAll)",
            "picker_index": "\(c.pickerIndex)",
            "picker_count": "\(pickerRows.count)",
            // The row Return would choose (the picker opens on its first
            // row, and typing brings the best match there).
            "picker_first": pickerRows.first ?? "",
            // Width of the open picker panel, in whole points.
            "picker_width": {
                if case .picker = c.overlay { return "\(Int(UIScript.pickerWidth.rounded()))" }
                return ""
            }(),
            // Rows of the open picker below its bottom edge, to scroll to.
            "picker_hidden_rows": {
                if case .picker = c.overlay { return "\(max(0, pickerRows.count - UIScript.pickerVisibleRows))" }
                return ""
            }(),
            // Every row of the open picker, in order, " | " between them.
            "picker_rows": pickerRows.joined(separator: " | "),
            // Detail of the first row ("dd mm yy" for six digits read two
            // ways, "domain"…).
            "picker_first_detail": {
                if case .picker(let k) = c.overlay { return c.pickerEntries(k).first?.detail ?? "" }
                return ""
            }(),
            // Symbol of the first row ("lock.fill" for a verification code).
            "picker_first_icon": {
                if case .picker(let k) = c.overlay { return c.pickerEntries(k).first?.icon ?? "" }
                return ""
            }(),
            // The hint of the input line, shown while nothing is typed.
            "picker_hint": {
                if case .picker(let k) = c.overlay, c.pickerQuery.isEmpty { return c.pickerHint(k) ?? "" }
                return ""
            }(),
            "screen_index": "\(c.screenIndex)",
            "contacts_count": "\(c.contacts.count)",
            "contacts_filter": c.contactsFilterChip ?? "",
            // The origin column of the contact under the cursor ("human",
            // "robot *" when forced; `expediteurs.md`, section 9).
            "contact_origin": c.contacts.indices.contains(c.screenIndex) ? c.contacts[c.screenIndex].originLabel : "",
            "history_count": "\(c.historyRows.count)",
            // The row under the cursor of the History screen.
            "history_who": c.historyRows.indices.contains(c.screenIndex) ? HistoryDisplay.who(c.historyRows[c.screenIndex].group.source) : "",
            "history_title": c.historyRows.indices.contains(c.screenIndex) ? HistoryDisplay.title(c.historyRows[c.screenIndex].group) : "",
            "history_verb": c.historyRows.indices.contains(c.screenIndex) ? HistoryDisplay.action(c.historyRows[c.screenIndex].group).verb : "",
            "history_detail": c.historyRows.indices.contains(c.screenIndex) ? HistoryDisplay.action(c.historyRows[c.screenIndex].group).detail : "",
            "history_group": c.historyRows.indices.contains(c.screenIndex) ? "\(c.historyRows[c.screenIndex].group.count)" : "",
            "history_unfolded": "\(c.historyUnfolded.count)",
            "history_read_shown": "\(c.historyIncludesRead)",
            "history_actions": c.historyRows.map { let a = HistoryDisplay.action($0.group); return a.verb + " " + a.detail }.joined(separator: " | "),
            "tags_count": "\(c.tags.count)",
            "groups_count": "\(c.groups.count)",
            // The groups of the folder column, in order, and how many had
            // no room ("N more…", `GroupRanking.shown`).
            "column_groups": c.columnGroups.rows.map(\.name).joined(separator: ", "),
            "column_groups_hidden": "\(c.columnGroups.hidden)",
            // The groups of the column drawn with the grey row of the
            // current folder (`GeekController.isHighlighted`).
            "column_groups_highlighted": c.columnGroups.rows.filter(c.isHighlighted).map(\.name).joined(separator: ", "),
            "columns": "\(c.columns)",
            "reading": "\(c.reading)",
            "status_position": c.statusPosition.rawValue,
            // The key the AI block of the status bar reminds of: `a`, or
            // `⌘I` while a reply is written.
            "status_ai_key": c.aiKeyLabel(composing: m.composer != nil),
            "sort": m.sort,
            "body_format": m.bodyFormat,
            // Remote images (`RemoteImages.swift`): the switch offers
            // "images", pixels removed by the core, images the page may
            // load, images shown and requests sent since launch.
            "images_offered": "\(m.imagesOffered)",
            "tracking_pixels": "\(m.trackingPixels)",
            "remote_allowed": "\(UIScript.remoteImagesAllowed)",
            "remote_shown": "\(UIScript.remoteImagesShown)",
            "inline_shown": "\(UIScript.inlineImagesShown)",
            "remote_requests": "\(UIScript.remoteRequests)",
            // The content rule list of the HTML reader (`RemoteBlocker`):
            // `on` once compiled, `off` if WebKit refused it.
            "remote_blocker": RemoteBlocker.status,
            // The reader's title line: subject and controls on 1 line, or
            // the subject above them on 2 (`ReaderTitleBar`).
            "title_lines": "\(UIScript.titleLines)",
            // Where the fonts and translations were found (`AppResources`):
            // `Contents/Resources` of NiceMailer.app, or the build folder.
            "resources_bundle": AppResources.bundle.bundlePath,
            // What the private pasteboard holds (a link copied with `l`, a
            // mail copied with `sc`), and its number of lines (`sc` keeps
            // one line per paragraph).
            "pasteboard": controller.pasteboard.string(forType: .string) ?? "",
            // The menu "AI Agents" has its helper (`NICEMAILER_MCP`), and
            // the command of NiceMailer's entry in the scenario's Claude
            // Desktop configuration, empty when there is none.
            "agents_available": AgentSetup.find() == nil ? "false" : "true",
            "claude_desktop_entry": {
                let file = DataDirectory.current.appendingPathComponent("claude_desktop_config.json")
                guard let data = try? Data(contentsOf: file),
                      let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                      let servers = o["mcpServers"] as? [String: Any],
                      let entry = servers["nicemailer"] as? [String: Any] else { return "" }
                return entry["command"] as? String ?? ""
            }(),
            "pasteboard_lines": "\((controller.pasteboard.string(forType: .string) ?? "").split(separator: "\n", omittingEmptySubsequences: false).count)",
            "reader_subject": m.openThread?.subject ?? "",
            // The reader shows the mail under the cursor (not one passed
            // over while moving fast, `AppModel.loadOpenThread`).
            "reader_is_selected": "\(m.openThread != nil && m.openThread?.id == m.selectedThreadId)",
            "unsubscribe": m.unsubscribeOffer.map {
                $0.senderUnsubscribedAt != nil ? "done" : "\($0.source):\($0.method)"
            } ?? "",
            "reader_attachments": "\(UIScript.readerAttachments)",
            "opened_file": UIScript.openedFile,
            "confirmed_calls": UIScript.confirmedCalls.joined(separator: ","),
            "auto_confirm_calls": "\(m.autoConfirmCalls)",
            "mailbox_switched": UIScript.mailboxesSwitched.joined(separator: ","),
            "mailbox_question": m.pendingUpgrade.map { "\($0.url.lastPathComponent) \($0.from)->\($0.to)" } ?? "",
            "mailbox_upgrading": m.upgradingMailbox?.lastPathComponent ?? "",
            // The connection: connecting, connected, maintenance: <short>
            // | <message>, failed: <message>.
            "core_status": {
                switch m.status {
                case .connecting: "connecting"
                case .connected: "connected"
                case .maintenance(let text, let short): "maintenance: \(short) | \(text)"
                case .failed(let message): "failed: \(message)"
                }
            }(),
            // The maintenance that left the app without a core: its reason.
            "last_maintenance": m.lastMaintenance?.reason.rawValue ?? "",
            // "Check for Updates…" can be used (`Updater`).
            "updates_available": "\(Updater.shared.canCheck)",
            // Menu items that would open another window (`SingleWindow`),
            // comma-separated: empty while the app keeps a single window.
            "window_openers": SingleWindow.windowOpeners(in: NSApp.mainMenu).joined(separator: ","),
            // Menus of the menu bar, so that an empty `window_openers` is
            // not read before SwiftUI has built them.
            "menu_count": "\(NSApp.mainMenu?.items.count ?? 0)",
            // macOS window tabs allowed (`SingleWindow.disableTabs`).
            "window_tabs": "\(NSWindow.allowsAutomaticWindowTabbing)",
            // The launch (`AppModel.StartupPhase`): connecting,
            // startingCore, loading, ready; and how long it took, in
            // milliseconds, once ready.
            "startup_phase": "\(m.startupPhase)",
            // Refreshes after the core's events: the conversations that
            // changed alone, or the whole list (`AppModel.patchThreads`).
            "list_patches": "\(m.patchCount)",
            "list_reloads": "\(m.wholeReloadCount)",
            "startup_ms": m.startupTimings.map { "\(StartupTimings.milliseconds($0.firstList))" } ?? "",
            "sticky": m.stickyThreadId.map(String.init) ?? "",
            "reader_folder": m.selectedThreadFolderLabel ?? "",
            "list_rows": "\(c.listVisibleRows)",
            "reader_offset": "\(Int(c.readerScroll?.contentView.bounds.origin.y ?? 0))",
            "list_filter": m.listFilter.map { "\($0)" } ?? "",
            "export_files": exports,
            "font_size": "\(Int(TextSize.shared.size))",
            "reader_text": UIScript.readerText,
            "reader_links": "\(UIScript.readerLinks)",
            "reader_folded": "\(m.openThread.map { ThreadDisplay.foldedCount(ThreadDisplay.blocks($0, c.readerState)) } ?? 0)",
            "show_all": "\(c.readerState.showAll)",
            // The address book's icons drawn in the reader, in order: the
            // sender of each unfolded message and whether they are in the
            // book ("bob@example.org known | news@example.net unknown").
            "reader_books": m.openThread.map { thread in
                ThreadDisplay.blocks(thread, c.readerState, book: m.senderBook).compactMap { block -> String? in
                    guard case .header(let id, _, _, _, _, _, _, _, let book?) = block else { return nil }
                    return "\(thread.messages.first { $0.id == id }?.fromAddr ?? "") \(book.name)"
                }.joined(separator: " | ")
            } ?? "",
            "filters": m.attributes.chips.map(\.label).joined(separator: ", "),
            // Position of the first row of the open menu, in window
            // coordinates (origin bottom-left): where the menu is drawn.
            "menu_index": c.menuIndex.map(String.init) ?? "",
            // The rows of the open menu, "key label", a virtual folder
            // marked "*", blocks separated by " || ".
            "menu_rows": {
                guard case .menu(let p) = c.overlay else { return "" }
                return c.menuEntries(p).map { e in
                    (e.gapBefore ? "|| " : "") + "\(e.key) \(e.label)" + (e.virtual ? "*" : "")
                }.joined(separator: " | ").replacingOccurrences(of: " | ||", with: " ||")
            }(),
            "menu_x": "\(Int(UIScript.frames["menu-0"]?.minX ?? -1))",
            "menu_y": "\(Int(UIScript.frames["menu-0"]?.minY ?? -1))",
            // Height of the selected row, as laid out.
            "row_height": "\(Int((UIScript.frames["thread-\(m.selectedThreadId ?? -1)"]?.height ?? 0).rounded()))",
            "html_text": UIScript.html["text"] ?? "",
            "html_font_loaded": UIScript.html["font"] ?? "",
            "html_font_family": UIScript.html["family"] ?? "",
            "html_inline_drawn": UIScript.html["inline"] ?? "",
            "html_author_font": UIScript.html["authorFont"] ?? "",
            "html_table_width": UIScript.html["tableWidth"] ?? "",
            "html_csp": UIScript.html["csp"] ?? "",
            "html_frames": UIScript.html["frames"] ?? "",
            "dark_reader": UIScript.darkReader,
            "dark_reader_fetches": UIScript.darkReaderFetches,
            "html_paper": UIScript.html["paper"] ?? "",
            "html_styled_width": UIScript.html["styledWidth"] ?? "",
            "html_styled_background": UIScript.html["styledBackground"] ?? "",
            "html_offset": "\(c.htmlOffset)",
            // The folder workflow: steps `z` can undo, a suspect move
            // waiting for its confirmation (its sender), the question shown,
            // and the decision of the contact under the cursor (its own, or
            // its domain's after "@").
            "undo_count": "\(m.undoStack.count)",
            "suspect": m.suspectMove?.addr ?? "",
            "choice": c.activeChoice.map { c.choiceTitle($0) } ?? "",
            // The rows of the choice shown, "key label | key label".
            // The row of the open choice highlighted with the arrows (or
            // first when it opens, for the AI menu), empty if none.
            "choice_index": c.choiceIndex.map(String.init) ?? "",
            "choice_rows": c.activeChoice.map { ch in c.choiceEntries(ch).map { "\($0.key) \($0.label)" }.joined(separator: " | ") } ?? "",
            // The AI panel of the reply (`AIPanel`): `none`, `running`,
            // `proposal`, `failed`; its title, the text received, the
            // proposal (a replacement as `[-removed-]{+added+}`, subjects
            // joined by " | "), the failure.
            "ai_state": {
                switch m.aiRun?.phase {
                case nil: "none"
                case .running?: "running"
                case .proposal?: "proposal"
                case .failed?: "failed"
                }
            }(),
            "ai_title": m.aiRun?.title ?? "",
            "ai_text": (m.aiRun?.text ?? "").replacingOccurrences(of: "\n", with: "\\n"),
            "ai_diff": {
                guard case .proposal(.replace(let segments, _))? = m.aiRun?.phase else { return "" }
                return AI.diffText(segments).replacingOccurrences(of: "\n", with: "\\n")
            }(),
            // A proposal only read, or a list of choices, as text
            // (`AI.resultText`, line breaks as `\n`).
            "ai_result": {
                guard case .proposal(let p)? = m.aiRun?.phase else { return "" }
                switch p {
                case .text, .summary, .verdict, .choices, .events, .contact: return AI.resultText(p).replacingOccurrences(of: "\n", with: "\\n")
                default: return ""
                }
            }(),
            // A sorting under way (`a` `s`): "done/total", empty otherwise;
            // the suggestions of the rows shown, and the one of the row
            // under the cursor ("target: reason").
            // Where the AI shows (`GeekController.aiPlacement`): `center`,
            // `headers`, `none`.
            "ai_placement": c.aiPlacement,
            "ai_sort": m.aiSort.map { "\($0.done)/\($0.total)" } ?? "",
            "ai_suggestions": "\(m.aiSuggestions.count)",
            "selected_suggestion": m.selectedSuggestion.map { "\($0.target): \($0.reason)" } ?? "",
            // The text of a list proposal Return takes, from 1 as the
            // panel numbers them; empty without a list.
            "ai_choice": {
                switch m.aiRun?.phase {
                case .proposal(.subjects)?, .proposal(.choices)?, .proposal(.events)?: "\((m.aiRun?.choice ?? 0) + 1)"
                default: ""
                }
            }(),
            "ai_subjects": {
                guard case .proposal(.subjects(let items))? = m.aiRun?.phase else { return "" }
                return items.joined(separator: " | ")
            }(),
            "ai_error": {
                guard case .failed(let message)? = m.aiRun?.phase else { return "" }
                return message
            }(),
            "contact_addr": c.contacts.indices.contains(c.screenIndex) ? c.contacts[c.screenIndex].addr : "",
            "contact_decision": c.contacts.indices.contains(c.screenIndex)
                ? c.contacts[c.screenIndex].screeningTarget ?? c.contacts[c.screenIndex].domainTarget.map { "@" + $0 } ?? ""
                : "",
            // The reply being written (`reponse.md`): whether one is open,
            // what the core prepared, the text with `|` at the caret (line
            // breaks as `\n`), its lines as drawn, and the mails waiting
            // to leave.
            "composing": "\(m.composer != nil)",
            "compose_from": m.composer?.headers.from.addr ?? "",
            "compose_from_account": m.composer.flatMap { c in m.senders.first { $0.accountId == c.headers.sendAccountId }?.account } ?? "",
            "compose_senders": m.senders.map(\.addr).joined(separator: ","),
            "compose_to": m.composer.map { $0.headers.to.map(\.addr).joined(separator: ",") } ?? "",
            "compose_cc": m.composer.map { $0.headers.cc.map(\.addr).joined(separator: ",") } ?? "",
            "compose_bcc": m.composer.map { $0.headers.bcc.map(\.addr).joined(separator: ",") } ?? "",
            "compose_bcc_shown": "\(m.composer?.showsBcc ?? false)",
            "compose_subject": m.composer?.headers.subject ?? "",
            // The part of the subject selected (⇧⌥←, ⌘A), empty when none.
            "compose_subject_selection": m.composer?.subjectLine.selectedText ?? "",
            "compose_focus": m.composer?.focus.rawValue ?? "",
            "compose_line": m.composer.map { $0.line.beforeCursor + "|" + String($0.line.text.dropFirst($0.line.cursor)) } ?? "",
            // The line is a whole-text selection (an address taken back
            // with ⌫ or a click): typing or ⌫ replaces all of it.
            "compose_line_selected": "\(m.composer?.line.selectedAll ?? false)",
            "compose_suggestions": m.composer.map { $0.suggestions.map(\.addr).joined(separator: ",") } ?? "",
            "compose_attachments": m.composer.map { $0.attachments.map(\.filename).joined(separator: ",") } ?? "",
            "compose_attachment_index": m.composer.map { String($0.attachmentIndex) } ?? "",
            "compose_suggestion_index": m.composer?.suggestionIndex.map(String.init) ?? "",
            "compose_after": m.composer.flatMap { Compose.afterSend($0.draft) } ?? "",
            "compose_after_line": m.composer.flatMap { Compose.afterSendLine($0.draft) } ?? "",
            // The text as the editor last reported it, `|` at the cursor in
            // text only (its offsets are UTF-16 units, as `NSString`'s).
            "compose_text": m.composer.map { c in
                let t = c.body as NSString
                guard c.format == .text, let r = m.editorState.textRange, r.start <= t.length else {
                    return c.body.replacingOccurrences(of: "\n", with: "\\n")
                }
                return (t.substring(to: r.start) + "|" + t.substring(from: r.start)).replacingOccurrences(of: "\n", with: "\\n")
            } ?? "",
            "compose_format": m.composer?.format.rawValue ?? "",
            "editor_ready": "\((m.editor as? EditorBridge)?.isReady ?? false)",
            "editor_focused": "\(m.editorState.focused)",
            "editor_modal": "\(m.editorState.modal)",
            "editor_marks": [m.editorState.marks.bold ? "bold" : nil, m.editorState.marks.italic ? "italic" : nil,
                             m.editorState.marks.underline ? "underline" : nil, m.editorState.marks.link ? "link" : nil]
                .compactMap { $0 }.joined(separator: ","),
            "editor_block": m.editorState.block,
            "editor_recent_fonts": MacFonts.recent.joined(separator: ","),
            "compose_saved": m.composer.map { "\(!$0.isDirty)" } ?? "",
            "compose_written": m.composer.map { "\($0.isWritten)" } ?? "",
            // The text selected in the reply (line breaks as `\n`).
            "compose_selection": m.editorState.selection.replacingOccurrences(of: "\n", with: "\\n"),
            "sending_count": "\(m.sending.count)",
            "fullscreen_toggles": "\(UIScript.fullScreenToggles)",
            // The reply's text is the window's focused view: what the
            // accessibility API gives another app (Kerlig) is WebKit's.
            "ax_focused": "\(controller.editorView.window?.firstResponder === controller.editorView)",
            "editor_messages": (m.editor as? EditorBridge).map { $0.received.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }.joined(separator: ",") } ?? "",
            "editor_in_window": "\(controller.editorView.window != nil)",
        ]
        return main.merging(preferencesState()) { $1 }
    }

    /// The Preferences' fields (`UITests/README.md`): empty while closed.
    func preferencesState() -> [String: String] {
        guard let list = UIScript.preferencesList, let s = UIScript.preferencesSetup else { return [:] }
        let names: [AccountSetup.Section: String] = [.imap: "imap", .smtpPassword: "smtp-password", .smtp: "smtp", .server: "server", .importScreen: "import"]
        let act = UIScript.preferencesActivation
        // A plan as one line: `role=folder` for each role, `+` before a
        // folder to create.
        func roles(_ p: ServerPlan?) -> String {
            (p?.roles ?? []).map { "\($0.role)=\(p!.isNew($0.path) ? "+" : "")\($0.path)" }.joined(separator: ",")
        }
        let sections = s.sectionsShown.compactMap { names[$0] }
        let selection: String = {
            switch list.selection {
            case .add: return "add"
            case .account(let id): return list.items.first { $0.id == id }?.key ?? "\(id)"
            case nil: return ""
            }
        }()
        let focused: String = {
            guard let w = UIScript.preferencesWindow, let r = w.firstResponder as? NSText,
                  let field = r.delegate as? NSView else { return "" }
            return UIScript.frames.first { $0.key.hasPrefix("prefs-") && $0.value.contains(field.convert(field.bounds, to: nil).center) }?.key ?? "?"
        }()
        return [
            "prefs_accounts": list.items.map(\.key).joined(separator: ","),
            "prefs_selection": selection,
            "prefs_error": list.error ?? "",
            "prefs_focus": focused,
            "setup_email": s.email,
            "setup_name": s.displayName,
            "setup_password_set": "\(!s.password.isEmpty)",
            "setup_sections": sections.joined(separator: ","),
            "setup_imap_host": s.imapServer?.host ?? "",
            "setup_imap_reason": s.imapReason ?? "",
            "setup_smtp_host": s.smtpServer?.host ?? "",
            "setup_smtp_reason": s.smtpReason ?? "",
            "setup_smtp_password_reason": s.smtpPasswordReason ?? "",
            "setup_error": s.error ?? "",
            "setup_busy": s.busy ?? "",
            "setup_added": s.added ?? "",
            "setup_calls": (UIScript.setupCore?.calls ?? []).joined(separator: " | "),
            "setup_look_only": "\(s.plan?.lookOnly == true)",
            "setup_created": s.plan?.created.joined(separator: ",") ?? "",
            "setup_roles": roles(s.plan),
            "activation_plan": "\(act?.plan != nil)",
            "activation_import": "\(act?.importShown == true)",
            "activation_created": act?.plan?.created.joined(separator: ",") ?? "",
            "activation_roles": roles(act?.plan),
            "activation_error": act?.error ?? "",
            "activation_done": act?.done ?? "",
        ]
    }
}

private extension CGRect {
    var center: CGPoint { CGPoint(x: midX, y: midY) }
}

/// The window of UI script mode. It says it is the key window, without the
/// app ever being activated: WebKit gives its page the focus, and so the
/// keys, only in the key window, and the editor of a reply's text is a web
/// view (`EditorWebView.swift`). Nothing reaches the user's keyboard: the
/// app stays inactive, the window off screen and transparent.
final class ScriptWindow: NSWindow {
    override var isKeyWindow: Bool { true }
    override var canBecomeKey: Bool { true }
}
