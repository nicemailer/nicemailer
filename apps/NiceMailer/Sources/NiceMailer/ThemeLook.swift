import AppKit
import SwiftUI

// What the views draw differently from one theme to another
// (`specifications/themes.md`, section 6): the rows, the panels, the scroll
// bars, the separators, the animations. Each view keeps its structure and
// asks here how to draw it; geek's values give what the app always drew.
//
// Swift note: these read `ThemeStore.shared.current`, an `@Observable`
// property: a `body` that reads one is redrawn when the theme changes (the
// window is rebuilt anyway, `ThemeStore.revision`).

@MainActor
enum Look {
    static var theme: ThemeView { ThemeStore.shared.current }

    /// Height of a row of the lists (`layout.row_height`), at the current
    /// text size: 20 pt in geek for a 12 pt text.
    static var rowHeight: CGFloat { scaled(CGFloat(theme.layout.rowHeight)) }

    /// The command of the window is a toolbar (`window.chrome`), not the
    /// status bar.
    static var toolbar: Bool { theme.window.chrome == "toolbar" }

    /// The blocks of the status bar remind their key (`window.key_hints`).
    static var keyHints: Bool { theme.window.keyHints }

    /// The blocks of actions say their name beside their icon
    /// (`window.block_labels`).
    static var blockLabels: Bool { theme.window.blockLabels }

    /// Height of the status bar (`window.status_bar_height`), at the text
    /// size of the bar: 20 pt in geek for a 12 pt text.
    static var statusBarHeight: CGFloat { barScaled(CGFloat(theme.window.statusBarHeight)) }

    /// Height of a row of the menus and pickers (`layout.menu_row_height`),
    /// for a 12 pt text; nil: the height of its text (geek).
    static var menuRowHeight: CGFloat? {
        theme.layout.menuRowHeight > 0 ? CGFloat(theme.layout.menuRowHeight) : nil
    }

    /// Whether the status bar shows `block` (`window.blocks`). A block left
    /// out is hidden; its key still works (`themes.md`, section 2).
    static func shows(_ block: String) -> Bool { theme.window.blocks.contains(block) }

    /// The red, yellow and green buttons sit in the status bar
    /// (`window.title_bar = "merged"`), at its left end: the room they take.
    static var mergedButtonsWidth: CGFloat { 72 }

    /// Whether the selected row has a text color of its own
    /// (`colors.selection_text` other than `colors.text`): white on the
    /// accent in regular. Then what is colored on the row (a count, a key)
    /// takes it too, or it would vanish on the accent.
    static var strongSelection: Bool { theme.colors.selectionText != theme.colors.text }

    /// The key of a menu row on the right, as in a macOS menu
    /// (`layout.menu_keys`), rather than first, as in Helix.
    static var menuKeysRight: Bool { theme.layout.menuKeys == "right" }

    /// The lines between the areas of the window, or none
    /// (`layout.separators = "space"`): the areas are then set apart by
    /// their background and their margins.
    static var separator: Color { theme.layout.separators == "space" ? .clear : Palette.separator }

    /// Whether macOS asks for less motion (System Settings, Accessibility):
    /// every animation of the theme is then off.
    static var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }

    /// The folder column sliding in (`motion.sidebar_ms`); nil: at once.
    static var sidebarAnimation: Animation? { animation(theme.motion.sidebarMs) }

    /// Menus and pickers appearing (`motion.popup_ms`); nil: at once.
    static var popupAnimation: Animation? { animation(theme.motion.popupMs) }

    /// Whether the theme animates anything: geek animates nothing, and
    /// removes even SwiftUI's default fades (`WindowContent`).
    static var animates: Bool { sidebarAnimation != nil || popupAnimation != nil }

    private static func animation(_ ms: Int) -> Animation? {
        ms > 0 && !reduceMotion ? .easeOut(duration: Double(ms) / 1000) : nil
    }

    /// The scroll bars of the lists and of the text reader
    /// (`layout.scrollers`): none in geek; thin ones show while scrolling
    /// (macOS's overlay scroll bars), standard ones follow System Settings.
    static var scrollIndicators: ScrollIndicatorVisibility {
        theme.layout.scrollers == "none" ? .never : .automatic
    }

    /// The same, for an AppKit scroll view (the text reader).
    static func applyScrollers(to scroll: NSScrollView) {
        switch theme.layout.scrollers {
        case "none":
            scroll.hasVerticalScroller = false
        case "thin":
            scroll.hasVerticalScroller = true
            scroll.scrollerStyle = .overlay
            scroll.verticalScroller?.controlSize = .small
        default:
            scroll.hasVerticalScroller = true
            scroll.autohidesScrollers = true
        }
    }

    /// Widest text of the reader, centered (`layout.reader_max_width`, in
    /// points for a 12 pt text); nil: the whole width.
    static var readerMaxWidth: CGFloat? {
        theme.layout.readerMaxWidth > 0 ? scaled(CGFloat(theme.layout.readerMaxWidth)) : nil
    }

    /// Size of the subject at the top of the reader (`fonts.title_scale`).
    static var titleScale: CGFloat { CGFloat(theme.fonts.titleScale) }
}

extension View {
    /// The background of a row of a list: `fill` (the cursor, a marked
    /// row) or nothing, inset and rounded as the theme says
    /// (`layout.row_inset`, `row_radius`), and every other row tinted when
    /// the theme alternates them (`layout.zebra`, `index`).
    @MainActor
    func rowFill(_ fill: Color?, index: Int? = nil) -> some View {
        let layout = Look.theme.layout
        let radius = scaled(CGFloat(layout.rowRadius))
        let inset = scaled(CGFloat(layout.rowInset))
        let zebra = layout.zebra && (index ?? 0) % 2 == 1
        return background(
            ZStack {
                if zebra { Palette.statusBar.opacity(0.45) }
                if let fill {
                    RoundedRectangle(cornerRadius: radius, style: .continuous)
                        .fill(fill)
                        .padding(.horizontal, inset)
                }
            }
        )
    }

    /// The frame of a menu, a picker, a question or a panel: square with a
    /// line in geek; rounded, with a shadow, when the theme says so
    /// (`layout.popup_radius`, `popup_shadow`).
    @MainActor
    func popupChrome() -> some View {
        let layout = Look.theme.layout
        let radius = scaled(CGFloat(layout.popupRadius))
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        return background(shape.fill(Palette.popup))
            .clipShape(shape)
            .overlay(shape.stroke(Palette.separator, lineWidth: 1))
            .shadow(color: .black.opacity(layout.popupShadow ? 0.18 : 0), radius: layout.popupShadow ? scaled(14) : 0, y: layout.popupShadow ? scaled(6) : 0)
    }
}
