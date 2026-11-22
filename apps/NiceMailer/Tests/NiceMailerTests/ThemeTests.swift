import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The themes in the app (`docs/themes.md`): the built-in ones embedded,
/// their colors, the theme received from the core and followed.
@MainActor
struct ThemeTests {
    /// The built-in theme `name` as the core sends it, editable: the JSON
    /// the app embeds.
    nonisolated static func themeJSON(_ name: String) -> [String: Any] {
        let url = AppResources.bundle.url(forResource: "themes-builtin", withExtension: "json")!
        let all = try! JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
        return all[name] as! [String: Any]
    }

    @Test func theThreeBuiltInThemesAreEmbedded() {
        #expect(Set(ThemeStore.builtins.keys) == ["geek", "clean", "regular"])
        #expect(ThemeStore.builtins["regular"]?.window.chrome == "toolbar")
        #expect(ThemeStore.builtins["clean"]?.motion.sidebarMs == 120)
        #expect(ThemeStore.builtins["geek"]?.layout.rowHeight == 20)
    }

    /// geek draws what it drew before the themes became files: the colors
    /// of its file are those the palette had in its code, but the
    /// secondary text, darkened in November 2026 to reach WCAG AA.
    @Test func geekKeepsThePaletteOfBefore() {
        let c = ThemeStore.builtins["geek"]!.colors
        let pairs: [(ThemeColor, String, String)] = [
            (c.background, "#fafaf8", "#1b1d23"), (c.text, "#24262b", "#c3c6c1"), (c.muted, "#6b6e75", "#868b97"),
            (c.accent, "#8aa581", "#8aa581"), (c.accentText, "#56704e", "#8aa581"), (c.selection, "#d3e0ce", "#33432f"),
            (c.marked, "#f3e3ee", "#3a2e45"), (c.markedBar, "#c2408f", "#e08ac0"), (c.bar, "#ededea", "#24262e"),
            (c.popup, "#ffffff", "#262833"), (c.separator, "#ddddd8", "#30333c"), (c.unread, "#b07d16", "#e6c07b"),
            (c.error, "#c0392b", "#e06c75"), (c.flag, "#d9661c", "#f0883e"),
        ]
        for (color, light, dark) in pairs {
            #expect(color.hex(dark: false) == light)
            #expect(color.hex(dark: true) == dark)
        }
    }

    @Test func aPairResolvesPerAppearance() {
        let color = ThemeColor.pair(light: "#FFFFFF", dark: "#00000080")
        let dark = color.resolved(for: NSAppearance(named: .darkAqua)!)
        #expect(dark.alphaComponent < 0.51 && dark.alphaComponent > 0.49)
        #expect(color.resolved(for: NSAppearance(named: .aqua)!).redComponent == 1)
        #expect(NSColor(hex: "red") == nil && NSColor(hex: "#12345") == nil)
    }

    /// A color of the system is resolved in each appearance: a web page
    /// receives it as a plain hexadecimal value.
    @Test func aColorOfTheSystemFollowsTheAppearance() {
        let label = ThemeColor.system("label")
        #expect(label.hex(dark: false) != label.hex(dark: true))
        #expect(label.hex(dark: false).hasPrefix("#"))
        for name in ["accent", "secondary_label", "separator", "text_background", "orange", "alternating_content_background"] {
            #expect(ThemeColor.systemColor(name) != .magenta, "\(name)")
        }
    }

    /// The theme of the core is applied, the window rebuilt with it, and a
    /// refused file said; an equal theme changes nothing.
    @Test func theThemeOfTheCoreIsAppliedAndFollowed() async {
        final class Box { var theme = ThemeTests.themeJSON("clean") }
        let box = Box()
        let core = FakeCore { method, params in
            method == "theme_get" ? box.theme : Fixture.answer(method, params)
        }
        let model = AppModel()
        model.theme = ThemeStore()
        #expect(model.theme.current.name == "geek", "geek until the core answers")
        await model.attach(core)
        #expect(model.theme.current.name == "clean")
        let revision = model.theme.revision
        #expect(model.themeProblem == nil)

        // Another theme, with a refused file: applied, and the problem said.
        var regular = ThemeTests.themeJSON("regular")
        regular["error"] = ["file": "themes/regular.toml", "line": 3, "key": "layout.row_height", "message": "must be from 14 to 48"]
        // A first event places the stream (a gap for the model, which
        // reads its settings again), then `ThemeChanged` in order.
        model.handle(CoreClient.Event(method: "FolderChanged", epoch: "e_test", seq: 1, params: [:]))
        #expect(await eventually { model.theme.revision == revision })
        box.theme = regular
        model.handle(CoreClient.Event(method: "ThemeChanged", epoch: "e_test", seq: 2, params: [:]))
        #expect(await eventually { model.theme.current.name == "regular" })
        #expect(model.theme.revision == revision + 1)
        #expect(model.themeProblem?.text.contains("themes/regular.toml") == true)

        // The same theme again: no rebuild.
        await model.loadTheme()
        #expect(model.theme.revision == revision + 1)
    }

    /// The title bar: as the theme says, unless `ui.title_bar` is written
    /// in `config.toml` (`themes.md`, section 9).
    @Test func theTitleBarFollowsTheThemeUnlessConfigSaysIt() async {
        final class Box { var titleBar: Any = NSNull() }
        let box = Box()
        var hidden = ThemeTests.themeJSON("geek")
        var window = hidden["window"] as! [String: Any]
        window["title_bar"] = "hidden"
        hidden["window"] = window
        let core = FakeCore { method, params in
            switch method {
            case "theme_get": return hidden
            case "config_status": return ["ui": ["title_bar": box.titleBar]]
            default: return Fixture.answer(method, params)
            }
        }
        let model = AppModel()
        model.theme = ThemeStore()
        #expect(!model.titleBarHidden)
        await model.attach(core)
        #expect(model.titleBarHidden, "the theme hides it")
        box.titleBar = "shown"
        await model.loadUISettings()
        #expect(!model.titleBarHidden, "written in config.toml, it wins")
    }

    /// Events were missed (a gap in the stream): the theme is read again,
    /// a `ThemeChanged` may have been among them.
    @Test func aGapReadsTheThemeAgain() async {
        final class Box { var theme = ThemeTests.themeJSON("geek") }
        let box = Box()
        let core = FakeCore { method, params in
            method == "theme_get" ? box.theme : Fixture.answer(method, params)
        }
        let model = AppModel()
        model.theme = ThemeStore()
        await model.attach(core)
        box.theme = ThemeTests.themeJSON("clean")
        model.handle(CoreClient.Event(method: "FolderChanged", epoch: "e_test", seq: 7, params: [:]))
        #expect(await eventually { model.theme.current.name == "clean" })
    }

    /// A store of a test leaves the colors of the app alone: the tests run
    /// in parallel, and the palette is shared.
    @Test func aStoreOfATestDoesNotDriveThePalette() {
        let before = Palette.hex(\.background, dark: false)
        let store = ThemeStore()
        store.apply(ThemeStore.builtins["clean"]!)
        #expect(store.current.name == "clean")
        #expect(Palette.hex(\.background, dark: false) == before)
    }
}
