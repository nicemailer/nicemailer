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
