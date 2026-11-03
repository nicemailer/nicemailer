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
    let textSize: TextSize
    let controller: GeekController

    init() async {
        defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        textSize = TextSize(defaults: defaults)
        controller = GeekController(model: model, defaults: defaults, textSize: textSize)
        model.exportDirectory = "/tmp/nicemailer-tests"
        // Never launch an app from the tests.
        model.openFile = { _ in }
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
        case "shift-down": chars = "\u{F701}"; code = KeyCode.down; mods = .shift
        case "cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = .command
        case "shift-up": chars = "\u{F700}"; code = KeyCode.up; mods = .shift
        default:
            if name.hasPrefix("ctrl-") { mods = .control; chars = String(name.dropFirst(5)) }
            if name.hasPrefix("cmd-") { mods = .command; chars = String(name.dropFirst(4)) }
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

    @Test func ctrlDAndCtrlUMoveByHalfAPageWithinTheList() {
        // 6 rows fit: half a page is 3 rows. The fixture lists 8 mails.
        controller.listVisibleRows = 6
        press("ctrl-d")
        #expect(selectedIndex == 3)
        press("ctrl-d")
        #expect(selectedIndex == 6)
        press("ctrl-d")
        #expect(selectedIndex == 7)  // stops on the last mail
        press("ctrl-u")
        #expect(selectedIndex == 4)
        press("ctrl-u"); press("ctrl-u")
        #expect(selectedIndex == 0)  // and on the first
    }

    @Test func theHTMLReaderScrollsByAFractionOfThePage() {
        // Space: a page down, and the new position comes back to the app.
        #expect(GeekController.htmlScrollScript(pages: 1)
            == "window.scrollBy(0, Math.round(window.innerHeight * 1.0)); window.scrollY")
        #expect(GeekController.htmlScrollScript(pages: -0.5).contains("innerHeight * -0.5"))
    }

    @Test func halfAPageIsHalfTheVisibleRowsAndNeverZero() {
        #expect(GeekController.halfPage(40) == 20)
        #expect(GeekController.halfPage(7) == 3)
        #expect(GeekController.halfPage(1) == 1)
        #expect(GeekController.halfPage(0) == 1)
        #expect(GeekController.visibleRows(height: 900, rowHeight: 20) == 45)
        #expect(GeekController.visibleRows(height: 39, rowHeight: 20) == 1)
        #expect(GeekController.visibleRows(height: 0, rowHeight: 20) == 1)
        #expect(GeekController.visibleRows(height: 100, rowHeight: 0) == 1)
    }

    @Test func shiftArrowsMarkAndMoveLikeJK() {
        press("shift-down"); press("shift-down")
        // Ids 1, 2, 3 marked; the cursor on the third row.
        #expect(model.marked == [1, 2, 3] && selectedIndex == 2)
        press("shift-up")
        #expect(selectedIndex == 1 && model.marked == [1, 2, 3])
        // Same result as K from there.
        press("esc")
        press("K")
        let viaK = model.marked
        press("esc"); press("j")
        press("shift-up")
        #expect(model.marked == viaK)
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

    @Test func theMoveMessageNamesTheMailAndTheFolder() {
        press("d")
        // Whatever the language: the subject and the folder are both in it.
        let flash = model.flash ?? ""
        #expect(flash.contains("Subject 1") && flash.contains("Archive"))
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

    @Test func semicolonListsTheFoldersAndEverythingAndHIsFree() {
        let keys = controller.menuEntries(.goto).map(\.key)
        // Four folders, then "Everything", then the screens.
        #expect(keys == ["i", "t", "a", "d", "e", "c", "T", "g", "h"])
        // A gap before each new block: Archive, Deleted, Everything,
        // Contacts, History.
        let gaps = controller.menuEntries(.goto).filter(\.gapBefore).map(\.key)
        #expect(gaps == ["a", "d", "e", "c", "h"])
        press("h")
        #expect(controller.overlay == .none && model.flash != nil)
    }

    @Test func theArrowsHighlightAMenuRowAndReturnGoesThere() {
        press(";")
        #expect(controller.menuIndex == nil)
        press("down")
        #expect(controller.menuIndex == 0 && controller.overlay == .menu(.goto))
        press("down")
        #expect(controller.menuIndex == 1)  // Todo
        press("ret")
        #expect(controller.overlay == .none && model.selectedFolderId == 2)
    }

    @Test func menuArrowsTurnAroundAndKeysStillAct() {
        press(";")
        let count = controller.menuEntries(.goto).count
        press("up")
        #expect(controller.menuIndex == count - 1)  // History, the last row
        press("down")
        #expect(controller.menuIndex == 0)
        // A key still acts at once, whatever is highlighted.
        press("a")
        #expect(controller.overlay == .none && model.selectedFolderId == 3)
        // A new menu starts without a highlight; Return alone closes it.
        press(";")
        #expect(controller.menuIndex == nil)
        press("ret")
        #expect(controller.overlay == .none && model.selectedFolderId == 3)
    }

    @Test func foldersAreSortedInBlocksWhateverTheCoresOrder() {
        // Every role, shuffled, plus a folder of the user (no role).
        let roles: [String?] = ["junk", "sent", nil, "archive", "banned", "inbox", "drafts", "paperwork",
                                "deleted", "quarantine", "newsletter", "waiting", "todo"]
        let folders = roles.enumerated().map { i, r in
            GeekController.ScopeFolder(folderId: i, role: r, name: r ?? "Projets", letter: "", unread: 0)
        }
        let sorted = GeekController.ordered(folders)
        #expect(sorted.map(\.name) == [
            "inbox", "todo", "waiting", "quarantine",
            "archive", "newsletter", "paperwork",
            "Projets",
            "sent", "drafts",
            "deleted", "junk", "banned",
        ])
        // Blocks: 0 to deal with, 2 filed, 3 the user's, 4 sent, 6 thrown away.
        #expect(sorted.map(\.group) == [0, 0, 0, 0, 2, 2, 2, 3, 4, 4, 6, 6, 6])
    }

    @Test func theMoveSaveAndDisplayBlocksOpenTheirMenus() {
        for p in [GeekController.Prefix.move, .save, .view] {
            controller.openMenuFromStatusBar(p)
            #expect(controller.overlay == .menu(p))
            #expect(!p.icon.isEmpty)
        }
        // A click on a row of the display menu: two columns.
        controller.openMenuFromStatusBar(.view)
        let two = controller.menuEntries(.view).firstIndex { $0.key == "2" }!
        controller.chooseMenuEntry(two)
        #expect(controller.overlay == .none)
    }

    @Test func aClickOnAMenuRowActsLikeItsKey() {
        controller.openMenuFromStatusBar(.goto)
        #expect(controller.overlay == .menu(.goto))
        controller.chooseMenuEntry(2)
        #expect(controller.overlay == .none && model.selectedFolderId == 3)
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

    @Test func stepDownTurnsAroundFromTheLastRowOnly() {
        #expect(GeekController.stepDown(4, count: 5) == 0)
        #expect(GeekController.stepDown(1, count: 5) == 2)
        #expect(GeekController.stepDown(0, count: 1) == 0)
        #expect(GeekController.stepDown(0, count: 0) == 0)
        // An index left past the end (the list shrank) starts again on top.
        #expect(GeekController.stepDown(9, count: 5) == 0)
    }

    @Test func stepUpWrapsFromTheFirstRowOnly() {
        #expect(GeekController.stepUp(0, count: 5) == 4)
        #expect(GeekController.stepUp(3, count: 5) == 2)
        #expect(GeekController.stepUp(0, count: 1) == 0)
        #expect(GeekController.stepUp(0, count: 0) == 0)
        // An index left past the end (the list shrank) comes back inside.
        #expect(GeekController.stepUp(9, count: 5) == 4)
    }

    // MARK: Attachments

    /// Opens thread 4, whose message has three attachments (`Fixture`).
    func openThreadWithFiles() async {
        model.selectedThreadId = 4
        _ = await eventually { model.openThread?.id == 4 }
    }

    @Test func theLinkPickerListsTheLinksThenTheFiles() async {
        await openThreadWithFiles()
        press("l")
        let entries = controller.pickerEntries(.links)
        // One link in the text, then the three files with their icons.
        #expect(entries.count == 4)
        #expect(entries[0].icon == "link")
        #expect(entries[1].label.hasPrefix("devis.pdf") && entries[1].icon == "doc.richtext")
        #expect(entries[2].icon == "photo")
        #expect(entries[3].label.hasPrefix(L("Unnamed attachment")) && entries[3].icon == "doc.zipper")
        #expect(entries.map(\.key) == ["1", "2", "3", "4"])
        // Typing filters the files too.
        type("photo")
        #expect(controller.pickerEntries(.links).count == 1)
    }

    @Test func choosingAFileSavesItWithoutOverwriting() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-save-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model.exportDirectory = dir.path
        await openThreadWithFiles()
        press("l"); type("devis"); press("ret")
        #expect(await eventually { FileManager.default.fileExists(atPath: dir.appendingPathComponent("devis.pdf").path) })
        #expect(core.calls("get_attachment").first?["attachment_id"] as? Int == 41)
        // The user's copy is writable, unlike the core's.
        let mode = try FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent("devis.pdf").path)[.posixPermissions] as? Int
        #expect(mode == 0o644)
        #expect(model.flash?.contains("devis.pdf") == true)
        // A second save: "devis (2).pdf".
        press("l"); type("devis"); press("ret")
        #expect(await eventually { FileManager.default.fileExists(atPath: dir.appendingPathComponent("devis (2).pdf").path) })
    }

    @Test func commandReturnOnAFileSavesItThenOpensTheSavedCopy() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-open-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model.exportDirectory = dir.path
        // Records what would be opened (no app is launched by the tests).
        var opened: [URL] = []
        model.openFile = { opened.append($0) }
        await openThreadWithFiles()
        press("l"); type("devis"); press("cmd-ret")
        #expect(controller.overlay == .none)
        let saved = dir.appendingPathComponent("devis.pdf")
        #expect(await eventually { opened.count == 1 })
        // The copy in Downloads is opened, not the core's own file.
        #expect(opened.first?.path == saved.path)
        #expect(FileManager.default.fileExists(atPath: saved.path))
        // Return alone saves without opening.
        press("l"); type("devis"); press("ret")
        #expect(await eventually { FileManager.default.fileExists(atPath: dir.appendingPathComponent("devis (2).pdf").path) })
        #expect(opened.count == 1)
    }

    @Test func aTileLinkSavesTheFile() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-tile-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model.exportDirectory = dir.path
        await openThreadWithFiles()
        #expect(controller.readerLink(URL(string: "nm://attachment/42")!))
        #expect(await eventually { FileManager.default.fileExists(atPath: dir.appendingPathComponent("photo.jpg").path) })
    }

    // MARK: Search (/)

    @Test func slashSearchesEverythingWithTheContainsAttribute() async {
        press("/")
        #expect(controller.overlay == .picker(.search))
        type("devis")
        #expect(controller.pickerEntries(.search).map(\.label) == ["devis"])
        press("ret")
        // The same list as F → Contains…, over every folder of the account.
        #expect(controller.overlay == .none)
        #expect(model.attributes.contains == "devis" && model.everything == .account(1))
        #expect(await eventually {
            let call = core.calls("list_threads").last
            return (call?["folder_ids"] as? [Int])?.sorted() == [1, 2, 3, 4]
                && (call?["filter"] as? [String: Any])?["contains"] as? String == "devis"
        })
    }

    @Test func escapeAfterASearchGoesBackToTheFolder() {
        press(";"); press("a")
        #expect(model.selectedFolderId == 3)
        press("/"); type("devis"); press("ret")
        #expect(model.everything != nil && controller.searchOrigin?.folderId == 3)
        press("esc")
        #expect(model.attributes.isEmpty && model.everything == nil && model.selectedFolderId == 3)
        #expect(controller.searchOrigin == nil)
    }

    @Test func aSecondSearchKeepsTheFirstOrigin() {
        press("/"); type("devis"); press("ret")
        press("/"); type("facture"); press("ret")
        #expect(model.attributes.contains == "facture")
        press("esc")
        #expect(model.everything == nil && model.selectedFolderId == 1)
    }

    @Test func goingElsewhereForgetsWhereTheSearchStarted() {
        press("/"); type("devis"); press("ret")
        press(";"); press("t")
        #expect(model.selectedFolderId == 2 && controller.searchOrigin == nil)
        // Escape then only clears the attributes: the list stays in Todo.
        press("esc")
        #expect(model.attributes.isEmpty && model.selectedFolderId == 2)
    }

    @Test func anEmptySearchDoesNothing() {
        press("/"); press("ret")
        #expect(controller.overlay == .none && model.attributes.isEmpty && model.everything == nil)
        press("/"); type("devis"); press("esc")
        #expect(model.attributes.isEmpty && model.everything == nil)
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
        // Not available yet: a message, no call.
        press("s"); press("p")
        #expect(core.calls("export_message").count == 1 && model.flash != nil)
    }

    // MARK: Saving attachments (sa, sg)

    /// A fresh export directory, and the names of the files saved into it.
    func exportDir() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-sa-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model.exportDirectory = dir.path
        return dir
    }

    func saved(_ dir: URL) -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).sorted()
    }

    @Test func saSavesEveryAttachmentOfTheMailUnderTheCursor() async throws {
        let dir = try exportDir()
        model.selectedThreadId = 4
        _ = await eventually { model.openThread?.id == 4 }
        press("s"); press("a")
        #expect(await eventually { saved(dir) == ["attachment-43", "devis.pdf", "photo.jpg"] })
        #expect(await eventually { model.flash == L("%lld attachments saved to %@", 3, dir.lastPathComponent) })
    }

    @Test func sgLeavesTheGraphicsOut() async throws {
        let dir = try exportDir()
        model.selectedThreadId = 4
        _ = await eventually { model.openThread?.id == 4 }
        press("s"); press("g")
        #expect(await eventually { saved(dir) == ["attachment-43", "devis.pdf"] })
    }

    @Test func saActsOnEveryMarkedMail() async throws {
        let dir = try exportDir()
        // Mark threads 4, 5 and 6 (J from the fourth row); 4 and 6 have
        // files, 5 none. Thread 6 is not open: the model reads it.
        model.selectedThreadId = 4
        _ = await eventually { model.openThread?.id == 4 }
        press("J"); press("J")
        #expect(model.marked == [4, 5, 6])
        press("s"); press("a")
        #expect(await eventually { saved(dir) == ["attachment-43", "devis.pdf", "logo.png", "notes.txt", "photo.jpg"] })
        #expect(core.calls("get_thread").contains { $0["thread_id"] as? Int == 6 })
        // sg on the same marks: no graphics, and nothing overwritten.
        press("s"); press("g")
        #expect(await eventually { saved(dir).contains("notes (2).txt") && saved(dir).contains("devis (2).pdf") })
        #expect(!saved(dir).contains("logo (2).png") && !saved(dir).contains("photo (2).jpg"))
    }

    @Test func seStSmSaveTheLatestMailOfEveryMarkedThread() async {
        press("J"); press("J")
        #expect(model.marked == [1, 2, 3])
        press("s"); press("t")
        #expect(await eventually { core.calls("export_message").count == 3 })
        // The latest message of each thread (the fake core gives one
        // message per thread, id × 10), as text.
        let ids = Set(core.calls("export_message").compactMap { $0["message_id"] as? Int })
        #expect(ids == [10, 20, 30])
        #expect(core.calls("export_message").allSatisfy { $0["format"] as? String == "txt" })
        #expect(await eventually { model.flash?.contains("3") == true })
    }

    @Test func saWithoutAttachmentsSaysSo() async throws {
        let dir = try exportDir()
        press("s"); press("a")
        #expect(await eventually { model.flash == L("No attachment to save") })
        #expect(saved(dir).isEmpty)
    }

    // MARK: Text size

    @Test func commandPlusMinusAndZeroChangeTheTextSize() {
        #expect(textSize.size == 12)
        press("cmd-=")
        press("cmd-+")
        #expect(textSize.size == 14)
        #expect(model.flash?.contains("14") == true)
        press("cmd--")
        #expect(textSize.size == 13)
        press("cmd-0")
        #expect(textSize.size == 12)
    }

    @Test func otherCommandShortcutsAreLeftToTheMenus() {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "q", charactersIgnoringModifiers: "q", isARepeat: false, keyCode: 12)!
        #expect(!controller.handle(event))
    }

    // MARK: Thread display

    @Test func capitalTTogglesTheWholeConversationForThisThread() {
        #expect(!controller.readerState.showAll)
        press("T")
        #expect(controller.readerState.showAll)
        press("T")
        #expect(!controller.readerState.showAll)
    }

    @Test func readerLinksFoldMessagesAndRevealParts() {
        #expect(controller.readerLink(URL(string: "nm://fold/10")!))
        #expect(controller.readerState.folding[10] == false)
        #expect(controller.readerLink(URL(string: "nm://reveal/10/2")!))
        #expect(controller.readerState.revealed.contains("10:2"))
        // Other links are left to the text view (opened in the browser).
        #expect(!controller.readerLink(URL(string: "https://example.org")!))
    }

    // MARK: Attributes

    @Test func capitalFOpensTheAttributeSelectorAndAToggleNarrowsTheList() async {
        press("F")
        #expect(controller.overlay == .picker(.attributes))
        let labels = controller.pickerEntries(.attributes).map(\.label)
        // Compared with the translations: tests run in the machine's language.
        #expect(Array(labels.prefix(2)) == [L("unread"), L("read")])
        #expect(labels.contains(L("From…")) && labels.contains(L("Date…")))
        type(L("unread")); press("ret")
        #expect(model.attributes.seen == false)
        #expect(await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["seen"] as? Bool == false })
        // Choosing it again removes it.
        press("F"); type(L("unread")); press("ret")
        #expect(model.attributes.isEmpty)
    }

    @Test func attributesWithAValueExpandThePanel() async {
        press("F"); type(L("From…")); press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.from)))
        type("alice")
        #expect(controller.pickerEntries(.attributeValue(.from)).first?.label == "alice")
        // Each key sends its own request, in a task of its own: they may run
        // in any order (the app ignores stale answers), so look for "alice"
        // among them rather than last.
        #expect(await eventually { core.calls("list_contacts").contains { $0["query"] as? String == "alice" } })
        press("ret")
        #expect(model.attributes.from == ["alice"])
        #expect(controller.overlay == .none)
    }

    @Test func theDatePanelOffersPresetsAndTypedDates() {
        press("F"); type(L("Date…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.date)).count == 5)
        type("2026-10-01")
        let labels = controller.pickerEntries(.attributeValue(.date)).map(\.label)
        #expect(labels.first == L("after %@", "2026-10-01"))
        press("ret")
        #expect(model.attributes.after != nil && model.attributes.dateLabel == L("after %@", "2026-10-01"))
    }

    @Test func escapeClearsTheAttributesLast() {
        model.attributes.toggle(.flagged, true)
        press("J")
        press("esc")
        // First the marks…
        #expect(!model.attributes.isEmpty && model.marked.isEmpty)
        press("esc")
        // …then the attributes.
        #expect(model.attributes.isEmpty)
    }

    @Test func whenAttributesEmptyTheListTheReaderIsEmptyToo() async {
        // The fake core answers no thread when a filter is given.
        core.handler = { method, params in
            if method == "list_threads", params["filter"] != nil {
                return ["items": [], "next_cursor": NSNull(), "as_of": Fixture.position]
            }
            return Fixture.answer(method, params)
        }
        model.attributes.toggle(.flagged, true)
        #expect(await eventually { model.threads.isEmpty })
        #expect(model.selectedThreadId == nil)
        #expect(await eventually { model.openThread == nil })
        // Back without attributes: the first mail is selected again.
        model.attributes = ThreadFilter()
        #expect(await eventually { model.selectedThreadId == 1 })
    }
}
