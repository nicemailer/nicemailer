import Foundation
import Observation

/// The themes the picker `v` `t` lists, and the choice of one
/// (`specifications/themes.md`, section 9): the core lists them
/// (`theme_list`) and writes the one chosen in `config.toml`
/// (`config_set_theme`), which applies it at once.
@MainActor
@Observable
final class ThemeChooser {
    /// One theme of the list.
    struct Entry: Decodable, Equatable, Identifiable {
        let name: String
        /// `builtin`, `builtin_modified` or `user`.
        let origin: String
        let parent: String?
        /// Why it cannot be used: its file, or one it extends, is refused.
        let error: KeymapProblem?
        var id: String { name }

        /// What the list says under its name.
        var detail: String {
            switch origin {
            case "builtin": L("Built in")
            case "builtin_modified": L("Built in, modified by its file")
            default: L("Extends %@", parent ?? "geek")
            }
        }
    }

    /// The result of `theme_list`.
    private struct ListView: Decodable {
        let path: String
        let selected: String
        let themes: [Entry]
    }

    private(set) var themes: [Entry] = []
    /// `ui.theme` of `config.toml`.
    private(set) var selected = ""
    /// The folder of the themes, next to `config.toml`.
    private(set) var path = ""
    /// What went wrong at the last choice, said under the list.
    private(set) var error: String?
    private(set) var loaded = false

    private let client: any CoreCalling

    init(client: any CoreCalling) {
        self.client = client
    }

    func load() async {
        do {
            let list: ListView = try await client.call("theme_list", [:])
            themes = list.themes
            selected = list.selected
            path = list.path
            loaded = true
        } catch {
            self.error = AccountSetup.message(error)
        }
    }

    /// Chooses `name`: written in `config.toml` by the core, which answers
    /// the theme now in effect; `apply` gives it to the app at once, without
    /// waiting for `ThemeChanged`.
    func choose(_ name: String, apply: (ThemeView) -> Void) async {
        guard name != selected, themes.first(where: { $0.name == name })?.error == nil else { return }
        error = nil
        do {
            let view: ThemeView = try await client.call("config_set_theme", ["name": name])
            selected = name
            apply(view)
        } catch {
            self.error = AccountSetup.message(error)
        }
    }
}
