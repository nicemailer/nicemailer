import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The History screen (`tracabilite.md`, section 6): the words of each
/// action, the rows of the groups, and Return, Escape and `u` against a core
/// played by the test. `history.uiscript` plays them with real keys.
@MainActor
struct HistoryScreenTests {
    private func group(_ id: Int, kind: String = "move", value: String? = "archive", count: Int = 1,
                       entity: String = "message", entityId: Int = 1, source: String = "app",
                       title: String? = "La facture") -> HistoryGroup {
        HistoryGroup(firstId: id - count + 1, lastId: id, count: count, date: 1_700_000_000, source: source,
                     entity: entity, entityId: entityId, kind: kind, value: value, title: title)
    }

    // MARK: Words

    @Test func everyActionOfTheLogHasWords() {
        let kinds = ["read", "unread", "flagged", "unflagged", "move", "screen", "decision", "decision_removed",
                     "tag_added", "tag_removed", "send", "delete", "revert", "add_member", "remove_member", "create",
                     "rename", "update", "remove", "enable", "disable", "import_mbox", "reorder", "set_password",
                     "reload", "role", "initial_archive", "sender_kind", "unsubscribe", "auth_policy"]
        for kind in kinds {
            let words = HistoryDisplay.action(group(1, kind: kind))
            #expect(!words.verb.isEmpty && !words.verb.contains("_"), "\(kind): \(words)")
        }
    }

    @Test func aFolderIsNamedAndAnUnknownActionIsStillShown() {
        let move = HistoryDisplay.action(group(1, kind: "move", value: "archive"))
        #expect(move == .init(verb: L("Moved"), detail: L("to %@", "Archive")))
        #expect(HistoryDisplay.action(group(1, kind: "move", value: "Clients/2026")).detail == L("to %@", "Clients/2026"))
        #expect(HistoryDisplay.action(group(1, kind: "read", value: nil)) == .init(verb: L("Marked"), detail: L("as read")))
        #expect(HistoryDisplay.action(group(1, kind: "send", value: nil)).detail.isEmpty)
        #expect(HistoryDisplay.action(group(1, kind: "brand_new")).verb == "brand new")
    }

    @Test func whoDidItIsSaidAsTheUserSaysIt() {
        #expect(HistoryDisplay.who("app") == L("Me"))
        #expect(HistoryDisplay.who("core") == "NiceMailer")
        #expect(HistoryDisplay.who("mcp") == L("AI agent"))
        #expect(HistoryDisplay.who("imap") == L("Mail server"))
        #expect(HistoryDisplay.who("robot") == "robot")
    }

    @Test func aRowAlwaysSaysWhatItIsAbout() {
        #expect(HistoryDisplay.title(group(1)) == "La facture")
        #expect(HistoryDisplay.title(group(1, title: "")) == L("(no subject)"))
        #expect(HistoryDisplay.title(group(1, title: nil)) == L("Deleted item"))
        #expect(HistoryDisplay.title(group(1, entity: "config", title: nil)) == L("Settings"))
    }

    @Test func theDateIsShort() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let today = Format.historyDate(1_700_000_000, now: now, calendar: calendar)
        let earlier = Format.historyDate(1_700_000_000 - 40 * 86_400, now: now, calendar: calendar)
        #expect(today.count <= 8)
        #expect(earlier.count > today.count && earlier.count <= 16, "\(earlier)")
    }

    // MARK: Rows

    @Test func theLinesOfAnUnfoldedGroupComeUnderIt() {
        let a = group(10, count: 2), b = group(5)
        let lines = [group(10), group(9)]
        let folded = HistoryDisplay.rows([a, b], unfolded: [:])
        #expect(folded.map(\.isDetail) == [false, false])
        let open = HistoryDisplay.rows([a, b], unfolded: [10: lines])
        #expect(open.map(\.group.lastId) == [10, 10, 9, 5])
        #expect(open.map(\.isDetail) == [false, true, true, false])
        #expect(open[0].isUnfolded && !open[3].isUnfolded)
        #expect(Set(open.map(\.id)).count == 4, "a group and its line are two rows")
    }

    // MARK: Keys

    /// A controller on a core whose log holds a mail moved, a batch and a
    /// sender decided.
    private func screen(pages: [[[String: Any]]]? = nil) async -> (GeekController, AppModel, FakeCore) {
        let row = { (id: Int, count: Int, entity: String, kind: String, title: String) -> [String: Any] in
            ["first_id": id - count + 1, "last_id": id, "count": count, "date": 1_700_000_000, "source": "app",
             "entity": entity, "entity_id": id, "kind": kind, "value": "archive", "title": title]
        }
        let first = [row(30, 1, "message", "move", "Mail"), row(20, 3, "message", "read", "Lot"),
                     row(10, 1, "address", "decision", "c1@example.org")]
        let pages = pages ?? [first]
        let core = FakeCore { method, params in
            switch method {
            case "list_history_groups":
                if params["first_id"] != nil {
                    return ["items": [row(20, 1, "message", "read", "Un"), row(19, 1, "message", "read", "Deux"),
                                      row(18, 1, "message", "read", "Trois")],
                            "next_cursor": NSNull(), "as_of": ["epoch": "e", "seq": 0]]
                }
                let page = params["cursor"] as? String == "p2" ? pages.count - 1 : 0
                return ["items": pages[page], "next_cursor": page == 0 && pages.count > 1 ? "p2" : NSNull(),
                        "as_of": ["epoch": "e", "seq": 0]] as [String: Any]
            case "locate_message":
                return ["message_id": params["message_id"] ?? 0, "thread_id": 2, "folder_id": 1, "account_id": 1]
            default:
                return Fixture.answer(method, params)
            }
        }
        let model = AppModel()
        let defaults = TestDefaults.make()
        let controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        model.addressBook = FixtureAddressBook(cards: [:])
        await model.attach(core)
        controller.open(.history)
        _ = await eventually { !controller.history.isEmpty }
        return (controller, model, core)
    }

    private func press(_ controller: GeekController, _ chars: String, code: UInt16 = 0, mods: NSEvent.ModifierFlags = []) {
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0,
                                     context: nil, characters: chars, charactersIgnoringModifiers: chars,
                                     isARepeat: false, keyCode: code)!
        _ = controller.handle(event)
    }

    @Test func returnOnAMailOpensItAndEscapeComesBackToTheRow() async {
        let (controller, model, _) = await screen()
        press(controller, "\r", code: KeyCode.returnKey)
        #expect(await eventually { controller.screen == .mail })
        #expect(model.selectedThreadId == 2)
        press(controller, "\u{1b}", code: KeyCode.escape)
        #expect(controller.screen == .history)
        #expect(controller.screenIndex == 0)
        #expect(!controller.returnsToHistory)
    }

    @Test func aMailGoneStaysOnTheHistoryAndSaysSo() async {
        let (controller, model, core) = await screen()
        let answer = core.handler
        core.handler = { method, params in
            method == "locate_message" ? CoreClient.RPCError(code: CoreClient.RPCError.notFound, message: "gone") : answer(method, params)
        }
        press(controller, "\r", code: KeyCode.returnKey)
        #expect(await eventually { model.lastError == L("This mail is no longer in NiceMailer") })
        #expect(controller.screen == .history)
    }

    @Test func returnOnABatchUnfoldsItAndFoldsItAgain() async {
        let (controller, _, core) = await screen()
        press(controller, "j")
        press(controller, "\r", code: KeyCode.returnKey)
        #expect(await eventually { controller.historyRows.count == 6 })
        let asked = core.calls("list_history_groups").last
        #expect(asked?["first_id"] as? Int == 18 && asked?["last_id"] as? Int == 20)
        #expect(controller.historyRows[1].isUnfolded)
        press(controller, "\r", code: KeyCode.returnKey)
        #expect(await eventually { controller.historyRows.count == 3 })
    }

    @Test func uShowsTheMailsMarkedAsRead() async {
        let (controller, _, core) = await screen()
        #expect(core.calls("list_history_groups").last?["include_read"] as? Bool == false)
        press(controller, "u")
        #expect(await eventually { core.calls("list_history_groups").last?["include_read"] as? Bool == true })
        #expect(controller.historyIncludesRead)
        #expect(controller.screenIndex == 0)
    }

    /// `ctrl-d` and `ctrl-u` move by half of the rows that fit, and stop on
    /// the first and the last row.
    @Test func controlDAndUMoveByHalfAPage() async {
        let row = { (id: Int) -> [String: Any] in
            ["first_id": id, "last_id": id, "count": 1, "date": 1_700_000_000, "source": "app", "entity": "message",
             "entity_id": id, "kind": "move", "value": "archive", "title": "Mail \(id)"]
        }
        let (controller, _, _) = await screen(pages: [(1...12).map(row)])
        controller.screenVisibleRows = 10
        press(controller, "d", mods: .control)
        #expect(controller.screenIndex == 5)
        press(controller, "d", mods: .control)
        #expect(controller.screenIndex == 10)
        press(controller, "d", mods: .control)
        #expect(controller.screenIndex == 11)
        press(controller, "u", mods: .control)
        #expect(controller.screenIndex == 6)
        press(controller, "u", mods: .control)
        press(controller, "u", mods: .control)
        #expect(controller.screenIndex == 0)
        #expect(!controller.historyIncludesRead, "ctrl-u is not u")
    }

    /// The wheel moves no cursor: the rows coming into view load the next
    /// page, once.
    @Test func scrollingToTheEndLoadsTheNextPageOnce() async {
        let row = { (id: Int) -> [String: Any] in
            ["first_id": id, "last_id": id, "count": 1, "date": 1_700_000_000, "source": "app", "entity": "message",
             "entity_id": id, "kind": "move", "value": "archive", "title": "Mail \(id)"]
        }
        let (controller, _, core) = await screen(pages: [(1...30).map(row), (31...40).map(row)])
        #expect(controller.history.count == 30)
        controller.screenRowAppeared(3)
        #expect(core.calls("list_history_groups").count == 1, "far from the end: nothing to load")
        controller.screenRowAppeared(25)
        controller.screenRowAppeared(26)
        #expect(await eventually { controller.history.count == 40 })
        #expect(core.calls("list_history_groups").count == 2)
    }
}
