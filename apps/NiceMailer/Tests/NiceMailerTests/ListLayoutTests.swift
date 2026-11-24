import Foundation
import Testing
@testable import NiceMailer

/// The two presentations of the list (`ListLayout`, `ui.md`, "Liste",
/// "Deux présentations"): pure functions.
@MainActor
struct ListLayoutTests {
    @Test func vlGoesFromOnePresentationToTheOther() {
        #expect(ListLayout.columns.toggled == .stacked)
        #expect(ListLayout.stacked.toggled == .columns)
        #expect(ListLayout.columns.toggled.toggled == .columns)
    }

    @Test func theChoiceIsStoredAsItsName() {
        // `UserDefaults` keeps the raw value; a later version's unknown
        // value gives nil, which the controller replaces by columns.
        #expect(ListLayout(rawValue: "stacked") == .stacked)
        #expect(ListLayout(rawValue: "columns") == .columns)
        #expect(ListLayout(rawValue: "cards") == nil)
    }

    @Test func theSnippetHoldsOnOneLine() {
        // Line breaks and runs of spaces become one space.
        #expect(ListLayout.snippetLine("Bonjour Alex,\n\nEst-ce que   tu as\tpu") == "Bonjour Alex, Est-ce que tu as pu")
        #expect(ListLayout.snippetLine("  au début et à la fin \n") == "au début et à la fin")
        // None, or only white space: an empty line, the row keeps its height.
        #expect(ListLayout.snippetLine(nil) == "")
        #expect(ListLayout.snippetLine(" \n\t ") == "")
    }

    @Test func aStackedRowHoldsThreeLinesAndItsPadding() {
        // Three lines of 15 and 7 above and below: 59.
        #expect(ListLayout.stackedRowHeight(lineHeight: 15, padding: 7) == 59)
        // Rounded up to a whole point.
        #expect(ListLayout.stackedRowHeight(lineHeight: 14.2, padding: 7) == 57)
    }

    @Test func aStackedRowIsTallerThanARowInColumnsInEveryTheme() {
        // The rows of a theme are at most 30 pt (clean); three lines of a
        // 12 pt text take more, so `ctrl-d` counts fewer rows in stacked.
        #expect(ListLayout.rowHeight(.stacked) > ListLayout.rowHeight(.columns))
        #expect(ListLayout.rowHeight(.stacked) > 3 * scaled(12))
        let height: CGFloat = 600
        #expect(InterfaceController.visibleRows(height: height, rowHeight: ListLayout.rowHeight(.stacked))
            < InterfaceController.visibleRows(height: height, rowHeight: ListLayout.rowHeight(.columns)))
    }
}

/// The presentation the theme gives (`window.list_layout`) until the user
/// chooses with `v` `l`, and the width of the list each presentation keeps.
@MainActor
struct ListLayoutChoiceTests {
    func controller(_ theme: String, defaults: UserDefaults = TestDefaults.make()) -> InterfaceController {
        let model = AppModel()
        model.theme = ThemeStore(ThemeStore.builtins[theme]!)
        return InterfaceController(model: model, defaults: defaults, textSize: TextSize())
    }

    @Test func theBuiltInThemesGiveTheirPresentation() {
        #expect(ThemeStore.builtins["geek"]!.window.listLayout == "columns")
        #expect(ThemeStore.builtins["clean"]!.window.listLayout == "stacked")
        #expect(ThemeStore.builtins["regular"]!.window.listLayout == "stacked")
        #expect(controller("geek").listLayout == .columns)
        #expect(controller("clean").listLayout == .stacked)
        #expect(controller("regular").listLayout == .stacked)
    }

    @Test func aNewThemeGivesItsPresentationUntilTheUserChooses() {
        let defaults = TestDefaults.make()
        let c = controller("clean", defaults: defaults)
        #expect(c.listLayout == .stacked)
        // Another theme, nothing chosen: its presentation, not remembered.
        c.model.theme.apply(ThemeStore.builtins["geek"]!)
        c.applyThemeLayout()
        #expect(c.listLayout == .columns)
        #expect(defaults.string(forKey: ListLayout.defaultsKey) == nil)
        // Chosen with v l: kept whatever the theme.
        c.perform(.viewToggleListLayout)
        #expect(c.listLayout == .stacked)
        c.model.theme.apply(ThemeStore.builtins["geek"]!)
        c.applyThemeLayout()
        #expect(c.listLayout == .stacked)
        #expect(controller("geek", defaults: defaults).listLayout == .stacked)
    }

    @Test func eachPresentationKeepsItsWidthOfTheList() {
        let defaults = TestDefaults.make()
        let c = controller("geek", defaults: defaults)
        // Until dragged: a third in columns, a quarter in stacked.
        #expect(c.listLayout == .columns && c.listFraction == 0.32)
        c.listFraction = 0.4
        c.perform(.viewToggleListLayout)
        #expect(c.listLayout == .stacked && c.listFraction == 0.24)
        c.listFraction = 0.2
        c.perform(.viewToggleListLayout)
        #expect(c.listFraction == 0.4)
        c.perform(.viewToggleListLayout)
        #expect(c.listFraction == 0.2)
        // Remembered: a new window finds both.
        let again = controller("geek", defaults: defaults)
        #expect(again.listLayout == .stacked && again.listFraction == 0.2)
        again.perform(.viewToggleListLayout)
        #expect(again.listFraction == 0.4)
        // The width of columns keeps the key it always had.
        #expect(defaults.double(forKey: "geek.listFraction") == 0.4)
    }
}
