import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Quarantine in the geek theme (`ui.md`, "Quarantine"): its move targets
/// are single keys shown in the status bar, the keys that would change the
/// mail otherwise are refused, and viewing a mail there never marks it
/// read.
///
/// The fake core adds a Quarantine folder (id 5) to the fixture, with the
/// targets the real core gives it (`dossiers.md`, "Cibles proposées") and
/// four unread threads (51 to 54): 51 from a person, 52 and 54 from
/// robots, 53 without verdict. The Inbox (id 1) keeps the fixture's eight
/// threads.
@MainActor
struct QuarantineTests {
    static let targets = ["inbox", "newsletter", "paperwork", "junk", "banned", "deleted"]
    /// Who wrote each Quarantine thread, as the core says it
    /// (`correspondent_kind`); 53 has no verdict.
    static let kinds: [Int: String] = [51: "person", 52: "robot", 54: "robot"]

    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async {
        core = FakeCore { method, params in
            switch method {
            case "list_folders":
                var folders = Fixture.folders
                folders.append(Fixture.folder(5, "quarantine", "Quarantine", "Q", targets: QuarantineTests.targets))
                return ["items": folders, "as_of": Fixture.position]
            case "list_threads":
                let folder = (params["folder_ids"] as? [Int])?.first ?? params["folder_id"] as? Int ?? 1
                guard folder == 5 else { return Fixture.answer(method, params) }
                let items = (51...54).map { id -> [String: Any] in
                    var t = Fixture.thread(id, folder: 5)
                    t["unread_count"] = 1
                    t["correspondent_kind"] = QuarantineTests.kinds[id] ?? NSNull()
                    return t
                }
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            case "get_thread":
                // Every message unread, so that only the Quarantine rule
                // keeps the app from marking them.
                var view = Fixture.threadView(params["thread_id"] as? Int ?? 1)
                var messages = view["messages"] as! [[String: Any]]
                messages[0]["is_seen"] = false
                view["messages"] = messages
                return view
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = TestDefaults.make()
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize())
        model.seenDelay = 0.05
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        switch name {
        case "ret": chars = "\r"; code = KeyCode.returnKey
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        default: break
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        _ = controller.handle(event)
    }

    /// `;q`: goes to Quarantine, as the user does.
    func goToQuarantine() async {
        press(";")
        press("q")
        _ = await eventually { model.threads.first?.id == 51 && model.openThread?.id == 51 }
    }

    /// Waits longer than `seenDelay`, for the negative checks.
    func pause() async { try? await Task.sleep(for: .milliseconds(250)) }

    func moves() -> [(thread: Int, target: String)] {
        core.moveCalls().map { ($0["thread_id"] as? Int ?? 0, $0["target"] as? String ?? "") }
    }

    // MARK: Targets in the status bar

    @Test func theStatusBarShowsTheTargetsOfQuarantineOnly() async {
        // Elsewhere, the `m` block: no direct target.
        #expect(controller.directTargets.isEmpty)
        await goToQuarantine()
        #expect(model.currentFolderName == "Quarantine")
        #expect(controller.directTargets == Self.targets)
        // Their keys: the folder letters, `x` for Junk.
        #expect(controller.directTargets.map { Role.letter($0).lowercased() } == ["i", "n", "p", "x", "b", "d"])
        // Another screen has no mail to move.
        controller.open(.contacts)
        #expect(controller.directTargets.isEmpty)
    }

    // MARK: Person or robot before the name

    @Test func theSenderColumnTellsPeopleFromRobotsInQuarantineOnly() async {
        // The Inbox draws no icon: its rows carry no kind (the fixture's),
        // and the list would not show one anyway.
        #expect(!model.listShowsSenderKind)
        await goToQuarantine()
        #expect(model.listShowsSenderKind)
        let kinds = model.threads.map(\.correspondentKind)
        #expect(kinds == ["person", "robot", nil, "robot"])
        #expect(model.threads.map(GeekSender.kindSymbol) == ["person.fill", "gearshape.fill", nil, "gearshape.fill"])
        // The name itself is unchanged: the icon is drawn beside it.
        #expect(GeekSender.plain(model.threads[0]) == "Sender 51")
        // VoiceOver hears it with the name; without verdict, the name alone.
        #expect(GeekSender.spoken(model.threads[0], showsKind: true) == L("%@, written by a person", "Sender 51"))
        #expect(GeekSender.spoken(model.threads[1], showsKind: true) == L("%@, written by a robot", "Sender 52"))
        #expect(GeekSender.spoken(model.threads[2], showsKind: true) == "Sender 53")
        // Back to the Inbox: no icon.
        press(";")
        press("i")
        _ = await eventually { model.threads.first?.id == 1 }
        #expect(!model.listShowsSenderKind)
    }

    @Test func aTargetsKeyMovesAtOnce() async {
        await goToQuarantine()
        press("i")
        _ = await eventually { moves().count == 1 }
        #expect(moves().first?.thread == 51 && moves().first?.target == "inbox")
        // No menu opened: the key acted by itself.
        #expect(controller.overlay == .none)
        // The cursor went to the next mail; `n` sends it to Newsletter.
        #expect(model.selectedThreadId == 52)
        press("n")
        _ = await eventually { moves().count == 2 }
        #expect(moves().last?.thread == 52 && moves().last?.target == "newsletter")
        press("x")
        _ = await eventually { moves().count == 3 }
        #expect(moves().last?.target == "junk")
    }

    @Test func dDeletesInQuarantineAndStillArchivesElsewhere() async {
        await goToQuarantine()
        press("d")
        _ = await eventually { moves().count == 1 }
        #expect(moves().first?.target == "deleted")
        press(";")
        press("i")
        _ = await eventually { model.threads.first?.id == 1 }
        press("d")
        _ = await eventually { moves().count == 2 }
        #expect(moves().last?.target == "archive")
    }

    @Test func aTargetsKeyMovesEveryMarkedMail() async {
        await goToQuarantine()
        press("J")
        press("J")
        // J marks the current mail and the next one: three after two J.
        #expect(model.marked == [51, 52, 53])
        press("b")
        _ = await eventually { moves().count == 3 }
        #expect(Set(moves().map(\.thread)) == [51, 52, 53])
        #expect(moves().allSatisfy { $0.target == "banned" })
    }

    @Test func aClickOnATargetMovesAsItsKey() async {
        await goToQuarantine()
        controller.moveFromStatusBar("paperwork")
        _ = await eventually { moves().count == 1 }
        #expect(moves().first?.thread == 51 && moves().first?.target == "paperwork")
    }

    // MARK: Refused keys

    @Test func mSaysWhichKeysToPressInstead() async {
        await goToQuarantine()
        press("m")
        #expect(controller.overlay == .none)
        #expect(model.flash == L("In Quarantine, press the folder's key: %@", "i n p x b d"))
        #expect(moves().isEmpty)
    }

    @Test func keysThatWouldChangeTheMailAreRefused() async {
        await goToQuarantine()
        for key in ["u", "U", "t", "g", "r", "s"] {
            press(key)
            #expect(model.flash == L("In Quarantine, a mail can only be moved or flagged"), "key \(key)")
            #expect(controller.overlay == .none, "key \(key)")
        }
        #expect(model.composer == nil)
        await pause()
        #expect(core.calls("set_thread_seen").isEmpty)
        #expect(core.calls("create_draft").isEmpty)
    }

    @Test func theFlagAndTheKeysThatOnlyReadStay() async {
        await goToQuarantine()
        press("*")
        _ = await eventually { !core.calls("set_thread_flagged").isEmpty }
        #expect(core.calls("set_thread_flagged").first?["thread_ids"] as? [Int] == [51])
        press("j")
        #expect(model.selectedThreadId == 52)
        press("v")
        #expect(controller.overlay == .menu(.view))
    }

    @Test func refusedKeysWorkAgainOutsideQuarantine() async {
        await goToQuarantine()
        press(";")
        press("i")
        _ = await eventually { model.threads.first?.id == 1 }
        press("m")
        #expect(controller.overlay == .menu(.move))
        press("esc")
        press("s")
        #expect(controller.overlay == .menu(.save))
    }

    // MARK: Never marked read

    @Test func aMailChosenByHandIsNeverMarkedRead() async {
        await goToQuarantine()
        press("j")
        await pause()
        press("j")
        await pause()
        #expect(core.calls("set_thread_seen").isEmpty)
    }

    @Test func returnInOneColumnDoesNotMarkItRead() async {
        await goToQuarantine()
        controller.columns = 1
        press("ret")
        #expect(controller.reading)
        await pause()
        #expect(core.calls("set_thread_seen").isEmpty)
    }

    @Test func theInboxStillMarksReadAfterTheDelay() async {
        // The rule is Quarantine's only: the same gesture in the Inbox marks.
        press("j")
        // Leaving the mail selected at the opening, after the delay, may
        // mark it too (`ReadingTests`): mail 2 is the one to look for.
        let marked = { core.calls("set_thread_seen").compactMap { $0["thread_id"] as? Int } }
        #expect(await eventually { marked().contains(2) })
    }
}
