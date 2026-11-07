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
        // A pasteboard of its own: a test that copies (a link to a mail)
        // never overwrites what the user copied.
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        model.exportDirectory = "/tmp/nicemailer-tests"
        // Never launch an app from the tests.
        model.openFile = { _ in }
        // Never read the user's address book: a book of the test's own,
        // empty unless a test fills it.
        model.addressBook = FixtureAddressBook(cards: [:])
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
        case "ctrl-cmd-f": chars = "\u{6}"; code = 3; mods = [.control, .command]
        case "shift-up": chars = "\u{F700}"; code = KeyCode.up; mods = .shift
        case "left": chars = "\u{F702}"; code = KeyCode.left; mods = .function
        case "right": chars = "\u{F703}"; code = KeyCode.right; mods = .function
        case "alt-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .option]
        case "cmd-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .command]
        case "cmd-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .command]
        case "home": chars = "\u{F729}"; code = KeyCode.home; mods = .function
        case "end": chars = "\u{F72B}"; code = KeyCode.end; mods = .function
        case "fdel": chars = "\u{F728}"; code = KeyCode.forwardDelete; mods = .function
        case "alt-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .option
        case "cmd-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .command
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

    /// `r` replies (`ComposeTests`); `R` and `f` are not there yet.
    @Test func replyToTheSenderIsNotAvailableYet() {
        press("R")
        #expect(model.flash == L("Not available yet: %@", L("reply to the sender only")))
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
        // The threads of the contact: Everything narrowed to the contact.
        #expect(await eventually {
            let call = core.calls("list_threads").last
            return (call?["filter"] as? [String: Any])?["address_id"] as? Int == 2 && call?["everything"] != nil
        })
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
        // The link to the mail itself first, then the link of its text.
        #expect(controller.pickerEntries(.links).map(\.label) == [L("Link to this mail"), "example.org"])
        #expect(controller.pickerEntries(.links).map(\.detail) == ["nicemailer://m/10", "https://example.org/1"])
        press("esc")
        #expect(controller.overlay == .none)
    }

    @Test func theLinkPickerShowsTheTextOfHtmlLinksAndFindsThemByAddress() async {
        // A newsletter: its text, made from the HTML, has no address; the
        // core gives the links of the HTML with their text.
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            var thread = Fixture.threadView(2)
            var m = (thread["messages"] as! [[String: Any]])[0]
            m["body_text"] = "Votre facture est disponible. Voir ma facture"
            m["links"] = [
                ["url": "https://example.org/plain", "text": NSNull()],
                ["url": "https://t.example.com/c?u=1", "text": "Voir ma facture"],
                ["url": "mailto:aide@example.org", "text": "Écrire au support"],
                ["url": "tel:0612345678", "text": "06 12 34 56 78"],
            ]
            thread["messages"] = [m]
            return thread
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        press("l")
        let rows = controller.pickerEntries(.links)
        // After the link to the mail itself: links with a text, the phone
        // number, then the link without text.
        #expect(rows.first?.label == L("Link to this mail"))
        let entries = Array(rows.dropFirst())
        #expect(entries.map(\.label) == ["Voir ma facture", "Écrire au support", "06 12 34 56 78", "example.org"])
        #expect(entries.map(\.detail) == ["https://t.example.com/c?u=1", "aide@example.org", nil, "https://example.org/plain"])
        #expect(entries.map(\.icon) == ["link", "link", "phone", "link"])
        // Where a link goes is information, not a count.
        #expect(entries.allSatisfy { $0.detailStyle == .info })
        // A part of the address finds the row, though it is not shown.
        type("u=1")
        #expect(controller.pickerEntries(.links).map(\.label) == ["Voir ma facture"])
        // Return on the number hands its tel: URL to the system, which
        // opens the phone app.
        var opened: URL?
        model.openFile = { opened = $0 }
        press("esc"); press("l"); type("06 12")
        press("ret")
        #expect(opened?.absoluteString == "tel:0612345678")
    }

    // MARK: Deep links

    @Test func theFirstRowOfLCopiesAMarkdownLinkAndCommandReturnTheURL() {
        press("l")
        press("ret")
        #expect(controller.pasteboard.string(forType: .string) == "[Subject 1](nicemailer://m/10)")
        #expect(controller.overlay == .none)
        #expect(model.flash?.contains("nicemailer://m/10") == true)
        press("l")
        press("cmd-ret")
        #expect(controller.pasteboard.string(forType: .string) == "nicemailer://m/10")
        // Found by typing what one looks for.
        press("l"); type("obsidian")
        #expect(controller.pickerEntries(.links).first?.label == L("Link to this mail"))
    }

    @Test func aDeepLinkOpensTheMailInItsFolderAndClosesThePanels() async {
        core.handler = { method, params in
            guard method == "locate_message" else { return Fixture.answer(method, params) }
            if params["message_id"] as? Int == 50 {
                return ["message_id": 50, "thread_id": 5, "folder_id": 3, "account_id": 1]
            }
            return CoreClient.RPCError(code: CoreClient.RPCError.notFound, message: "message not found")
        }
        controller.columns = 1
        press("l")
        model.openDeepLink(URL(string: "nicemailer://m/50")!)
        #expect(await eventually { model.selectedThreadId == 5 })
        #expect(model.selectedFolderId == 3)
        #expect(await eventually { controller.overlay == .none && controller.reading })
        #expect(await eventually { model.openThread?.id == 5 })
    }

    @Test func aDeepLinkToAMissingMailSaysSo() async {
        core.handler = { method, params in
            guard method == "locate_message" else { return Fixture.answer(method, params) }
            return CoreClient.RPCError(code: CoreClient.RPCError.notFound, message: "message not found")
        }
        let before = model.selectedThreadId
        model.openDeepLink(URL(string: "nicemailer://m/999")!)
        #expect(await eventually { model.lastError == L("This mail is no longer in NiceMailer") })
        #expect(model.selectedThreadId == before)
        // Not a link of this app: refused without asking the core.
        model.openDeepLink(URL(string: "nicemailer://x/1")!)
        #expect(model.lastError == L("Not a link to a NiceMailer mail: %@", "nicemailer://x/1"))
        #expect(core.calls("locate_message").count == 1)
    }

    @Test func unreadCountsStandOutAndInformationStaysMuted() {
        // The account picker's unread counts stand out.
        press("@")
        #expect(controller.pickerEntries(.accounts).filter { $0.detail != nil }.allSatisfy { $0.detailStyle == .count })
        press("esc")
        // How six digits were read is information.
        press("F"); type(L("Date…")); press("ret")
        type("010120")
        #expect(controller.pickerEntries(.attributeValue(.date)).prefix(4).allSatisfy { $0.detailStyle == .info })
        press("esc")
        // An interval left empty is a warning: it stands out.
        press("F"); type(L("Date…")); press("ret")
        type("2026-10-01"); press("ret")
        press("F"); type(L("Date…")); press("ret")
        type("2026-09-01")
        let warning = controller.pickerEntries(.attributeValue(.date))[1]
        #expect(warning.detail == L("nothing in between") && warning.detailStyle == .count)
        press("esc")
        // "domain" on a From… value.
        press("F"); type(L("From…")); press("ret")
        type("@acme.fr")
        #expect(controller.pickerEntries(.attributeValue(.from)).first?.detailStyle == .info)
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
        // After the link to the mail itself (first row): the three files
        // with their icons, then the link written in the text (no text of
        // its own: after the files).
        let all = controller.pickerEntries(.links)
        #expect(all.first?.label == L("Link to this mail") && all.map(\.key) == ["1", "2", "3", "4", "5"])
        let entries = Array(all.dropFirst())
        #expect(entries.count == 4)
        // The name on the left, the size in the right half.
        #expect(entries[0].label == "devis.pdf" && entries[0].icon == "doc.richtext")
        #expect(entries.prefix(3).allSatisfy { $0.detail?.isEmpty == false && $0.detailStyle == .info })
        #expect(entries[1].icon == "photo")
        #expect(entries[2].label == L("Unnamed attachment") && entries[2].icon == "doc.zipper")
        #expect(entries[3].icon == "link" && entries[3].label == "example.org" && entries[3].detail == "https://example.org/4")
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
        // The user's copy is writable, unlike the core's: its permissions
        // are set a moment after the copy appears.
        let mode = { (try? FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent("devis.pdf").path))?[.posixPermissions] as? Int }
        #expect(await eventually { mode() == 0o644 })
        // Said once the copy is done, a moment after the file appears.
        #expect(await eventually { model.flash?.contains("devis.pdf") == true })
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

    // MARK: Editing the typed text

    /// Puts text on the controller's own pasteboard (never the user's).
    func copy(_ text: String) {
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        controller.pasteboard.clearContents()
        controller.pasteboard.setString(text, forType: .string)
    }

    @Test func theLeftArrowGoesBackToEditTheTypedText() {
        press("F"); type(L("From…")); press("ret")
        type("alce")
        press("left"); press("left")
        #expect(controller.pickerLine.cursor == 2)
        // The arrow adds nothing to the text (it used to add its character).
        #expect(controller.pickerQuery == "alce")
        type("i")
        #expect(controller.pickerQuery == "alice" && controller.pickerLine.cursor == 3)
        press("right"); press("fdel")
        #expect(controller.pickerQuery == "alic")
        press("home"); press("fdel")
        #expect(controller.pickerQuery == "lic" && controller.pickerLine.cursor == 0)
        press("end"); press("del")
        #expect(controller.pickerQuery == "li")
        press("cmd-left"); type("a")
        #expect(controller.pickerQuery == "ali")
        press("ctrl-e"); type("ce")
        #expect(controller.pickerQuery == "alice")
        press("ctrl-a")
        #expect(controller.pickerLine.cursor == 0)
        press("cmd-right")
        #expect(controller.pickerLine.cursor == 5)
        // Still a picker: Return applies the edited text.
        press("ret")
        #expect(model.attributes.from == ["alice"])
    }

    @Test func wordKeysEditTheSearch() {
        press("/"); type("devis toiture")
        press("alt-left")
        #expect(controller.pickerLine.afterCursor == "toiture")
        press("alt-del")
        #expect(controller.pickerQuery == "toiture")
        press("end"); press("cmd-del")
        #expect(controller.pickerQuery == "" && controller.overlay == .picker(.search))
    }

    @Test func editingKeysKeepThePickerOpenAndCtrlNPStillMove() {
        press("F")
        press("down")
        #expect(controller.pickerIndex == 1)
        press("left"); press("right")
        #expect(controller.pickerIndex == 1)
        press("ctrl-n")
        #expect(controller.pickerIndex == 2)
        #expect(controller.overlay == .picker(.attributes))
    }

    @Test func commandVPastesAtTheCursor() async {
        copy("  alice@acme.fr\n")
        press("F"); type(L("From…")); press("ret")
        press("cmd-v")
        #expect(controller.pickerQuery == "alice@acme.fr")
        // Pasted text is looked for like typed text.
        #expect(await eventually { core.calls("list_contacts").contains { $0["query"] as? String == "alice@acme.fr" } })
        press("ret")
        #expect(model.attributes.from == ["alice@acme.fr"])
        // In the middle of the text, at the cursor; the spaces around a
        // pasted text go (a double-click copy often takes one), those
        // typed stay.
        copy("toiture\n")
        press("/"); type("devis  2024")
        press("alt-left"); press("left"); press("cmd-v")
        #expect(controller.pickerQuery == "devis toiture 2024")
    }

    @Test func commandVOutsideATypedLineIsLeftToTheMenus() {
        copy("x")
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "v", charactersIgnoringModifiers: "v", isARepeat: false, keyCode: 9)!
        #expect(!controller.handle(event))
        #expect(controller.overlay == .none)
    }

    @Test func anEmptyPasteChangesNothing() {
        copy("\n ")
        press("F"); type("un")
        press("cmd-v")
        #expect(controller.pickerQuery == "un")
    }

    @Test func theContactsFilterIsEditedTheSameWay() async {
        press(";"); press("c")
        press("/"); type("ab")
        press("left"); type("x")
        #expect(controller.contactsQuery == "axb")
        copy("acme")
        press("cmd-v")
        #expect(controller.contactsQuery == "axacmeb")
        #expect(await eventually { core.calls("list_contacts").last?["query"] as? String == "axacmeb" })
        // Control keys other than ctrl-a / ctrl-e type nothing.
        press("ctrl-n")
        #expect(controller.contactsQuery == "axacmeb")
    }

    @Test func aClickOutsideClosesThePickerTheMenuAndTheHelp() {
        press("F")
        #expect(controller.closesOnClickOutside)
        controller.clickOutsideOverlay()
        #expect(controller.overlay == .none)
        // Nothing else happens: the mail under the click is not selected.
        #expect(selectedIndex == 0)
        press("/"); type("devis")
        controller.clickOutsideOverlay()
        #expect(controller.overlay == .none && model.attributes.contains == nil)
        press("m")
        controller.clickOutsideOverlay()
        #expect(controller.overlay == .none)
        press("?")
        controller.clickOutsideOverlay()
        #expect(controller.overlay == .none)
        #expect(!controller.closesOnClickOutside)
    }

    @Test func aClickOutsideDoesNotCloseTheContactsFilter() {
        press(";"); press("c"); press("/")
        #expect(!controller.closesOnClickOutside)
        controller.clickOutsideOverlay()
        #expect(controller.overlay == .contactFilter)
    }

    // MARK: Search (/)

    @Test func slashSearchesEverythingWithTheAnywhereCriterion() async {
        press("/")
        #expect(controller.overlay == .picker(.search))
        type("devis")
        #expect(controller.pickerEntries(.search).map(\.label) == ["devis"])
        press("ret")
        // Words anywhere, addresses included, over every folder of the account.
        #expect(controller.overlay == .none)
        #expect(model.attributes.contains == "devis" && model.everything == .account(1))
        #expect(await eventually {
            let call = core.calls("list_threads").last
            return (call?["everything"] as? [String: Any])?["account_id"] as? Int == 1
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
        // At the top by default (the title bar is hidden); `vs` hides it.
        #expect(controller.statusPosition == .top)
        press("v"); press("s")
        #expect(controller.statusPosition == .hidden)
        // A new controller reads the remembered choices.
        let again = GeekController(model: model, defaults: defaults)
        #expect(again.columns == 3 && again.statusPosition == .hidden)
    }

    @Test func returnReadsTheMailInOneColumnAndEscapeGoesBackToTheList() {
        press("v"); press("1")
        #expect(controller.columns == 1 && !controller.reading)
        press("ret")
        #expect(controller.reading)
        // j / k keep moving the cursor: the reader follows the next mail.
        press("j")
        #expect(controller.reading && selectedIndex == 1)
        press("esc")
        #expect(!controller.reading && selectedIndex == 1)
    }

    @Test func theStatusBarShowsTheAppIconInTheNormalModeOnly() {
        #expect(controller.modeLabel == "NOR" && controller.showsAppIcon)
        // The image is in the app's resources.
        #expect(AppIcon.statusImage != nil)
        press("v")
        #expect(controller.modeLabel == "v" && !controller.showsAppIcon)
        press("esc")
        #expect(controller.showsAppIcon)
        press("@")
        #expect(controller.modeLabel == "PCK" && !controller.showsAppIcon)
    }

    @Test func controlCommandFTogglesFullScreenEverywhere() {
        var toggles = 0
        controller.toggleFullScreen = { toggles += 1 }
        press("ctrl-cmd-f")
        #expect(toggles == 1)
        // Not taken for the f of the list (find), even in a menu.
        press("v")
        press("ctrl-cmd-f")
        #expect(toggles == 2)
        press("esc")
        // ⌘F alone is not it: left to AppKit.
        let commandF = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "f", charactersIgnoringModifiers: "f", isARepeat: false, keyCode: 3)!
        #expect(!controller.handle(commandF))
        #expect(toggles == 2)
    }

    @Test func spaceOpensTheMailInOneColumnThenScrollsIt() {
        press("v"); press("1")
        #expect(!controller.reading)
        press(" ")
        #expect(controller.reading)
        // Open: Space scrolls the mail, it does not close it.
        press(" ")
        #expect(controller.reading)
        press("esc")
        #expect(!controller.reading)
        // Two columns: the mail is on screen, Space scrolls it.
        press("v"); press("2")
        press(" ")
        #expect(!controller.reading)
    }

    @Test func escapeLeavesTheReaderBeforeUnmarking() {
        press("v"); press("1")
        press("J")
        let marked = model.marked
        #expect(!marked.isEmpty)
        press("ret"); press("esc")
        // The first Escape only closes the reader; the marks stay.
        #expect(!controller.reading && model.marked == marked)
        press("esc")
        #expect(model.marked.isEmpty)
    }

    @Test func aDoubleClickReadsTheMailInOneColumn() {
        press("v"); press("1")
        controller.clickThread(3, .none)
        #expect(!controller.reading)
        controller.clickThread(3, .none)
        #expect(controller.reading && model.selectedThreadId == 3)
        press("esc")
        #expect(!controller.reading)
    }

    @Test func onlyTwoPlainClicksOnTheSameRowMakeADoubleClick() {
        press("v"); press("1")
        // Two different rows.
        controller.clickThread(3, .none)
        controller.clickThread(4, .none)
        #expect(!controller.reading)
        // A ⌘-click, then a click on the same row.
        controller.clickThread(5, .command)
        controller.clickThread(5, .none)
        #expect(!controller.reading)
        // Two ⌘-clicks: marking, not reading.
        controller.clickThread(6, .command)
        controller.clickThread(6, .command)
        #expect(!controller.reading)
    }

    @Test func aDoubleClickWithTwoColumnsIsAPlainClick() {
        press("v"); press("2")
        controller.clickThread(3, .none)
        controller.clickThread(3, .none)
        #expect(!controller.reading && model.selectedThreadId == 3)
    }

    @Test func returnDoesNothingWithTwoColumnsAndAnotherLayoutLeavesTheReader() {
        press("v"); press("2")
        press("ret")
        #expect(!controller.reading)
        press("v"); press("1"); press("ret")
        #expect(controller.reading)
        press("v"); press("3")
        #expect(!controller.reading)
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

    /// Thread 3 with a remote image and two tracking pixels, from a sender
    /// whose mail is `auth`, in `role`; `from_body_format` follows
    /// `set_body_format`, as with the core.
    func remoteImages(auth: String = "pass", role: String = "inbox", remembered: String? = nil) {
        var format: Any = remembered ?? NSNull()
        core.handler = { method, params in
            switch method {
            case "set_body_format":
                format = params["format"] ?? NSNull()
                return ["changed": 1]
            case "get_thread" where params["thread_id"] as? Int == 3:
                var thread = Fixture.threadView(3)
                var m = (thread["messages"] as! [[String: Any]])[0]
                m["body_html"] = #"<p>Hi</p><img src="nm-remote://image?u=https%3A%2F%2Fimg.example.com%2Fa.png">"#
                m["has_remote_content"] = true
                m["tracking_pixels"] = 2
                m["auth_result"] = auth
                m["folder_role"] = role
                m["from_body_format"] = format
                thread["messages"] = [m]
                return thread
            default:
                return Fixture.answer(method, params)
            }
        }
    }

    func openThree() async {
        press("j"); press("j")
        _ = await eventually { model.openThread?.id == 3 }
        await model.reloadOpenThread()
    }

    @Test func imagesAreRememberedForAnAuthenticatedSender() async {
        remoteImages()
        await openThree()
        #expect(model.imagesOffered && model.trackingPixels == 2 && model.bodyFormat == "text")
        press("v"); press("i")
        #expect(await eventually { model.bodyFormat == "images" })
        #expect(core.calls("set_body_format").last?["format"] as? String == "images")
        #expect(core.calls("set_body_format").last?["addr"] as? String == "sender3@example.org")
        #expect(model.remoteImagesOnce == nil)
        // `vi` again: HTML without images; `vh` from images: text.
        press("v"); press("i")
        #expect(await eventually { model.bodyFormat == "html" })
        #expect(core.calls("set_body_format").last?["format"] as? String == "html")
        press("v"); press("i")
        #expect(await eventually { model.bodyFormat == "images" })
        press("v"); press("h")
        #expect(await eventually { model.bodyFormat == "text" })
    }

    @Test func anUnauthenticatedSenderGetsImagesForThisConversationOnly() async {
        remoteImages(auth: "unknown")
        await openThree()
        press("v"); press("i")
        #expect(await eventually { model.bodyFormat == "images" })
        // Remembered, but applied here only because it was asked here.
        #expect(core.calls("set_body_format").last?["format"] as? String == "images")
        #expect(model.remoteImagesOnce == 3)
        #expect(await eventually {
            model.flash == L("Images loaded for this conversation only: %@ is not authenticated", "sender3@example.org")
        })
        press("v"); press("i")
        #expect(await eventually { model.bodyFormat == "html" })
        #expect(model.remoteImagesOnce == nil)
    }

    @Test func aRememberedChoiceDoesNotLoadTheImagesOfAnUnauthenticatedMail() async {
        remoteImages(auth: "none", remembered: "images")
        await openThree()
        #expect(model.bodyFormat == "html")
        remoteImages(auth: "pass", remembered: "images")
        await model.reloadOpenThread()
        #expect(model.bodyFormat == "images")
    }

    @Test func imagesAreNeverLoadedInJunk() async {
        remoteImages(role: "junk")
        await openThree()
        #expect(!model.imagesOffered)
        press("v"); press("i")
        #expect(core.calls("set_body_format").isEmpty)
        #expect(model.flash == L("Images are never loaded in Junk, Quarantine or Banned"))
    }

    @Test func imagesAreRefusedForAMailWithoutRemoteImages() async {
        press("j"); press("j")
        #expect(await eventually { model.openThread?.id == 3 })
        #expect(!model.imagesOffered)
        press("v"); press("i")
        #expect(core.calls("set_body_format").isEmpty && model.flash == L("This mail has no remote images"))
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

    @Test func commandAMarksEveryMailOfTheList() {
        press("cmd-a")
        #expect(model.marked == Set(model.threads.map(\.id)) && model.marked.count == 8)
        // The status bar shows the count already: no message on top.
        #expect(model.flash == nil)
        // Escape unmarks, as after J / K.
        press("esc")
        #expect(model.marked.isEmpty)
        // Not with a panel open: ⌘A is left to the rest of the app.
        press("?")
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "a", charactersIgnoringModifiers: "a", isARepeat: false, keyCode: 0)!
        #expect(!controller.handle(event))
        #expect(model.marked.isEmpty)
    }

    @Test func ctrlKAndCtrlJStepThroughTheConversation() async {
        // A single mail: nowhere to go.
        press("ctrl-k")
        #expect(model.flash == L("A single mail: no other message"))
        // Thread 2: two messages, the second unfolded.
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            var thread = Fixture.threadView(2)
            var first = (thread["messages"] as! [[String: Any]])[0]
            var second = first
            first["id"] = 21
            first["expanded"] = false
            second["id"] = 22
            thread["messages"] = [first, second]
            return thread
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.messages.count == 2 }
        press("ctrl-k")
        #expect(controller.readerState.folding == [22: false, 21: true])
        #expect(controller.readerState.current == 21)
        press("ctrl-k")
        #expect(model.flash == L("First message of the conversation"))
        press("ctrl-j")
        #expect(controller.readerState.folding == [21: false, 22: true])
        press("ctrl-j")
        #expect(model.flash == L("Last message of the conversation"))
    }

    @Test func aClickOnTheSendersAddressCopiesItAndCommandFiltersOnIt() {
        // The open thread's message is 10, from sender1@example.org.
        let url = URL(string: "nm://sender/10")!
        #expect(controller.readerLink(url, command: false))
        #expect(controller.pasteboard.string(forType: .string) == "sender1@example.org")
        #expect(model.flash == L("Copied: %@", "sender1@example.org"))
        #expect(model.attributes.from.isEmpty)
        // ⌘-click: a "from:" chip, as `F` would add.
        #expect(controller.readerLink(url, command: true))
        #expect(model.attributes.from == ["sender1@example.org"])
        #expect(model.flash == L("Mails from %@", "sender1@example.org"))
        // A message not in the open thread: nothing.
        model.flash = nil
        #expect(controller.readerLink(URL(string: "nm://sender/999")!, command: false))
        #expect(model.flash == nil)
    }

    @Test func capitalTAndMSaveThenOpenInFront() async {
        var opened: [String] = []
        model.openFile = { opened.append($0.path) }
        // `t` saves only.
        press("s"); press("t")
        #expect(await eventually { core.calls("export_message").count == 1 })
        #expect(opened.isEmpty)
        // `T` saves, then opens the file, and says so.
        press("s"); press("T")
        #expect(await eventually { opened == ["/tmp/x.eml"] })
        #expect(core.calls("export_message").last?["format"] as? String == "txt")
        #expect(model.flash == L("Saved and opened: %@", "/tmp/x.eml"))
        // `M`: Markdown, three marked mails, three files opened.
        opened = []
        press("J"); press("J")
        press("s"); press("M")
        #expect(await eventually { opened.count == 3 })
        #expect(core.calls("export_message").suffix(3).allSatisfy { $0["format"] as? String == "md" })
        #expect(await eventually { model.flash?.contains("3") == true })
    }

    @Test func beyondFiveFilesTheyAreSavedButNotOpened() async {
        var opened: [String] = []
        model.openFile = { opened.append($0.path) }
        for _ in 0..<5 { press("J") }
        #expect(model.marked.count == 6)
        press("s"); press("T")
        #expect(await eventually { core.calls("export_message").count == 6 })
        #expect(await eventually { model.flash == L("%lld mails saved to %@, not opened: more than %lld", 6, "tmp", 5) })
        #expect(opened.isEmpty)
    }

    @Test func theSaveMenuShowsTheSecondKeyAndCommandReturnRunsIt() async {
        let rows = controller.menuEntries(.save)
        let text = rows.first { $0.key == "t" }
        #expect(text?.altKey == "T" && text?.detail == L("%@: and open", "T") && text?.detailStyle == .info)
        #expect(rows.first { $0.key == "m" }?.altKey == "M")
        // The arrows to the Markdown row, then ⌘-Return.
        var opened: [String] = []
        model.openFile = { opened.append($0.path) }
        press("s")
        let index = rows.firstIndex { $0.key == "m" }!
        for _ in 0...index { press("down") }
        press("cmd-ret")
        #expect(await eventually { opened.count == 1 })
        #expect(core.calls("export_message").last?["format"] as? String == "md")
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

    @Test func typingBringsTheBestMatchFirstInTheAttributeSelector() {
        // The opposite value contains the typed word ("unread" / "non lus")
        // and is listed first: ranking puts the exact one under Return.
        press("F"); type(L("read"))
        #expect(controller.pickerEntries(.attributes).first?.label == L("read"))
        press("ret")
        #expect(model.attributes.seen == true)
        // Initials reach a negative value.
        let initials = String(PickerMatch.words(PickerMatch.normalize(L("not flagged"))).compactMap(\.first))
        press("F"); type(initials)
        #expect(controller.pickerEntries(.attributes).first?.label == L("not flagged"))
        press("ret")
        #expect(model.attributes.flagged == false)
        // A hidden search word reaches To….
        press("F"); type("cc")
        #expect(controller.pickerEntries(.attributes).first?.label == L("To…"))
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

    @Test func aValuePanelOffersTheValueThenItsExclusion() async {
        press("F"); type(L("From…")); press("ret")
        type("@acme.fr")
        var rows = controller.pickerEntries(.attributeValue(.from))
        #expect(rows.map(\.key).prefix(2) == ["+", "−"])
        #expect(rows[1].label == L("not %@", "@acme.fr") && rows[0].detail == L("domain"))
        // One arrow down, then Return: excluded.
        press("down"); press("ret")
        #expect(model.attributes.notFrom == ["@acme.fr"] && model.attributes.from.isEmpty)
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["not_from"] as? [String] == ["@acme.fr"]
        })
        // A leading "-" leaves only the exclusion, and the contacts are
        // looked for without it.
        press("F"); type(L("To…")); press("ret")
        type("-alice")
        rows = controller.pickerEntries(.attributeValue(.to))
        #expect(rows.first?.key == "−" && rows.first?.label == L("not %@", "alice"))
        #expect(await eventually { core.calls("list_contacts").contains { $0["query"] as? String == "alice" } })
        press("ret")
        #expect(model.attributes.notTo == ["alice"])
    }

    @Test func textAndSubjectPanelsTakeWordsAndExclusions() {
        press("F"); type(L("Text…")); press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.text)))
        type("acme.fr"); press("ret")
        #expect(model.attributes.text == "acme.fr")
        // An exclusion decided after the fact: → leaves the current words,
        // selected, for the end of the line.
        press("F"); type(L("Text…")); press("ret")
        press("right"); type(" -facture"); press("ret")
        #expect(model.attributes.text == "acme.fr -facture")
        press("F"); type(L("Subject…")); press("ret")
        type("-panne"); press("ret")
        #expect(model.attributes.subject == "-panne")
        #expect(model.attributes.contains == nil)
    }

    @Test func aSecondSubjectReplacesTheFirst() async {
        press("F"); type(L("Subject…")); press("ret")
        type("devis"); press("ret")
        #expect(model.attributes.subject == "devis")
        // The panel opens on the current subject, selected: typing
        // replaces it.
        press("F"); type(L("Subject…")); press("ret")
        #expect(controller.pickerQuery == "devis" && controller.pickerLine.selectedAll)
        type("facture")
        #expect(controller.pickerQuery == "facture" && !controller.pickerLine.selectedAll)
        press("ret")
        #expect(model.attributes.subject == "facture")
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["subject"] as? String == "facture"
        })
        // Return without touching it keeps it as it is.
        press("F"); type(L("Subject…")); press("ret"); press("ret")
        #expect(model.attributes.subject == "facture")
        // A paste replaces it too.
        copy("rapport\n")
        press("F"); type(L("Subject…")); press("ret"); press("cmd-v"); press("ret")
        #expect(model.attributes.subject == "rapport")
    }

    @Test func erasingTheWordsOfThePanelRemovesTheCriterion() {
        press("F"); type(L("Subject…")); press("ret")
        type("devis"); press("ret")
        press("F"); type(L("Subject…")); press("ret")
        // Delete clears the selected words; Return then removes the chip.
        press("del")
        #expect(controller.pickerQuery == "")
        let rows = controller.pickerEntries(.attributeValue(.subject))
        #expect(rows.map(\.label) == [L("remove %@", L("subject: %@", "devis"))])
        press("ret")
        #expect(model.attributes.subject == nil && model.attributes.isEmpty)
        // With no criterion, an empty panel offers nothing.
        press("F"); type(L("Subject…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.subject)).isEmpty && !controller.pickerLine.selectedAll)
    }

    @Test func valuePanelsSayWhatCanBeTyped() {
        press("F"); type(L("Date…")); press("ret")
        // The Date… panel opens on its presets: the hint is on the input
        // line, not only in an empty panel.
        #expect(controller.pickerHint(.attributeValue(.date))?.contains("011026") == true)
        #expect(controller.pickerHint(.attributes) == nil)
        for field in AttributeField.allCases { #expect(controller.pickerHint(.attributeValue(field)) != nil) }
        // Six digits read two ways: four rows, each saying its reading.
        type("010120")
        let rows = controller.pickerEntries(.attributeValue(.date))
        #expect(rows.prefix(4).map(\.detail) == [L("dd mm yy"), L("dd mm yy"), L("yy mm dd"), L("yy mm dd")])
        press("down"); press("down"); press("ret")
        #expect(model.attributes.afterLabel == L("after %@", "2001-01-20"))
    }

    @Test func theDatePanelOffersPresetsAndTypedDates() {
        press("F"); type(L("Date…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.date)).count == 5)
        type("2026-10-01")
        let labels = controller.pickerEntries(.attributeValue(.date)).map(\.label)
        #expect(labels.first == L("after %@", "2026-10-01"))
        press("ret")
        #expect(model.attributes.after != nil && model.attributes.afterLabel == L("after %@", "2026-10-01"))
    }

    @Test func theDatePanelBuildsAnIntervalAndReadsDurations() {
        press("F"); type(L("Date…")); press("ret")
        type("2026-10-01"); press("ret")
        // An upper bound kept with the lower one: two chips.
        press("F"); type(L("Date…")); press("ret")
        type("2026-11-01"); press("down"); press("ret")
        #expect(model.attributes.chips.map(\.label) == [L("after %@", "2026-10-01"), L("before %@", "2026-11-01")])
        // An upper bound before the lower one: the row says so.
        press("F"); type(L("Date…")); press("ret")
        type("2026-09-01")
        let rows = controller.pickerEntries(.attributeValue(.date))
        #expect(rows[1].label == L("before %@", "2026-09-01") && rows[1].detail == L("nothing in between"))
        #expect(rows[0].detail == nil)
        press("esc")
        // A duration: newer than, then older than, with the date it gives.
        press("F"); type(L("Date…")); press("ret")
        type("3s")
        let d = controller.pickerEntries(.attributeValue(.date))
        #expect(d.prefix(2).map(\.label) == [L("newer than %@", L("%lld weeks", 3)), L("older than %@", L("%lld weeks", 3))])
        #expect(d[0].detail?.count == 10)
        press("ret")
        #expect(model.attributes.afterLabel == L("newer than %@", L("%lld weeks", 3)))
        #expect(model.attributes.beforeLabel == L("before %@", "2026-11-01"))
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
