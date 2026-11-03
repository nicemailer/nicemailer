import AppKit
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
        let w = NSWindow(contentRect: NSRect(x: -30_000, y: -30_000, width: 1400, height: 900),
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
        Task { @MainActor in
            let runner = Runner(model: model, controller: controller)
            let failures = await runner.run(path: path)
            exit(failures == 0 ? 0 : 1)
        }
    }

    static var testWindow: NSWindow? { window }

    /// Records the frame of a clickable row (see `View.uiTarget`).
    static func record(_ id: String, _ globalFrame: CGRect) {
        guard let content = window?.contentView else { return }
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
/// | `key K` | one key: a character, or `esc` `ret` `del` `up` `down` `space`, or `ctrl-X` / `cmd-X` |
/// | `type TEXT` | each character of TEXT as a key |
/// | `click TARGET [cmd] [shift]` | a mouse click on a row |
/// | `dblclick TARGET` | two clicks |
/// | `resize WxH` | window size |
/// | `snap NAME` | PNG of the window in the output directory |
/// | `wait MS` | pause |
/// | `expect FIELD OP VALUE` | check the state (`=`, `!=`, `~` contains, `!~` does not contain, `>`, `<`), waiting up to 5 s |
///
/// Targets: `thread:N` (N-th row of the list), `search:N`, `picker:N`,
/// `screen:N`, `folder:NAME` (folder of the current view), `reader-link:N` (N-th header or
/// "···" line of the reader).
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

    var window: NSWindow? { UIScript.testWindow }

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
        case "click", "dblclick":
            let words = arg.split(separator: " ").map(String.init)
            var mods: NSEvent.ModifierFlags = []
            if words.contains("cmd") { mods.insert(.command) }
            if words.contains("shift") { mods.insert(.shift) }
            guard let target = words.first, let point = await locate(target) else {
                fail(lineNumber, line, "target not found")
                return
            }
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
        default:
            if name.hasPrefix("ctrl-") {
                mods = .control
                chars = String(name.dropFirst(5))
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
        if target is NSTextView {
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

    /// Center of a target row, waiting for it to be laid out.
    func locate(_ target: String) async -> CGPoint? {
        let parts = target.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
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
        let pickerCount: Int = {
            if case .picker(let k) = c.overlay { return c.pickerEntries(k).count }
            return 0
        }()
        return [
            "screen": "\(c.screen)",
            "overlay": "\(c.overlay)",
            "mode": c.modeLabel,
            "location": c.locationLabel,
            "flash": m.flash ?? "",
            "error": m.lastError ?? "",
            "thread_count": "\(m.threads.count)",
            "selected": index.map(String.init) ?? "-1",
            "selected_subject": sel?.subject ?? "",
            "selected_unread": "\(sel?.unreadCount ?? 0)",
            "selected_flagged": "\(sel?.isFlagged ?? false)",
            "selected_tags": sel?.tags.joined(separator: ",") ?? "",
            "marked": markedIdx.map(String.init).joined(separator: ","),
            "marked_count": "\(m.marked.count)",
            "folder": m.currentFolderName,
            "account": m.scopeName,
            "search_text": m.searchText,
            "search_count": "\(m.searchResults.count)",
            "search_index": "\(c.searchIndex)",
            "picker_query": c.pickerQuery,
            "picker_index": "\(c.pickerIndex)",
            "picker_count": "\(pickerCount)",
            "screen_index": "\(c.screenIndex)",
            "contacts_count": "\(c.contacts.count)",
            "history_count": "\(c.history.count)",
            "tags_count": "\(c.tags.count)",
            "groups_count": "\(c.groups.count)",
            "columns": "\(c.columns)",
            "status_position": c.statusPosition.rawValue,
            "sort": m.sort,
            "body_format": m.bodyFormat,
            "reader_subject": m.openThread?.subject ?? "",
            "reader_offset": "\(Int(c.readerScroll?.contentView.bounds.origin.y ?? 0))",
            "list_filter": m.listFilter.map { "\($0)" } ?? "",
            "export_files": exports,
            "font_size": "\(Int(TextSize.shared.size))",
            "reader_text": UIScript.readerText,
            "reader_links": "\(UIScript.readerLinks)",
            "reader_folded": "\(m.openThread.map { ThreadDisplay.foldedCount(ThreadDisplay.blocks($0, c.readerState)) } ?? 0)",
            "show_all": "\(c.readerState.showAll)",
            "filters": m.attributes.chips.map(\.label).joined(separator: ", "),
            // Position of the first row of the open menu, in window
            // coordinates (origin bottom-left): where the menu is drawn.
            "menu_x": "\(Int(UIScript.frames["menu-0"]?.minX ?? -1))",
            "menu_y": "\(Int(UIScript.frames["menu-0"]?.minY ?? -1))",
            // Height of the selected row, as laid out.
            "row_height": "\(Int((UIScript.frames["thread-\(m.selectedThreadId ?? -1)"]?.height ?? 0).rounded()))",
            "html_text": UIScript.html["text"] ?? "",
            "html_font_loaded": UIScript.html["font"] ?? "",
            "html_font_family": UIScript.html["family"] ?? "",
        ]
    }
}
