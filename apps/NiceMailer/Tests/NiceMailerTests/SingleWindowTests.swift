import AppKit
import Testing
@testable import NiceMailer

/// The app keeps a single window (`SingleWindow`, `ui.md`, "Fenêtres"):
/// the menu items that would open another one are found in any menu.
@MainActor
struct SingleWindowTests {
    /// A menu item with a key and an action given by its selector name.
    func item(_ title: String, _ action: String?, key: String = "",
              mods: NSEvent.ModifierFlags = .command) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action.map(NSSelectorFromString), keyEquivalent: key)
        i.keyEquivalentModifierMask = mods
        return i
    }

    /// A menu bar of two menus holding `file` and `view`.
    func bar(file: [NSMenuItem], view: [NSMenuItem]) -> NSMenu {
        let bar = NSMenu()
        for (title, items) in [("File", file), ("View", view)] {
            let menu = NSMenu(title: title)
            items.forEach(menu.addItem)
            let top = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            top.submenu = menu
            bar.addItem(top)
        }
        return bar
    }

    @Test func newWindowAndTabsAreFoundInSubmenus() {
        let menu = bar(file: [item("New Window", "_privateNewWindow:", key: "n"),
                              item("New Tab", "newWindowForTab:", key: "t")],
                       view: [item("Show Tab Bar", "toggleTabBar:"),
                              item("Show All Tabs", "toggleTabOverview:", key: "\\", mods: [.command, .shift])])
        #expect(SingleWindow.windowOpeners(in: menu) == ["New Window", "New Tab", "Show Tab Bar", "Show All Tabs"])
    }

    @Test func theOtherItemsAreLeftAlone() {
        // ⌘R, ⌘W, and an n with other modifiers (⇧⌘N) open no window.
        let menu = bar(file: [item("Close", "performClose:", key: "w"),
                              item("Something", "other:", key: "n", mods: [.command, .shift])],
                       view: [item("Reload", "reload:", key: "r"), .separator()])
        #expect(SingleWindow.windowOpeners(in: menu).isEmpty)
        #expect(SingleWindow.windowOpeners(in: nil).isEmpty)
    }

    @Test func tabsAreTurnedOff() {
        SingleWindow.disableTabs()
        #expect(NSWindow.allowsAutomaticWindowTabbing == false)
    }
}
