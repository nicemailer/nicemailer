import AppKit
import SwiftUI

/// Which window a key belongs to (`ui.md`, "Clavier et menus": the keys go
/// to the key window only).
///
/// The key monitor of the app (`RootView`) sees the keys of *every*
/// window of the process: the Preferences, the open and save panels, the
/// windows of the updater. Only those of the main window are commands; in
/// any other window a letter is text for the field being edited.
enum KeyRouting {
    /// Whether a key typed in `eventWindow` is for the main window. A key
    /// of no known window is left to AppKit, like one of another window: a
    /// command run by mistake is worse than a key ignored.
    ///
    /// Swift note: `===` compares object identity (the same window), where
    /// `==` would compare values.
    static func isForMainWindow(eventWindow: NSWindow?, mainWindow: NSWindow?) -> Bool {
        guard let eventWindow, let mainWindow else { return false }
        return eventWindow === mainWindow
    }
}

extension InterfaceController {
    /// What the key monitor does with a key: nil when the controller took
    /// it, the event when AppKit must go on with it (the field of another
    /// window, the menus).
    func route(_ event: NSEvent, mainWindow: NSWindow?) -> NSEvent? {
        guard KeyRouting.isForMainWindow(eventWindow: event.window, mainWindow: mainWindow) else { return event }
        return handle(event) ? nil : event
    }
}

/// The window `RootView` is shown in, for the key monitor.
///
/// Swift note: `weak` does not keep the window alive, and becomes nil by
/// itself once the window is gone. A class, so that the monitor's closure and
/// the view below share one object.
@MainActor
final class MainWindowReference {
    weak var window: NSWindow?
}

/// Records the window it is in. SwiftUI does not give a view its window: an
/// `NSViewRepresentable` puts an AppKit view in the hierarchy, and that view
/// is told when it joins one. It draws nothing.
struct MainWindowReader: NSViewRepresentable {
    let reference: MainWindowReference

    func makeNSView(context: Context) -> ReaderView {
        let view = ReaderView()
        view.reference = reference
        return view
    }

    func updateNSView(_ view: ReaderView, context: Context) {
        view.reference = reference
    }

    final class ReaderView: NSView {
        var reference: MainWindowReference?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            reference?.window = window
        }
    }
}
