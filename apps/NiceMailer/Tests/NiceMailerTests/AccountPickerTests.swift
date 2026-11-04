import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The account picker (`@`, or a click on the account in the status bar),
/// with two accounts.
@MainActor
struct AccountPickerTests {
    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async { await self.init(unified: false) }

    init(unified: Bool) async {
        // A second account "archive" with Inbox (id 11) and Archive (id 13).
        var archive = Fixture.folders.filter { $0["role"] as? String == "inbox" || $0["role"] as? String == "archive" }
            .map { f -> [String: Any] in
                var f = f
                f["id"] = (f["id"] as! Int) + 10
                f["account_id"] = 2
                f["account_name"] = "archive"
                return f
            }
        archive[0]["unread_count"] = 5
        let folders = Fixture.folders + archive
        core = FakeCore { method, params in
            method == "list_folders" ? ["items": folders, "as_of": Fixture.position] : Fixture.answer(method, params)
        }
        let defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        model.startUnified = unified
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 }
    }

    /// Sends one key: a character, `\r`, `esc`, or `up` / `down` (arrows).
    func press(_ name: String) {
        var chars = name
        var code: UInt16 = name == "\r" ? KeyCode.returnKey : 0
        if name == "up" { chars = "\u{F700}"; code = KeyCode.up }
        if name == "down" { chars = "\u{F701}"; code = KeyCode.down }
        if name == "esc" { chars = "\u{1b}"; code = KeyCode.escape }
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        _ = controller.handle(e)
    }

    @Test func atOpensTheAccountPickerWithUnreadCounts() {
        press("@")
        #expect(controller.overlay == .picker(.accounts))
        let entries = controller.pickerEntries(.accounts)
        // The current account is marked; unread mails of each account.
        // "all" first (the unified view), with the total.
        #expect(entries.map(\.label) == ["all", "perso", "archive"])
        // The counts are details, aligned on the right of the rows.
        #expect(entries.map(\.detail) == ["15", "8", "7"])
        #expect(entries.map(\.key) == [" ", "•", " "])
    }

    @Test func theAccountPickerTurnsAroundAtBothEnds() {
        press("@")
        #expect(controller.pickerIndex == 0)
        press("up")
        #expect(controller.pickerIndex == 2)  // "archive", the last row
        press("down")
        #expect(controller.pickerIndex == 0)  // back to the first row
        press("down"); press("down")
        #expect(controller.pickerIndex == 2)
        press("\r")
        #expect(model.scopeName == "archive")
    }

    @Test func choosingAnAccountKeepsTheFolderRole() {
        press(";"); press("a")
        #expect(model.selectedFolderId == 3)
        press("@")
        press("a"); press("r"); press("c"); press("h"); press("i")
        press("\r")
        // Archive of the other account.
        #expect(model.selectedFolderId == 13)
    }

    @Test func aRoleMissingInTheOtherAccountFallsBackToItsInbox() {
        press(";"); press("t")
        #expect(model.selectedFolderId == 2)
        controller.goToAccount(2)
        #expect(model.selectedFolderId == 11)
    }

    @Test func statusBarClicksOpenThePickers() {
        controller.openMenuFromStatusBar(.goto)
        #expect(controller.overlay == .menu(.goto))
        controller.overlay = .none
        controller.openPickerFromStatusBar(.accounts)
        #expect(controller.overlay == .picker(.accounts))
    }

    // MARK: Unified view "all"

    @Test func choosingAllShowsTheRoleOfEveryAccount() async {
        press(";"); press("a")
        controller.goToAccount(nil)
        #expect(model.unifiedRole == "archive" && model.selectedFolderId == nil)
        #expect(model.scopeName == "all" && model.currentFolderName == "Archive")
        #expect(model.listedFolderIds.sorted() == [3, 13])
        #expect(await eventually { core.calls("list_threads").last?["folder_ids"] as? [Int] == [3, 13] })
    }

    @Test func theSemicolonMenuOfAllListsRolesWithoutAccounts() {
        controller.goToAccount(nil)
        let entries = controller.menuEntries(.goto).filter { ![L("Contacts"), L("History"), L("Tags"), L("Groups")].contains($0.label) }
        let labels = entries.map(\.label)
        // Inbox: 2 (perso) + 5 (archive); Archive: 2 + 2; Todo and Deleted
        // exist in one account only. The unread count is a separate detail,
        // shown at the right end of the row; none for Everything.
        #expect(labels == ["Inbox", "Todo", "Archive", "Deleted", "Everything"])
        #expect(entries.map(\.detail) == ["7", "2", "4", "2", nil])
        #expect(!labels.contains { $0.contains("perso") || $0.contains("archive ·") })
    }

    @Test func leavingAllForAnAccountKeepsTheRole() {
        controller.goToAccount(nil)
        press(";"); press("a")
        #expect(model.unifiedRole == "archive")
        controller.goToAccount(2)
        #expect(model.selectedFolderId == 13 && model.unifiedRole == nil)
    }

    // MARK: Everything

    @Test func everythingOfAllListsEveryFolderOfEveryAccount() async {
        controller.goToAccount(nil)
        let everything = try! #require(controller.scopeFolders.last)
        #expect(everything.everything && everything.name == "Everything")
        controller.open(everything)
        #expect(model.everything == .all && model.currentFolderName == "Everything" && model.scopeName == "all")
        #expect(model.listedFolderIds.sorted() == [1, 2, 3, 4, 11, 13])
        // The core is asked for Everything itself, not for the 6 folders:
        // it then reads whole threads in one query.
        #expect(await eventually {
            let last = core.calls("list_threads").last
            return last?["everything"] as? [String: Any] != nil && last?["folder_ids"] == nil
        })
        #expect((core.calls("list_threads").last?["everything"] as? [String: Any])?.isEmpty == true)
        #expect(controller.isCurrent(everything))
    }

    @Test func everythingOfAnAccountAsksForThatAccount() async {
        controller.open(controller.scopeFolders.last!)
        controller.goToAccount(2)
        #expect(await eventually {
            (core.calls("list_threads").last?["everything"] as? [String: Any])?["account_id"] as? Int == 2
        })
    }

    @Test func aTagGroupOrContactListIsEverythingWithItsCriterion() {
        let tag = TagView(id: 4, accountId: 2, name: "Clients", color: nil, messageCount: 3)
        let scope = AppModel.listScope(everything: nil, folderIds: [1], listFilter: .tag(tag))
        #expect((scope["everything"] as? [String: Any])?.isEmpty == true && scope["folder_ids"] == nil)
        // The tag goes by its name (the core joins the tags of that name in
        // every account), with the attributes chosen.
        var attributes = ThreadFilter()
        attributes.contains = "devis"
        let filter = AppModel.filterParameters(attributes, listFilter: .tag(tag))
        #expect(filter?["tag"] as? String == "Clients" && filter?["contains"] as? String == "devis")
        let group = GroupView(id: 7, name: "Équipe", color: nil, memberCount: 2)
        #expect(AppModel.filterParameters(ThreadFilter(), listFilter: .group(group))?["group_id"] as? Int == 7)
        #expect(AppModel.filterParameters(ThreadFilter(), listFilter: nil) == nil)
    }

    @Test func theListScopeIsEverythingOrTheFolders() {
        #expect((AppModel.listScope(everything: .all, folderIds: [1, 2])["everything"] as? [String: Any])?.isEmpty == true)
        let account = AppModel.listScope(everything: .account(3), folderIds: [7])
        #expect((account["everything"] as? [String: Any])?["account_id"] as? Int == 3 && account["folder_ids"] == nil)
        let folders = AppModel.listScope(everything: nil, folderIds: [1, 11])
        #expect(folders["folder_ids"] as? [Int] == [1, 11] && folders["everything"] == nil)
    }

    @Test func everythingStaysEverythingWhenChangingAccount() {
        controller.open(controller.scopeFolders.last!)
        controller.goToAccount(2)
        #expect(model.everything == .account(2) && model.listedFolderIds.sorted() == [11, 13])
        #expect(model.scopeName == "archive")
        // A real folder leaves "Everything".
        press(";"); press("i")
        #expect(model.everything == nil && model.selectedFolderId == 11)
    }

    @Test func everythingOfAllShowsTheFolderAndAccountOfEachMail() async {
        // A plain folder: no folder label, the status bar names it.
        #expect(model.everythingFolderLabels == nil && model.selectedThreadFolderLabel == nil)
        controller.goToAccount(nil)
        controller.open(controller.scopeFolders.last!)
        #expect(model.everythingFolderLabels?[1]?.text == "Inbox · perso")
        #expect(model.everythingFolderLabels?[13]?.text == "Archive · archive")
        // The fake core puts every thread in the first folder listed.
        #expect(await eventually { model.selectedThread != nil })
        let folder = try! #require(model.selectedThread?.folderId)
        #expect(model.selectedThreadFolderLabel == model.everythingFolderLabels?[folder]?.text)
        #expect(model.selectedThreadFolderLabel?.contains(" · ") == true)
    }

    @Test func everythingOfAnAccountShowsTheFolderOnly() async {
        press(";"); press("e")
        #expect(model.everythingFolderLabels?[2] == .init(folder: "Todo", account: nil))
        #expect(await eventually { model.selectedThread != nil })
        #expect(model.selectedThreadFolderLabel == "Inbox")
        // Leaving "Everything" hides the labels again.
        press(";"); press("a")
        #expect(model.everythingFolderLabels == nil && model.selectedThreadFolderLabel == nil)
    }

    @Test func folderLabelsCarryTheAccountOnlyWhenAsked() {
        let f = model.folders.first { $0.id == 11 }!
        #expect(AppModel.folderLabel(f, withAccount: false).text == "Inbox")
        #expect(AppModel.folderLabel(f, withAccount: true).text == "Inbox · archive")
        #expect(AppModel.folderLabels(model.folders, withAccount: false).count == model.folders.count)
    }

    @Test func aSearchInAllCoversEveryAccountAndEscapeGoesBack() {
        controller.goToAccount(nil)
        #expect(model.unifiedRole == "inbox")
        press("/"); press("d"); press("\r")
        #expect(model.everything == .all && model.attributes.contains == "d")
        press("esc")
        #expect(model.unifiedRole == "inbox" && model.everything == nil && model.attributes.isEmpty)
    }

    @Test func semicolonEGoesToEverything() {
        press(";"); press("e")
        #expect(model.everything == .account(1))
    }
}

/// The geek theme opens on the unified view when there are several accounts.
@MainActor
struct UnifiedStartTests {
    @Test func theAppStartsOnAllInbox() async {
        let t = await AccountPickerTests(unified: true)
        #expect(t.model.unifiedRole == "inbox")
        #expect(t.model.scopeName == "all")
        #expect(await eventually { t.core.calls("list_threads").last?["folder_ids"] as? [Int] == [1, 11] })
    }
}
