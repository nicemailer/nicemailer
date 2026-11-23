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

/// The Appearance tab of the Preferences (`themes.md`, section 9): the
/// themes listed by the core, the choice written by it.
@MainActor
struct ThemeChooserTests {
    static func list(selected: String) -> [String: Any] {
        [
            "path": "/tmp/nm/themes", "selected": selected,
            "themes": [
                ["name": "geek", "origin": "builtin", "parent": NSNull(), "error": NSNull()],
                ["name": "clean", "origin": "builtin_modified", "parent": NSNull(), "error": NSNull()],
                ["name": "regular", "origin": "builtin", "parent": NSNull(), "error": NSNull()],
                ["name": "sepia", "origin": "user", "parent": "clean", "error": NSNull()],
                ["name": "broken", "origin": "user", "parent": NSNull(),
                 "error": ["file": "themes/broken.toml", "line": 2, "key": "layout.zebra", "message": "expected true or false"]],
            ],
        ]
    }

    @Test func theThemesAreListedAndOneIsChosen() async {
        final class Calls { var set: [String] = [] }
        let calls = Calls()
        let core = FakeCore { method, params in
            switch method {
            case "theme_list": return ThemeChooserTests.list(selected: "geek")
            case "config_set_theme":
                calls.set.append(params["name"] as? String ?? "")
                return ThemeTests.themeJSON("clean")
            default: return Fixture.answer(method, params)
            }
        }
        let chooser = ThemeChooser(client: core)
        await chooser.load()
        #expect(chooser.themes.map(\.name) == ["geek", "clean", "regular", "sepia", "broken"])
        #expect(chooser.selected == "geek")
        #expect(chooser.themes[1].detail == L("Built in, modified by its file"))
        #expect(chooser.themes[3].detail == L("Extends %@", "clean"))

        // The theme the core answers is applied at once.
        let store = ThemeStore()
        await chooser.choose("clean") { store.apply($0) }
        #expect(calls.set == ["clean"])
        #expect(chooser.selected == "clean")
        #expect(store.current.name == "clean")

        // The one chosen already, or a refused one: nothing asked.
        await chooser.choose("clean") { store.apply($0) }
        await chooser.choose("broken") { store.apply($0) }
        #expect(calls.set == ["clean"])
    }

    @Test func aRefusalIsSaid() async {
        let core = FakeCore { method, params in
            switch method {
            case "theme_list": return ThemeChooserTests.list(selected: "geek")
            case "config_set_theme":
                return CoreClient.RPCError(code: -32602, message: "config.toml was not changed: refused",
                                           data: ["reason": "config_not_written", "detail": "refused"])
            default: return Fixture.answer(method, params)
            }
        }
        let chooser = ThemeChooser(client: core)
        await chooser.load()
        await chooser.choose("regular") { _ in }
        #expect(chooser.selected == "geek")
        #expect(chooser.error == L("config.toml was not changed: %@", "refused"))
    }
}

/// The order of the blocks of the status bar and the toolbar
/// (`window.blocks`, `themes.md`, section 6.1).
struct StatusLayoutTests {
    let geek = ["account", "folder", "attributes", "|", "view", "|", "move", "save", "ai", "->", "help"]

    @Test func everythingShownIsTheThemesOrder() {
        #expect(StatusLayout.items(geek) { _ in true } == geek)
    }

    @Test func aGapStaysOnlyBetweenTwoBlocksShown() {
        // In Quarantine, m and s go: the gap after `v` now leads to the AI.
        #expect(StatusLayout.items(geek) { !["move", "save"].contains($0) }
                == ["account", "folder", "attributes", "|", "view", "|", "ai", "->", "help"])
        // A screen (contacts): no attributes nor menus of the mail, no gap.
        #expect(StatusLayout.items(geek) { ["folder", "help"].contains($0) }
                == ["folder", "->", "help"])
        // Gaps at the start, doubled, or before the spring: dropped.
        #expect(StatusLayout.items(["|", "view", "|", "|", "move", "|", "->", "|", "help"]) { _ in true }
                == ["view", "|", "move", "->", "help"])
    }

    @Test func theMessagesAlwaysHaveTheirPlace() {
        // Without a spring, one at the end: what the app says always shows.
        #expect(StatusLayout.items(["help", "move"]) { _ in true } == ["help", "move", "->"])
        #expect(StatusLayout.items([]) { _ in true } == ["->"])
        // Two springs: the first.
        #expect(StatusLayout.items(["view", "->", "move", "->", "help"]) { _ in true } == ["view", "->", "move", "help"])
        // A theme's own order.
        #expect(StatusLayout.items(["help", "->", "ai", "folder"]) { _ in true } == ["help", "->", "ai", "folder"])
    }
}

/// The size of the bar (`window.bar_size`): a proportion of the text size
/// of the bar, as the three sizes of a Mac toolbar.
@MainActor
struct BarSizeTests {
    /// clean, with `window.bar_size` set to `size`.
    private func theme(_ size: String) -> ThemeView {
        var json = ThemeTests.themeJSON("clean")
        var window = json["window"] as! [String: Any]
        window["bar_size"] = size
        json["window"] = window
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try! decoder.decode(ThemeView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    @Test func theThreeSizesScaleTheBar() {
        #expect(Look.barScale(theme("small")) == 0.85)
        #expect(Look.barScale(theme("medium")) == 1)
        #expect(Look.barScale(theme("large")) == 1.2)
    }

    @Test func theBuiltInThemesKeepTheBarTheyDrew() {
        for name in ["geek", "clean", "regular"] {
            #expect(ThemeStore.builtins[name]!.window.barSize == "medium", "\(name)")
            #expect(Look.barScale(ThemeStore.builtins[name]!) == 1, "\(name)")
        }
    }
}
