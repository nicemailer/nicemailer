import AppKit
import Testing
@testable import NiceMailer

/// The key monitor sees the keys of every window of the app: only those of
/// the main window are commands (`KeyRouting`, `ui.md`, "Clavier et menus").
/// `preferences.uiscript` plays the same rule with real keys.
@MainActor
struct KeyRoutingTests {
    private func window() -> NSWindow {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 10, height: 10), styleMask: [.borderless],
                         backing: .buffered, defer: true)
        w.isReleasedWhenClosed = false
        return w
    }

    @Test func onlyTheKeysOfTheMainWindowAreForIt() {
        let main = window()
        let preferences = window()
        #expect(KeyRouting.isForMainWindow(eventWindow: main, mainWindow: main))
        #expect(!KeyRouting.isForMainWindow(eventWindow: preferences, mainWindow: main))
    }

    @Test func aKeyOfNoKnownWindowIsLeftToAppKit() {
        let main = window()
        #expect(!KeyRouting.isForMainWindow(eventWindow: nil, mainWindow: main))
        #expect(!KeyRouting.isForMainWindow(eventWindow: main, mainWindow: nil))
        #expect(!KeyRouting.isForMainWindow(eventWindow: nil, mainWindow: nil))
    }
}
