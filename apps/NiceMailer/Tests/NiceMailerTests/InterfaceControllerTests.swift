import AppKit
import Foundation
import PDFKit
import Testing
@testable import NiceMailer

/// Keyboard and mouse handling of the geek theme (`InterfaceController`),
/// against a fake core (`FakeCore`). Each test builds a fresh model,
/// controller and preferences domain.
@MainActor
struct InterfaceControllerTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()
    let defaults: UserDefaults
    let textSize: TextSize
    let controller: InterfaceController

    init() async {
        defaults = TestDefaults.make()
        textSize = TextSize()
        controller = InterfaceController(model: model, defaults: defaults, textSize: textSize)
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
        #expect(InterfaceController.htmlScrollScript(pages: 1)
            == "window.scrollBy(0, Math.round(window.innerHeight * 1.0)); window.scrollY")
        #expect(InterfaceController.htmlScrollScript(pages: -0.5).contains("innerHeight * -0.5"))
    }

    @Test func halfAPageIsHalfTheVisibleRowsAndNeverZero() {
        #expect(InterfaceController.halfPage(40) == 20)
        #expect(InterfaceController.halfPage(7) == 3)
        #expect(InterfaceController.halfPage(1) == 1)
        #expect(InterfaceController.halfPage(0) == 1)
        #expect(InterfaceController.visibleRows(height: 900, rowHeight: 20) == 45)
        #expect(InterfaceController.visibleRows(height: 39, rowHeight: 20) == 1)
        #expect(InterfaceController.visibleRows(height: 0, rowHeight: 20) == 1)
        #expect(InterfaceController.visibleRows(height: 100, rowHeight: 0) == 1)
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
        #expect(await eventually { core.moveCalls().count == 1 })
        let call = core.moveCalls()[0]
        #expect(call["target"] as? String == "todo")
        #expect(call["from_folder_id"] as? Int == 1)
        #expect(call["thread_id"] as? Int == 1)
        // Optimistic update: the thread left the list, the cursor stays.
        #expect(model.threads.count == 7 && selectedIndex == 0)
    }

    /// From Sent, `m` + a letter files the conversation (`dossiers.md`,
    /// "Cibles proposées"). Its row is not removed at once: the copies in
    /// Sent keep the conversation in that list unless it has nothing else,
    /// so the list is read again from the core.
    @Test func aMoveFromSentKeepsTheRowUntilTheCoreAnswers() async {
        core.handler = { method, params in
            if method == "list_folders" {
                let sent = Fixture.folder(5, "sent", "Sent", "S", targets: ["todo", "waiting", "inbox", "archive", "deleted"])
                return ["items": Fixture.folders + [sent], "as_of": Fixture.position]
            }
            return Fixture.answer(method, params)
        }
        await model.reloadFolders()
        model.selectedFolderId = 5
        #expect(await eventually { model.threads.first?.id == 105 })
        let listed = core.calls("list_threads").count
        press("m"); press("w")
        #expect(await eventually { core.moveCalls().count == 1 })
        let call = core.moveCalls()[0]
        #expect(call["target"] as? String == "waiting")
        #expect(call["from_folder_id"] as? Int == 5)
        // The row stayed, and the list was read again after the answer.
        #expect(model.threads.contains { $0.id == 105 })
        #expect(await eventually { core.calls("list_threads").count > listed })
        #expect(AppModel.moveMayKeepRow(from: "sent") && !AppModel.moveMayKeepRow(from: "inbox"))
    }

    /// Holding `j` over the list asks the core for the mail finally under
    /// the cursor, not for every mail passed over: one `get_thread` at a
    /// time, then the selection of the moment (`AppModel.loadOpenThread`).
    @Test func movingFastLoadsOnlyTheMailFinallyUnderTheCursor() async {
        core.delays["get_thread"] = .milliseconds(300)
        let before = core.calls("get_thread").count
        let asked = { core.calls("get_thread").dropFirst(before).map { $0["thread_id"] as? Int } }
        // The first move: its load is in flight (the core takes 300 ms).
        press("j")
        #expect(await eventually { asked() == [2] })
        // Four more moves while it loads, as a held key repeats.
        for _ in 0..<4 { press("j") }
        #expect(model.selectedThreadId == 6)
        #expect(await eventually { model.openThread?.id == 6 })
        // Then only the mail finally under the cursor: the three passed
        // over are never asked for.
        if asked() != [2, 6] { Issue.record("asked: \(asked())") }
    }

    /// The merge of two loads asked for: the latest selection, marked read
    /// if either asked for it on the same mail.
    @Test func aLoadAskedMeanwhileReplacesTheOneWaiting() {
        let chosen = ThreadLoad(id: 3, markSeen: true)
        let refresh = ThreadLoad(id: 3, markSeen: false)
        #expect(ThreadLoad.merge(nil, refresh) == refresh)
        #expect(ThreadLoad.merge(chosen, refresh) == chosen)
        #expect(ThreadLoad.merge(refresh, chosen) == chosen)
        #expect(ThreadLoad.merge(chosen, ThreadLoad(id: 4, markSeen: false)) == ThreadLoad(id: 4, markSeen: false))
        #expect(ThreadLoad.merge(chosen, ThreadLoad(id: nil, markSeen: false)) == ThreadLoad(id: nil, markSeen: false))
    }

    @Test func dArchivesEveryMarkedThread() async {
        press("J"); press("J")
        press("d")
        #expect(await eventually { core.moveCalls().count == 3 })
        // One request for the three (`ui.md`, "Actions groupées").
        #expect(core.calls("move_threads").count == 1)
        #expect(core.moveCalls().allSatisfy { $0["target"] as? String == "archive" })
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
        #expect(await eventually { core.moveCalls().first?["target"] as? String == "deleted" })
    }

    @Test func escapeCancelsAPrefixAndUnknownKeysAreReported() {
        press("m"); press("esc")
        #expect(controller.overlay == .none && core.moveCalls().isEmpty)
        press("y")
        #expect(model.flash != nil)
        press("m"); press("z")
        #expect(model.flash != nil && core.moveCalls().isEmpty)
    }

    /// `U`, `*` and `t` on marked threads (`ui.md`, "Actions groupées"):
    /// one request with every thread, and the rows change at once, all
    /// together, before the core answers.
    @Test func groupedActionsAreOneRequestShownAtOnce() async {
        // The cursor's thread and the next two, marked.
        press("J"); press("J")
        let ids = Set(model.marked)
        #expect(ids.count == 3)
        press("U")
        // At once: no row of the three is unread any more.
        let read = model.threads.filter { ids.contains($0.id) }.allSatisfy { $0.unreadCount == 0 }
        #expect(read)
        #expect(await eventually { core.calls("set_thread_seen").count == 1 })
        #expect(Set(core.calls("set_thread_seen")[0]["thread_ids"] as? [Int] ?? []).isSubset(of: ids))
        press("I")
        let flagged = model.threads.filter { ids.contains($0.id) }.allSatisfy { $0.isFlagged }
        #expect(flagged)
        #expect(await eventually { core.calls("set_thread_flagged").count == 1 })
        #expect(Set(core.calls("set_thread_flagged")[0]["thread_ids"] as? [Int] ?? []) == ids)
    }

    /// The flag button of the reader's title line (`ui.md`, "Boutons de
    /// l'en-tête"): on, then off, for the mail shown alone, even with
    /// other rows marked (which `*` would flag).
    @Test func theFlagButtonOfTheReaderTogglesTheMailShownAlone() async {
        let shown = model.selectedThreadId!
        #expect(model.openThread?.id == shown)
        // J marks the second row and moves on; back to the first.
        press("j"); press("J")
        for _ in 0..<5 where model.selectedThreadId != shown { press("k") }
        #expect(model.selectedThreadId == shown)
        let marked = model.marked
        #expect(!marked.isEmpty && !marked.contains(shown))
        model.toggleFlag(of: shown)
        // Shown at once, on this mail only.
        #expect(model.threads.first { $0.id == shown }?.isFlagged == true)
        #expect(model.threads.filter { marked.contains($0.id) }.allSatisfy { !$0.isFlagged })
        #expect(await eventually { core.calls("set_thread_flagged").count == 1 })
        #expect(core.calls("set_thread_flagged")[0]["thread_ids"] as? [Int] == [shown])
        #expect(core.calls("set_thread_flagged")[0]["flagged"] as? Bool == true)
        // Off again.
        model.toggleFlag(of: shown)
        #expect(model.threads.first { $0.id == shown }?.isFlagged == false)
        #expect(await eventually { core.calls("set_thread_flagged").count == 2 })
        #expect(core.calls("set_thread_flagged")[1]["flagged"] as? Bool == false)
        // A conversation no longer in the list: nothing is sent.
        model.toggleFlag(of: -1)
        try? await Task.sleep(for: .milliseconds(50))
        #expect(core.calls("set_thread_flagged").count == 2)
    }

    @Test func readAndFlagToggles() async {
        press("I")
        #expect(await eventually { core.calls("set_thread_flagged").first?["flagged"] as? Bool == true })
        // Shown at once, before the core answers.
        #expect(model.selectedThread?.isFlagged == true)
        press("u")
        #expect(await eventually { !core.calls("set_thread_seen").isEmpty })
    }

    @Test func theHelpBlockOfTheStatusBarOpensAndClosesTheHelp() {
        model.flash = "something"
        controller.toggleHelpFromStatusBar()
        #expect(controller.overlay == .help && model.flash == nil)
        controller.toggleHelpFromStatusBar()
        #expect(controller.overlay == .none)
        // From an open menu, the help takes its place.
        press("m")
        controller.toggleHelpFromStatusBar()
        #expect(controller.overlay == .help)
    }

    /// Every block of the status bar closes what it opened, as `?` does:
    /// the bar is not under the click catcher.
    @Test func aSecondClickOnABlockClosesItsMenuOrPicker() {
        // The attributes (F): this mailbox has a single account, so @ opens
        // nothing here (`AccountPickerTests` has several).
        controller.openPickerFromStatusBar(.attributes)
        #expect(controller.overlay == .picker(.attributes))
        controller.openPickerFromStatusBar(.attributes)
        #expect(controller.overlay == .none)
        controller.openMenuFromStatusBar(.move)
        #expect(controller.overlay == .menu(.move))
        controller.openMenuFromStatusBar(.move)
        #expect(controller.overlay == .none)
        // Another block opens its own panel in place of the open one.
        controller.openMenuFromStatusBar(.move)
        controller.openMenuFromStatusBar(.goto)
        #expect(controller.overlay == .menu(.goto))
        controller.openPickerFromStatusBar(.attributes)
        #expect(controller.overlay == .picker(.attributes))
    }

    /// An empty part of the bar (the part that moves the window) closes
    /// the open menu, picker or help, as a click on the mail does.
    @Test func aClickOnTheEmptyBarClosesThePanel() {
        controller.openPickerFromStatusBar(.attributes)
        #expect(controller.overlay == .picker(.attributes))
        controller.clickBarBackground()
        #expect(controller.overlay == .none)
        controller.openMenuFromStatusBar(.view)
        controller.clickBarBackground()
        #expect(controller.overlay == .none)
        controller.toggleHelpFromStatusBar()
        controller.clickBarBackground()
        #expect(controller.overlay == .none)
        // Nothing open: nothing happens.
        controller.clickBarBackground()
        #expect(controller.overlay == .none)
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

    /// The filter kept after Return is shown as a chip (`ui.md`, the `;c`
    /// screen), also after leaving the screen and coming back; a click
    /// on the chip removes it.
    @Test func theContactsFilterKeptAfterReturnIsShownAsAChip() async {
        press(";"); press("c")
        #expect(controller.contactsFilterChip == nil)
        press("/"); type("ab")
        // While the line is open, it shows the text itself: no chip.
        #expect(controller.contactsFilterChip == nil)
        press("ret")
        #expect(controller.contactsFilterChip == "ab")
        // Left and opened again: still filtered, still shown.
        press("esc")
        #expect(controller.contactsFilterChip == nil)
        press(";"); press("c")
        #expect(controller.contactsFilterChip == "ab")
        #expect(await eventually { core.calls("list_contacts").last?["query"] as? String == "ab" })
        controller.clearContactsFilter()
        #expect(controller.contactsFilterChip == nil)
        #expect(await eventually { core.calls("list_contacts").last?["query"] == nil })
    }

    // MARK: Pickers and search

    @Test func semicolonListsTheFoldersAndEverythingAndHIsFree() {
        let keys = controller.menuEntries(.goto).map(\.key)
        // Four blocks (`ui.md`, "Dossiers virtuels"), each virtual folder
        // ending the block it gathers: Inbox, Todo, Work (`o`); Archive;
        // All Mail (`m`), alone here with no Sent nor Drafts; Deleted,
        // Everything (`e`); then the screens.
        #expect(keys == ["i", "t", "o", "a", "m", "d", "e", "c", "T", "g", "h"])
        // A gap before each new block.
        let gaps = controller.menuEntries(.goto).filter(\.gapBefore).map(\.key)
        #expect(gaps == ["a", "m", "d", "c", "h"])
        // The virtual folders are marked to be drawn apart, the others not.
        let virtual = controller.menuEntries(.goto).filter(\.virtual).map(\.key)
        #expect(virtual == ["o", "m", "e"])
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
            InterfaceController.ScopeFolder(folderId: i, role: r, name: r ?? "Projets", letter: "", unread: 0)
        }
        let sorted = InterfaceController.ordered(folders)
        #expect(sorted.map(\.name) == [
            "inbox", "todo", "waiting",
            "archive", "newsletter", "paperwork", "quarantine",
            "Projets",
            "sent", "drafts",
            "deleted", "junk", "banned",
        ])
        // Blocks: 0 to deal with, 2 filed, 3 the user's, 4 sent, 6 thrown away.
        #expect(sorted.map(\.group) == [0, 0, 0, 2, 2, 2, 2, 3, 4, 4, 6, 6, 6])
        // With the virtual folders, each ends its block, whatever the order.
        let all = InterfaceController.ordered(AppModel.Virtual.allCases.reversed().map { InterfaceController.virtualFolder($0) } + folders)
        #expect(all.map(\.name) == [
            "inbox", "todo", "waiting", "Work",
            "archive", "newsletter", "paperwork", "quarantine",
            "Projets",
            "sent", "drafts", "All Mail",
            "deleted", "junk", "banned", "Everything",
        ])
    }

    @Test func theMoveSaveAndDisplayBlocksOpenTheirMenus() {
        for p in [InterfaceController.Prefix.move, .save, .view] {
            controller.openMenuFromStatusBar(p)
            #expect(controller.overlay == .menu(p))
            #expect(!p.icon.isEmpty)
        }
        // A click on a row of the display menu: two columns (the loop
        // left it open: a second click on its block would close it).
        controller.overlay = .none
        controller.openMenuFromStatusBar(.view)
        let two = controller.menuEntries(.view).firstIndex { $0.key == "2" }!
        controller.chooseMenuEntry(two)
        #expect(controller.overlay == .none)
    }

    @Test func aClickOnAMenuRowActsLikeItsKey() {
        controller.openMenuFromStatusBar(.goto)
        #expect(controller.overlay == .menu(.goto))
        // Inbox, Todo, Work, Archive: the fourth row.
        controller.chooseMenuEntry(3)
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
        // The groups are read again: the folder column and the Group…
        // panel show the new group or member.
        let before = core.calls("list_groups").count
        #expect(await eventually { core.calls("list_groups").count > before })
        #expect(model.flash == L("Added to %@: %@", "Amis", "sender1@example.org"))
    }

    @Test func aRefusedThreadIsNotReportedAsAlreadyInTheGroup() async {
        // The core refuses a thread with nobody but the user (`groupes.md`,
        // "Adresses de l'utilisateur"): no one was added, nor was anyone
        // already there.
        core.handler = { method, params in
            guard method == "set_group_member" else { return Fixture.answer(method, params) }
            return CoreClient.RPCError(code: -32004, message: "correspondant de la conversation 1")
        }
        press("g"); type("Amis"); press("ret")
        #expect(await eventually { model.flash == L("Nobody to add to %@", "Amis") })
        #expect(model.lastError != nil)
    }

    // MARK: Adding and removing tags and groups

    /// A core where thread 1 has the tag Urgent and its person is in VIP,
    /// thread 2 has neither, and Urgent and Calme exist (Calme in another
    /// account only).
    func labelled() async {
        core.handler = { method, params in
            switch method {
            case "list_tags":
                let tag = { (id: Int, account: Int, name: String) -> [String: Any] in
                    ["id": id, "account_id": account, "name": name, "color": NSNull(), "message_count": 3]
                }
                // Calme only exists in another account than the mails'
                // (the fixture's folders are all in account 1), and Urgent
                // in both: one row each.
                return ["items": [tag(1, 1, "Urgent"), tag(2, 2, "Calme"), tag(3, 2, "urgent")], "as_of": Fixture.position]
            case "list_threads" where (params["folder_ids"] as? [Int])?.first ?? params["folder_id"] as? Int ?? 1 == 1:
                let items = (1...8).map { Fixture.thread($0, tags: $0 == 1 ? ["Urgent"] : []) }
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            case "get_thread" where params["thread_id"] as? Int == 1:
                var t = Fixture.threadView(1)
                t["tags"] = ["Urgent"]
                t["correspondent_groups"] = ["VIP"]
                return t
            case "thread_labels":
                let ids = params["thread_ids"] as? [Int] ?? []
                let items: [[String: Any]] = ids.map {
                    ["thread_id": $0, "tags": $0 == 1 ? ["Urgent"] : [], "correspondent": "sender\($0)@example.org",
                     "groups": $0 == 1 ? ["VIP"] : []]
                }
                return ["items": items, "as_of": Fixture.position]
            default:
                return Fixture.answer(method, params)
            }
        }
        await model.reloadThreads()
        await model.reloadOpenThread()
        _ = await eventually { model.threads.first?.tags == ["Urgent"] && model.openThread?.labels.groups == ["VIP"] }
    }

    @Test func theHeaderNamesTheTagsAndTheGroupsOfTheThread() async {
        await labelled()
        #expect(model.openThread?.labels.tags == ["Urgent"])
        #expect(model.openThread?.labels.groups == ["VIP"])
        #expect(LabelChoices.headerHelp(tags: ["Urgent"], groups: ["VIP"])
            == L("Tags: %@ (t to change)", "Urgent") + "\n" + L("Groups of the sender: %@ (g to change)", "VIP"))
    }

    @Test func theTagPickerOffersToRemoveTheTagsOfTheMail() async {
        await labelled()
        press("t")
        _ = await eventually { controller.tags.count == 3 }
        #expect(controller.pickerEntries(.tag).map(\.label) == [L("remove %@", "Urgent"), "Calme"])
        #expect(controller.pickerEntries(.tag).map(\.key) == ["−", " "])
        // A single mail: no count.
        #expect(controller.pickerEntries(.tag).allSatisfy { $0.detail == nil })
        press("ret")
        #expect(await eventually { core.calls("tag_thread").count == 1 })
        #expect(core.calls("tag_thread")[0]["remove"] as? Bool == true)
        #expect(core.calls("tag_thread")[0]["thread_ids"] as? [Int] == [1])
        #expect(model.flash == L("Tag removed: %@", "Urgent"))
    }

    @Test func aMixedSelectionCanAddOrRemoveTheSameTag() async {
        await labelled()
        press("J")
        #expect(model.marked == [1, 2])
        press("t")
        _ = await eventually { controller.tags.count == 3 }
        let rows = controller.pickerEntries(.tag)
        #expect(rows.map(\.label) == [L("remove %@", "Urgent"), "Urgent", "Calme"])
        #expect(rows.map(\.detail) == [L("%lld of %lld", 1, 2), L("%lld of %lld", 1, 2), nil])
        // Adding Urgent: both threads are sent (the core leaves alone the
        // one that has it).
        press("down"); press("ret")
        #expect(await eventually { core.calls("tag_thread").count == 1 })
        #expect(core.calls("tag_thread")[0]["remove"] as? Bool == false)
        #expect(core.calls("tag_thread")[0]["thread_ids"] as? [Int] == [1, 2])
        // Removing it: only the thread that has it.
        press("t"); press("ret")
        #expect(await eventually { core.calls("tag_thread").count == 2 })
        #expect(core.calls("tag_thread")[1]["remove"] as? Bool == true)
        #expect(core.calls("tag_thread")[1]["thread_ids"] as? [Int] == [1])
    }

    @Test func theGroupPickerOffersToRemoveTheSenderFromItsGroups() async {
        await labelled()
        // The open thread's header gives its groups at once, before the
        // core answers `thread_labels`.
        press("g")
        #expect(controller.pickerEntries(.group).map(\.label) == [L("remove %@", "VIP"), "Famille"])
        #expect(await eventually { core.calls("thread_labels").count == 1 })
        #expect(core.calls("thread_labels")[0]["thread_ids"] as? [Int] == [1])
        let reads = core.calls("get_thread").count
        press("ret")
        #expect(await eventually { core.calls("set_group_member").count == 1 })
        let call = core.calls("set_group_member")[0]
        #expect(call["group"] as? String == "VIP" && call["thread_id"] as? Int == 1 && call["remove"] as? Bool == true)
        #expect(await eventually { model.flash == L("Removed from %@: %@", "VIP", "sender1@example.org") })
        // No event says a membership changed: the header is read again.
        #expect(await eventually { core.calls("get_thread").count > reads })
    }

    @Test func aMixedSelectionCanAddOrRemoveTheSendersOfAGroup() async {
        await labelled()
        press("J")
        press("g")
        // The groups of every marked thread come from the core.
        #expect(await eventually { controller.pickerGroupLabels?.count == 2 })
        #expect(core.calls("thread_labels").last?["thread_ids"] as? [Int] == [1, 2])
        let rows = controller.pickerEntries(.group)
        #expect(rows.map(\.label) == [L("remove %@", "VIP"), "VIP", "Famille"])
        #expect(rows.map(\.detail) == [L("%lld of %lld", 1, 2), L("%lld of %lld", 1, 2), nil])
        press("down"); press("ret")
        #expect(await eventually { core.calls("set_group_member").count == 2 })
        #expect(core.calls("set_group_member").map { $0["remove"] as? Bool } == [false, false])
        #expect(core.calls("set_group_member").map { $0["thread_id"] as? Int } == [1, 2])
    }

    // MARK: Tags and Groups screens: rename, delete (`ui.md`, "Écrans Tags et Groupes")

    @Test func theTagsScreenHasOneRowPerName() async {
        await labelled()
        press(";"); press("T")
        #expect(await eventually { controller.tags.count == 3 })
        // Urgent of both accounts is one row, its mails summed.
        #expect(controller.screenTags.map(\.name) == ["Urgent", "Calme"])
        #expect(controller.screenTags.map(\.messageCount) == [6, 3])
    }

    @Test func rOnTheTagsScreenRenamesTheTag() async {
        await labelled()
        model.attributes.toggleTag("urgent")
        model.showThreads(of: .tag(TagView(id: 1, accountId: 1, name: "Urgent", color: nil, messageCount: 3)))
        press(";"); press("T")
        _ = await eventually { controller.tags.count == 3 }
        press("r")
        let kind = InterfaceController.PickerKind.rename(.tag, name: "Urgent", id: nil)
        #expect(controller.overlay == .picker(kind))
        // The panel opens on the current name: nothing to do yet.
        #expect(controller.pickerQuery == "Urgent")
        #expect(controller.pickerEntries(kind).isEmpty)
        // The name of another tag (without case): refused, Return does nothing.
        type("calme")
        #expect(controller.pickerEntries(kind).map(\.label) == [L("%@ already exists", "calme")])
        #expect(controller.pickerEntries(kind).first?.disabled == true)
        press("ret")
        #expect(core.calls("rename_tag").isEmpty)
        // A new name.
        press("r")
        controller.pickerQuery = "Très urgent "
        #expect(controller.pickerEntries(kind).map(\.label) == [L("rename %@ to %@", "Urgent", "Très urgent")])
        let reads = core.calls("list_tags").count
        press("ret")
        #expect(controller.overlay == .none)
        #expect(await eventually { core.calls("rename_tag").count == 1 })
        let call = core.calls("rename_tag")[0]
        #expect(call["name"] as? String == "Urgent" && call["new_name"] as? String == "Très urgent")
        #expect(await eventually { model.flash == L("Tag renamed: %@ → %@", "Urgent", "Très urgent") })
        // The attribute and the list of the tag follow the new name; the
        // screen is read again.
        #expect(await eventually { model.attributes.tags == ["Très urgent"] })
        if case .tag(let t) = model.listFilter { #expect(t.name == "Très urgent") } else { Issue.record("no tag list") }
        #expect(await eventually { core.calls("list_tags").count > reads })
    }

    @Test func dOnTheTagsScreenDeletesTheTagOnceConfirmed() async {
        await labelled()
        model.attributes.toggleTag("Urgent", excluded: true)
        press(";"); press("T")
        _ = await eventually { controller.tags.count == 3 }
        press("d")
        let choice = InterfaceController.Choice.deleteLabel(.tag, name: "Urgent", id: nil, count: 6)
        #expect(controller.activeChoice == choice)
        #expect(controller.choiceTitle(choice) == L("Delete the tag %@ (taken off its %lld mails)?", "Urgent", 6))
        // Escape keeps it.
        press("esc")
        #expect(controller.activeChoice == nil)
        #expect(core.calls("delete_tag").isEmpty)
        press("d"); press("y")
        #expect(await eventually { core.calls("delete_tag").count == 1 })
        #expect(core.calls("delete_tag")[0]["name"] as? String == "Urgent")
        #expect(await eventually { model.flash == L("Tag deleted: %@", "Urgent") })
        #expect(await eventually { model.attributes.notTags.isEmpty })
        // Still on the screen.
        #expect(controller.screen == .tags)
    }

    @Test func theGroupsScreenRenamesAndDeletesAGroup() async {
        press(";"); press("g")
        #expect(await eventually { controller.groups.count == 2 })
        model.attributes.toggleGroup(ThreadFilter.GroupRef(id: 1, name: "VIP"))
        press("r")
        let kind = InterfaceController.PickerKind.rename(.group, name: "VIP", id: 1)
        #expect(controller.overlay == .picker(kind))
        type("FAMILLE")
        #expect(controller.pickerEntries(kind).map(\.label) == [L("%@ already exists", "FAMILLE")])
        // A change of case is a rename.
        controller.pickerQuery = "vip"
        #expect(controller.pickerEntries(kind).map(\.label) == [L("rename %@ to %@", "VIP", "vip")])
        controller.pickerQuery = "Proches"
        press("ret")
        #expect(await eventually { core.calls("rename_group").count == 1 })
        #expect(core.calls("rename_group")[0]["new_name"] as? String == "Proches")
        #expect(await eventually { model.attributes.groups == [ThreadFilter.GroupRef(id: 1, name: "Proches")] })
        #expect(await eventually { model.flash == L("Group renamed: %@ → %@", "VIP", "Proches") })

        press("d")
        let choice = InterfaceController.Choice.deleteLabel(.group, name: "VIP", id: 1, count: 2)
        #expect(controller.activeChoice == choice)
        #expect(controller.choiceTitle(choice) == L("Delete the group %@ and its %lld members? Their mails stay", "VIP", 2))
        press("y")
        #expect(await eventually { core.calls("delete_group").count == 1 })
        #expect(await eventually { model.attributes.groups.isEmpty })
        #expect(await eventually { model.flash == L("Group deleted: %@", "VIP") })
    }

    @Test func nothingToRemoveFromAGroupIsSaid() async {
        core.handler = { method, params in
            guard method == "set_group_member" else { return Fixture.answer(method, params) }
            return ["group_id": 1, "addr": "sender1@example.org", "changed": false]
        }
        model.setSendersGroup("VIP", remove: true)
        #expect(await eventually { model.flash == L("Not in %@", "VIP") })
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

    /// The core writes the link by the mail's key (`MessageView.link`): the
    /// row shows it and Return copies it, so the note opens the mail on
    /// another Mac too.
    @Test func theLinkCopiedIsTheKeyTheCoreGives() async {
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            var thread = Fixture.threadView(2)
            var m = (thread["messages"] as! [[String: Any]])[0]
            m["link"] = "nicemailer://m/3f9a0c2b7e41"
            thread["messages"] = [m]
            return thread
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        press("l")
        #expect(controller.pickerEntries(.links).first?.detail == "nicemailer://m/3f9a0c2b7e41")
        press("ret")
        #expect(controller.pasteboard.string(forType: .string)?.hasSuffix("(nicemailer://m/3f9a0c2b7e41)") == true)
        press("l"); press("cmd-ret")
        #expect(controller.pasteboard.string(forType: .string) == "nicemailer://m/3f9a0c2b7e41")
    }

    /// A conversation of three messages, each with its link, for the link
    /// of the message the reader is on.
    private static func threeMessages() -> [String: Any] {
        var thread = Fixture.threadView(2)
        let m = (thread["messages"] as! [[String: Any]])[0]
        thread["messages"] = (1...3).map { n -> [String: Any] in
            var x = m
            x["id"] = 20 + n
            x["link"] = "nicemailer://m/00000000002\(n)"
            x["expanded"] = n == 3
            return x
        }
        return thread
    }

    /// `l` links the message the reader is on: up the conversation with
    /// ctrl-k, the link names that older message, not the latest.
    @Test func theLinkCopiedIsTheOneOfTheMessageTheReaderIsOn() async {
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            return Self.threeMessages()
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.messages.count == 3 }
        press("l")
        #expect(controller.pickerEntries(.links).first?.detail == "nicemailer://m/000000000023")
        press("esc")
        press("ctrl-k")
        press("l")
        #expect(controller.pickerEntries(.links).first?.detail == "nicemailer://m/000000000022")
        press("cmd-ret")
        #expect(controller.pasteboard.string(forType: .string) == "nicemailer://m/000000000022")
        press("ctrl-k"); press("l"); press("ret")
        #expect(controller.pasteboard.string(forType: .string)?.hasSuffix("(nicemailer://m/000000000021)") == true)
    }

    /// Opening the link of an older message shows that message: current,
    /// unfolded, and the reader scrolls to it.
    @Test func aDeepLinkShowsTheMessageOfTheLinkInItsConversation() async {
        core.handler = { method, params in
            switch method {
            case "locate_message": return ["message_id": 21, "thread_id": 2, "folder_id": 1, "account_id": 1]
            case "get_thread" where params["thread_id"] as? Int == 2: return Self.threeMessages()
            default: return Fixture.answer(method, params)
            }
        }
        model.openDeepLink(URL(string: "nicemailer://m/000000000021")!)
        #expect(await eventually { model.openThread?.id == 2 })
        #expect(controller.readerState.current == 21)
        #expect(controller.readerState.folding[21] == true)
        // The link of the open mail is now that message's.
        #expect(controller.mailLinkURL?.absoluteString == "nicemailer://m/000000000021")
        // ctrl-j goes on from it.
        press("ctrl-j")
        #expect(controller.readerState.current == 22)
    }

    /// A link by key asks the core by key (`link_key`), in lowercase; a key
    /// this Mac does not have is said as such, not as a mail deleted.
    @Test func aDeepLinkByKeyAsksTheCoreByKey() async {
        core.handler = { method, params in
            guard method == "locate_message" else { return Fixture.answer(method, params) }
            if params["link_key"] as? String == "3f9a0c2b7e41" {
                return ["message_id": 50, "thread_id": 5, "folder_id": 3, "account_id": 1]
            }
            return CoreClient.RPCError(code: CoreClient.RPCError.notFound, message: "link not found")
        }
        model.openDeepLink(URL(string: "nicemailer://m/3F9A0C2B7E41")!)
        #expect(await eventually { model.selectedThreadId == 5 })
        #expect(model.selectedFolderId == 3)
        #expect(core.calls("locate_message").first?["message_id"] == nil)
        model.openDeepLink(URL(string: "nicemailer://m/000000000000")!)
        #expect(await eventually { model.lastError == L("This mail is not in NiceMailer on this Mac") })
        #expect(model.selectedThreadId == 5)
    }

    // MARK: Verification codes

    /// A code mail: its code is the first row of `l`, with a lock, and
    /// Return (or ⌘-Return) copies it. In a thread, the latest code wins:
    /// a site asked twice sends two codes, and only the last one works.
    @Test func theCodeOfACodeMailIsTheFirstRowOfLAndReturnCopiesIt() async {
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            var thread = Fixture.threadView(2)
            let m = (thread["messages"] as! [[String: Any]])[0]
            var older = m, newer = m, after = m
            older["id"] = 20; older["verification_code"] = "111111"
            newer["id"] = 21; newer["verification_code"] = "482913"
            // A later message without a code does not hide it.
            after["id"] = 22; after["verification_code"] = NSNull()
            thread["messages"] = [older, newer, after]
            return thread
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        press("l")
        let rows = controller.pickerEntries(.links)
        #expect(rows.first?.label == "482913")
        #expect(rows.first?.detail == L("Verification code"))
        #expect(rows.first?.icon == "lock.fill")
        #expect(rows.first?.key == "1")
        #expect(rows.dropFirst().first?.label == L("Link to this mail"))
        press("ret")
        #expect(controller.pasteboard.string(forType: .string) == "482913")
        #expect(controller.overlay == .none)
        #expect(model.flash == L("Code %@ copied", "482913"))
        // ⌘-Return copies it too, and typing "code" finds the row.
        controller.pasteboard.clearContents()
        press("l"); type("code")
        #expect(controller.pickerEntries(.links).first?.label == "482913")
        press("cmd-ret")
        #expect(controller.pasteboard.string(forType: .string) == "482913")
    }

    @Test func aMailWithoutACodeHasNoCodeRow() {
        press("l")
        #expect(controller.pickerEntries(.links).first?.label == L("Link to this mail"))
        #expect(!controller.pickerEntries(.links).contains { $0.icon == "lock.fill" })
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
        // The account picker's unread counts stand out (its rows, read
        // without opening it: `@` does nothing with this single account).
        #expect(controller.pickerEntries(.accounts).filter { $0.detail != nil }.allSatisfy { $0.detailStyle == .count })
        // How six digits were read is information.
        press("f"); type(L("Conversation date…")); press("ret")
        type("010120")
        #expect(controller.pickerEntries(.attributeValue(.date)).prefix(4).allSatisfy { $0.detailStyle == .info })
        press("esc")
        // An interval left empty is a warning: it stands out.
        press("f"); type(L("Conversation date…")); press("ret")
        type("2026-10-01"); press("ret")
        press("f"); type(L("Conversation date…")); press("ret")
        type("2026-09-01")
        let warning = controller.pickerEntries(.attributeValue(.date))[1]
        #expect(warning.detail == L("nothing in between") && warning.detailStyle == .count)
        press("esc")
        // "domain" on a From… value.
        press("f"); type(L("From…")); press("ret")
        type("@acme.fr")
        #expect(controller.pickerEntries(.attributeValue(.from)).first?.detailStyle == .info)
    }

    @Test func stepDownTurnsAroundFromTheLastRowOnly() {
        #expect(InterfaceController.stepDown(4, count: 5) == 0)
        #expect(InterfaceController.stepDown(1, count: 5) == 2)
        #expect(InterfaceController.stepDown(0, count: 1) == 0)
        #expect(InterfaceController.stepDown(0, count: 0) == 0)
        // An index left past the end (the list shrank) starts again on top.
        #expect(InterfaceController.stepDown(9, count: 5) == 0)
    }

    @Test func stepUpWrapsFromTheFirstRowOnly() {
        #expect(InterfaceController.stepUp(0, count: 5) == 4)
        #expect(InterfaceController.stepUp(3, count: 5) == 2)
        #expect(InterfaceController.stepUp(0, count: 1) == 0)
        #expect(InterfaceController.stepUp(0, count: 0) == 0)
        // An index left past the end (the list shrank) comes back inside.
        #expect(InterfaceController.stepUp(9, count: 5) == 4)
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
        press("f"); type(L("From…")); press("ret")
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
        press("f")
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
        press("f"); type(L("From…")); press("ret")
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
        press("f"); type("un")
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
        press("f")
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

    @Test func slashSearchesAllMailWithTheAnywhereCriterion() async {
        press("/")
        #expect(controller.overlay == .picker(.search))
        type("devis")
        #expect(controller.pickerEntries(.search).map(\.label) == ["devis"])
        press("ret")
        // Words anywhere, addresses included, over every folder of the
        // account but Deleted, Junk and Banned.
        #expect(controller.overlay == .none)
        #expect(model.attributes.contains == "devis" && model.everything == .account(1) && model.allMail)
        #expect(model.currentFolderName == "All Mail")
        #expect(await eventually {
            let call = core.calls("list_threads").last
            let scope = call?["everything"] as? [String: Any]
            return scope?["account_id"] as? Int == 1 && scope?["all_mail"] as? Bool == true
                && (call?["filter"] as? [String: Any])?["contains"] as? String == "devis"
        })
    }

    /// `;m` opens All Mail, `;e` Everything; each is the current one in the
    /// menu only when shown. A search from Everything stays in Everything.
    @Test func allMailAndEverythingAreTwoVirtualFolders() async {
        press(";"); press("m")
        #expect(model.everything == .account(1) && model.allMail && model.currentFolderName == "All Mail")
        // Deleted (folder 4) is not listened to: its events change nothing shown.
        #expect(!model.listedFolderIds.contains(4) && model.listedFolderIds.contains(1))
        let current = controller.scopeFolders.filter { controller.isCurrent($0) }.map(\.name)
        #expect(current == ["All Mail"])
        press(";"); press("e")
        #expect(model.everything == .account(1) && !model.allMail && model.currentFolderName == "Everything")
        #expect(model.listedFolderIds.contains(4))
        #expect(await eventually { (core.calls("list_threads").last?["everything"] as? [String: Any])?["all_mail"] == nil })
        press("/"); type("devis"); press("ret")
        #expect(!model.allMail && model.everything == .account(1))
        // A folder leaves both.
        press("esc")
        press(";"); press("m")
        press(";"); press("a")
        #expect(model.everything == nil && !model.allMail && model.selectedFolderId == 3)
    }

    /// `;o` opens Work: Inbox, Todo and Waiting together, asked of the
    /// core with `work`, listening to those folders only.
    @Test func workGathersInboxTodoAndWaiting() async {
        press(";"); press("o")
        #expect(model.everything == .account(1) && model.virtual == .work && model.currentFolderName == "Work")
        #expect(model.listedFolderIds.sorted() == [1, 2])
        #expect(controller.scopeFolders.filter { controller.isCurrent($0) }.map(\.name) == ["Work"])
        #expect(await eventually { (core.calls("list_threads").last?["everything"] as? [String: Any])?["work"] as? Bool == true })
        // `/` from Work searches All Mail, and Escape comes back to Work.
        press("/"); type("devis"); press("ret")
        #expect(model.virtual == .allMail)
        press("esc")
        #expect(model.virtual == .work && model.everything == .account(1))
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

    /// The Display menu in four blocks (`ui.md`, menu `v`): the mail
    /// shown, the columns, the presentation, the sort; a gap before each
    /// block but the first.
    @Test func theDisplayMenuIsInFourBlocks() {
        let rows = controller.menuEntries(.view)
        #expect(rows.map(\.key) == ["h", "i", "1", "2", "3", "t", "l", "s", "r", "c", "m"])
        #expect(rows.filter(\.gapBefore).map(\.key) == ["1", "t", "c"])
    }

    @Test func theShowAndSaveMenusStartEachRowWithACapital() {
        // "Text / HTML", "As PDF"…: the rows of the Show and Save menus are
        // capitalized, the help keeps the label as it is (`KeyAction.label`).
        let C = ThreadFilter.capitalizedFirst
        let view = controller.menuEntries(.view)
        #expect(view.first { $0.key == "h" }?.label == C(L("text / HTML")))
        #expect(view.first { $0.key == "1" }?.label == C(L("one column")))
        #expect(view.first { $0.key == "s" }?.label == C(L("status bar: bottom, top, hidden")))
        let save = controller.menuEntries(.save)
        #expect(save.first { $0.key == "p" }?.label == C(L("as PDF")))
        #expect(save.first { $0.key == "c" }?.label == C(L("to clipboard")))
        #expect(KeyAction.viewToggleHtml.label == L("text / HTML"))
    }

    @Test func theDisplayMenuKeepsItsBlocksInTheOrderOfKeymapToml() {
        // Blocks follow the actions, not the places: two sorts moved first
        // stay one block, set apart from the next.
        #expect(KeyAction.viewSortReceived.viewBlock == KeyAction.viewSortModified.viewBlock)
        #expect(KeyAction.viewToggleHtml.viewBlock != KeyAction.viewOneColumn.viewBlock)
        #expect(KeyAction.viewChooseTheme.viewBlock == KeyAction.viewCycleStatusBar.viewBlock)
        #expect(KeyAction.viewResetColumnsWidth.viewBlock == KeyAction.viewCycleStatusBar.viewBlock)
        #expect(KeyAction.mailToggleFlag.viewBlock == nil)
    }

    @Test func theDisplayMenuChangesAndRemembersTheLayout() {
        press("v"); press("3")
        #expect(controller.columns == 3)
        // At the top by default; `vs` hides it.
        #expect(controller.statusPosition == .top)
        press("v"); press("s")
        #expect(controller.statusPosition == .hidden)
        // A new controller reads the remembered choices.
        let again = InterfaceController(model: model, defaults: defaults)
        #expect(again.columns == 3 && again.statusPosition == .hidden)
    }

    /// `v` `l` (`ui.md`, "Liste", "Deux présentations"): one line per
    /// mail in columns, or three, back and forth; remembered, and said in
    /// the status bar.
    @Test func vlSwitchesTheListBetweenOneLineAndThreeLinesPerMail() {
        #expect(controller.listLayout == .columns)
        press("v"); press("l")
        #expect(controller.listLayout == .stacked)
        #expect(model.flash == L("List: three lines per mail"))
        #expect(controller.overlay == .none)
        // A new controller reads the remembered choice.
        #expect(InterfaceController(model: model, defaults: defaults).listLayout == .stacked)
        // The keys of the list work the same.
        press("j")
        #expect(selectedIndex == 1)
        press("v"); press("l")
        #expect(controller.listLayout == .columns)
        #expect(model.flash == L("List: one line per mail"))
        #expect(InterfaceController(model: model, defaults: defaults).listLayout == .columns)
    }

    @Test func anUnknownListLayoutFallsBackToColumns() {
        defaults.set("cards", forKey: ListLayout.defaultsKey)
        #expect(InterfaceController(model: model, defaults: defaults).listLayout == .columns)
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

    /// The mode is no longer drawn (November 2026: a legacy of Helix the
    /// status bar does without), but the scenarios read it.
    @Test func theModeFollowsThePrefixAndThePicker() {
        #expect(controller.modeLabel == "NOR")
        press("v")
        #expect(controller.modeLabel == "v")
        press("esc")
        #expect(controller.modeLabel == "NOR")
        press("/")
        #expect(controller.modeLabel == "PCK")
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

    /// The pointer over a row of a menu highlights it, as the arrows do;
    /// Return and the arrows start from it; leaving the row clears it,
    /// unless the pointer already entered the next one.
    @Test func thePointerOverAMenuRowHighlightsIt() {
        press(";")
        #expect(controller.menuIndex == nil)
        controller.hoverMenu(2, inside: true)
        #expect(controller.menuIndex == 2)
        // Down from row 2 to row 3: entered first, then left.
        controller.hoverMenu(3, inside: true)
        controller.hoverMenu(2, inside: false)
        #expect(controller.menuIndex == 3)
        press("down")
        #expect(controller.menuIndex == 4)
        controller.hoverMenu(4, inside: false)
        #expect(controller.menuIndex == nil)
        // The rows of a choice, the same way.
        controller.hoverChoice(1, inside: true)
        #expect(controller.choiceIndex == 1)
        controller.hoverChoice(1, inside: false)
        #expect(controller.choiceIndex == nil)
    }

    @Test func textHtmlIsRememberedForTheSender() async {
        press("j"); press("j")
        #expect(await eventually { model.openThread?.id == 3 })
        press("v"); press("h")
        #expect(await eventually { core.calls("set_body_format").count == 1 })
        #expect(core.calls("set_body_format")[0]["format"] as? String == "html")
        #expect(core.calls("set_body_format")[0]["addr"] as? String == "sender3@example.org")
    }

    /// Without a choice remembered for the sender, the display is the
    /// theme's (`reader.body_format`, `themes.md`, section 6.6): text in
    /// geek, HTML in clean and regular, and it follows a new theme at once.
    /// A remembered choice wins in every theme; a mail without HTML stays
    /// text.
    @Test func withoutAChoiceTheDisplayIsTheThemes() async {
        model.theme = ThemeStore()
        remoteImages()
        await openThree()
        #expect(model.bodyFormat == "text", "geek reads text")
        model.theme.apply(ThemeStore.builtins["clean"]!)
        #expect(model.bodyFormat == "html", "clean reads HTML")
        model.theme.apply(ThemeStore.builtins["regular"]!)
        #expect(model.bodyFormat == "html", "regular reads HTML")

        // A choice remembered for the sender wins over the theme.
        remoteImages(remembered: "text")
        await model.reloadOpenThread()
        #expect(model.bodyFormat == "text")
        model.theme.apply(ThemeStore.builtins["geek"]!)
        remoteImages(remembered: "html")
        await model.reloadOpenThread()
        #expect(model.bodyFormat == "html", "remembered as HTML, it stays HTML in geek")
    }

    /// A mail without HTML is text, whatever the theme says.
    @Test func aMailWithoutHtmlIsTextInEveryTheme() async {
        model.theme = ThemeStore(ThemeStore.builtins["clean"]!)
        press("j")
        #expect(await eventually { model.openThread?.id == 2 })
        #expect(model.bodyFormat == "text")
    }

    /// `vh` under a theme that reads HTML writes an explicit "text", and
    /// back an explicit "html", the theme's value: the sender keeps it in
    /// every theme (`themes.md`, section 6.6).
    @Test func vhAlwaysRemembersAnExplicitValue() async {
        model.theme = ThemeStore(ThemeStore.builtins["clean"]!)
        remoteImages()
        await openThree()
        #expect(model.bodyFormat == "html")
        press("v"); press("h")
        #expect(await eventually { model.bodyFormat == "text" })
        #expect(core.calls("set_body_format").last?["format"] as? String == "text")
        press("v"); press("h")
        #expect(await eventually { model.bodyFormat == "html" })
        #expect(core.calls("set_body_format").last?["format"] as? String == "html")
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
    }

    // MARK: Saving as PDF (sp, sP)

    @Test func spSavesAPDFOfTheLatestMailWithoutOverwriting() async throws {
        let dir = try exportDir()
        var opened: [String] = []
        model.openFile = { opened.append($0.path) }
        press("s"); press("p")
        #expect(await eventually { saved(dir) == ["2020-06-23 Subject 1.pdf"] })
        #expect(core.calls("message_record").map { $0["message_id"] as? Int } == [10])
        #expect(core.calls("export_message").isEmpty && opened.isEmpty)
        let path = dir.appendingPathComponent("2020-06-23 Subject 1.pdf").path
        #expect(await eventually { model.flash == L("Saved: %@", path) })
        // The file is the plain PDF: the mail, no hash.
        let text = try #require(PDFDocument(url: URL(fileURLWithPath: path))).string ?? ""
        #expect(text.contains("Paragraph of 10."))
        #expect(!text.contains("SHA-256"))
        // Again: a second file, the first one kept.
        press("s"); press("p")
        #expect(await eventually { saved(dir) == ["2020-06-23 Subject 1 (2).pdf", "2020-06-23 Subject 1.pdf"] })
    }

    @Test func capitalPSavesThenOpensEveryMarkedMail() async throws {
        let dir = try exportDir()
        var opened: [String] = []
        model.openFile = { opened.append(($0.path as NSString).lastPathComponent) }
        press("J"); press("J")
        press("s"); press("P")
        #expect(await eventually { opened.count == 3 })
        #expect(Set(opened) == ["2020-06-23 Subject 1.pdf", "2020-06-23 Subject 2.pdf", "2020-06-23 Subject 3.pdf"])
        #expect(saved(dir).count == 3)
        #expect(await eventually { model.flash == L("%lld mails saved to %@ and opened", 3, dir.lastPathComponent) })
    }

    @Test func siSavesTheEvidenceWithTheTechnicalInformationAndCapitalIOpensIt() async throws {
        let dir = try exportDir()
        var opened: [String] = []
        model.openFile = { opened.append($0.path) }
        // The last row of the menu.
        #expect(controller.menuEntries(.save).last?.key == "i")
        #expect(controller.menuEntries(.save).last?.label == ThreadFilter.capitalizedFirst(L("as PDF with technical information")))
        press("s"); press("i")
        let path = dir.appendingPathComponent("2020-06-23 Subject 1.pdf").path
        #expect(await eventually { saved(dir) == ["2020-06-23 Subject 1.pdf"] })
        #expect(opened.isEmpty)
        let text = try #require(PDFDocument(url: URL(fileURLWithPath: path))).string ?? ""
        #expect(text.contains("SHA-256 " + String(repeating: "0f", count: 32)))
        #expect(text.contains(L("Technical information")))
        // `I` saves another one, then opens it.
        press("s"); press("I")
        #expect(await eventually { opened == [dir.appendingPathComponent("2020-06-23 Subject 1 (2).pdf").path] })
        #expect(await eventually { model.flash?.hasPrefix(L("Saved and opened: %@", "")) == true })
    }

    /// Thread 2 as a conversation of two messages, 21 then 22 (unfolded),
    /// the files of the fixture attached to 22 only.
    func twoMessageConversation() async {
        core.handler = { method, params in
            guard method == "get_thread", params["thread_id"] as? Int == 2 else { return Fixture.answer(method, params) }
            var thread = Fixture.threadView(2)
            var first = (thread["messages"] as! [[String: Any]])[0]
            var second = first
            first["id"] = 21
            first["expanded"] = false
            second["id"] = 22
            second["attachments"] = Fixture.attachments
            thread["messages"] = [first, second]
            return thread
        }
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.messages.count == 2 }
    }

    @Test func everyExportOfSTakesTheMessageTheReaderIsOnNotAlwaysTheLatest() async throws {
        let dir = try exportDir()
        await twoMessageConversation()
        // Nothing chosen: the latest, for every kind of export.
        press("s"); press("p")
        press("s"); press("e")
        press("s"); press("c")
        #expect(await eventually { core.calls("message_record").count == 1 && core.calls("export_message").count == 1
                && core.calls("message_text").count == 1 })
        #expect(core.calls("message_record")[0]["message_id"] as? Int == 22)
        #expect(core.calls("export_message")[0]["message_id"] as? Int == 22)
        #expect(core.calls("message_text")[0]["message_id"] as? Int == 22)
        // ctrl-k to the first message: every export takes it.
        press("ctrl-k")
        #expect(controller.readerState.current == 21)
        press("s"); press("c")
        press("s"); press("e")
        press("s"); press("t")
        press("s"); press("M")
        press("s"); press("p")
        press("s"); press("i")
        press("s"); press("I")
        #expect(await eventually { core.calls("message_text").count == 2 && core.calls("export_message").count == 4
                && core.calls("message_record").count == 4 })
        #expect(core.calls("message_text").last?["message_id"] as? Int == 21)
        #expect(core.calls("export_message").suffix(3).allSatisfy { $0["message_id"] as? Int == 21 })
        #expect(core.calls("message_record").suffix(3).allSatisfy { $0["message_id"] as? Int == 21 })
        #expect(await eventually { saved(dir).filter { $0.hasSuffix(".pdf") }.count == 4 })
    }

    @Test func attachmentsAreThoseOfTheChosenMessageElseOfTheWholeConversation() async throws {
        let dir = try exportDir()
        await twoMessageConversation()
        // Nothing chosen: the files of the whole conversation (22's).
        press("s"); press("a")
        #expect(await eventually { saved(dir) == ["attachment-43", "devis.pdf", "photo.jpg"] })
        // ctrl-k to 21, which has none: nothing saved, and said.
        press("ctrl-k")
        press("s"); press("a")
        #expect(await eventually { model.flash == L("No attachment to save") })
        #expect(saved(dir).count == 3)
        // ctrl-j back to 22, chosen this time: its files again.
        press("ctrl-j")
        press("s"); press("g")
        #expect(await eventually { saved(dir).count == 5 })
    }

    @Test func theExportedMessageIsTheReadersElseTheLatest() throws {
        let m = { (id: Int) throws -> MessageView in
            var json = Fixture.threadView(1)["messages"] as! [[String: Any]]
            json[0]["id"] = id
            return try CoreClient.decoder.decode(MessageView.self, from: JSONSerialization.data(withJSONObject: json[0]))
        }
        let t = ThreadView(id: 1, subject: "S", messages: [try m(1), try m(2), try m(3)], asOf: Position(epoch: "e", seq: 0))
        #expect(AppModel.exportMessage(t, current: nil)?.id == 3)
        #expect(AppModel.exportMessage(t, current: 1)?.id == 1)
        // A message no longer in the thread: the latest.
        #expect(AppModel.exportMessage(t, current: 99)?.id == 3)
        #expect(AppModel.exportMessage(ThreadView(id: 2, subject: "S", messages: [], asOf: Position(epoch: "e", seq: 0)),
                                    current: nil) == nil)
        // Attachments: of the chosen message, else of every message.
        #expect(AppModel.attachmentMessages(t, current: 2).map(\.id) == [2])
        #expect(AppModel.attachmentMessages(t, current: nil).map(\.id) == [1, 2, 3])
        #expect(AppModel.attachmentMessages(t, current: 99).map(\.id) == [1, 2, 3])
    }

    @Test func aPDFRefusedByTheCoreIsReported() async throws {
        let dir = try exportDir()
        core.handler = { method, params in
            guard method == "message_record" else { return Fixture.answer(method, params) }
            return CoreClient.RPCError(code: 1011, message: "raw damaged")
        }
        press("s"); press("p")
        #expect(await eventually { model.lastError?.contains("raw damaged") == true })
        #expect(saved(dir).isEmpty)
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

    // MARK: Copying to the clipboard (sc)

    @Test func scCopiesTheTextOfTheMailUnderTheCursor() async {
        let rows = controller.menuEntries(.save)
        #expect(rows.first?.key == "c" && rows.first?.label == ThreadFilter.capitalizedFirst(L("to clipboard")))
        press("s"); press("c")
        #expect(await eventually { controller.pasteboard.string(forType: .string) != nil })
        #expect(core.calls("message_text").map { $0["message_id"] as? Int } == [10])
        #expect(controller.pasteboard.string(forType: .string) == "From: sender1@example.org\nSubject: Subject 1\n\nParagraph of 10.")
        #expect(model.flash == L("“%@” copied to the clipboard", "Subject 1"))
        #expect(controller.overlay == .none)
        // Nothing saved to a file.
        #expect(core.calls("export_message").isEmpty)
    }

    @Test func scCopiesEveryMarkedMailInTheOrderOfTheList() async {
        press("J"); press("J")
        #expect(model.marked == [1, 2, 3])
        press("s"); press("c")
        #expect(await eventually { core.calls("message_text").count == 3 })
        #expect(await eventually { model.flash == L("%lld mails copied to the clipboard", 3) })
        let text = controller.pasteboard.string(forType: .string) ?? ""
        #expect(text.components(separatedBy: "\n\nFrom: ").count == 3)
        #expect(text.hasPrefix("From: sender1@") && text.hasSuffix("Paragraph of 30."))
    }

    @Test func scWithAFailingCoreLeavesThePasteboardAndSaysWhy() async {
        controller.pasteboard.clearContents()
        controller.pasteboard.setString("before", forType: .string)
        core.handler = { method, params in
            guard method == "message_text" else { return Fixture.answer(method, params) }
            return CoreClient.RPCError(code: CoreClient.RPCError.notFound, message: "message not found")
        }
        press("s"); press("c")
        #expect(await eventually { model.lastError != nil })
        #expect(controller.pasteboard.string(forType: .string) == "before")
        #expect(model.flash == nil)
    }

    @Test func clipboardTextPutsAnEmptyLineBetweenTwoMails() {
        #expect(AppModel.clipboardText(["A\n\nb"]) == "A\n\nb")
        #expect(AppModel.clipboardText(["A", "B"]) == "A\n\nB")
        #expect(AppModel.clipboardText([]) == "")
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
        // 16 pt, 18 pt in the status bar: the message says both.
        #expect(textSize.size == 16 && textSize.statusBarSize == 18)
        press("cmd-=")
        #expect(model.flash == said(17))
        press("cmd-+")
        #expect(textSize.size == 18 && textSize.statusBarSize == 20)
        // Each press says the size reached, not the first one's.
        #expect(model.flash == said(18))
        press("cmd--")
        #expect(textSize.size == 17)
        #expect(model.flash == said(17))
        press("cmd-0")
        #expect(textSize.size == 16 && textSize.statusBarSize == 18)
        #expect(model.flash == said(16))
    }

    /// The message of the text size at the default sizes moved by the same
    /// step: `size` for the window, two more for the status bar.
    func said(_ size: Int) -> String {
        L("Text size: %lld pt, status bar %lld pt", size, size + 2)
    }

    /// The View menu's items (Bigger Text…) reach the size through the
    /// model, as the keys do: a ⌘+ the key monitor does not recognize
    /// (another keyboard layout) lands there, and the status bar must
    /// still say the new size instead of keeping an older message.
    @Test func theMenuItemsSayTheNewSizeToo() {
        press("cmd-=")
        #expect(model.flash == said(17))
        model.changeTextSize(.bigger)
        #expect(textSize.size == 18 && model.flash == said(18))
        model.changeTextSize(.smaller)
        model.changeTextSize(.smaller)
        #expect(textSize.size == 16 && model.flash == said(16))
        model.changeTextSize(.bigger)
        model.changeTextSize(.reset)
        #expect(textSize.size == 16 && model.flash == said(16))
    }

    /// The sizes of `config.toml` come with `config_status`, at launch and
    /// after each `ConfigChanged`; ⌘0 comes back to them.
    @Test func theSizesOfTheFileApplyAndCommandZeroComesBackToThem() async {
        let sized = FakeCore { method, params in
            method == "config_status"
                ? ["ui": ["title_bar": "shown", "text_size": 18, "status_bar_text_size": 12]] as [String: Any]
                : Fixture.answer(method, params)
        }
        let model = AppModel()
        let sizes = TextSize()
        let controller = InterfaceController(model: model, defaults: defaults, textSize: sizes)
        await model.attach(sized)
        #expect(await eventually { sizes.size == 18 && sizes.statusBarSize == 12 })
        controller.perform(.textBigger)
        #expect(sizes.size == 19 && sizes.statusBarSize == 13)
        #expect(model.flash == L("Text size: %lld pt, status bar %lld pt", 19, 13))
        controller.perform(.textReset)
        #expect(sizes.size == 18 && sizes.statusBarSize == 12)
        // The file read again with the same sizes (another key changed):
        // a step of ⌘+ stays; with a new size, that size.
        controller.perform(.textBigger)
        await model.loadUISettings()
        #expect(sizes.size == 19)
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
        press("f")
        #expect(controller.overlay == .picker(.attributes))
        let labels = controller.pickerEntries(.attributes).map(\.label)
        // Compared with the translations: tests run in the machine's language.
        #expect(Array(labels.prefix(2)) == [ThreadFilter.capitalizedFirst(L("unread")), ThreadFilter.capitalizedFirst(L("read"))])
        #expect(labels.contains(L("From…")) && labels.contains(L("Conversation date…")))
        type(L("unread")); press("ret")
        #expect(model.attributes.seen == false)
        #expect(await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["seen"] as? Bool == false })
        // Choosing it again removes it.
        press("f"); type(L("unread")); press("ret")
        #expect(model.attributes.isEmpty)
    }

    @Test func typingBringsTheBestMatchFirstInTheAttributeSelector() {
        // The opposite value contains the typed word ("unread" / "non lus")
        // and is listed first: ranking puts the exact one under Return.
        press("f"); type(L("read"))
        #expect(controller.pickerEntries(.attributes).first?.label == ThreadFilter.capitalizedFirst(L("read")))
        press("ret")
        #expect(model.attributes.seen == true)
        // Initials reach a negative value.
        let initials = String(PickerMatch.words(PickerMatch.normalize(L("not flagged"))).compactMap(\.first))
        press("f"); type(initials)
        #expect(controller.pickerEntries(.attributes).first?.label == ThreadFilter.capitalizedFirst(L("not flagged")))
        press("ret")
        #expect(model.attributes.flagged == false)
        // A hidden search word reaches To….
        press("f"); type("cc")
        #expect(controller.pickerEntries(.attributes).first?.label == L("To…"))
    }

    @Test func theSelectorListsItsRowsInPairsWithALineBetween() {
        // `ui.md`, "Sélecteur d'attributs": the order asked for, every row
        // with a capital, a thin line before each pair but the first.
        model.attributes.toggle(.seen, false)
        press("f")
        let entries = controller.pickerEntries(.attributes)
        let C = ThreadFilter.capitalizedFirst
        #expect(entries.map(\.label) == [
            C(L("unread")), C(L("read")),
            C(L("flagged")), C(L("not flagged")),
            L("From…"), L("To…"),
            L("Subject…"), L("Text…"),
            L("Conversation date…"), L("Mail date…"),
            L("Group…"), L("Tag…"),
            C(L("human")), C(L("robot")),
            C(L("with attachments")), C(L("without attachments")),
            C(L("with my reply")), C(L("without my reply")),
            C(L("conversation")), C(L("single mail")),
            C(L("clear all attributes")),
        ])
        #expect(entries.allSatisfy { $0.label.first.map { String($0) == String($0).uppercased() } ?? false })
        // A line before rows 2, 4, 6…, and before "clear".
        #expect(entries.indices.filter { entries[$0].gapBefore } == [2, 4, 6, 8, 10, 12, 14, 16, 18, 20])
        // Typed: ranked rows, no line.
        type("a")
        #expect(controller.pickerEntries(.attributes).allSatisfy { !$0.gapBefore })
        // Every attribute is in the layout once.
        let rows = ThreadFilter.selectorLayout.joined()
        #expect(rows.count == ThreadFilter.Flag.allCases.count * 2 + AttributeField.allCases.count)
        #expect(Set(rows.map { "\($0)" }).count == rows.count)
        // The panel is tall enough for the rows and the lines.
        #expect(PickerPopup.height(.attributes, scale: 1)
            == PickerPopup.headerHeight + 1 + CGFloat(entries.count) * PickerPopup.rowHeight + 10 * PickerPopup.gapHeight + 4)
    }

    @Test func withMyReplyIsStillFoundByItsFormerName() {
        press("f"); type(L("answered replied").split(separator: " ").first.map(String.init) ?? "")
        #expect(controller.pickerEntries(.attributes).first?.label == ThreadFilter.capitalizedFirst(L("with my reply")))
        press("ret")
        #expect(model.attributes.answered == true)
        // The chip keeps the label as it is.
        #expect(model.attributes.chips.map(\.label) == [L("with my reply")])
    }

    @Test func humanAndRobotAreReachedInTheSelectorAndSentToTheCore() async {
        press("f")
        let labels = controller.pickerEntries(.attributes).map(\.label)
        #expect(labels.contains(ThreadFilter.capitalizedFirst(L("human"))) && labels.contains(ThreadFilter.capitalizedFirst(L("robot"))))
        type(L("human")); press("ret")
        #expect(model.attributes.human == true)
        #expect(await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["human"] as? Bool == true })
        // A hidden word reaches it too: "person" (French "personne").
        press("f"); type(L("person people").split(separator: " ").first.map(String.init) ?? "")
        #expect(controller.pickerEntries(.attributes).first?.label == ThreadFilter.capitalizedFirst(L("human")))
        press("esc")
        // The opposite replaces it.
        press("f"); type(L("robot")); press("ret")
        #expect(model.attributes.human == false)
        #expect(await eventually { (core.calls("list_threads").last?["filter"] as? [String: Any])?["human"] as? Bool == false })
    }

    // MARK: Tag… attribute

    /// Tag… (`ui.md`, "Sélecteur d'attributs"): one row per tag name, every
    /// account, most used first; Return chooses one and the panel stays
    /// open; `-` excludes; the core gets their names.
    @Test func theTagPanelOffersTheTagsAndTakesSeveral() async {
        await labelled()
        press("f"); type(L("Tag…"))
        #expect(controller.pickerEntries(.attributes).first?.label == L("Tag…"))
        press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.tag)))
        #expect(controller.pickerHint(.attributeValue(.tag)) != nil)
        // Urgent in two accounts (3 + 3 mails) is one row, before Calme.
        #expect(await eventually {
            controller.pickerEntries(.attributeValue(.tag)).map(\.label) == ["Urgent", "Calme", L("without tag")]
        })
        #expect(controller.pickerEntries(.attributeValue(.tag)).map(\.detail) == [L("%lld mails", 6), L("%lld mails", 3), nil])
        press("ret")
        #expect(model.attributes.tags == ["Urgent"])
        #expect(controller.overlay == .picker(.attributeValue(.tag)))
        #expect(controller.pickerEntries(.attributeValue(.tag)).map(\.key) == ["•", " ", " "])
        type("-cal")
        #expect(controller.pickerEntries(.attributeValue(.tag)).map(\.label) == [L("not %@", "Calme")])
        press("ret")
        #expect(model.attributes.notTags == ["Calme"])
        #expect(await eventually {
            let f = core.calls("list_threads").last?["filter"] as? [String: Any]
            return f?["tags"] as? [String] == ["Urgent"] && f?["not_tags"] as? [String] == ["Calme"]
        })
        #expect(model.attributes.chips.map(\.label) == [L("tag: %@", "Urgent"), L("not tag: %@", "Calme")])
        press("esc")
        press("f")
        #expect(controller.pickerEntries(.attributes).first { $0.label == L("Tag…") }?.key == "•")
    }

    /// "without tag" and "without group" (`ui.md`, "Sélecteur
    /// d'attributs"): last row of their panel, first once chosen; a
    /// wanted tag or group and they replace each other.
    @Test func withoutTagAndWithoutGroupAreRowsOfTheirPanels() async {
        await labelled()
        press("f"); type(L("Tag…")); press("ret")
        _ = await eventually { controller.pickerEntries(.attributeValue(.tag)).count == 3 }
        press("up"); press("ret")
        #expect(model.attributes.noTag)
        #expect(controller.overlay == .picker(.attributeValue(.tag)))
        #expect(controller.pickerEntries(.attributeValue(.tag)).first.map { [$0.key, $0.label] } == ["•", L("without tag")])
        #expect(controller.pickerEntries(.attributeValue(.tag))[controller.pickerIndex].label == L("without tag"))
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["no_tag"] as? Bool == true
        })
        #expect(model.attributes.chips.map(\.label) == [L("without tag")])
        // A wanted tag replaces it; "-" hides it (an exclusion already).
        type("urg"); press("ret")
        #expect(!model.attributes.noTag && model.attributes.tags == ["Urgent"])
        type("-")
        #expect(!controller.pickerEntries(.attributeValue(.tag)).contains { $0.label == L("without tag") })
        press("esc")
        // Typed, in either language: "none" finds it.
        press("f"); type(L("Group…")); press("ret")
        _ = await eventually { controller.pickerEntries(.attributeValue(.group)).count == 3 }
        type(L("none no").components(separatedBy: " ")[0])
        #expect(controller.pickerEntries(.attributeValue(.group)).first?.label == L("without group"))
        press("ret")
        #expect(model.attributes.noGroup)
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["no_group"] as? Bool == true
        })
        // Escape twice: the panel, then every attribute.
        press("esc"); press("esc")
        #expect(model.attributes.isEmpty)
    }

    @Test func withoutTagTheTagPanelSaysHowToMakeOne() async {
        press("f"); type(L("Tag…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.tag)).map(\.disabled) == [true])
    }

    /// The selector is as tall as all its rows (`PickerPopup.height`),
    /// "clear" included, unless the window is shorter.
    @Test func theAttributeSelectorShowsEveryRow() {
        model.attributes.toggleTag("Urgent")
        press("f")
        let rows = controller.pickerEntries(.attributes).count
        #expect(rows == InterfaceController.attributeRowCount)
        let height = PickerPopup.height(.attributes, scale: 1)
        #expect(height >= PickerPopup.headerHeight + 1 + CGFloat(rows) * PickerPopup.rowHeight)
        // Larger text, taller panel; a short window caps it.
        #expect(PickerPopup.height(.attributes, scale: 1.5) == height * 1.5)
        #expect(PickerPopup.height(.attributes, scale: 1, available: 300) == 268)
    }

    // MARK: Groups

    /// Group… (`ui.md`, "Sélecteur d'attributs"): the existing groups, most
    /// relevant first; Return chooses one and the panel stays open for
    /// another; `-` excludes; the core gets their ids.
    @Test func theGroupPanelOffersTheGroupsAndTakesSeveral() async {
        press("f"); type(L("Group…"))
        #expect(controller.pickerEntries(.attributes).first?.label == L("Group…"))
        press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.group)))
        #expect(controller.pickerHint(.attributeValue(.group)) != nil)
        #expect(await eventually { controller.pickerEntries(.attributeValue(.group)).count == 3 })
        // VIP changed last, Famille receives the most mail: one each; then
        // "without group".
        #expect(controller.pickerEntries(.attributeValue(.group)).map(\.label) == ["VIP", "Famille", L("without group")])
        #expect(controller.pickerEntries(.attributeValue(.group))[1].detail == L("%lld members", 3))
        press("ret")
        #expect(model.attributes.groups.map(\.name) == ["VIP"])
        // Still open, the chosen group marked and under the cursor.
        #expect(controller.overlay == .picker(.attributeValue(.group)))
        #expect(controller.pickerEntries(.attributeValue(.group)).map(\.key) == ["•", " ", " "])
        #expect(controller.pickerIndex == 0)
        // Typing narrows the rows; a second group is OR-ed.
        type("fam")
        #expect(controller.pickerEntries(.attributeValue(.group)).map(\.label) == ["Famille"])
        press("ret")
        #expect(model.attributes.groups.map(\.id) == [1, 2])
        #expect(controller.pickerQuery.isEmpty)
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["group_ids"] as? [Int] == [1, 2]
        })
        // The current folder stays: the list is narrowed, not replaced.
        #expect((core.calls("list_threads").last?["folder_ids"] as? [Int]) == [1])
        // "-" turns the rows into exclusions; excluding VIP removes it from
        // the wanted groups.
        type("-vip")
        #expect(controller.pickerEntries(.attributeValue(.group)).map(\.label) == [L("not %@", "VIP")])
        press("ret")
        #expect(model.attributes.groups.map(\.id) == [2] && model.attributes.notGroups.map(\.id) == [1])
        #expect(model.attributes.chips.map(\.label) == [L("group: %@", "Famille"), L("not group: %@", "VIP")])
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["not_group_ids"] as? [Int] == [1]
        })
        // Return on a chosen group removes it.
        let famille = controller.pickerEntries(.attributeValue(.group)).firstIndex { $0.keywords == ["Famille"] } ?? -1
        controller.pickerIndex = famille
        press("ret")
        #expect(model.attributes.groups.isEmpty)
        press("esc")
        #expect(controller.overlay == .none)
        // Chosen again in the selector, Group… is marked active.
        press("f")
        #expect(controller.pickerEntries(.attributes).first { $0.label == L("Group…") }?.key == "•")
    }

    @Test func theGroupPanelSaysWhenThereIsNoGroup() async {
        core.handler = { method, params in
            method == "list_groups" ? ["items": [], "as_of": Fixture.position] : Fixture.answer(method, params)
        }
        await model.loadGroups()
        controller.openGroupPanel()
        let rows = controller.pickerEntries(.attributeValue(.group))
        #expect(rows.count == 1 && rows[0].disabled)
        press("ret")
        #expect(model.attributes.isEmpty)
        #expect(controller.columnGroups.rows.isEmpty)
    }

    /// The folder column (`ui.md`, "Groupes dans la colonne des dossiers"):
    /// a click narrows the current list, a second one removes the group,
    /// ⌘-click excludes it, Escape clears it as any attribute.
    @Test func aClickOnAGroupOfTheColumnNarrowsTheCurrentList() async {
        #expect(await eventually { model.groups.count == 2 })
        #expect(controller.columnGroups.rows.map(\.name) == ["VIP", "Famille"] && controller.columnGroups.hidden == 0)
        let vip = model.groups[0], famille = model.groups[1]
        #expect(!controller.isHighlighted(vip) && !controller.isHighlighted(famille))
        controller.clickGroup(vip, .none)
        // Highlighted like the current folder, which stays highlighted.
        #expect(controller.isHighlighted(vip) && !controller.isHighlighted(famille))
        controller.clickGroup(famille, .none)
        #expect(model.attributes.groups.map(\.id) == [1, 2])
        #expect(model.selectedFolderId == 1)
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["group_ids"] as? [Int] == [1, 2]
        })
        controller.clickGroup(vip, .none)
        #expect(model.attributes.groups.map(\.id) == [2])
        #expect(!controller.isHighlighted(vip))
        controller.clickGroup(vip, .command)
        #expect(model.attributes.notGroups.map(\.id) == [1])
        // An excluded group filters the list too: highlighted as well.
        #expect(controller.isHighlighted(vip))
        // Another folder keeps them, as every attribute.
        controller.goToFolder(3)
        #expect(model.attributes.groupChoice(2) == true && model.attributes.groupChoice(1) == false)
        press("esc")
        #expect(model.attributes.isEmpty)
        #expect(!controller.isHighlighted(vip) && !controller.isHighlighted(famille))
    }

    @Test func attributesWithAValueExpandThePanel() async {
        press("f"); type(L("From…")); press("ret")
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
        press("f"); type(L("From…")); press("ret")
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
        press("f"); type(L("To…")); press("ret")
        type("-alice")
        rows = controller.pickerEntries(.attributeValue(.to))
        #expect(rows.first?.key == "−" && rows.first?.label == L("not %@", "alice"))
        #expect(await eventually { core.calls("list_contacts").contains { $0["query"] as? String == "alice" } })
        press("ret")
        #expect(model.attributes.notTo == ["alice"])
    }

    @Test func textAndSubjectPanelsTakeWordsAndExclusions() {
        press("f"); type(L("Text…")); press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.text)))
        type("acme.fr"); press("ret")
        #expect(model.attributes.text == "acme.fr")
        // An exclusion decided after the fact: → leaves the current words,
        // selected, for the end of the line.
        press("f"); type(L("Text…")); press("ret")
        press("right"); type(" -facture"); press("ret")
        #expect(model.attributes.text == "acme.fr -facture")
        press("f"); type(L("Subject…")); press("ret")
        type("-panne"); press("ret")
        #expect(model.attributes.subject == "-panne")
        #expect(model.attributes.contains == nil)
    }

    @Test func aSecondSubjectReplacesTheFirst() async {
        press("f"); type(L("Subject…")); press("ret")
        type("devis"); press("ret")
        #expect(model.attributes.subject == "devis")
        // The panel opens on the current subject, selected: typing
        // replaces it.
        press("f"); type(L("Subject…")); press("ret")
        #expect(controller.pickerQuery == "devis" && controller.pickerLine.selectedAll)
        type("facture")
        #expect(controller.pickerQuery == "facture" && !controller.pickerLine.selectedAll)
        press("ret")
        #expect(model.attributes.subject == "facture")
        #expect(await eventually {
            (core.calls("list_threads").last?["filter"] as? [String: Any])?["subject"] as? String == "facture"
        })
        // Return without touching it keeps it as it is.
        press("f"); type(L("Subject…")); press("ret"); press("ret")
        #expect(model.attributes.subject == "facture")
        // A paste replaces it too.
        copy("rapport\n")
        press("f"); type(L("Subject…")); press("ret"); press("cmd-v"); press("ret")
        #expect(model.attributes.subject == "rapport")
    }

    @Test func erasingTheWordsOfThePanelRemovesTheCriterion() {
        press("f"); type(L("Subject…")); press("ret")
        type("devis"); press("ret")
        press("f"); type(L("Subject…")); press("ret")
        // Delete clears the selected words; Return then removes the chip.
        press("del")
        #expect(controller.pickerQuery == "")
        let rows = controller.pickerEntries(.attributeValue(.subject))
        #expect(rows.map(\.label) == [L("remove %@", L("subject: %@", "devis"))])
        press("ret")
        #expect(model.attributes.subject == nil && model.attributes.isEmpty)
        // With no criterion, an empty panel offers nothing.
        press("f"); type(L("Subject…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.subject)).isEmpty && !controller.pickerLine.selectedAll)
    }

    @Test func valuePanelsSayWhatCanBeTyped() {
        press("f"); type(L("Conversation date…")); press("ret")
        // The Date… panel opens on its presets: the hint is on the input
        // line, not only in an empty panel.
        #expect(controller.pickerHint(.attributeValue(.date))?.contains(DateChoice.Order.ui.examples) == true)
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
        press("f"); type(L("Conversation date…")); press("ret")
        #expect(controller.pickerEntries(.attributeValue(.date)).count == 5)
        type("2026-10-01")
        let labels = controller.pickerEntries(.attributeValue(.date)).map(\.label)
        #expect(labels.first == L("after %@", "2026-10-01"))
        press("ret")
        #expect(model.attributes.after != nil && model.attributes.afterLabel == L("after %@", "2026-10-01"))
    }

    @Test func theDatePanelBuildsAnIntervalAndReadsDurations() {
        press("f"); type(L("Conversation date…")); press("ret")
        type("2026-10-01"); press("ret")
        // An upper bound kept with the lower one: two chips. Far ahead, so
        // that the duration below ("3s", from today) never passes it: an
        // upper bound in the past would make that row "nothing in between"
        // once the day of the test is late enough.
        press("f"); type(L("Conversation date…")); press("ret")
        type("2099-01-01"); press("down"); press("ret")
        #expect(model.attributes.chips.map(\.label) == [L("after %@", "2026-10-01"), L("before %@", "2099-01-01")])
        // An upper bound before the lower one: the row says so.
        press("f"); type(L("Conversation date…")); press("ret")
        type("2026-09-01")
        let rows = controller.pickerEntries(.attributeValue(.date))
        #expect(rows[1].label == L("before %@", "2026-09-01") && rows[1].detail == L("nothing in between"))
        #expect(rows[0].detail == nil)
        press("esc")
        // A duration: newer than, then older than, with the date it gives.
        press("f"); type(L("Conversation date…")); press("ret")
        type("3s")
        let d = controller.pickerEntries(.attributeValue(.date))
        #expect(d.prefix(2).map(\.label) == [L("newer than %@", L("%lld weeks", 3)), L("older than %@", L("%lld weeks", 3))])
        #expect(d[0].detail?.count == 10)
        press("ret")
        #expect(model.attributes.afterLabel == L("newer than %@", L("%lld weeks", 3)))
        #expect(model.attributes.beforeLabel == L("before %@", "2099-01-01"))
    }

    /// Two date criteria (`ui.md`, "Deux critères de date"): Conversation
    /// date… first (two letters reach it), Mail date… apart, each with its
    /// own bounds, chips and request parameters.
    @Test func mailDateIsACriterionOfItsOwn() async {
        press("f")
        let labels = controller.pickerEntries(.attributes).map(\.label)
        let conversation = labels.firstIndex(of: L("Conversation date…"))
        let mail = labels.firstIndex(of: L("Mail date…"))
        #expect(conversation != nil && mail != nil && conversation! < mail!)
        type(L("Mail date…")); press("ret")
        #expect(controller.overlay == .picker(.attributeValue(.mailDate)))
        #expect(controller.pickerHint(.attributeValue(.mailDate))?.contains(DateChoice.Order.ui.examples) == true)
        type("2020-01-01"); press("down"); press("ret")
        #expect(model.attributes.mailBefore != nil && model.attributes.before == nil)
        #expect(model.attributes.chips.map(\.label) == [L("mail: %@", L("before %@", "2020-01-01"))])
        #expect(await eventually {
            let f = core.calls("list_threads").last?["filter"] as? [String: Any]
            return f?["mail_before"] as? Int == model.attributes.mailBefore && f?["before"] == nil
        })
        // Its interval is its own: a conversation bound does not empty it.
        press("f"); type(L("Conversation date…")); press("ret")
        type("2021-01-01")
        #expect(controller.pickerEntries(.attributeValue(.date))[0].detail != L("nothing in between"))
        press("ret")
        #expect(model.attributes.after != nil && model.attributes.mailBefore != nil)
        // An empty interval of Mail date… is said in its own panel.
        press("f"); type(L("Mail date…")); press("ret")
        type("2021-01-01")
        #expect(controller.pickerEntries(.attributeValue(.mailDate))[0].detail == L("nothing in between"))
        press("esc")
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
