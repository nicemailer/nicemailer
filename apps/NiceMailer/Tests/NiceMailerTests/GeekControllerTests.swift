import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Keyboard and mouse handling of the geek theme (`GeekController`),
/// against a fake core (`FakeCore`). Each test builds a fresh model,
/// controller and preferences domain.
@MainActor
struct GeekControllerTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()
    let defaults: UserDefaults
    let controller: GeekController

    init() async {
        defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults)
        model.exportDirectory = "/tmp/nicemailer-tests"
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    /// Sends one key: a character, `esc`, `ret`, `del`, `down`, `up`, or
    /// `ctrl-X`.
    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        var mods: NSEvent.ModifierFlags = []
        switch name {
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        case "ret": chars = "\r"; code = KeyCode.returnKey
        case "del": chars = "\u{7f}"; code = KeyCode.delete
        case "down": chars = "\u{F701}"; code = KeyCode.down
        case "up": chars = "\u{F700}"; code = KeyCode.up
        default:
            if name.hasPrefix("ctrl-") { mods = .control; chars = String(name.dropFirst(5)) }
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        #expect(controller.handle(event))
    }

    func type(_ text: String) {
        for c in text { press(String(c)) }
    }

    var selectedIndex: Int? { model.threads.firstIndex { $0.id == model.selectedThreadId } }

    // MARK: Navigation and selection

    @Test func theFirstMailIsSelectedWhenTheListOpens() {
        #expect(selectedIndex == 0)
        #expect(model.openThread?.id == 1)
    }

    @Test func jkAndArrowsMoveTheCursorWithinTheList() {
        press("j"); press("j")
        #expect(selectedIndex == 2)
        press("k")
        #expect(selectedIndex == 1)
        press("down"); press("up"); press("up"); press("up")
        #expect(selectedIndex == 0)
    }

    @Test func shiftJKMarkAndEscapeUnmarks() {
        press("J"); press("J")
        #expect(model.marked == [1, 2, 3])
        press("esc")
        #expect(model.marked.isEmpty)
    }

    @Test func clicksSelectMarkAndExtend() {
        controller.clickThread(5, .none)
        #expect(selectedIndex == 4 && model.marked.isEmpty)
        controller.clickThread(7, .command)
        #expect(model.marked == [5, 7])
        controller.clickThread(2, .shift)
        #expect(model.marked == [2, 3, 4, 5, 6, 7])
    }

    // MARK: Prefix menus and actions

    @Test func theMoveMenuOffersOnlyTheTargetsOfTheFolder() {
        let keys = controller.menuEntries(.move).map(\.key)
        #expect(keys == ["t", "w", "a", "d", "x"])
    }

    @Test func mThenALetterMovesFromTheThreadsFolder() async {
        press("m")
        #expect(controller.overlay == .menu(.move))
        press("t")
        #expect(controller.overlay == .none)
        #expect(await eventually { core.calls("move_thread").count == 1 })
        let call = core.calls("move_thread")[0]
        #expect(call["target"] as? String == "todo")
        #expect(call["from_folder_id"] as? Int == 1)
        #expect(call["thread_id"] as? Int == 1)
        // Optimistic update: the thread left the list, the cursor stays.
        #expect(model.threads.count == 7 && selectedIndex == 0)
    }

    @Test func dArchivesEveryMarkedThread() async {
        press("J"); press("J")
        press("d")
        #expect(await eventually { core.calls("move_thread").count == 3 })
        #expect(core.calls("move_thread").allSatisfy { $0["target"] as? String == "archive" })
        #expect(model.marked.isEmpty)
    }

    @Test func deleteMovesToDeleted() async {
        press("del")
        #expect(await eventually { core.calls("move_thread").first?["target"] as? String == "deleted" })
    }

    @Test func escapeCancelsAPrefixAndUnknownKeysAreReported() {
        press("m"); press("esc")
        #expect(controller.overlay == .none && core.calls("move_thread").isEmpty)
        press("y")
        #expect(model.flash != nil)
        press("m"); press("z")
        #expect(model.flash != nil && core.calls("move_thread").isEmpty)
    }

    @Test func readAndFlagToggles() async {
        press("*")
        #expect(await eventually { core.calls("set_thread_flagged").first?["flagged"] as? Bool == true })
        press("u")
        #expect(await eventually { !core.calls("set_thread_seen").isEmpty })
    }

    @Test func replyIsNotAvailableYet() {
        press("r")
        #expect(model.flash?.contains("SMTP") == true)
    }

    @Test func helpOpensAndAnyKeyClosesIt() {
        press("?")
        #expect(controller.overlay == .help)
        press("j")
        #expect(controller.overlay == .none && selectedIndex == 0)
    }

    // MARK: Folders and screens

    @Test func semicolonGoesToAFolderOfTheAccount() async {
        press(";"); press("a")
        #expect(model.selectedFolderId == 3)
        #expect(await eventually { model.threads.first?.id == 103 })
    }

    @Test func screensOpenLoadAndCloseWithEscape() async {
        press(";"); press("c")
        #expect(controller.screen == .contacts)
        #expect(await eventually { controller.contacts.count == 3 })
        press("j")
        #expect(controller.screenIndex == 1)
        press("ret")
        #expect(controller.screen == .mail)
        #expect(await eventually { !core.calls("list_threads_by").isEmpty })
        #expect(core.calls("list_threads_by")[0]["address_id"] as? Int == 2)
        press(";"); press("h")
        #expect(controller.screen == .history)
        press("esc")
        #expect(controller.screen == .mail)
    }

    @Test func contactsCanBeFiltered() async {
        press(";"); press("c")
        press("/"); type("ab")
        #expect(controller.contactsQuery == "ab")
        #expect(await eventually { core.calls("list_contacts").last?["query"] as? String == "ab" })
    }

    // MARK: Pickers and search

    @Test func theFolderPickerFiltersOnEveryWordTyped() {
        press("h")
        #expect(controller.overlay == .picker(.folders))
        #expect(controller.pickerEntries(.folders).count == 4)
        type("arc")
        // One folder left; its label ends with its unread count.
        #expect(controller.pickerEntries(.folders).map(\.label) == ["Archive  2"])
        press("ret")
        #expect(model.selectedFolderId == 3)
    }

    @Test func theTagPickerCreatesATypedTag() async {
        press("t"); type("VIP"); press("ret")
        #expect(await eventually { core.calls("tag_thread").count == 1 })
        #expect(core.calls("tag_thread")[0]["name"] as? String == "VIP")
        #expect(core.calls("tag_thread")[0]["remove"] as? Bool == false)
    }

    @Test func theGroupPickerAddsTheSender() async {
        press("g"); type("Amis"); press("ret")
        #expect(await eventually { core.calls("set_group_member").count == 1 })
        #expect(core.calls("set_group_member")[0]["group"] as? String == "Amis")
        #expect(core.calls("set_group_member")[0]["thread_id"] as? Int == 1)
    }

    @Test func theLinkPickerListsTheLinksOfTheMail() {
        press("l")
        #expect(controller.pickerEntries(.links).map(\.label) == ["https://example.org/1"])
        press("esc")
        #expect(controller.overlay == .none)
    }

    @Test func searchTextIsTypedAndEscapeClearsIt() {
        press("/"); type("devis")
        #expect(model.searchText == "devis")
        press("esc")
        #expect(model.searchText.isEmpty && controller.overlay == .none)
    }

    // MARK: Display and saving

    @Test func theDisplayMenuChangesAndRemembersTheLayout() {
        press("v"); press("3")
        #expect(controller.columns == 3)
        press("v"); press("s")
        #expect(controller.statusPosition == .top)
        // A new controller reads the remembered choices.
        let again = GeekController(model: model, defaults: defaults)
        #expect(again.columns == 3 && again.statusPosition == .top)
    }

    @Test func sortingByModificationReloadsTheList() async {
        press("v"); press("m")
        #expect(model.sort == "modified")
        #expect(await eventually { core.calls("list_threads").last?["sort"] as? String == "modified" })
    }

    @Test func textHtmlIsRememberedForTheSender() async {
        press("j"); press("j")
        #expect(await eventually { model.openThread?.id == 3 })
        press("v"); press("h")
        #expect(await eventually { core.calls("set_body_format").count == 1 })
        #expect(core.calls("set_body_format")[0]["format"] as? String == "html")
        #expect(core.calls("set_body_format")[0]["addr"] as? String == "sender3@example.org")
    }

    @Test func textHtmlIsRefusedForATextOnlyMail() {
        press("v"); press("h")
        #expect(core.calls("set_body_format").isEmpty && model.flash != nil)
    }

    @Test func savingUsesTheExportDirectory() async {
        press("s"); press("e")
        #expect(await eventually { core.calls("export_message").count == 1 })
        #expect(core.calls("export_message")[0]["format"] as? String == "eml")
        #expect(core.calls("export_message")[0]["dir"] as? String == "/tmp/nicemailer-tests")
        press("s"); press("a")
        #expect(core.calls("export_message").count == 1 && model.flash != nil)
    }
}
