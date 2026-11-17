import AppKit

/// The app has a single window, for now (`ui.md`, "Fenêtres").
///
/// Every window of the app shares one `AppModel`: the folder shown, the
/// selected mail, the search and the reply being written. A second window
/// was therefore only a mirror of the first, and opening it reconnected to
/// the core (its `.task` calls `connect()` again). Until each window has a
/// model of its own, nothing in the menus opens another window:
/// - File > New Window (⌘N), removed in `NiceMailerApp` with
///   `CommandGroup(replacing: .newItem)`;
/// - the window tabs of macOS (View > Show Tab Bar, Window > Show All Tabs,
///   the "+" of the tab bar), turned off by `disableTabs()`.
@MainActor
enum SingleWindow {
    /// Turns off the window tabs. Swift note: `allowsAutomaticWindowTabbing`
    /// is a class property of `NSWindow`, for the whole app; set before the
    /// first window appears, AppKit never adds the tab items to the menus.
    static func disableTabs() {
        NSWindow.allowsAutomaticWindowTabbing = false
    }

    /// Actions of the menu items that open a window or a tab: AppKit's
    /// standard ones (`newWindowForTab:` is the "+" of the tab bar and
    /// File > New Tab, the toggles show the tabs that would offer it).
    static let openingActions: Set<String> = [
        "newWindowForTab:", "toggleTabBar:", "toggleTabOverview:",
        "mergeAllWindows:", "moveTabToNewWindow:",
    ]

    /// Titles of the items of `menu`, submenus included, that would open
    /// another window: an action of `openingActions`, or ⌘N, the key of
    /// SwiftUI's File > New Window (whose action is private to SwiftUI, so
    /// it is known by its key). Empty when the app keeps a single window.
    /// A pure function of the menu, so the unit tests build their own.
    static func windowOpeners(in menu: NSMenu?) -> [String] {
        guard let menu else { return [] }
        var titles: [String] = []
        for item in menu.items {
            let action = item.action.map(NSStringFromSelector)
            let commandN = item.keyEquivalent == "n"
                && item.keyEquivalentModifierMask.intersection(.deviceIndependentFlagsMask) == .command
            if commandN || action.map(openingActions.contains) == true {
                titles.append(item.title)
            }
            titles += windowOpeners(in: item.submenu)
        }
        return titles
    }
}
