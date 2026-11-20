import Foundation
import Testing
@testable import NiceMailer

/// The names of the folders (`FolderNames`, `configuration.md`, "Noms des
/// dossiers"): those of `[folder_names]` in `config.toml`, which the core
/// sends with `config_status`, English for a role the file does not name.
/// For display only: what the app sends to the core keeps the role.
@MainActor
struct FolderNamesTests {
    /// The names a French user would write: two folders renamed, the
    /// others left English.
    static let french: [String: String] = [
        "inbox": "Réception", "todo": "À faire", "waiting": "Waiting", "archive": "Archives",
        "sent": "Sent", "drafts": "Drafts", "newsletter": "Newsletter", "paperwork": "Paperwork",
        "quarantine": "Quarantine", "deleted": "Corbeille", "junk": "Junk", "banned": "Banned",
        "work": "Work", "all_mail": "Tous les mails", "everything": "Tout",
    ]

    let defaults = TestDefaults.make()

    /// A model attached to a fake core whose `config_status` gives `names`
    /// (none: an older core, without `folder_names`), with its controller.
    func attached(_ names: @escaping () -> [String: String]?) async -> (AppModel, GeekController, FakeCore) {
        let core = FakeCore { method, params in
            guard method == "config_status" else { return Fixture.answer(method, params) }
            var status: [String: Any] = ["ui": ["title_bar": "shown"]]
            if let names = names() { status["folder_names"] = names }
            return status
        }
        let model = AppModel()
        let controller = GeekController(model: model, defaults: defaults, textSize: TextSize())
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 }
        return (model, controller, core)
    }

    /// A line of the history: a mail moved to `value`.
    func move(to value: String) -> HistoryGroup {
        HistoryGroup(firstId: 1, lastId: 1, count: 1, date: 1_700_000_000, source: "app",
                     entity: "message", entityId: 1, kind: "move", value: value, title: "La facture")
    }

    // MARK: The pure value

    @Test func aRoleTheFileDoesNotNameKeepsItsEnglishName() {
        let names = FolderNames(byRole: ["waiting": "En attente"])
        #expect(names.name("waiting") == "En attente")
        #expect(names.name("todo") == "Todo")
        #expect(FolderNames.english.name("waiting") == "Waiting")
        // No role, or one this version does not know.
        #expect(names.name(nil) == "?")
        #expect(names.name("brand_new") == "?")
    }

    /// The virtual folders: the file's name, else the English one, which
    /// `Virtual.name` keeps for the targets of the scenarios.
    @Test func theVirtualFoldersAreNamedByTheFile() {
        let names = FolderNames(byRole: Self.french)
        #expect(names.name(of: AppModel.Virtual.allMail) == "Tous les mails")
        #expect(names.name(of: AppModel.Virtual.everything) == "Tout")
        #expect(names.name(of: AppModel.Virtual.work) == "Work")
        #expect(FolderNames.english.name(of: AppModel.Virtual.allMail) == "All Mail")
        #expect(AppModel.Virtual.allMail.name == "All Mail")
        #expect(AppModel.Virtual.allCases.map(\.id) == ["work", "all_mail", "everything"])
        #expect(GeekController.virtualFolder(.everything, names: names).name == "Tout")
    }

    @Test func aFolderWithoutRoleIsNamedByItsPath() throws {
        let data = #"{"id": 9, "role": null, "name": "Clients/2026", "letter": null, "category": null, "total_count": 0, "unread_count": 0, "move_targets": [], "account_id": 1, "account_name": "perso", "account_kind": "imap"}"#
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let path = try decoder.decode(FolderView.self, from: Data(data.utf8))
        #expect(FolderNames(byRole: Self.french).name(of: path) == "Clients/2026")
    }

    /// The pure functions that word a folder take the names they are given,
    /// English by default.
    @Test func theMessagesNameTheFoldersAsTheyAreGiven() {
        let names = FolderNames(byRole: Self.french)
        #expect(AppModel.refusedMoveFlash(from: "drafts", to: "archive", names: names)
            == L("A draft is not moved: Delete puts it in %@", "Corbeille"))
        #expect(AppModel.refusedMoveFlash(from: nil, to: "todo", names: names) == L("Not a target from this folder: %@", "À faire"))
        #expect(AppModel.refusedMoveFlash(from: nil, to: "todo") == L("Not a target from this folder: %@", "Todo"))
        let d = DecisionView(kind: "address", name: "a@b.fr", before: nil, after: "inbox")
        #expect(Workflow.decisionFlash(d, filed: 0, names: names).hasPrefix(L("%@ → %@", "a@b.fr", "Réception")))
        #expect(Compose.sentFlash(movedTo: "archive", undo: false, names: names) == L("Sent") + " · " + L("filed in %@", "Archives"))
        #expect(HistoryDisplay.action(move(to: "todo"), names: names).detail == L("to %@", "À faire"))
        // A folder without role, in the history, keeps its path.
        #expect(HistoryDisplay.action(move(to: "Clients"), names: names).detail == L("to %@", "Clients"))
    }

    /// The sections of the sidebar of the classic theme follow the app's
    /// language, never French whatever the language (as they were).
    @Test func theSectionsOfTheSidebarAreTranslated() {
        #expect(Role.categoryTitles.map(\.1) == [L("Working"), L("Filing"), L("Rejected"), L("Technical")])
        #expect(L(in: "en", "Working") == "Working")
        #expect(L(in: "fr", "Working") == "Travail")
        #expect(L(in: "fr", "Filing") == "Rangement")
        #expect(L(in: "fr", "Rejected") == "Rejet")
        #expect(L(in: "fr", "Technical") == "Technique")
    }

    // MARK: Through the model and the keys

    @Test func theFilesNamesReachTheMenusTheStatusBarAndTheMessages() async {
        let (model, controller, core) = await attached { Self.french }
        #expect(await eventually { model.folderNames.name("inbox") == "Réception" })
        // The folder shown, the menu `;` and the folders of the view.
        #expect(model.currentFolderName == "Réception")
        #expect(controller.scopeFolders.filter { $0.virtual == nil }.map(\.name) == ["Réception", "À faire", "Archives", "Corbeille"])
        let goto = controller.menuEntries(.goto).map(\.label)
        #expect(goto.contains("Réception") && goto.contains("Corbeille") && !goto.contains("Inbox"))
        // The virtual folders, in the menu and once shown.
        #expect(goto.contains("Tous les mails") && goto.contains("Tout") && !goto.contains("All Mail"))
        #expect(controller.scopeFolders.contains { $0.virtual == .allMail && $0.name == "Tous les mails" })
        // The menu of moves: the targets of the Inbox, renamed or not.
        #expect(controller.menuEntries(.move).map(\.label) == ["À faire", "Waiting", "Archives", "Corbeille", "Junk"])
        // A move is sent with the role, and said with the name.
        controller.perform(KeyAction(rawValue: "move.todo")!)
        #expect(await eventually { core.moveCalls().count == 1 })
        #expect(core.moveCalls()[0]["target"] as? String == "todo")
        #expect(await eventually { model.flash?.contains("À faire") == true })
        // Everything shown: the status bar names it as the file does.
        controller.perform(.gotoEverything)
        #expect(await eventually { model.currentFolderName == "Tout" })
    }

    /// An older core sends no names: the English ones stay.
    @Test func anOlderCoreLeavesTheEnglishNames() async {
        let (model, controller, _) = await attached { nil }
        #expect(model.folderNames == .english)
        #expect(model.currentFolderName == "Inbox")
        #expect(controller.menuEntries(.move).map(\.label) == ["Todo", "Waiting", "Archive", "Deleted", "Junk"])
    }

    /// The file edited: the names are read again with `config_status`
    /// (after `ConfigChanged`), and the interface follows at once.
    @Test func aNewNameInTheFileIsShownAtOnce() async {
        var names: [String: String]? = nil
        let (model, controller, _) = await attached { names }
        #expect(model.currentFolderName == "Inbox")
        names = Self.french
        await model.loadUISettings()
        #expect(model.currentFolderName == "Réception")
        #expect(controller.menuEntries(.move).first?.label == "À faire")
        // Back to no name: English again.
        names = nil
        await model.loadUISettings()
        #expect(model.folderNames == .english && model.currentFolderName == "Inbox")
    }
}
