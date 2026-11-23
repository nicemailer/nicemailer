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

    /// The attachments of a message as a row each, with the icon macOS gives
    /// their type and their whole name (`layout.attachments = "rows"`,
    /// clean and regular), rather than geek's tiles.
    static var attachmentRows: Bool { theme.layout.attachments == "rows" }

    /// The command of the window is a toolbar (`window.bar_type`), not the
    /// status bar.
    static var toolbar: Bool { theme.window.barType == "toolbar" }

    /// The blocks of the status bar remind their key (`window.key_hints`).
    static var keyHints: Bool { theme.window.keyHints }

    /// Each block of the status bar in a pill, the account and the folder
    /// in bold, the block whose menu is open highlighted
    /// (`window.block_style = "pill"`, clean).
    static var pillBlocks: Bool { theme.window.blockStyle == "pill" }

    /// A line between the status bar and the mails (`window.bar_line`).
    static var barLine: Bool { theme.window.barLine }

    /// The blocks of actions say their name beside their icon
    /// (`window.block_labels`).
    static var blockLabels: Bool { theme.window.blockLabels }

    /// Factor of the icons, texts and room of the status bar or toolbar
    /// (`window.bar_size`), on top of its text size.
    static var barScale: CGFloat { barScale(theme) }

    /// The factor of `window.bar_size` in `theme`: small, medium or large,
    /// as the sizes of a Mac toolbar. A proportion, not a size: the text
    /// size stays the user's (`ui.status_bar_text_size`, ⌘+ ⌘−), as every
    /// size in a theme (`themes.md`, section 5.3). medium, the three
    /// built-in themes, is what the bar always drew.
    static func barScale(_ theme: ThemeView) -> CGFloat {
        switch theme.window.barSize {
        case "small": 0.85
        case "large": 1.2
        default: 1
        }
    }

    /// Height of the status bar (`window.bar_height`), at the text
    /// size of the bar: 20 pt in geek for a 12 pt text.
    static var barHeight: CGFloat { barScaled(CGFloat(theme.window.barHeight)) }

    /// What the theme adds to the margins of geek inside menus, pickers and
    /// panels (`layout.popup_padding`, 10 in geek): 0 in geek, 8 in clean,
    /// whose rounded corners would otherwise touch the title.
    static var popupExtra: CGFloat { max(0, CGFloat(theme.layout.popupPadding) - 10) }

    /// The panels are rounded (`layout.popup_radius`): clean and regular.
    /// Their menus then title their rows as a macOS menu does a section, a
    /// small grey heading over a line, and leave room inside the
    /// highlighted row (`KeyMenu`).
    static var roundedPanels: Bool { theme.layout.popupRadius > 0 }

    /// Room between the edge of the folder column and the text of its rows
    /// (folders, groups, titles) in the current theme.
    static var sidebarTextInset: CGFloat { sidebarTextInset(theme) }

    /// The same, in `theme`: 8 pt in geek, whose highlight is the whole
    /// row. With a rounded highlight inset from the edges (clean, regular:
    /// `layout.row_inset`, `row_radius`), the inset of the highlight plus
    /// 10 pt inside it, as in the menu `;` (`menuRowTextInset`): the text
    /// no longer touches the rounded edge.
    static func sidebarTextInset(_ theme: ThemeView) -> CGFloat {
        guard theme.layout.rowRadius > 0 else { return 8 }
        return scaled(CGFloat(theme.layout.rowInset)) + scaled(10)
    }

    /// Room between the edge of the highlighted row of a menu and its text
    /// (the label on the left, the key on the right): none in geek, whose
    /// highlight is the whole row.
    static var menuRowTextInset: CGFloat { roundedPanels ? scaled(10) : 0 }

    /// Height of the bar at the top of the window when the title bar is
    /// merged into it: the toolbar (at least 38 pt) or the status bar (at
    /// least 28 pt, the height of a title bar). The window's buttons are
    /// centered in it (`WindowChrome.centerButtons`).
    static var mergedBarHeight: CGFloat {
        toolbar ? max(barHeight, 38) : max(barHeight, 28)
    }

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
    static var strongSelection: Bool { strongSelection(theme) }

    /// The same, for `theme`.
    static func strongSelection(_ theme: ThemeView) -> Bool { theme.colors.selectionText != theme.colors.text }

    /// The color of a text or an icon drawn on a selection (`selected`):
    /// on a fill of the accent (`strongSelection`, regular), the text color
    /// of the selection, white, whatever its own color (`normal`): black,
    /// grey, the accent or the unread color would be lost in the blue; a
    /// secondary text (`faint`: a date, a count, a key) a little fainter,
    /// as in a macOS list. Elsewhere, or not selected, its own color.
    static func ink(_ normal: Color, selected: Bool, faint: Bool = false) -> Color {
        ink(normal, selected: selected, faint: faint, strong: strongSelection)
    }

    /// The same, for a theme whose selection is `strong` or not.
    static func ink(_ normal: Color, selected: Bool, faint: Bool, strong: Bool) -> Color {
        guard selected && strong else { return normal }
        return faint ? Palette.selectionText.opacity(0.8) : Palette.selectionText
    }

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

    /// Room around the subject at the top of the reader (`ReaderTitleBar`)
    /// in the current theme.
    static var readerTitleInsets: EdgeInsets { readerTitleInsets(theme) }

    /// Room around the subject at the top of the reader in `theme`
    /// (`layout.reader_title_padding`, at the current text size): that
    /// much above and below it, and as much again beyond the 12 pt of geek
    /// on its sides. 0 in geek and regular, whose line under the title
    /// already sets it apart: 12 pt on the sides alone, as always. 12 in
    /// clean, which draws no line: without room, its big subject
    /// (`fonts.title_scale`) touches the status bar above and the edge of
    /// the window on the left.
    ///
    /// Swift note: `EdgeInsets` holds the four margins of a view; passed to
    /// `.padding(_:)`, each side gets its own.
    static var readerTitleLineGap: CGFloat { readerTitleLineGap(theme) }

    /// Corner radius of the small buttons of the reader's title line (the
    /// text | html | images switch, unsubscribe) in the current theme.
    static var titleControlRadius: CGFloat { titleControlRadius(theme) }

    /// Corner radius of the small buttons of the reader's title line in
    /// `theme`: the rounding of the rows of the list (`layout.row_radius`)
    /// in proportion to their height (`layout.row_height`), brought to the
    /// height of a button, about 16 pt for a 12 pt text. The buttons are
    /// then as rounded as the rows, for their size: square in geek, 4 pt in
    /// clean (8 on rows of 30), 4 pt in regular (6 on rows of 24).
    static func titleControlRadius(_ theme: ThemeView) -> CGFloat {
        let row = theme.layout.rowHeight
        guard row > 0 else { return 0 }
        return scaled(16 * CGFloat(theme.layout.rowRadius) / CGFloat(row))
    }

    /// Gap between the subject and the line of the controls when the title
    /// takes two lines (`ReaderTitleBar`): half of
    /// `layout.reader_title_padding`, so that the two lines stay closer to
    /// each other than to the edges around them, and grow apart with them.
    /// 0 in geek, 6 pt in clean, 16 pt at the largest padding (32).
    static func readerTitleLineGap(_ theme: ThemeView) -> CGFloat {
        scaled(CGFloat(theme.layout.readerTitlePadding)) / 2
    }

    static func readerTitleInsets(_ theme: ThemeView) -> EdgeInsets {
        let pad = scaled(CGFloat(theme.layout.readerTitlePadding))
        return EdgeInsets(top: pad, leading: 12 + pad, bottom: pad, trailing: 12 + pad)
    }
}

extension View {
    /// The background of a row of a list: `fill` (the cursor, a marked
    /// row) or nothing, inset and rounded as the theme says
    /// (`layout.row_inset`, `row_radius`), and every other row tinted when
    /// the theme alternates them (`layout.zebra`, `index`).
    @MainActor
    func rowFill(_ fill: Color?, index: Int? = nil, inset: CGFloat? = nil) -> some View {
        let layout = Look.theme.layout
        let radius = scaled(CGFloat(layout.rowRadius))
        let inset = inset ?? scaled(CGFloat(layout.rowInset))
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
