import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The rules of the AI panel (`AI`), pure functions.
struct AIRulesTests {
    let replace = AIProposal.replace(segments: [AISegmentChange(old: "teh", new: "the", diff: [
        AIDiffPiece(kind: "removed", text: "teh "), AIDiffPiece(kind: "added", text: "The "), AIDiffPiece(kind: "same", text: "text"),
    ])], unchanged: false)

    @Test func theKeysOfThePanelFollowItsPhase() {
        func act(_ p: AIRun.Phase, _ key: String = "", ret: Bool = false, esc: Bool = false) -> AIKeyAction {
            AI.action(p, key: key, isReturn: ret, isEscape: esc)
        }
        // Running: Escape stops it; anything else only says so.
        #expect(act(.running, esc: true) == .cancel)
        #expect(act(.running, "y") == .hint)
        #expect(act(.running, ret: true) == .hint)
        // A replacement: Return or y accepts, Escape or n refuses.
        #expect(act(.proposal(replace), ret: true) == .accept)
        #expect(act(.proposal(replace), "y") == .accept)
        #expect(act(.proposal(replace), esc: true) == .refuse)
        #expect(act(.proposal(replace), "n") == .refuse)
        #expect(act(.proposal(replace), "x") == .hint)
        // Nothing to change: Return or Escape close it.
        let none = AIProposal.replace(segments: [], unchanged: true)
        #expect(act(.proposal(none), ret: true) == .close)
        #expect(act(.proposal(none), "y") == .hint)
        // Subjects: a digit chooses one, from 1; out of range says so.
        let subjects = AIProposal.subjects(["A", "B", "C"])
        #expect(act(.proposal(subjects), "2") == .choose(1))
        #expect(act(.proposal(subjects), "4") == .hint)
        #expect(act(.proposal(subjects), "0") == .hint)
        #expect(act(.proposal(subjects), ret: true) == .hint)
        #expect(act(.proposal(subjects), esc: true) == .refuse)
        // A failure: Escape or Return close it.
        #expect(act(.failed("x"), esc: true) == .close)
        #expect(act(.failed("x"), "n") == .hint)
    }

    @Test func theProposalsOfAMailAreReadOrChosen() {
        func act(_ p: AIProposal, _ key: String = "", ret: Bool = false, esc: Bool = false) -> AIKeyAction {
            AI.action(.proposal(p), key: key, isReturn: ret, isEscape: esc)
        }
        for p in [AIProposal.text("x"), .summary("s", actions: []), .verdict(risk: "low", reasons: ["r"], raised: false)] {
            #expect(act(p, ret: true) == .close)
            #expect(act(p, esc: true) == .close)
            #expect(act(p, "y") == .hint, "nothing to accept in \(p)")
            #expect(AI.hint(.proposal(p)) == L("esc close"))
        }
        let replies = AIProposal.choices(["Oui", "Non"], then: "reply")
        #expect(act(replies, "1") == .choose(0))
        #expect(act(replies, "3") == .hint)
        #expect(act(replies, esc: true) == .refuse)
        #expect(AI.hint(.proposal(replies)) == L("1–%lld reply with it · esc or n refuse", 2))
    }

    @Test func aResultReadsAsText() {
        let summary = AIProposal.summary("Alice attend jeudi.", actions: [
            AIAction(task: "Apporter les clés", owner: "you", due: "jeudi"), AIAction(task: "Réserver", owner: "", due: ""),
        ])
        #expect(AI.resultText(summary) == "Alice attend jeudi.\n- Apporter les clés (you) · jeudi\n- Réserver")
        #expect(AI.resultText(.verdict(risk: "medium", reasons: ["a link lies"], raised: true)) == L("Risk: medium") + "\n- a link lies")
        #expect(AI.resultText(.choices(["Oui", "Non"], then: "reply")) == "1. Oui\n2. Non")
        #expect(AI.riskLabel("anything") == L("Risk: low"))
    }

    @Test func aQuickReplyGoesAboveTheQuotationApartFromWordsAlreadyTyped() {
        let fresh = AI.insertReply("Oui.", into: TextBuffer("\n\nOn Nov 7, A wrote:\n> Hi"))
        #expect(fresh.text == "Oui.\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(fresh.cursor == 4)
        let started = AI.insertReply("Oui.", into: TextBuffer("merci\n\nOn Nov 7, A wrote:\n> Hi", cursor: 3))
        #expect(started.text == "Oui.\n\nmerci\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(started.cursor == 4)
    }

    @Test func theHintSaysTheKeys() {
        #expect(AI.hint(.running) == L("esc stop"))
        #expect(AI.hint(.proposal(.subjects(["A", "B"]))) == L("1–%lld choose · esc or n refuse", 2))
        #expect(AI.hint(.proposal(.replace(segments: [], unchanged: true))) == L("No change to make · esc close"))
    }

    @Test func aReplacementReadsAsAWordDiff() {
        guard case .replace(let segments, _) = replace else { return }
        #expect(AI.diffText(segments) == "[-teh -]{+The +}text")
        let two = [segments[0], AISegmentChange(old: "a", new: "a", diff: [AIDiffPiece(kind: "same", text: "Yes.")])]
        #expect(AI.diffText(two) == "[-teh -]{+The +}text\n---\nYes.")
    }

    @Test func proposalsDecodeByTheirKind() throws {
        func decode(_ json: [String: Any]) throws -> AIProposal {
            try CoreClient.decoder.decode(AIProposal.self, from: JSONSerialization.data(withJSONObject: json))
        }
        let r = try decode(["kind": "replace", "unchanged": false, "body": "x", "segments": [
            ["start": 0, "end": 3, "old": "teh", "new": "the", "diff": [["kind": "added", "text": "the"]]],
        ]])
        #expect(r == .replace(segments: [AISegmentChange(old: "teh", new: "the", diff: [AIDiffPiece(kind: "added", text: "the")])], unchanged: false))
        #expect(try decode(["kind": "subjects", "items": ["A", "B"]]) == .subjects(["A", "B"]))
        #expect(try decode(["kind": "text", "text": "Hi"]) == .text("Hi"))
        #expect(try decode(["kind": "summary", "summary": "S", "actions": [["task": "T", "owner": "", "due": ""]]])
            == .summary("S", actions: [AIAction(task: "T", owner: "", due: "")]))
        #expect(try decode(["kind": "verdict", "risk": "high", "reasons": ["x"], "raised": false])
            == .verdict(risk: "high", reasons: ["x"], raised: false))
        #expect(try decode(["kind": "choices", "items": ["A"], "then": "reply"]) == .choices(["A"], then: "reply"))
        #expect(throws: (any Error).self) { try decode(["kind": "verdict"]) }
        #expect(throws: (any Error).self) { try decode(["kind": "targets", "items": []]) }
    }

    @Test func theCursorFollowsTheText() {
        let old = "merci je vien jeudi\n\n> quote"
        let new = "Merci, je viens jeudi.\n\n> quote"
        // At the end of the words typed, which changed: at the end of the
        // new words, not in the middle of one.
        #expect(AI.cursorAfter(old: old, new: new, cursor: 19) == 22)
        // In the quotation, after the change: the same distance to the end.
        #expect(AI.cursorAfter(old: old, new: new, cursor: old.count) == new.count)
        #expect(AI.cursorAfter(old: old, new: new, cursor: 21) == 24)
        // Before the change: where it was.
        #expect(AI.cursorAfter(old: "abc def", new: "abc deF", cursor: 2) == 2)
        // Right where words were added: after them.
        #expect(AI.cursorAfter(old: "merci", new: "merci.", cursor: 5) == 6)
        #expect(AI.cursorAfter(old: "abc", new: "abcd", cursor: 0) == 0)
        // Nothing changed, an empty text, a cursor out of range.
        #expect(AI.cursorAfter(old: "same", new: "same", cursor: 2) == 2)
        #expect(AI.cursorAfter(old: "", new: "Hello", cursor: 0) == 5)
        #expect(AI.cursorAfter(old: "ab", new: "abc", cursor: 9) == 3)
    }

    @Test func errorsAreSaidInTheAppsLanguage() {
        #expect(AI.errorFlash(CoreClient.RPCError(code: AIError.disabled, message: "x"))
            == L("The AI is off: set ai.enabled = true in config.toml"))
        #expect(AI.errorFlash(CoreClient.RPCError(code: ComposeError.revisionConflict, message: "x"))
            == L("The reply changed since the AI read it: run it again"))
        #expect(AI.errorFlash(CoreClient.RPCError(code: AIError.ai, message: "not signed in")) == L("AI: %@", "not signed in"))
    }
}

/// The AI menu and panel through the keys (`GeekController`), against a
/// fake core: ⌘I in a reply, `a` in the list, a run from start to answer.
/// What the fake core answers to the AI methods, set by each test. The
/// fake core calls its handler on any thread: the state is behind a lock.
final class AIFakeState: @unchecked Sendable {
    private let lock = NSLock()
    private var _enabled = true
    private var _proposal: [String: Any] = [
        "kind": "replace", "unchanged": false, "body": "Merci.\n\nOn Nov 7, Sender 1 wrote:\n> Hello",
        "segments": [["start": 0, "end": 5, "old": "merci", "new": "Merci.",
                      "diff": [["kind": "removed", "text": "merci"], ["kind": "added", "text": "Merci."]]]],
    ]
    private var _appliedBody = "Merci.\n\nOn Nov 7, Sender 1 wrote:\n> Hello"
    private var _appliedSubject = "Re: Subject 1"
    private var _lastPrompt = ""

    func locked<T>(_ f: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return f()
    }

    var enabled: Bool { get { locked { _enabled } } set { locked { _enabled = newValue } } }
    var proposal: [String: Any] { get { locked { _proposal } } set { locked { _proposal = newValue } } }
    var appliedBody: String { get { locked { _appliedBody } } set { locked { _appliedBody = newValue } } }
    var appliedSubject: String { get { locked { _appliedSubject } } set { locked { _appliedSubject = newValue } } }

    static let mailPrompts: [[String: Any]] = [
        ["id": "summarize-thread", "key": "r", "label": "Summarize", "scope": "thread", "output": "summary", "variants": [],
         "source": "builtin", "available": true],
        ["id": "quick-replies", "key": "p", "label": "Quick replies", "scope": "thread", "output": "choices", "variants": [],
         "source": "builtin", "available": true],
        ["id": "translate-message", "key": "t", "label": "Translate", "scope": "message", "output": "panel",
         "variants": [["key": "e", "label": "Into English"]], "source": "builtin", "available": true],
    ]

    static let prompts: [[String: Any]] = [
        ["id": "proofread", "key": "c", "label": "Proofread", "scope": "draft", "output": "replace", "variants": [],
         "source": "builtin", "available": true],
        ["id": "rewrite", "key": "r", "label": "Rewrite", "scope": "draft", "output": "replace",
         "variants": [["key": "c", "label": "Shorter"], ["key": "f", "label": "More formal"]], "source": "builtin", "available": true],
        ["id": "subject-ideas", "key": "o", "label": "Suggest subjects", "scope": "draft", "output": "subjects", "variants": [],
         "source": "builtin", "available": true],
        ["id": "write-from-notes", "key": "w", "label": "Write from my notes", "scope": "draft", "output": "replace", "variants": [],
         "source": "builtin", "available": false],
    ]

    func answer(_ method: String, _ params: [String: Any]) -> Any {
        switch method {
        case "ai_status": return ["enabled": enabled, "ready": enabled, "reason": NSNull()] as [String: Any]
        case "ai_list_prompts":
            switch params["screen"] as? String {
            case "compose": return Self.prompts
            case "reading": return Self.mailPrompts
            default: return [Self.prompts[3]]
            }
        case "ai_run":
            locked { _lastPrompt = params["prompt"] as? String ?? "" }
            return ["run_id": 7, "streamed": true] as [String: Any]
        case "ai_get_run":
            let p: [String: Any] = switch locked({ _lastPrompt }) {
            case "summarize-thread": ["kind": "summary", "summary": "Alice attend jeudi.",
                                      "actions": [["task": "Apporter les clés", "owner": "you", "due": "jeudi"]]]
            case "quick-replies": ["kind": "choices", "items": ["C'est noté !", "On peut décaler ?"], "then": "reply"]
            default: proposal
            }
            return ["state": "done", "proposal": p, "error": NSNull()] as [String: Any]
        case "ai_apply" where locked({ _lastPrompt }) == "quick-replies":
            let items = ["C'est noté !", "On peut décaler ?"]
            return ["draft": NSNull(), "text": items[params["choice"] as? Int ?? 0]] as [String: Any]
        case "ai_apply":
            var d = Fixture.draft(replyTo: 10, body: appliedBody, revision: (params["revision"] as? Int ?? 1) + 1)
            d["subject"] = appliedSubject
            return ["draft": d]
        case "ai_dismiss", "ai_cancel": return ["changed": 0]
        default: return Fixture.answer(method, params)
        }
    }
}

/// The AI menu and panel through the keys (`GeekController`), against a
/// fake core: ⌘I in a reply, `a` in the list, a run from start to answer.
@MainActor
final class AIControllerTests {
    let ai = AIFakeState()
    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async {
        let state = ai
        core = FakeCore { method, params in state.answer(method, params) }
        let defaults = TestDefaults.make()
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        model.openFile = { _ in }
        model.addressBook = FixtureAddressBook(cards: [:])
        await model.attach(core)
        _ = await eventually { self.model.threads.count == 8 && self.model.openThread != nil }
    }

    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        var mods: NSEvent.ModifierFlags = []
        switch name {
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        case "ret": chars = "\r"; code = KeyCode.returnKey
        default:
            if name.hasPrefix("cmd-") { mods = .command; chars = String(name.dropFirst(4)) }
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        _ = controller.handle(event)
    }

    func type(_ text: String) {
        for c in text { press(String(c)) }
    }

    /// A reply with `text` typed above the quotation.
    func reply(_ text: String = "merci") async {
        press("r")
        _ = await eventually { self.model.composer != nil }
        type(text)
    }

    /// The AI menu open.
    var aiMenu: GeekController.AIMenu? {
        if case .choice(.ai(let m)) = controller.overlay { return m }
        return nil
    }

    var seq = 0
    /// An event of the core, as the socket would give it.
    func event(_ method: String, _ params: [String: JSONValue], transient: Bool = false) {
        if !transient { seq += 1 }
        model.handle(CoreClient.Event(method: method, epoch: "e_test", seq: transient ? nil : seq, params: params))
    }

    @Test func cmdIOpensTheMenuOfTheReplyAndItsVariants() async throws {
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        let rows = controller.choiceEntries(.ai(try #require(aiMenu))).map { "\($0.key) \($0.label)" }
        #expect(rows == ["c Proofread", "r Rewrite…", "o Suggest subjects", "w Write from my notes", "esc " + L("cancel")])
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose")
        // ⌘I was not typed.
        #expect(model.composer?.buffer.text.hasPrefix("merci\n") == true)
        // A prompt not available yet says so, and starts nothing.
        press("w")
        #expect(model.flash == L("Not available yet: %@", "Write from my notes"))
        #expect(core.calls("ai_run").isEmpty)
        // Variants: a second menu.
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("r")
        #expect(aiMenu?.parent?.id == "rewrite")
        #expect(controller.choiceTitle(.ai(try #require(aiMenu))) == "Rewrite")
        press("esc")
        #expect(aiMenu == nil)
        #expect(core.calls("ai_run").isEmpty)
    }

    @Test func aProofreadingRunsIsShownAndAccepted() async throws {
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // The text was saved first, and the run asked on that revision.
        let saved = try #require(core.calls("create_draft").last ?? core.calls("update_draft").last)
        #expect((saved["body"] as? String)?.hasPrefix("merci") == true)
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "proofread")
        #expect(run["draft_id"] as? Int == model.composer?.draft.id)
        #expect(run["revision"] as? Int == model.composer?.revision)
        #expect(run["lang"] as? String == AI.language)
        #expect(model.aiRun?.title == "Proofread")

        // The text as it comes; the keys do not reach the text meanwhile.
        event("AiDelta", ["run_id": .int(7), "text": .string("Mer")], transient: true)
        event("AiDelta", ["run_id": .int(7), "text": .string("ci.")], transient: true)
        #expect(model.aiRun?.text == "Merci.")
        type("zz")
        #expect(model.composer?.buffer.text.hasPrefix("merci\n") == true)
        // The answer to a key goes through a task: the message comes a
        // moment later.
        #expect(await eventually { self.model.flash == L("esc stop") })

        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        let apply = try #require(core.calls("ai_apply").last)
        #expect(apply["run_id"] as? Int == 7)
        #expect(model.composer?.buffer.text == ai.appliedBody)
        #expect(model.composer?.isDirty == false, "what the core saved is the saved text")
        #expect(model.flash == L("Applied · ⌘Z undoes it"))

        // ⌘Z gives the text back, and saves it.
        press("cmd-z")
        #expect(model.composer?.buffer.text.hasPrefix("merci\n") == true)
        #expect(model.composer?.isDirty == true)
        #expect(model.flash == L("AI change undone"))
        // Once: a second ⌘Z is not the AI's.
        #expect(model.undoAI() == false)
    }

    @Test func escapeRefusesAProposalAndStopsARun() async throws {
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // Running: Escape cancels.
        press("esc")
        _ = await eventually { self.core.calls("ai_cancel").count == 1 }
        #expect(model.aiRun == nil)
        #expect(model.flash == L("AI stopped"))
        // The AiFailed of the cancel that follows changes nothing.
        event("AiFailed", ["run_id": .int(7), "code": .string("ai_cancelled"), "message": .string("cancelled")])
        #expect(model.aiRun == nil)

        // Again, up to the proposal, then refused.
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("n")
        _ = await eventually { self.core.calls("ai_dismiss").count == 1 }
        #expect(model.aiRun == nil)
        #expect(core.calls("ai_apply").isEmpty)
        #expect(model.composer?.buffer.text.hasPrefix("merci\n") == true, "\(model.composer?.buffer.text ?? "")")
    }

    @Test func aFailureIsShownThenClosed() async {
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiFailed", ["run_id": .int(7), "code": .string("ai_not_logged_in"), "message": .string("run claude to sign in")])
        #expect(model.aiRun?.phase == .failed("run claude to sign in"))
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_dismiss").isEmpty, "nothing to dismiss after a failure")
    }

    @Test func aSubjectIsChosenByItsNumber() async throws {
        ai.proposal = ["kind": "subjects", "items": ["Jeudi 14h", "Confirmation pour jeudi"]]
        ai.appliedBody = "merci\n\nOn Nov 7, Sender 1 wrote:\n> Hello"
        ai.appliedSubject = "Confirmation pour jeudi"
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("o")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("2")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        #expect(model.composer?.headers.subject == "Confirmation pour jeudi")
        #expect(model.composer?.subjectLine.text == "Confirmation pour jeudi")
        #expect(model.composer?.isDirty == false)
        #expect(model.composer?.aiUndo == nil, "a subject is not undone by ⌘Z in the text")
    }

    @Test func anEndThatComesBeforeTheRunIdIsNotLost() async {
        // The core ends the run before `ai_run` has answered.
        core.delays["ai_run"] = .milliseconds(200)
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun != nil }
        #expect(model.aiRun?.runId == 0, "the panel is open, the id not known yet")
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        #expect(model.aiRun?.runId == 7)
    }

    @Test func anEndAfterAGapIsNotLost() async {
        await reply()
        press("cmd-i")
        _ = await eventually { self.aiMenu != nil }
        press("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // A numbered event out of order: a gap, which reloads everything.
        model.handle(CoreClient.Event(method: "AiDone", epoch: "e_test", seq: 99, params: ["run_id": .int(7)]))
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
    }

    @Test func theAIOffSaysHowToTurnItOn() async {
        ai.enabled = false
        await reply()
        press("cmd-i")
        _ = await eventually { self.model.flash == L("The AI is off: set ai.enabled = true in config.toml") }
        #expect(aiMenu == nil)
        #expect(core.calls("ai_list_prompts").isEmpty)
    }

    @Test func aSummaryOfTheMailReadIsShownThenClosed() async throws {
        // Two columns: the mail is shown, `a` opens the reading menu.
        press("a")
        _ = await eventually { self.aiMenu != nil }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "reading")
        press("r")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["thread_id"] as? Int == model.openThread?.id)
        #expect(run["message_id"] == nil, "a thread prompt names no mail")
        #expect(run["draft_id"] == nil)
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        #expect(AI.resultText(try #require({ if case .proposal(let p)? = model.aiRun?.phase { p } else { nil } }()))
            == "Alice attend jeudi.\n- Apporter les clés (you) · jeudi")
        // The panel takes the keys: `j` does not move the list.
        let selected = model.selectedThreadId
        press("j")
        #expect(model.selectedThreadId == selected)
        #expect(await eventually { self.model.flash == L("esc close") })
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_dismiss").count == 1)
        // The keys are the list's again.
        press("j")
        #expect(model.selectedThreadId != selected)
    }

    @Test func aMessagePromptNamesTheMailUnderTheCursor() async throws {
        press("a")
        _ = await eventually { self.aiMenu != nil }
        press("t")
        #expect(aiMenu?.parent?.id == "translate-message")
        press("e")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["variant"] as? String == "e")
        #expect(run["thread_id"] as? Int == model.openThread?.id)
    }

    @Test func aQuickReplyStartsTheReplyWithIt() async {
        press("a")
        _ = await eventually { self.aiMenu != nil }
        press("p")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("2")
        _ = await eventually { self.model.composer != nil }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        #expect(core.calls("ai_apply").last?["revision"] == nil, "no draft yet")
        #expect(model.aiRun == nil)
        // The reply to the mail, the text above the quotation, the cursor
        // after it, to be saved as if typed.
        #expect(core.calls("prepare_draft").count == 1)
        #expect(model.composer?.buffer.text.hasPrefix("On peut décaler ?\n\nOn Nov 7") == true, "\(model.composer?.buffer.text ?? "")")
        #expect(model.composer?.buffer.cursor == "On peut décaler ?".count)
        #expect(model.composer?.isDirty == true)
    }

    @Test func aInTheListOpensTheMenuOfTheMails() async {
        // One column, on the list: the list's menu.
        controller.columns = 1
        press("a")
        _ = await eventually { self.aiMenu != nil }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "list")
        #expect(core.calls("ai_list_prompts").last?["folder"] as? String == "inbox")
        press("w")
        #expect(model.flash == L("Not available yet: %@", "Write from my notes"))
    }
}
