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
        #expect(act(.proposal(subjects), esc: true) == .refuse)
        // Return takes the one highlighted: the first by default.
        #expect(act(.proposal(subjects), ret: true) == .choose(0))
        #expect(AI.action(.proposal(subjects), key: "", isReturn: true, isEscape: false, choice: 2) == .choose(2))
        #expect(AI.hint(.proposal(subjects)) == L("%@ or 1–%lld choose · %@ move · esc or %@ refuse", "↩", 3, "↑↓", "n") + " · " + L("%@ copy", "c"))
        // A failure: Escape or Return close it.
        #expect(act(.failed("x"), esc: true) == .close)
        #expect(act(.failed("x"), "n") == .hint)
    }

    @Test func anAppointmentIsSavedWithReturnAndClosedWithEscape() {
        func act(_ p: AIProposal, _ key: String = "", ret: Bool = false, esc: Bool = false, arrow: Int = 0, choice: Int = 0) -> AIKeyAction {
            AI.action(.proposal(p), key: key, isReturn: ret, isEscape: esc, arrow: arrow, choice: choice)
        }
        let e = AIEvent(title: "Visite", start: "2020-06-25T14:00", end: "", location: "", details: "")
        let two = AIProposal.events([e, e])
        #expect(act(two, ret: true) == .choose(0), "the first, highlighted")
        #expect(act(two, ret: true, choice: 1) == .choose(1))
        #expect(AI.action(.proposal(two), key: "", isReturn: true, isEscape: false, choice: 1, isCommand: true) == .open(1))
        // ⌘↩ elsewhere is Return as before.
        let subjects = AIProposal.subjects(["A"])
        #expect(AI.action(.proposal(subjects), key: "", isReturn: true, isEscape: false, isCommand: true) == .choose(0))
        #expect(act(two, arrow: 1) == .move(1))
        #expect(act(two, arrow: 1, choice: 1) == .move(1))
        #expect(act(two, "2") == .choose(1))
        #expect(act(two, "3") == .hint)
        #expect(act(two, esc: true) == .refuse)
        #expect(act(two, "n") == .hint, "nothing to turn down: Escape closes")
        #expect(AI.hint(.proposal(two)) == L("%@ or 1–%lld save (.ics) · %@ add to the calendar · %@ move · esc close", "↩", 2, "⌘↩", "↑↓") + " · " + L("%@ copy", "c"))
        // A mail without a date: only read.
        #expect(act(.events([]), ret: true) == .close)
        #expect(act(.events([]), esc: true) == .close)
        #expect(act(.events([]), arrow: 1) == .hint)
        #expect(AI.hint(.proposal(.events([]))) == L("esc close"))
        #expect(AI.resultText(.events([])) == L("No date or appointment in this mail."))
    }

    @Test func aContactCardIsSavedWithReturnAndOpenedWithCommandReturn() {
        let card = AIProposal.contact(AIContact(
            name: "Alice Martin", given: "Alice", family: "Martin", organization: "Acme", title: "", emails: ["alice@acme.fr"],
            phones: [], address: AIAddress(street: "", locality: "", postcode: "", region: "", country: ""), urls: []))
        func act(_ key: String = "", ret: Bool = false, esc: Bool = false, cmd: Bool = false, arrow: Int = 0) -> AIKeyAction {
            AI.action(.proposal(card), key: key, isReturn: ret, isEscape: esc, arrow: arrow, isCommand: cmd)
        }
        #expect(act(ret: true) == .accept)
        #expect(act(ret: true, cmd: true) == .open(0))
        #expect(act(esc: true) == .refuse)
        #expect(act(arrow: 1) == .hint, "one card: nothing to move")
        #expect(act("1") == .hint)
        #expect(act("c") == .copy)
        #expect(AI.hint(.proposal(card)) == L("%@ save (.vcf) · %@ add to Contacts · esc close", "↩", "⌘↩") + " · " + L("%@ copy", "c"))
        #expect(AI.copyText(card) == "Alice Martin\nAcme\nalice@acme.fr")
    }

    @Test func aContactCardReadsAsInTheAddressBook() {
        let c = AIContact(
            name: "", given: "Alice", family: "Martin", organization: "Acme", title: "Directrice",
            emails: ["alice@acme.fr", "a@gmail.com"],
            phones: [AIPhone(kind: "mobile", number: "+33 6 12 34 56 78"), AIPhone(kind: "fax", number: "01 23 45 67 89"),
                     AIPhone(kind: "pager", number: "0123")],
            address: AIAddress(street: "12 rue de la Paix\nBâtiment B", locality: "Paris", postcode: "75002", region: "", country: "France"),
            urls: ["https://acme.fr"])
        // The name from its parts when the whole is not given; the job and
        // the company on one line; an unknown kind labelled as a phone.
        #expect(AI.contactLines(c) == [
            "Alice Martin", "Directrice · Acme", "alice@acme.fr", "a@gmail.com",
            L("Mobile") + " +33 6 12 34 56 78", L("Fax") + " 01 23 45 67 89", L("Phone") + " 0123",
            "12 rue de la Paix, Bâtiment B, 75002 Paris, France", "https://acme.fr",
        ])
        // Only an address: one line, nothing empty shown.
        let bare = AIContact(name: "", given: "", family: "", organization: "", title: "", emails: ["x@y.fr"], phones: [],
                             address: AIAddress(street: "", locality: "", postcode: "", region: "", country: ""), urls: [])
        #expect(AI.contactLines(bare) == ["x@y.fr"])
        #expect(AI.phoneLabel("work") == L("Work") && AI.phoneLabel("home") == L("Home"))
        // What the core left out: said under the card (and to the UI
        // scripts), not copied with it.
        var odd = bare
        odd.dropped = ["www.acme.fr", "06 12 (direct)"]
        #expect(AI.droppedLine(bare) == nil)
        #expect(AI.droppedLine(odd) == L("Left out, not valid: %@", "www.acme.fr, 06 12 (direct)"))
        #expect(AI.resultText(.contact(odd)) == "x@y.fr\n" + AI.droppedLine(odd)!)
        #expect(AI.copyText(.contact(odd)) == "x@y.fr")
    }

    @Test func anAppointmentSaysWhenInTheUsersLanguage() {
        let fr = Locale(identifier: "fr_FR")
        func when(_ start: String, _ end: String = "", in locale: Locale) -> String {
            AI.eventWhen(AIEvent(title: "t", start: start, end: end, location: "", details: ""), locale: locale)
        }
        #expect(when("2020-06-25T14:00", in: fr) == "jeudi 25 juin 2020 · 14:00")
        #expect(when("2020-06-25T14:00", "2020-06-25T15:30", in: fr) == "jeudi 25 juin 2020 · 14:00–15:30")
        #expect(when("2020-06-25T22:00", "2020-06-26T02:00", in: fr) == "jeudi 25 juin 2020 · 22:00 – vendredi 26 juin 2020 · 02:00")
        #expect(when("2020-06-30", in: fr) == "mardi 30 juin 2020")
        #expect(when("2026-11-28", "2026-11-30", in: fr) == "samedi 28 novembre 2026 – lundi 30 novembre 2026")
        // The day of a summer date stays the day the core gave, whatever
        // the zone of the Mac (floating dates, read in UTC).
        #expect(when("2026-03-29T02:30", in: fr) == "dimanche 29 mars 2026 · 02:30")
        let en = when("2020-06-25T14:00", in: Locale(identifier: "en_US"))
        #expect(en.contains("Thursday") && en.contains("June 25") && en.contains("2:00"), "\(en)")
        // A date the app cannot read is shown as given.
        #expect(when("jeudi", in: fr) == "jeudi")
        let e = AIEvent(title: "Visite", start: "2020-06-25T14:00", end: "", location: "12 rue de la Paix", details: "Le 25.")
        #expect(AI.resultText(.events([e])) == "1. " + AI.eventWhen(e) + " · Visite · 12 rue de la Paix · Le 25.")
    }

    @Test func theArrowsMoveTheTextReturnTakesWithinTheList() {
        func act(_ p: AIProposal, arrow: Int, choice: Int) -> AIKeyAction {
            AI.action(.proposal(p), key: "", isReturn: false, isEscape: false, arrow: arrow, choice: choice)
        }
        // Every list: other wordings, subjects, quick replies.
        for p in [AIProposal.choices(["a", "b", "c"], then: "replace"), .subjects(["a", "b", "c"]),
                  .choices(["a", "b", "c"], then: "reply")] {
            #expect(act(p, arrow: 1, choice: 0) == .move(1))
            #expect(act(p, arrow: -1, choice: 2) == .move(1))
            // Kept within the list: no wrap, no index out of it.
            #expect(act(p, arrow: -1, choice: 0) == .move(0))
            #expect(act(p, arrow: 1, choice: 2) == .move(2))
        }
        // No list: the arrows mean nothing.
        #expect(act(.text("x"), arrow: 1, choice: 0) == .hint)
        #expect(AI.action(.proposal(replace), key: "", isReturn: false, isEscape: false, arrow: 1) == .hint)
    }

    @Test func theProposalsOfAMailAreReadOrChosen() {
        func act(_ p: AIProposal, _ key: String = "", ret: Bool = false, esc: Bool = false) -> AIKeyAction {
            AI.action(.proposal(p), key: key, isReturn: ret, isEscape: esc)
        }
        for p in [AIProposal.text("x"), .summary("s", actions: []), .verdict(risk: "low", reasons: ["r"], raised: false)] {
            #expect(act(p, ret: true) == .close)
            #expect(act(p, esc: true) == .close)
            #expect(act(p, "y") == .hint, "nothing to accept in \(p)")
            #expect(AI.hint(.proposal(p)) == L("esc close") + " · " + L("%@ copy", "c"))
        }
        let replies = AIProposal.choices(["Oui", "Non"], then: "reply")
        #expect(act(replies, "1") == .choose(0))
        #expect(act(replies, "3") == .hint)
        #expect(act(replies, ret: true) == .choose(0), "the first, highlighted")
        #expect(act(replies, esc: true) == .refuse)
        #expect(AI.hint(.proposal(replies)) == L("%@ or 1–%lld reply with it · %@ move · esc or %@ refuse", "↩", 2, "↑↓", "n") + " · " + L("%@ copy", "c"))
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
        let fresh = AI.insertReply("Oui.", into: "\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(fresh.body == "Oui.\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(fresh.cursor == 4)
        let started = AI.insertReply("Oui.", into: "merci\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(started.body == "Oui.\n\nmerci\n\nOn Nov 7, A wrote:\n> Hi")
        #expect(started.cursor == 4)
        // The cursor in UTF-16 units, the editor's.
        #expect(AI.insertReply("Oui 🎉", into: "x").cursor == 6)
    }

    @Test func theScreenOfThePaletteFollowsItsTarget() {
        let screen = GeekController.aiScreen
        #expect(screen(true, 0) == "compose")
        #expect(screen(true, 5) == "compose", "the reply written, whatever is marked")
        #expect(screen(false, 0) == "mail", "the mail shown or under the cursor")
        #expect(screen(false, 1) == "mail", "one mail marked is one mail")
        #expect(screen(false, 2) == "selection")
        // The conversation of a prompt of one mail.
        let thread = GeekController.aiThread
        #expect(thread([7], 3, 4) == 7, "the one marked alone first")
        #expect(thread([], 3, 4) == 3, "then the one shown")
        #expect(thread([], nil, 4) == 4, "then the row under the cursor")
        #expect(thread([7, 8], 3, 4) == 3)
        #expect(thread([], nil, nil) == nil)
    }

    @Test @MainActor func theSuggestedTargetIsCutShortInANarrowList() {
        #expect(AI.suggestionTag("Newsletter", narrow: false) == "→ Newsletter")
        #expect(AI.suggestionTag("Newsletter", narrow: true) == "→ Newsl…")
        #expect(AI.suggestionTag("Paperwork", narrow: true) == "→ Paper…")
        #expect(AI.suggestionTag("Banned", narrow: true) == "→ Banned", "six letters stay whole")
        #expect(AI.suggestionTag("Inbox", narrow: true) == "→ Inbox")
        // Two columns are narrow, a wide list alone is not.
        #expect(RowLayout(width: 640, scale: 1).narrowSuggestion)
        #expect(!RowLayout(width: 1800, scale: 1).narrowSuggestion)
    }

    @Test func theStatusBarSaysTheSortAndTheSuggestion() {
        #expect(AI.sortLine(AISort(runId: 1, title: "Sort", done: 20, total: 47)) == L("%@: %lld of %lld sorted · esc stop", "Sort", 20, 47))
        #expect(AI.sortLine(AISort(runId: 0, title: "Sort")) == L("%@: sorting… · esc stop", "Sort"))
        let s = AISuggestion(runId: 1, threadId: 3, target: "newsletter", reason: "A shop's offers.")
        #expect(AI.suggestionLine(s) == L("AI: %@ (%@) · %@", Role.name("newsletter"), "↩", "A shop's offers."))
        #expect(AI.sortedFlash(found: 5, failedBatches: 0) == L("AI: %lld suggestions · %@ on a row moves it there", 5, "↩"))
        #expect(AI.sortedFlash(found: 5, failedBatches: 2) == L("AI: %lld suggestions, %lld batches without an answer · %@ on a row moves it there", 5, 2, "↩"))
    }

    @Test func cCopiesTheWholeResultAndKeepsThePanelOpen() {
        let summary = AIProposal.summary("Alice attend jeudi.", actions: [AIAction(task: "Réserver", owner: "", due: "")])
        func act(_ p: AIProposal, _ key: String, command: Bool = false) -> AIKeyAction {
            AI.action(.proposal(p), key: key, isReturn: false, isEscape: false, isCommand: command)
        }
        // Every result with a text answers `c`, before the keys of its kind.
        #expect(act(summary, "c") == .copy)
        #expect(act(.text("Hello"), "c") == .copy)
        #expect(act(.verdict(risk: "low", reasons: [], raised: false), "c") == .copy)
        #expect(act(.subjects(["A"]), "c") == .copy)
        #expect(act(.choices(["Oui"], then: "reply"), "c") == .copy)
        #expect(act(replace, "c") == .copy)
        let e = AIEvent(title: "Visite", start: "2020-06-25T14:00", end: "", location: "", details: "")
        #expect(act(.events([e]), "c") == .copy)
        // Nothing to copy: no change, no date, the targets of a sort.
        #expect(act(.replace(segments: [], unchanged: true), "c") == .hint)
        #expect(act(.events([]), "c") == .hint)
        #expect(act(.targets([], failedBatches: 0), "c") == .hint)
        // Not while it runs or after a failure; not ⌘C, not `C`.
        #expect(AI.action(.running, key: "c", isReturn: false, isEscape: false) == .hint)
        #expect(AI.action(.failed("x"), key: "c", isReturn: false, isEscape: false) == .hint)
        #expect(act(summary, "c", command: true) == .hint)
        #expect(act(summary, "C") == .hint)

        // What is copied: the text as read, a replacement's new text (not
        // its differences), the segments a blank line apart.
        #expect(AI.copyText(summary) == "Alice attend jeudi.\n- Réserver")
        #expect(AI.copyText(replace) == "the")
        let two = AIProposal.replace(segments: [
            AISegmentChange(old: "a", new: "A.", diff: []), AISegmentChange(old: "b", new: "B.", diff: []),
        ], unchanged: false)
        #expect(AI.copyText(two) == "A.\n\nB.")
        #expect(AI.copyText(.subjects(["Jeudi", "Vendredi"])) == "1. Jeudi\n2. Vendredi")
        #expect(AI.copyText(.events([])) == nil)
        // The hint names `c` only where it copies.
        #expect(AI.hint(.proposal(summary)) == L("esc close") + " · " + L("%@ copy", "c"))
        #expect(AI.hint(.proposal(.events([]))) == L("esc close"))
    }

    @MainActor
    @Test func theAIBlockOfTheStatusBarRemindsOfItsKey() {
        let defaults = TestDefaults.make()
        let controller = GeekController(model: AppModel(), defaults: defaults, textSize: TextSize(defaults: defaults))
        #expect(controller.aiKeyLabel(composing: false) == "a")
        #expect(controller.aiKeyLabel(composing: true) == "⌃I")
    }

    @Test func theHintSaysTheKeys() {
        #expect(AI.hint(.running) == L("esc stop"))
        #expect(AI.hint(.proposal(.subjects(["A", "B"]))) == L("%@ or 1–%lld choose · %@ move · esc or %@ refuse", "↩", 2, "↑↓", "n") + " · " + L("%@ copy", "c"))
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

    /// The sentence comes from the reason, never from the core's English
    /// message; the provider's own text is cited.
    @Test func errorsAreSaidInTheAppsLanguage() {
        #expect(AI.errorFlash(CoreClient.RPCError(code: AIError.disabled, message: "x", data: ["reason": "ai_disabled"]))
            == L("The AI is off: set ai.enabled = true in config.toml"))
        #expect(AI.errorFlash(CoreClient.RPCError(code: ComposeError.revisionConflict, message: "x", data: ["reason": "ai_draft_changed"]))
            == L("The reply changed since the AI read it: run it again"))
        #expect(AI.errorFlash(CoreClient.RPCError(code: AIError.ai, message: "x", data: ["reason": "ai_not_logged_in", "code": "ai_not_logged_in"]))
            == L("Claude Code is not signed in: run claude in a terminal to sign in"))
        #expect(AI.errorFlash(CoreClient.RPCError(code: AIError.ai, message: "x", data: ["reason": "ai_failed", "provider_text": "overloaded"]))
            == L("The AI failed: %@", "overloaded"))
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
    private var _suggest = false
    private var _passageText: String?
    /// `ai_apply` answers a text for the passage of a formatted reply
    /// (`DraftText::HtmlPassage` in the core) instead of a draft.
    var passageText: String? { get { locked { _passageText } } set { locked { _passageText = newValue } } }
    /// `ai_apply` answers a formatted draft (`body_format: html`).
    var appliedFormat = "text"
    /// `ai_suggestions` suggests Archive for the first conversation asked.
    var suggest: Bool { get { locked { _suggest } } set { locked { _suggest = newValue } } }

    static let sortPrompt: [String: Any] = [
        "id": "suggest-folder", "key": "s", "label": "Suggest a folder", "scope": "selection", "output": "targets",
        "variants": [], "source": "builtin", "available": true, "unmarked": "cursor",
    ]

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
        ["id": "dates", "key": "d", "label": "Dates and appointments", "scope": "message", "output": "events", "variants": [],
         "source": "builtin", "available": true],
        ["id": "contact", "key": "v", "label": "Contact card", "scope": "message", "output": "contact", "variants": [],
         "source": "builtin", "available": true],
    ]

    /// The prompts of an attachment.
    static let attachmentPrompts: [[String: Any]] = [
        ["id": "summarize-attachment", "key": "r", "label": "Summarize", "scope": "attachment", "output": "panel",
         "variants": [], "source": "builtin", "available": true],
    ]

    /// The sender's card of `contact`.
    static let card: [String: Any] = [
        "name": "Alice Martin", "given": "Alice", "family": "Martin", "organization": "Agence A", "title": "",
        "emails": ["alice@a.example"], "phones": [["kind": "mobile", "number": "+33 6 12 34 56 78"]],
        "address": ["street": "", "locality": "Paris", "postcode": "", "region": "", "country": ""], "urls": [],
        "dropped": [],
    ]

    /// The appointments of `dates`.
    static let events: [[String: Any]] = [
        ["title": "Visite de l'appartement", "start": "2020-06-25T14:00", "end": "", "location": "12 rue de la Paix",
         "details": "Jeudi : le 25."],
        ["title": "Remise des clés", "start": "2020-06-30", "end": "", "location": "", "details": ""],
    ]

    /// The palette of a passage selected in the reply.
    static let passagePrompts: [[String: Any]] = [
        ["id": "alternatives", "key": "a", "label": "Other wordings", "scope": "draft", "output": "choices",
         "variants": [], "source": "builtin", "available": true],
        ["id": "proofread", "key": "c", "label": "Proofread", "scope": "draft", "output": "replace", "variants": [],
         "source": "builtin", "available": true],
        ["id": "improve", "key": "i", "label": "Improve writing", "scope": "draft", "output": "replace", "variants": [],
         "source": "builtin", "available": true],
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
            case "compose-selection": return Self.passagePrompts
            case "mail": return Self.mailPrompts
            case "attachment": return Self.attachmentPrompts
            default: return [Self.sortPrompt, Self.prompts[3]]
            }
        case "ai_suggestions":
            let ids = params["thread_ids"] as? [Int] ?? []
            return locked({ _suggest }) && !ids.isEmpty
                ? [["run_id": 7, "thread_id": ids[0], "target": "archive", "reason": "Done with it."]] as [[String: Any]]
                : [[String: Any]]()
        case "ai_run":
            locked { _lastPrompt = params["prompt"] as? String ?? "" }
            return ["run_id": 7, "streamed": true] as [String: Any]
        case "ai_get_run":
            let p: [String: Any] = switch locked({ _lastPrompt }) {
            case "summarize-thread": ["kind": "summary", "summary": "Alice attend jeudi.",
                                      "actions": [["task": "Apporter les clés", "owner": "you", "due": "jeudi"]]]
            case "quick-replies": ["kind": "choices", "items": ["C'est noté !", "On peut décaler ?"], "then": "reply"]
            case "alternatives": ["kind": "choices", "items": ["viens", "passe"], "then": "replace"]
            case "dates": ["kind": "events", "items": Self.events]
            case "contact": ["kind": "contact", "card": Self.card]
            case "summarize-attachment": ["kind": "text", "text": "- Un devis de toiture, 4 800 €."]
            default: proposal
            }
            return ["state": "done", "proposal": p, "error": NSNull()] as [String: Any]
        case "ai_apply" where locked({ _lastPrompt }) == "dates":
            let e = Self.events[params["choice"] as? Int ?? 0]
            let name = (e["start"] as! String).prefix(10) + " " + (e["title"] as! String) + ".ics"
            return ["draft": NSNull(), "text": NSNull(),
                    "calendar": ["filename": name, "content": "BEGIN:VCALENDAR\r\nSUMMARY:\(e["title"]!)\r\nEND:VCALENDAR\r\n"]] as [String: Any]
        case "ai_apply" where locked({ _lastPrompt }) == "contact":
            return ["draft": NSNull(), "text": NSNull(),
                    "contact": ["filename": "Alice Martin.vcf", "content": "BEGIN:VCARD\r\nFN:Alice Martin\r\nEND:VCARD\r\n"]] as [String: Any]
        case "ai_apply" where locked({ _lastPrompt }) == "quick-replies":
            let items = ["C'est noté !", "On peut décaler ?"]
            return ["draft": NSNull(), "text": items[params["choice"] as? Int ?? 0]] as [String: Any]
        case "ai_apply" where passageText != nil:
            return ["draft": NSNull(), "text": passageText!] as [String: Any]
        case "ai_apply":
            var d = Fixture.draft(replyTo: 10, body: appliedBody, revision: (params["revision"] as? Int ?? 1) + 1)
            d["subject"] = appliedSubject
            d["body_format"] = appliedFormat
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
    let editor: FakeEditor

    init() async {
        let state = ai
        core = FakeCore { method, params in state.answer(method, params) }
        let defaults = TestDefaults.make()
        editor = FakeEditor(model: model)
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
        case "cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = .command
        case "up": chars = "\u{F700}"; code = KeyCode.up; mods = .function
        case "down": chars = "\u{F701}"; code = KeyCode.down; mods = .function
        default:
            if name.hasPrefix("cmd-") { mods = .command; chars = String(name.dropFirst(4)) }
            if name.hasPrefix("ctrl-") { mods = .control; chars = String(name.dropFirst(5)) }
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        // What the app lets through while the text has the keys is the
        // editor's (`FakeEditor`).
        if !controller.handle(event), model.composer?.focus == .body, controller.overlay == .none, model.aiRun == nil,
           mods.intersection([.command, .control]).isEmpty {
            editor.key(chars)
        }
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

    /// The AI palette open (`a`, ⌘I).
    var paletteOpen: Bool { controller.overlay == .picker(.ai) }

    /// The rows of the AI palette, "key label".
    var paletteRows: [String] { controller.pickerEntries(.ai).map { "\($0.key) \($0.label)" } }

    /// Types `letters` in the open palette, then Return: the first row left.
    func choose(_ letters: String) {
        type(letters)
        press("ret")
    }

    var seq = 0
    /// An event of the core, as the socket would give it.
    func event(_ method: String, _ params: [String: JSONValue], transient: Bool = false) {
        if !transient { seq += 1 }
        model.handle(CoreClient.Event(method: method, epoch: "e_test", seq: transient ? nil : seq, params: params))
    }

    @Test func cmdIOpensThePaletteOfTheReplyWithItsVariantsAsRows() async throws {
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(controller.aiPlacement == "palette")
        // The most used first; a variant is a row of its own.
        #expect(paletteRows == ["c Proofread", "rc Rewrite · Shorter", "rf Rewrite · More formal", "o Suggest subjects",
                                "w Write from my notes"])
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose")
        // ⌘I was not typed; a letter typed filters, it is not the text's.
        #expect(model.composer?.body.hasPrefix("merci\n") == true)
        type("formal")
        #expect(paletteRows.first == "rf Rewrite · More formal")
        #expect(model.composer?.body.hasPrefix("merci\n") == true)
        press("esc")
        #expect(!paletteOpen)
        // A prompt not available yet says so, and starts nothing.
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("w")
        #expect(model.flash == L("Not available yet: %@", "Write from my notes"))
        #expect(core.calls("ai_run").isEmpty)
    }

    @Test func theFirstRowIsHighlightedSoReturnRunsIt() async throws {
        // ⌘I then Return: the first prompt of the reply, proofreading.
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(controller.pickerIndex == 0)
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        #expect(core.calls("ai_run").last?["prompt"] as? String == "proofread")
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        // A prompt's letters bring it first: `rc`, rewrite shorter.
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("rc")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "rewrite")
        #expect(run["variant"] as? String == "c")
    }

    @Test func aThenReturnRunsTheFirstPromptOfTheMailAndOfTheList() async throws {
        press("a")
        _ = await eventually { self.paletteOpen }
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        #expect(core.calls("ai_run").last?["prompt"] as? String == "summarize-thread")
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        // The list alone, nothing marked: the row under the cursor is the
        // mail, summarized without being opened.
        controller.columns = 1
        press("j")
        press("a")
        _ = await eventually { self.paletteOpen }
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "summarize-thread")
        #expect(run["thread_id"] as? Int == model.selectedThreadId)
        #expect(run["message_id"] == nil)
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        // Several marked: filing them.
        model.marked = Set(model.threads.prefix(2).map(\.id))
        press("a")
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "selection")
        press("ret")
        _ = await eventually { self.model.aiSort?.runId == 7 }
        #expect(core.calls("ai_run").last?["prompt"] as? String == "suggest-folder")
    }

    @Test func aProofreadingRunsIsShownAndAccepted() async throws {
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
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
        // In the palette's place, not above the text of the reply.
        #expect(controller.aiPlacement == "panel")

        // The text as it comes; the keys do not reach the text meanwhile.
        event("AiDelta", ["run_id": .int(7), "text": .string("Mer")], transient: true)
        event("AiDelta", ["run_id": .int(7), "text": .string("ci.")], transient: true)
        #expect(model.aiRun?.text == "Merci.")
        type("zz")
        #expect(model.composer?.body.hasPrefix("merci\n") == true)
        // The answer to a key goes through a task: the message comes a
        // moment later.
        #expect(await eventually { self.model.flash == L("esc stop") })

        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        let apply = try #require(core.calls("ai_apply").last)
        #expect(apply["run_id"] as? Int == 7)
        #expect(model.composer?.body == ai.appliedBody)
        #expect(model.composer?.isDirty == false, "what the core saved is the saved text")
        #expect(model.flash == L("Applied · ⌘Z undoes it"))
        // The change went to the editor as one range: its own ⌘Z undoes it.
        #expect(editor.content == ai.appliedBody)
        #expect(editor.ranges.count == 1)
    }

    @Test func aPassageSelectedGetsItsOwnPaletteAndItsChoiceReplacesIt() async throws {
        await reply("merci je vien jeudi")
        // Nothing selected: the palette of the whole text.
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose")
        press("esc")
        // "jeudi" selected: the palette of a passage, other wordings first.
        // (The editor says what is selected, in UTF-16 units of the text.)
        var selected = EditorStateView()
        selected.textRange = .init(start: 14, end: 19)
        selected.selection = "jeudi"
        selected.empty = false
        editor.post(selected)
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose-selection")
        #expect(paletteRows.first == "a Other wordings")
        // ⌘I ↩: the passage is sent, in UTF-16 units of the body.
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "alternatives")
        let selection = try #require(run["selection"] as? [String: Any])
        #expect(selection["start"] as? Int == 14 && selection["end"] as? Int == 19)
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        #expect(AI.hint(try #require(model.aiRun?.phase)) == L("%@ or 1–%lld replace the selection · %@ move · esc or %@ refuse · ⌘Z after choosing undoes", "↩", 2, "↑↓", "n") + " · " + L("%@ copy", "c"))
        // ↩ again takes the first, the best: the core puts it in the draft.
        let before = try #require(model.composer?.body)
        ai.appliedBody = before.replacingOccurrences(of: "jeudi", with: "viens")
        editor.post(EditorStateView())
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        let apply = try #require(core.calls("ai_apply").last)
        #expect(apply["choice"] as? Int == 0)
        #expect(apply["revision"] != nil)
        #expect(model.composer?.body == ai.appliedBody)
        // The cursor after the new word, in the editor.
        #expect(editor.cursor == "merci je vien viens".utf16.count)
        #expect(model.flash == L("Applied · ⌘Z undoes it"))
        // Nothing selected again: the next ⌘I works on the whole text.
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose")
        press("ret")
        _ = await eventually { self.core.calls("ai_run").count == 2 }
        #expect(core.calls("ai_run").last?["selection"] == nil)
    }

    /// A formatted reply (`edition.md`): the AI is offered; the core
    /// rewrote the user's blocks as HTML, which the editor takes whole, as
    /// one step of its undo.
    @Test func aFormattedReplyIsRewrittenAsHTMLByTheEditor() async throws {
        await reply("merci")
        await model.toggleComposeFormat()
        #expect(model.composer?.format == .html)
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        #expect(model.flash != L("The AI works on text only for now · ⇧⌘T for text only"))
        choose("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        ai.appliedBody = "<p>Merci.</p><p>On Nov 7, Sender 1 wrote:</p><blockquote><p>Hello</p></blockquote>"
        ai.appliedFormat = "html"
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        #expect(editor.contents == [ai.appliedBody])
        #expect(editor.ranges.isEmpty)
        #expect(model.composer?.body == ai.appliedBody)
        #expect(model.flash == L("Applied · ⌘Z undoes it"))
    }

    /// A passage of a formatted reply: the core gives the text back, and
    /// the editor puts it in place of the passage, measured in its text.
    @Test func aPassageOfAFormattedReplyIsReplacedByTheEditor() async throws {
        await reply("merci je vien jeudi")
        await model.toggleComposeFormat()
        var selected = EditorStateView()
        selected.textRange = .init(start: 14, end: 19)
        selected.selection = "jeudi"
        selected.empty = false
        editor.post(selected)
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let selection = try #require(core.calls("ai_run").last?["selection"] as? [String: Any])
        #expect(selection["start"] as? Int == 14 && selection["end"] as? Int == 19)
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        ai.passageText = "vendredi"
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        #expect(editor.ranges.last?.start == 14 && editor.ranges.last?.end == 19 && editor.ranges.last?.text == "vendredi")
        #expect(model.flash == L("Applied · ⌘Z undoes it"))
    }

    @Test func escapeRefusesAProposalAndStopsARun() async throws {
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
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
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("n")
        _ = await eventually { self.core.calls("ai_dismiss").count == 1 }
        #expect(model.aiRun == nil)
        #expect(core.calls("ai_apply").isEmpty)
        #expect(model.composer?.body.hasPrefix("merci\n") == true, "\(model.composer?.body ?? "")")
    }

    @Test func aFailureIsShownThenClosed() async {
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // Said from its reason, in the app's language.
        event("AiFailed", ["run_id": .int(7), "code": .string("ai_not_logged_in"), "message": .string("run claude to sign in"),
                           "why": .object(["reason": "ai_not_logged_in"])])
        #expect(model.aiRun?.phase == .failed(L("Claude Code is not signed in: run claude in a terminal to sign in")))
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_dismiss").isEmpty, "nothing to dismiss after a failure")
    }

    @Test func aSubjectIsChosenByItsNumber() async throws {
        ai.proposal = ["kind": "subjects", "items": ["Jeudi 14h", "Confirmation pour jeudi"]]
        ai.appliedBody = "merci\n\nOn Nov 7, Sender 1 wrote:\n> Hello"
        ai.appliedSubject = "Confirmation pour jeudi"
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("o")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("2")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        #expect(model.composer?.headers.subject == "Confirmation pour jeudi")
        #expect(model.composer?.subjectLine.text == "Confirmation pour jeudi")
        #expect(model.composer?.isDirty == false)
        #expect(editor.ranges.isEmpty, "a subject is not the text's change")
    }

    @Test func aSubjectIsChosenWithTheArrowsAndReturn() async throws {
        ai.proposal = ["kind": "subjects", "items": ["Jeudi 14h", "Confirmation pour jeudi", "Jeudi"]]
        ai.appliedBody = "merci\n\nOn Nov 7, Sender 1 wrote:\n> Hello"
        ai.appliedSubject = "Confirmation pour jeudi"
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("o")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        // The first highlighted; ↓ ↓ ↑ ends on the second, and the arrows
        // do not reach the text.
        #expect(model.aiRun?.choice == 0)
        press("down")
        press("down")
        press("up")
        #expect(model.aiRun?.choice == 1)
        #expect(model.composer?.body.hasPrefix("merci") == true)
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        #expect(model.composer?.headers.subject == "Confirmation pour jeudi")
    }

    @Test func anEndThatComesBeforeTheRunIdIsNotLost() async {
        // The core ends the run before `ai_run` has answered.
        core.delays["ai_run"] = .milliseconds(200)
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
        _ = await eventually { self.model.aiRun != nil }
        #expect(model.aiRun?.runId == 0, "the panel is open, the id not known yet")
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        #expect(model.aiRun?.runId == 7)
    }

    @Test func anEndAfterAGapIsNotLost() async {
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // A numbered event out of order: a gap, which reloads everything.
        model.handle(CoreClient.Event(method: "AiDone", epoch: "e_test", seq: 99, params: ["run_id": .int(7)]))
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
    }

    @Test func theAIOffSaysHowToTurnItOn() async {
        ai.enabled = false
        await reply()
        press("ctrl-i")
        _ = await eventually { self.model.flash == L("The AI is off: set ai.enabled = true in config.toml") }
        #expect(!paletteOpen)
        #expect(core.calls("ai_list_prompts").isEmpty)
    }

    @Test func aSummaryOfTheMailReadIsShownThenClosed() async throws {
        #expect(controller.aiPlacement == "none")
        // Two columns: the mail is shown, `a` opens the reading menu, a
        // palette at the top; its result in the middle of the window.
        press("a")
        _ = await eventually { self.paletteOpen }
        #expect(controller.aiPlacement == "palette")
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "mail")
        choose("r")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["thread_id"] as? Int == model.openThread?.id)
        #expect(run["message_id"] == nil, "a thread prompt names no mail")
        #expect(run["draft_id"] == nil)
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        #expect(AI.resultText(try #require({ if case .proposal(let p)? = model.aiRun?.phase { p } else { nil } }()))
            == "Alice attend jeudi.\n- Apporter les clés (you) · jeudi")
        // The result, in the palette's place; it takes the keys: `j` does
        // not move the list.
        #expect(controller.aiPlacement == "panel")
        let selected = model.selectedThreadId
        press("j")
        #expect(model.selectedThreadId == selected)
        #expect(await eventually { self.model.flash == L("esc close") + " · " + L("%@ copy", "c") })
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_dismiss").count == 1)
        #expect(controller.aiPlacement == "none")
        // The keys are the list's again.
        press("j")
        #expect(model.selectedThreadId != selected)
    }

    @Test func cInThePanelCopiesTheResultToTheClipboard() async throws {
        // A summary of the mail: `c` copies all of it, action items too,
        // and the panel stays open.
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("r")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        // While it runs, there is nothing to copy yet.
        press("c")
        #expect(controller.pasteboard.string(forType: .string) == nil)
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("c")
        #expect(controller.pasteboard.string(forType: .string) == "Alice attend jeudi.\n- Apporter les clés (you) · jeudi")
        #expect(model.flash == L("Result copied to the clipboard"))
        #expect(model.aiRun != nil, "the panel stays open")
        #expect(core.calls("ai_dismiss").isEmpty)
        press("esc")
        _ = await eventually { self.model.aiRun == nil }

        // A proofreading of the reply: the text proposed, not its
        // differences; the reply is untouched, the proposal still there.
        await reply()
        press("ctrl-i")
        _ = await eventually { self.paletteOpen }
        choose("c")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        controller.pasteboard.clearContents()
        press("c")
        let proposal = try #require({ if case .proposal(let p)? = model.aiRun?.phase { p } else { nil } }())
        #expect(controller.pasteboard.string(forType: .string) == AI.copyText(proposal))
        #expect(model.composer?.body.hasPrefix("merci\n") == true)
        #expect(core.calls("ai_apply").isEmpty)
        // Return still accepts it.
        press("ret")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_apply").count == 1)
    }

    @Test func aClickOnTheAIBlockOpensThePaletteOfItsTarget() async throws {
        // Out of a reply: the palette of the mail, as `a`.
        controller.openAIFromStatusBar()
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "mail")
        press("esc")
        #expect(!paletteOpen)
        // In a reply: the palette of the reply, as ⌘I; nothing typed.
        await reply()
        controller.openAIFromStatusBar()
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "compose")
        #expect(model.composer?.body.hasPrefix("merci\n") == true)
        // A result shown: the click leaves it, no palette over it.
        press("ret")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let lists = core.calls("ai_list_prompts").count
        controller.openAIFromStatusBar()
        try await Task.sleep(for: .milliseconds(50))
        #expect(!paletteOpen)
        #expect(model.aiRun != nil)
        #expect(core.calls("ai_list_prompts").count == lists)
    }

    @Test func theFilesTheAIReadsAreRowsOfTheMailsPalette() async throws {
        // A conversation without a readable file: the mail's prompts only,
        // and the core is not asked for those of an attachment.
        press("a")
        _ = await eventually { self.paletteOpen }
        #expect(!paletteRows.contains { $0.contains("·") && $0.contains(".pdf") })
        #expect(!core.calls("ai_list_prompts").contains { $0["screen"] as? String == "attachment" })
        press("esc")
        // Thread 4: devis.pdf can be read, the photo and the archive not.
        model.selectedThreadId = 4
        _ = await eventually { self.model.openThread?.id == 4 }
        press("a")
        _ = await eventually { self.paletteOpen && self.paletteRows.contains("r Summarize · devis.pdf") }
        #expect(paletteRows.first == "r Summarize", "the mail's prompts first: \(paletteRows)")
        #expect(paletteRows.filter { $0.contains(" · ") && !$0.contains("Into") }.count == 1, "\(paletteRows)")
        let row = try #require(controller.pickerEntries(.ai).first { $0.label == "Summarize · devis.pdf" })
        #expect(row.icon == "doc.richtext")
        // Typing the file's name finds it; Return runs it on that file.
        choose("devis")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "summarize-attachment")
        #expect(run["attachment_id"] as? Int == 41)
        #expect(model.aiRun?.title == "Summarize · devis.pdf")
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        let proposal = try #require({ if case .proposal(let p)? = model.aiRun?.phase { p } else { nil } }())
        #expect(AI.resultText(proposal) == "- Un devis de toiture, 4 800 €.")
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
    }

    @Test func aMessagePromptNamesTheMailUnderTheCursor() async throws {
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("te")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["variant"] as? String == "e")
        #expect(run["thread_id"] as? Int == model.openThread?.id)
    }

    @Test func theSendersCardIsSavedThenOpenedInContacts() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NiceMailerTests-vcf-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        model.exportDirectory = dir.path
        var opened: [URL] = []
        model.openFile = { opened.append($0) }
        func runContact() async {
            press("a")
            _ = await eventually { self.paletteOpen }
            choose("contact")
            _ = await eventually { self.model.aiRun?.runId == 7 }
            event("AiDone", ["run_id": .int(7)])
            _ = await eventually { if case .proposal(.contact)? = self.model.aiRun?.phase { true } else { false } }
        }
        await runContact()
        guard case .proposal(.contact(let c))? = model.aiRun?.phase else { Issue.record("no card"); return }
        #expect(c.emails == ["alice@a.example"])
        // Return: saved, not opened; one card, so the panel closes.
        press("ret")
        _ = await eventually { self.model.aiRun == nil && self.model.flash != nil }
        let first = dir.appendingPathComponent("Alice Martin.vcf")
        #expect(try String(contentsOf: first, encoding: .utf8).contains("FN:Alice Martin"))
        #expect(model.flash == L("%@ saved to %@", first.lastPathComponent, FileManager.default.displayName(atPath: dir.path)))
        #expect(opened.isEmpty)
        #expect(core.calls("ai_dismiss").isEmpty, "saved: nothing to turn down")
        // ⌘Return: saved beside the first, without overwriting it, then
        // opened, which imports it into Contacts.
        await runContact()
        press("cmd-ret")
        _ = await eventually { !opened.isEmpty }
        #expect(opened.map(\.lastPathComponent) == ["Alice Martin (2).vcf"])
        #expect(FileManager.default.fileExists(atPath: opened[0].path))
        #expect(model.flash == L("%@ saved to %@ and opened", opened[0].lastPathComponent, FileManager.default.displayName(atPath: dir.path)))
        #expect(model.aiRun == nil)
        // Escape closes a third without a file: turned down.
        await runContact()
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        _ = await eventually { self.core.calls("ai_dismiss").count == 1 }
        #expect(core.calls("ai_dismiss").count == 1)
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path).count == 2)
    }

    @Test func theAppointmentsOfAMailAreSavedOneByOneAsCalendarFiles() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NiceMailerTests-ics-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model.exportDirectory = dir.path
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("d")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        // The first highlighted; ↓ the second, saved with Return.
        #expect(model.aiRun?.choice == 0)
        press("down")
        press("ret")
        _ = await eventually { self.core.calls("ai_apply").count == 1 && self.model.flash != nil }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        let second = dir.appendingPathComponent("2020-06-30 Remise des clés.ics")
        #expect(try String(contentsOf: second, encoding: .utf8).contains("SUMMARY:Remise des clés"))
        #expect(model.flash == L("%@ saved to %@", second.lastPathComponent, FileManager.default.displayName(atPath: dir.path)))
        // The panel stays, to save another; the same again does not
        // overwrite the first file.
        #expect(model.aiRun != nil)
        press("ret")
        _ = await eventually { self.core.calls("ai_apply").count == 2 }
        _ = await eventually { FileManager.default.fileExists(atPath: dir.appendingPathComponent("2020-06-30 Remise des clés (2).ics").path) }
        #expect(FileManager.default.fileExists(atPath: dir.appendingPathComponent("2020-06-30 Remise des clés (2).ics").path))
        // ⌘↩ on the first: saved, then opened (here recorded, not opened).
        var opened: [URL] = []
        model.openFile = { opened.append($0) }
        press("up")
        press("cmd-ret")
        _ = await eventually { !opened.isEmpty }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 0)
        #expect(opened.map(\.lastPathComponent) == ["2020-06-25 Visite de l'appartement.ics"])
        #expect(FileManager.default.fileExists(atPath: opened[0].path))
        #expect(model.flash == L("%@ saved to %@ and opened", opened[0].lastPathComponent, FileManager.default.displayName(atPath: dir.path)))
        #expect(model.aiRun != nil, "still open, for another")
        press("esc")
        _ = await eventually { self.model.aiRun == nil }
        #expect(core.calls("ai_dismiss").count == 1)
        try? FileManager.default.removeItem(at: dir)
    }

    @Test func aQuickReplyStartsTheReplyWithIt() async {
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("p")
        _ = await eventually { self.model.aiRun?.runId == 7 }
        event("AiDone", ["run_id": .int(7)])
        _ = await eventually { if case .proposal? = self.model.aiRun?.phase { true } else { false } }
        press("2")
        // The reply opens first, then gets the text: wait for the text,
        // not only the reply (waiting for the reply alone was flaky).
        _ = await eventually { self.model.composer?.body.hasPrefix("On peut") == true }
        #expect(core.calls("ai_apply").last?["choice"] as? Int == 1)
        #expect(core.calls("ai_apply").last?["revision"] == nil, "no draft yet")
        #expect(model.aiRun == nil)
        // The reply to the mail, the text above the quotation, the cursor
        // after it, to be saved as if typed.
        #expect(core.calls("prepare_draft").count == 1)
        #expect(model.composer?.body.hasPrefix("On peut décaler ?\n\nOn Nov 7") == true, "\(model.composer?.body ?? "")")
        #expect(editor.loads.last?.cursor == "On peut décaler ?".utf16.count)
        #expect(model.composer?.isDirty == true)
    }

    @Test func aSuggestionShowsAndReturnMovesThere() async throws {
        ai.suggest = true
        model.scheduleSuggestions()
        _ = await eventually { !self.model.aiSuggestions.isEmpty }
        let first = try #require(model.threads.first?.id)
        #expect(model.aiSuggestions[first]?.target == "archive")
        #expect(model.selectedThreadId == first)
        #expect(model.selectedSuggestion?.reason == "Done with it.")
        // Return moves it to the target suggested, that conversation only.
        press("ret")
        _ = await eventually { !self.core.moveCalls().isEmpty }
        let moves = core.moveCalls()
        #expect(moves.count == 1)
        #expect(moves.first?["thread_id"] as? Int == first)
        #expect(moves.first?["target"] as? String == "archive")
    }

    @Test func aSortRunsInTheBackgroundWithTheKeysFree() async throws {
        let two = Set(model.threads.prefix(2).map(\.id))
        model.marked = two
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("s")
        _ = await eventually { self.model.aiSort?.runId == 7 }
        let run = try #require(core.calls("ai_run").last)
        #expect(run["prompt"] as? String == "suggest-folder")
        #expect(run["folder_ids"] as? [Int] == model.listedFolderIds)
        // The conversations marked.
        #expect(Set(run["thread_ids"] as? [Int] ?? []) == two)
        // The progress in the status bar; the keys are the list's.
        event("AiProgress", ["run_id": .int(7), "done": .int(20), "total": .int(47)], transient: true)
        #expect(model.aiSort?.done == 20 && model.aiSort?.total == 47)
        let selected = model.selectedThreadId
        press("j")
        #expect(model.selectedThreadId != selected)
        press("k")
        // A batch's suggestions reload those of the rows.
        ai.suggest = true
        event("AiSuggestions", ["run_id": .int(7), "thread_ids": .ints([model.threads[0].id])])
        _ = await eventually { !self.model.aiSuggestions.isEmpty }
        // The end, said.
        event("AiDone", ["run_id": .int(7)])
        #expect(model.aiSort == nil)
    }

    @Test func escapeStopsASort() async {
        model.marked = Set(model.threads.prefix(2).map(\.id))
        press("a")
        _ = await eventually { self.paletteOpen }
        choose("s")
        _ = await eventually { self.model.aiSort?.runId == 7 }
        press("esc")
        _ = await eventually { self.core.calls("ai_cancel").count == 1 }
        #expect(model.aiSort == nil)
        #expect(model.flash == L("AI stopped"))
    }

    @Test func aWithSeveralMarkedOpensThePaletteOfTheSelection() async {
        model.marked = Set(model.threads.prefix(2).map(\.id))
        press("a")
        _ = await eventually { self.paletteOpen }
        #expect(core.calls("ai_list_prompts").last?["screen"] as? String == "selection")
        #expect(core.calls("ai_list_prompts").last?["folder"] as? String == "inbox")
        choose("w")
        #expect(model.flash == L("Not available yet: %@", "Write from my notes"))
    }
}

struct AIAttachmentTests {
    func message(_ id: Int, _ files: [AttachmentView]) throws -> MessageView {
        let json: [String: Any] = [
            "id": id, "folder_id": 1, "folder_role": "inbox", "subject": "s", "from_name": NSNull(), "from_addr": "a@b.fr",
            "to": [], "cc": [], "date_sent": NSNull(), "date_received": 0, "body_text": "x", "is_seen": true,
            "is_flagged": false, "is_answered": false, "has_attachments": !files.isEmpty, "auth_result": "unknown",
            "body_html": NSNull(), "has_remote_content": false, "from_body_format": NSNull(), "segments": [],
            "only_quotes": false, "expanded": true, "show_recipients": false,
            "attachments": files.map { ["id": $0.id, "filename": $0.filename as Any, "mime_type": $0.mimeType, "size": $0.size,
                                        "readable": $0.readable as Any] },
            "unsubscribe": NSNull(), "tracking_pixels": 0, "links": [], "verification_code": NSNull(),
        ]
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return try d.decode(MessageView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    @Test func theFilesTheAICanReadComeOnceWithTheirMail() throws {
        let pdf = AttachmentView(id: 1, filename: "devis.pdf", mimeType: "application/pdf", size: 9, readable: true)
        let photo = AttachmentView(id: 2, filename: "photo.jpg", mimeType: "image/jpeg", size: 9, readable: false)
        let again = AttachmentView(id: 3, filename: "devis.pdf", mimeType: "application/pdf", size: 9, readable: true)
        let notes = AttachmentView(id: 4, filename: "notes.txt", mimeType: "text/plain", size: 3, readable: true)
        let older = AttachmentView(id: 5, filename: "old.pdf", mimeType: "application/pdf", size: 3)
        let files = AI.readableAttachments([try message(10, [pdf, photo, older]), try message(11, [again, notes])])
        #expect(files.map(\.file.id) == [1, 4], "the photo, the copy and a file of an older core are left out")
        #expect(files.map(\.message) == [10, 11])
    }
}
