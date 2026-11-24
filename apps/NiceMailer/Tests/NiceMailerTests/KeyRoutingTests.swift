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

    /// SwiftUI replaces the reader view of the main window: the new one
    /// joins the window, then the old one leaves it. The main window must
    /// stay known, or every key of the app is ignored.
    @Test func theMainWindowStaysKnownWhenItsReaderViewIsReplaced() {
        let main = window()
        let reference = MainWindowReference()
        let old = MainWindowReader.ReaderView()
        old.reference = reference
        main.contentView?.addSubview(old)
        #expect(reference.window === main)
        let new = MainWindowReader.ReaderView()
        new.reference = reference
        main.contentView?.addSubview(new)
        old.removeFromSuperview()
        #expect(reference.window === main, "the old view leaving does not forget the window")
    }

    @Test func aKeyOfNoKnownWindowIsLeftToAppKit() {
        let main = window()
        #expect(!KeyRouting.isForMainWindow(eventWindow: nil, mainWindow: main))
        #expect(!KeyRouting.isForMainWindow(eventWindow: main, mainWindow: nil))
        #expect(!KeyRouting.isForMainWindow(eventWindow: nil, mainWindow: nil))
    }

    /// SwiftUI makes a new reader of the window (a modifier of the window
    /// changed: a new theme's title bar) and removes the old one after the
    /// new one joined: the window recorded stays, else no key would reach
    /// the app any more.
    @Test func anOldReaderLeavingKeepsTheWindow() {
        let main = window()
        let reference = MainWindowReference()
        let old = MainWindowReader.ReaderView()
        old.reference = reference
        main.contentView?.addSubview(old)
        #expect(reference.window === main)
        let new = MainWindowReader.ReaderView()
        new.reference = reference
        main.contentView?.addSubview(new)
        old.removeFromSuperview()
        #expect(reference.window === main)
        // The last one leaving too: the window stays recorded (the app
        // quits with its last window anyway).
        new.removeFromSuperview()
        #expect(reference.window === main)
    }
}
