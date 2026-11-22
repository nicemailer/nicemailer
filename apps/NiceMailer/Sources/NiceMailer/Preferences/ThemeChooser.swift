import AppKit
import Observation
import SwiftUI

/// The tabs of the Preferences, for the scenarios that open one
/// (`UIScript`, `preferences appearance`).
enum PreferencesTab: String, Hashable {
    case accounts, appearance
}

/// The themes the Appearance tab lists, and the choice of one
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

/// The Appearance tab: the themes, the one chosen marked, and the folder
/// where they are, to modify one or add one's own (`docs/themes.md`).
struct AppearancePane: View {
    @Environment(AppModel.self) private var model
    @State private var chooser: ThemeChooser

    init(client: any CoreCalling) {
        _chooser = State(initialValue: ThemeChooser(client: client))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("Theme")).font(.headline)
            // A plain stack in a scroll view, not a `List`: a row of a list
            // takes the click for its own selection.
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                ForEach(chooser.themes) { t in
                    Button { Task { await chooser.choose(t.name) { model.theme.apply($0) } } } label: {
                    HStack(spacing: 10) {
                        Image(systemName: t.name == chooser.selected ? "largecircle.fill.circle" : "circle")
                            .foregroundStyle(t.error == nil ? Color.accentColor : .secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(t.name)
                            if let e = t.error {
                                Text(e.text).font(.caption).foregroundStyle(.red).lineLimit(2)
                            } else {
                                Text(t.detail).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                    }
                    .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .opacity(t.error == nil ? 1 : 0.6)
                    .uiTarget("prefs-theme-\(t.name)")
                }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 6))
            if let error = chooser.error {
                Text(error).foregroundStyle(.red)
            }
            HStack {
                Text(L("Each theme is a file you can modify, or copy to make your own.")).foregroundStyle(.secondary)
                Spacer()
                Button(L("Open the Folder of Themes")) {
                    model.openFile(URL(fileURLWithPath: chooser.path, isDirectory: true))
                }
                .disabled(chooser.path.isEmpty)
                .uiTarget("prefs-themes-folder")
            }
        }
        .padding(20)
        .task {
            await chooser.load()
            UIScript.appearanceOpened(chooser)
        }
    }
}
