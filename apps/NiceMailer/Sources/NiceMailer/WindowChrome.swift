import AppKit
import SwiftUI

/// The title bar of the window (its title and the red, orange and green
/// buttons), hidden or shown as `ui.title_bar` of `config.toml` says
/// (`docs/geek-ui.md`, "Window").
///
/// Hidden, the window keeps everything else of a macOS window: it is
/// resized by its edges, moved by the status bar (`WindowDragArea`),
/// closed with ⌘W, minimized with ⌘M, quit with ⌘Q, put in full screen
/// with ⌃⌘F. Only what is drawn changes: the content goes up to the top
/// edge, under a title bar made transparent and empty.
///
/// Swift note: an `NSViewRepresentable` puts an AppKit view in a SwiftUI
/// hierarchy; this one draws nothing, it only reaches the window it is in.
struct WindowChrome: NSViewRepresentable {
    /// What the title bar is (`window.title_bar` of the theme).
    enum Mode: String {
        /// A title bar, with the title and the buttons.
        case shown
        /// No title bar at all: the content goes up to the top edge.
        case hidden
        /// No title bar, but the red, yellow and green buttons, over the
        /// left end of the status bar, which leaves them room.
        case merged
    }

    let mode: Mode
    /// The window's title (the Window menu, Mission Control), with the
    /// mailbox in use when it is not the default one. Set on the window
    /// itself: SwiftUI's `.navigationTitle` would add a toolbar, whose
    /// opaque background covers the top of the content even with the title
    /// bar transparent.
    var title = "NiceMailer"

    func makeNSView(context: Context) -> ChromeView { ChromeView() }

    func updateNSView(_ view: ChromeView, context: Context) {
        view.mode = mode
        view.title = title
        view.apply()
    }

    final class ChromeView: NSView {
        var mode = Mode.shown
        var title = "NiceMailer"
        private var observers: [NSObjectProtocol] = []

        // The view is in a window only after it was made: apply then too,
        // and check again at each update of the window. At launch SwiftUI
        // finishes setting the window up after this view joined it, and
        // showed the title bar again; AppKit may too (full screen, a
        // resize). The check costs nothing when the window is as wanted
        // (`matches`): it only acts on a difference.
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
            if let window {
                let names: [Notification.Name] = [
                    NSWindow.didUpdateNotification, NSWindow.didBecomeKeyNotification, NSWindow.didResizeNotification,
                    NSWindow.didExitFullScreenNotification, NSWindow.didBecomeMainNotification,
                ]
                observers = names.map { name in
                    NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.apply() }
                    }
                }
            }
            apply()
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }

        func apply() {
            guard let window, !UIScript.isActive else { return }
            if window.title != title { window.title = title }
            if !WindowChrome.matches(mode, window) {
                WindowChrome.apply(mode, to: window)
            }
        }
    }

    /// Hides or shows the title bar.
    ///
    /// The content always runs under the title bar (`fullSizeContentView`,
    /// never removed: changing the style of a window back and forth left a
    /// blank band over the content), and SwiftUI keeps it below the title
    /// bar when the bar is shown (its safe area, `NiceMailerApp`). Hidden,
    /// the title bar's own container view is hidden, as Ghostty does: it
    /// then draws nothing, not even a transparent band, and takes no click,
    /// so the status bar under it gets them all.
    ///
    /// Merged, the title bar is transparent and without its title, as when
    /// hidden, but its container and its buttons stay: they are drawn over
    /// the left end of the status bar, which keeps them room.
    static func apply(_ mode: Mode, to window: NSWindow) {
        let bare = mode != .shown
        window.styleMask.insert(.fullSizeContentView)
        // Full screen (⌃⌘F) whether the title bar is shown or not.
        window.collectionBehavior.insert(.fullScreenPrimary)
        // No toolbar: its background would be drawn over the content.
        if bare { window.toolbar = nil }
        window.titlebarAppearsTransparent = bare
        window.titleVisibility = bare ? .hidden : .visible
        for kind in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            window.standardWindowButton(kind)?.isHidden = mode == .hidden
        }
        titlebarContainer(of: window)?.isHidden = mode == .hidden
    }

    /// The same, as a switch: hidden or shown.
    static func apply(hidden: Bool, to window: NSWindow) {
        apply(hidden ? .hidden : .shown, to: window)
    }

    /// Whether the window already shows (or hides) its title bar as asked:
    /// then nothing is done.
    static func matches(_ mode: Mode, _ window: NSWindow) -> Bool {
        let bare = mode != .shown, hidden = mode == .hidden
        return window.styleMask.contains(.fullSizeContentView)
            && window.collectionBehavior.contains(.fullScreenPrimary)
            && window.titlebarAppearsTransparent == bare
            && (window.titleVisibility == .hidden) == bare
            && (window.standardWindowButton(.closeButton)?.isHidden ?? hidden) == hidden
            && (titlebarContainer(of: window)?.isHidden ?? hidden) == hidden
            && (!bare || window.toolbar == nil)
    }

    /// The same, as a switch: hidden or shown.
    static func matches(hidden: Bool, _ window: NSWindow) -> Bool {
        matches(hidden ? .hidden : .shown, window)
    }

    /// The view that holds the title bar (AppKit's private
    /// `NSTitlebarContainerView`), found among the views of the window's
    /// frame; nil if a future macOS names it otherwise (the bar then stays
    /// transparent and empty).
    static func titlebarContainer(of window: NSWindow) -> NSView? {
        let isTitlebar = { (v: NSView) in String(describing: type(of: v)).contains("TitlebarContainer") }
        if let found = window.contentView?.superview?.subviews.first(where: isTitlebar) { return found }
        // Else the view two levels above the close button, if it is one.
        return window.standardWindowButton(.closeButton)?.superview?.superview.flatMap { isTitlebar($0) ? $0 : nil }
    }
}

/// A zone that moves the window when dragged, and zooms it when
/// double-clicked, as a title bar does: the empty parts of the status bar,
/// when the title bar is hidden. The blocks of the status bar keep their
/// clicks: they are drawn over it.
struct WindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> DragView { DragView() }
    func updateNSView(_ view: DragView, context: Context) {}

    final class DragView: NSView {
        override var mouseDownCanMoveWindow: Bool { true }

        override func mouseDown(with event: NSEvent) {
            if event.clickCount == 2 {
                window?.performZoom(nil)
            } else {
                window?.performDrag(with: event)
            }
        }
    }
}
