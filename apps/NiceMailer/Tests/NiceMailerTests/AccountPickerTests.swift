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

    func press(_ chars: String) {
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: chars == "\r" ? KeyCode.returnKey : 0)!
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
        controller.openPickerFromStatusBar(.folders)
        #expect(controller.overlay == .picker(.folders))
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

    @Test func theFolderPickerOfAllListsRolesWithoutAccounts() {
        controller.goToAccount(nil)
        let labels = controller.pickerEntries(.folders).map(\.label)
        // Inbox: 2 (perso) + 5 (archive); Archive: 2 + 2; Todo and Deleted
        // exist in one account only.
        #expect(labels == ["Inbox  7", "Todo  2", "Archive  4", "Deleted  2"])
        #expect(!labels.contains { $0.contains("perso") || $0.contains("archive ·") })
    }

    @Test func leavingAllForAnAccountKeepsTheRole() {
        controller.goToAccount(nil)
        press(";"); press("a")
        #expect(model.unifiedRole == "archive")
        controller.goToAccount(2)
        #expect(model.selectedFolderId == 13 && model.unifiedRole == nil)
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
