import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Where the app looks for the core (`DataDirectory`): the same rule as the
/// core and the CLI (`nm_api::paths::resolve`).
struct DataDirectoryTests {
    let fallback = "/Users/me/Library/Application Support/NiceMailer"

    @Test func theVariableThenTheChoiceThenTheDefault() {
        #expect(DataDirectory.resolve(env: "/tmp/test", choice: "/Users/me/Archive", defaultPath: fallback) == "/tmp/test")
        #expect(DataDirectory.resolve(env: nil, choice: "  /Users/me/Archive\n", defaultPath: fallback) == "/Users/me/Archive")
        #expect(DataDirectory.resolve(env: nil, choice: nil, defaultPath: fallback) == fallback)
    }

    @Test func aBrokenChoiceIsIgnored() {
        #expect(DataDirectory.resolve(env: nil, choice: "", defaultPath: fallback) == fallback)
        #expect(DataDirectory.resolve(env: nil, choice: "relative/dir", defaultPath: fallback) == fallback)
        #expect(DataDirectory.resolve(env: "", choice: nil, defaultPath: fallback) == fallback)
    }

    /// A new private directory, standing for the default one.
    func scratch() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nm-choice-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        return dir
    }

    func mode(_ url: URL) throws -> Int {
        try (FileManager.default.attributesOfItem(atPath: url.path)[.posixPermissions] as? NSNumber)?.intValue ?? -1
    }

    /// The choice is written `0600` and read back: the core reads it with
    /// the same rule (`nm_api::paths::choice`). The app used to write it
    /// `0644`, which the core of the branch ignored.
    @Test func aChoiceIsWrittenPrivateAndReadBack() throws {
        let dir = try scratch()
        defer { try? FileManager.default.removeItem(at: dir) }
        try DataDirectory.choose(URL(fileURLWithPath: "/Users/me/Archive"), in: dir)
        let file = dir.appendingPathComponent(DataDirectory.choiceName)
        #expect(try mode(file) == 0o600)
        #expect(DataDirectory.readChoice(in: dir) == "/Users/me/Archive\n")
        // Chosen again: replaced, still private, no file left behind.
        try DataDirectory.choose(URL(fileURLWithPath: "/Users/me/Other"), in: dir)
        #expect(DataDirectory.readChoice(in: dir) == "/Users/me/Other\n")
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path) == [DataDirectory.choiceName])
        // The default directory itself: the choice is removed.
        try DataDirectory.choose(dir, in: dir)
        #expect(DataDirectory.readChoice(in: dir) == nil)
        #expect(!FileManager.default.fileExists(atPath: file.path))
    }

    /// A file others could have written is ignored, as the core ignores it;
    /// one they can only read (`0644`, the old app's) is followed.
    @Test func aChoiceOthersCouldWriteIsIgnored() throws {
        let dir = try scratch()
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent(DataDirectory.choiceName)
        try "/Users/me/Archive\n".write(to: file, atomically: false, encoding: .utf8)
        for (mode, followed) in [(0o644, true), (0o600, true), (0o664, false), (0o666, false), (0o602, false)] {
            try FileManager.default.setAttributes([.posixPermissions: mode], ofItemAtPath: file.path)
            #expect((DataDirectory.readChoice(in: dir) != nil) == followed, "mode \(String(mode, radix: 8))")
        }
        // A directory others can write in: whoever can replace the file.
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        try FileManager.default.setAttributes([.posixPermissions: 0o777], ofItemAtPath: dir.path)
        #expect(DataDirectory.readChoice(in: dir) == nil)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
        #expect(DataDirectory.readChoice(in: dir) != nil)
    }

    /// A symbolic link is never followed, even towards a good file.
    @Test func aLinkIsNotAChoice() throws {
        let dir = try scratch()
        defer { try? FileManager.default.removeItem(at: dir) }
        let target = dir.appendingPathComponent("target")
        try "/Users/me/Archive\n".write(to: target, atomically: false, encoding: .utf8)
        try FileManager.default.createSymbolicLink(at: dir.appendingPathComponent(DataDirectory.choiceName), withDestinationURL: target)
        #expect(DataDirectory.readChoice(in: dir) == nil)
    }

    @Test func onlyTheUserWritingAloneIsTrusted() {
        #expect(DataDirectory.untrusted(owner: 501, mode: 0o100644, user: 501) == nil)
        #expect(DataDirectory.untrusted(owner: 501, mode: 0o100664, user: 501) != nil)
        #expect(DataDirectory.untrusted(owner: 502, mode: 0o100600, user: 501) != nil)
    }

    @Test func theNameShownIsTheFoldersOwn() {
        #expect(DataDirectory.displayName(URL(fileURLWithPath: "/Users/me/Library/Application Support/NiceMailer-archive")) == "NiceMailer-archive")
    }

    @Test func theTestsAreNeverAnAgent() {
        // `swift test` runs a bare executable: no agent, the core by hand.
        #expect(!CoreAgent.isBundled)
        #expect(CoreAgent.register() == .notBundled)
    }
}

/// The title bar of the window, as `ui.title_bar` of `config.toml` says
/// (`WindowChrome`).
@MainActor
struct WindowChromeTests {
    @Test func hiddenThenShown() {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300),
                         styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: true)
        let container = WindowChrome.titlebarContainer(of: w)
        #expect(container != nil)
        // Hidden, shown, hidden again: the last state is the first one (a
        // window changed back and forth kept a band over the content).
        for _ in 0..<2 {
            WindowChrome.apply(hidden: true, to: w)
            #expect(w.styleMask.contains(.fullSizeContentView))
            #expect(w.titlebarAppearsTransparent)
            #expect(w.titleVisibility == .hidden)
            #expect(w.standardWindowButton(.closeButton)?.isHidden == true)
            #expect(w.standardWindowButton(.zoomButton)?.isHidden == true)
            #expect(container?.isHidden == true)
            // Still a real window: resizable, closable.
            #expect(w.styleMask.contains(.resizable) && w.styleMask.contains(.closable))
            #expect(WindowChrome.matches(hidden: true, w))
            #expect(!WindowChrome.matches(hidden: false, w))
            // Shown again behind our back (SwiftUI at launch): seen.
            w.standardWindowButton(.closeButton)?.isHidden = false
            #expect(!WindowChrome.matches(hidden: true, w))
            WindowChrome.apply(hidden: true, to: w)
            #expect(WindowChrome.matches(hidden: true, w))
            WindowChrome.apply(hidden: false, to: w)
            #expect(WindowChrome.matches(hidden: false, w))
            #expect(w.titleVisibility == .visible)
            #expect(w.standardWindowButton(.closeButton)?.isHidden == false)
            #expect(container?.isHidden == false)
            #expect(!w.titlebarAppearsTransparent)
        }
    }

    @Test func theSettingComesFromTheCore() async {
        let core = FakeCore { method, params in
            method == "config_status" ? ["ui": ["title_bar": "hidden"]] : Fixture.answer(method, params)
        }
        let model = AppModel()
        // Shown by default, until the core says otherwise.
        #expect(!model.titleBarHidden)
        // The session reads the settings with the first list.
        await model.attach(core)
        #expect(model.titleBarHidden)
    }

    /// A `config.toml` without `ui.title_bar` (the theme says it now,
    /// `themes.md`): the settings still decode, and the bar is shown.
    @Test func noTitleBarSettingLeavesTheBarShown() async {
        let core = FakeCore { method, params in
            method == "config_status" ? ["ui": ["title_bar": NSNull(), "theme": "clean", "auto_confirm_calls": true]] : Fixture.answer(method, params)
        }
        let model = AppModel()
        await model.attach(core)
        #expect(!model.titleBarHidden)
        // The rest of `[ui]` was read: the decoding did not fail.
        #expect(model.autoConfirmCalls)
    }
}

/// The window's buttons in a bar of the theme (`window.title_bar =
/// "merged"`): centered in its height, aligned with its icons and names.
@MainActor
struct MergedTitleBarTests {
    /// A titled window, never shown: AppKit lays its title bar out anyway.
    func window() -> NSWindow {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 500),
                         styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        w.isReleasedWhenClosed = false
        return w
    }

    @Test func theButtonsAreInTheMiddleOfTheBar() throws {
        let w = window()
        WindowChrome.apply(.merged, to: w)
        #expect(w.standardWindowButton(.closeButton)?.isHidden == false)
        #expect(w.titleVisibility == .hidden && w.titlebarAppearsTransparent)
        for height: CGFloat in [40, 44, 30] {
            WindowChrome.centerButtons(in: w, barHeight: height)
            #expect(WindowChrome.buttonsCentered(in: w, barHeight: height), "bar of \(height)")
            let frameView = try #require(w.contentView?.superview)
            for kind in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
                let b = try #require(w.standardWindowButton(kind))
                let center = b.convert(NSPoint(x: b.bounds.midX, y: b.bounds.midY), to: frameView)
                #expect(abs(center.y - (frameView.bounds.height - height / 2)) <= 1, "\(kind) in a bar of \(height)")
            }
        }
        // Hidden: no buttons; shown: a title bar again.
        WindowChrome.apply(.hidden, to: w)
        #expect(w.standardWindowButton(.closeButton)?.isHidden == true)
        WindowChrome.apply(.shown, to: w)
        #expect(w.titleVisibility == .visible && !w.titlebarAppearsTransparent)
    }
}
