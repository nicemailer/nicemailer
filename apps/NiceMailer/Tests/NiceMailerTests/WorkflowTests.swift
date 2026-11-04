import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The folder workflow in the app (`AppModel+Workflow.swift`): what a move
/// says of the decision the core took, `z`, the confirmation of a suspect
/// move, and the decisions of the contacts screen. Against a fake core whose
/// Inbox also offers Newsletter, and which refuses to move thread 2 without
/// a confirmation, as the core does for a sender suspected of spoofing.
@MainActor
struct WorkflowTests {
    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async {
        core = FakeCore { method, params in
            switch method {
            case "list_folders":
                var folders = Fixture.folders
                folders[0] = Fixture.folder(1, "inbox", "Inbox", "I",
                                            targets: ["todo", "waiting", "archive", "newsletter", "deleted", "junk"])
                return ["items": folders, "as_of": Fixture.position]
            case "move_thread" where params["thread_id"] as? Int == 2 && params["confirm_suspect"] == nil:
                return CoreClient.RPCError(code: CoreClient.RPCError.confirmationRequired, message: "confirmation requise",
                                           data: ["addr": "support@banque.fr", "verdict": "fail"])
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        model.openFile = { _ in }
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        switch name {
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        case "ret": chars = "\r"; code = KeyCode.returnKey
        case "down": chars = "\u{F701}"; code = KeyCode.down
        default: break
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        #expect(controller.handle(event))
    }

    // MARK: Messages

    @Test func theMoveMessageTellsWhatTheDecisionDid() {
        let plain = Workflow.moveFlash(subjects: ["Devis"], folder: "Archive", decision: nil, filed: 0)
        #expect(plain.contains("Devis") && plain.contains("Archive") && !plain.contains("z"))
        let d = DecisionView(kind: "address", name: "news@shop.fr", before: nil, after: "newsletter")
        let decided = Workflow.moveFlash(subjects: ["Promo"], folder: "Newsletter", decision: d, filed: 12)
        #expect(decided.contains("news@shop.fr") && decided.contains("12") && decided.contains("z"))
        let several = Workflow.moveFlash(subjects: ["a", "b", "c"], folder: "Todo", decision: nil, filed: 0)
        #expect(several.contains("3") && several.contains("Todo"))
    }

    @Test func theDecisionMessageNamesTheFolderOrTheRemoval() {
        let d = DecisionView(kind: "domain", name: "shop.fr", before: nil, after: "banned")
        let text = Workflow.decisionFlash(d, filed: 4)
        #expect(text.contains("shop.fr") && text.contains("Banned") && text.contains("4"))
        let removed = Workflow.decisionFlash(DecisionView(kind: "address", name: "a@b.fr", before: "inbox", after: nil), filed: 0)
        #expect(removed.contains("a@b.fr") && !removed.contains("z"))
        #expect(!Workflow.decisionFlash(nil, filed: 0).isEmpty)
    }

    @Test func theDomainOfAnAddress() {
        #expect(GeekController.domain(of: "a@News.Shop.fr") == "news.shop.fr")
        #expect(GeekController.domain(of: "nobody") == nil)
        #expect(GeekController.domain(of: "x@") == nil)
    }

    // MARK: Moves and undo

    @Test func aMoveThatDecidesSaysSoAndCanBeUndone() async {
        press("m"); press("n")
        #expect(await eventually { model.flash?.contains("sender1@example.org") == true })
        #expect(model.flash?.contains("2") == true)
        #expect(model.undoStack == [UndoStep(tokens: ["1:1"], label: L("move to %@", "Newsletter"))])

        press("z")
        #expect(await eventually { core.calls("undo").count == 1 })
        #expect(core.calls("undo")[0]["token"] as? String == "1:1")
        #expect(await eventually { model.undoStack.isEmpty && model.flash?.contains("1") == true })
        // Nothing left: said, and nothing sent.
        press("z")
        #expect(await eventually { model.flash != nil })
        #expect(core.calls("undo").count == 1)
    }

    @Test func aMoveWithoutDecisionKeepsItsMessageAndIsUndoneToo() async {
        press("d")
        let flash = model.flash
        #expect(await eventually { model.undoStack.count == 1 })
        #expect(model.flash == flash)
    }

    @Test func zUndoesTheThreadsOfOneCommandTogetherLastFirst() async {
        // Threads 3 to 5 (thread 2 waits for a confirmation in this core).
        press("j"); press("j")
        press("J"); press("J")
        press("d")
        #expect(await eventually { model.undoStack.first?.tokens.count == 3 })
        #expect(model.undoStack.count == 1)
        press("z")
        #expect(await eventually { core.calls("undo").count == 3 })
        #expect(core.calls("undo").map { $0["token"] as? String } == ["5:5", "4:4", "3:3"])
    }

    @Test func theUndoStackKeepsTheMostRecentSteps() {
        for i in 0..<(Workflow.undoDepth + 5) { model.pushUndo(UndoStep(tokens: ["\(i)"], label: "")) }
        #expect(model.undoStack.count == Workflow.undoDepth)
        #expect(model.undoStack.last?.tokens == ["\(Workflow.undoDepth + 4)"])
    }

    // MARK: Suspect sender

    /// Thread 2 under the cursor, moved to the Inbox's Newsletter: refused,
    /// then asked.
    private func moveSuspect() async {
        press("j")
        press("m"); press("n")
        _ = await eventually { model.suspectMove != nil }
    }

    @Test func aSuspectMoveAsksAndConfirmsWithThePolicy() async {
        await moveSuspect()
        #expect(model.suspectMove?.addr == "support@banque.fr")
        #expect(controller.activeChoice == .suspect(model.suspectMove!))
        #expect(controller.choiceEntries(controller.activeChoice!).map(\.key) == ["y", "t", "esc"])
        #expect(controller.modeLabel == "ASK")
        // The thread came back to the list, under the cursor.
        #expect(await eventually { model.threads.contains { $0.id == 2 } && model.selectedThreadId == 2 })
        // Any other key is ignored while the question waits.
        press("d")
        #expect(model.suspectMove != nil && core.calls("move_thread").count == 1)

        press("t")
        #expect(model.suspectMove == nil && controller.activeChoice == nil)
        #expect(await eventually { core.calls("move_thread").count == 2 })
        let confirmed = core.calls("move_thread")[1]
        #expect(confirmed["confirm_suspect"] as? Bool == true)
        #expect(confirmed["auth_policy"] as? String == "not_required")
        #expect(await eventually { model.undoStack.count == 1 })
    }

    @Test func aSuspectMoveConfirmedWithYKeepsThePolicy() async {
        await moveSuspect()
        press("y")
        #expect(await eventually { core.calls("move_thread").count == 2 })
        #expect(core.calls("move_thread")[1]["auth_policy"] == nil)
    }

    @Test func escapeLeavesTheSuspectMailWhereItIs() async {
        await moveSuspect()
        press("esc")
        #expect(model.suspectMove == nil)
        #expect(model.flash?.contains("Subject 2") == true)
        try? await Task.sleep(for: .milliseconds(50))
        #expect(core.calls("move_thread").count == 1)
        #expect(model.undoStack.isEmpty)
    }

    // MARK: Contacts

    private func openContacts() async {
        controller.open(.contacts)
        _ = await eventually { controller.contacts.count == 3 }
    }

    @Test func mOnAContactChoosesTheDecisionOfItsAddress() async {
        await openContacts()
        press("m")
        #expect(controller.overlay == .choice(.decision(name: "c1@example.org", domain: false, current: nil)))
        #expect(controller.choiceEntries(controller.activeChoice!).map(\.key) == ["i", "n", "p", "b", "-"])
        #expect(controller.modeLabel == "ASK")
        press("n")
        #expect(controller.overlay == .none)
        #expect(await eventually { core.calls("set_address_screening").count == 1 })
        let call = core.calls("set_address_screening")[0]
        #expect(call["addr"] as? String == "c1@example.org" && call["target"] as? String == "newsletter")
        #expect(await eventually { model.undoStack.count == 1 && model.flash?.contains("Newsletter") == true })
    }

    @Test func shiftMChoosesTheDecisionOfTheDomainAndDashRemovesIt() async {
        await openContacts()
        press("down")
        press("M")
        #expect(controller.overlay == .choice(.decision(name: "example.org", domain: true, current: nil)))
        press("-")
        #expect(await eventually { core.calls("set_domain_screening").count == 1 })
        let call = core.calls("set_domain_screening")[0]
        #expect(call["domain"] as? String == "example.org" && call["target"] == nil)
    }

    @Test func aChoiceCanBeMadeWithTheArrowsOrCancelled() async {
        await openContacts()
        press("m"); press("esc")
        #expect(controller.overlay == .none && controller.screen == .contacts)
        press("m"); press("down"); press("down"); press("ret")
        #expect(await eventually { core.calls("set_address_screening").first?["target"] as? String == "newsletter" })
    }
}
