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

    /// Sends one key: a character, `\r`, or `up` / `down` (arrows).
    func press(_ name: String) {
        var chars = name
        var code: UInt16 = name == "\r" ? KeyCode.returnKey : 0
        if name == "up" { chars = "\u{F700}"; code = KeyCode.up }
        if name == "down" { chars = "\u{F701}"; code = KeyCode.down }
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
        #expect(entries.map(\.label) == ["all  15", "perso  8", "archive  7"])
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
        let labels = controller.menuEntries(.goto).map(\.label).filter { ![L("Contacts"), L("History"), L("Tags"), L("Groups")].contains($0) }
        // Inbox: 2 (perso) + 5 (archive); Archive: 2 + 2; Todo and Deleted
        // exist in one account only.
        #expect(labels == ["Inbox  7", "Todo  2", "Archive  4", "Deleted  2", "Everything"])
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
        #expect(await eventually { (core.calls("list_threads").last?["folder_ids"] as? [Int])?.sorted() == [1, 2, 3, 4, 11, 13] })
        #expect(controller.isCurrent(everything))
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
