import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// "Change Mailbox…" checks the folder before switching to it
/// (`MailboxCheck.swift`, `AppModel.changeMailbox`), and the screen without
/// a core says why the core stopped. Against a fake CLI (`FakeMailboxTool`)
/// and a switch that only records the folder: no test changes the user's
/// mailbox.

/// Answers `check` from a table, records `upgrade`, and can fail it.
final class FakeMailboxTool: MailboxTool, @unchecked Sendable {
    private let lock = NSLock()
    var reports: [String: MailboxReport] = [:]
    var upgradeError: MailboxToolError?
    private var upgradedPaths: [String] = []

    var upgraded: [String] {
        lock.lock()
        defer { lock.unlock() }
        return upgradedPaths
    }

    func check(_ dir: URL) async throws -> MailboxReport {
        guard let r = reports[dir.path] else { throw MailboxToolError(message: "no such folder") }
        return r
    }

    func upgrade(_ dir: URL) async throws {
        if let upgradeError { throw upgradeError }
        lock.lock()
        upgradedPaths.append(dir.path)
        lock.unlock()
    }
}

/// The JSON of `nicemailer mailbox check --json`, decoded as the app does.
private func report(_ json: String) throws -> MailboxReport {
    try CoreClient.decoder.decode(MailboxReport.self, from: Data(json.utf8))
}

struct MailboxDecisionTests {
    @Test func whatTheCLISaysDecidesWhatTheAppDoes() throws {
        #expect(try report(#"{"state":"new","path":"/m","in_use":false}"#).decision == .use)
        #expect(try report(#"{"state":"current","path":"/m","in_use":true}"#).decision == .use)
        let old = try report(#"{"state":"upgradable","from":1,"to":4,"path":"/m","in_use":false}"#)
        #expect(old.decision == .askUpgrade(from: 1, to: 4))
        #expect(old.inUse == false)
        // The three refusals name what is wrong, and say the current
        // mailbox stays (in the language of the test run: compared with
        // the translation itself).
        let refusals: [(String, String)] = [
            (#"{"state":"outdated","missing":"contact_group","path":"/m","in_use":false}"#,
             L("This mailbox was created by an early development version of NiceMailer (%@ missing): it cannot be upgraded. The current mailbox is kept.", "contact_group")),
            (#"{"state":"too_new","found":9,"expected":4,"path":"/m","in_use":false}"#,
             L("This mailbox was written by a newer version of NiceMailer (data format %lld; this one reads %lld): update NiceMailer to open it. The current mailbox is kept.", 9, 4)),
            (#"{"state":"unreadable","message":"file is not a database","path":"/m","in_use":false}"#,
             L("The database of this folder cannot be read (%@). The current mailbox is kept.", "file is not a database")),
        ]
        for (json, expected) in refusals {
            #expect(try report(json).decision == .refuse(expected))
        }
        #expect(refusals[1].1.contains("9") && refusals[1].1.contains("4"))
    }

    @Test func theCLIErrorLosesAnyhowsPrefix() {
        #expect(CLIMailboxTool.errorMessage(Data("Error: a core is using /m\n".utf8)) == "a core is using /m")
        #expect(CLIMailboxTool.errorMessage(Data("plain\n".utf8)) == "plain")
    }

    @Test func theCoreReasonReplacesUnreachable() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nm-startup-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        #expect(StartupErrorReport.read(in: dir) == nil)
        #expect(AppModel.noCoreMessage("unreachable", report: nil) == "unreachable")

        // As the core writes it (`nm_api::StartupError`).
        try Data(#"{"kind":"outdated","message":"ouverture de …","missing":"contact_group","at":1}"#.utf8)
            .write(to: dir.appendingPathComponent("core-error.json"))
        let outdated = try #require(StartupErrorReport.read(in: dir))
        let text = AppModel.noCoreMessage("unreachable", report: outdated)
        #expect(text == L("The core cannot open this mailbox: it was created by an early development version of NiceMailer (%@ missing) and cannot be upgraded. Choose another mailbox.", "contact_group"))
        #expect(text.contains("contact_group"))

        let newer = StartupErrorReport(kind: "too_new", message: "…", found: 7, expected: 4)
        #expect(newer.userMessage.contains("7") && newer.userMessage.contains("4"))
        let other = StartupErrorReport(kind: "other", message: "disque plein")
        #expect(other.userMessage.contains("disque plein"))
        // A file that is not a report is ignored.
        try Data("garbage".utf8).write(to: dir.appendingPathComponent("core-error.json"))
        #expect(StartupErrorReport.read(in: dir) == nil)
    }
}

@MainActor
struct MailboxChangeTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()
    let tool = FakeMailboxTool()
    let controller: GeekController
    /// The folders the app switched to.
    let switched = Switched()

    final class Switched: @unchecked Sendable {
        var paths: [String] = []
    }

    let old = URL(fileURLWithPath: "/mailboxes/old")
    let fine = URL(fileURLWithPath: "/mailboxes/fine")
    let early = URL(fileURLWithPath: "/mailboxes/early")

    init() async throws {
        let defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        model.openFile = { _ in }
        model.addressBook = FixtureAddressBook(cards: [:])
        tool.reports = [
            old.path: try report(#"{"state":"upgradable","from":2,"to":4,"in_use":false}"#),
            fine.path: try report(#"{"state":"current","in_use":false}"#),
            early.path: try report(#"{"state":"outdated","missing":"contact_group","in_use":false}"#),
        ]
        model.mailboxTool = tool
        let switched = self.switched
        model.switchMailboxOverride = { url in switched.paths.append(url.path) }
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 }
    }

    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        if name == "esc" { chars = "\u{1b}"; code = KeyCode.escape }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        #expect(controller.handle(event))
    }

    @Test func aCurrentMailboxIsUsedAtOnce() async {
        await model.changeMailbox(to: fine)
        #expect(switched.paths == [fine.path])
        #expect(model.pendingUpgrade == nil && model.lastError == nil)
    }

    @Test func aMailboxTheCoreWouldRefuseIsRefusedAndTheCurrentOneKept() async {
        await model.changeMailbox(to: early)
        #expect(switched.paths.isEmpty)
        #expect(model.lastError?.contains("contact_group") == true)
        // A folder the CLI cannot check: refused too.
        model.lastError = nil
        await model.changeMailbox(to: URL(fileURLWithPath: "/mailboxes/missing"))
        #expect(switched.paths.isEmpty)
        #expect(model.lastError?.contains("no such folder") == true)
        #expect(tool.upgraded.isEmpty)
        // The next choice clears the previous refusal.
        await model.changeMailbox(to: fine)
        #expect(model.lastError == nil)
    }

    @Test func anOlderMailboxIsUpgradedOnlyOnceTheUserSaysYes() async {
        await model.changeMailbox(to: old)
        #expect(model.pendingUpgrade == PendingUpgrade(url: old, from: 2, to: 4))
        #expect(switched.paths.isEmpty && tool.upgraded.isEmpty, "nothing done before the answer")
        // The question of the geek theme: its title and its two answers.
        guard case .mailboxUpgrade = controller.activeChoice else {
            Issue.record("no question: \(String(describing: controller.activeChoice))")
            return
        }
        let title = controller.choiceTitle(controller.activeChoice!)
        #expect(title.contains("old") && title.contains("2") && title.contains("4"), "\(title)")
        #expect(controller.choiceEntries(controller.activeChoice!).map(\.key) == ["y", "esc"])
        #expect(controller.modeLabel == "ASK")
        // Another key waits for the answer.
        press("j")
        #expect(model.pendingUpgrade != nil)

        press("y")
        #expect(model.pendingUpgrade == nil, "answered at once")
        _ = await eventually { switched.paths == [old.path] }
        #expect(tool.upgraded == [old.path])
        #expect(switched.paths == [old.path], "upgraded, then used")
        #expect(model.upgradingMailbox == nil)
    }

    @Test func escapeKeepsTheCurrentMailbox() async {
        await model.changeMailbox(to: old)
        press("esc")
        #expect(model.pendingUpgrade == nil && controller.activeChoice == nil)
        #expect(tool.upgraded.isEmpty && switched.paths.isEmpty)
        #expect(model.flash == L("%@ not upgraded: the current mailbox is kept", "old"))
    }

    @Test func aFailedUpgradeKeepsTheCurrentMailbox() async {
        tool.upgradeError = MailboxToolError(message: "a core is using /mailboxes/old")
        await model.changeMailbox(to: old)
        await model.confirmMailboxUpgrade()?.value
        #expect(switched.paths.isEmpty)
        #expect(model.lastError?.contains("a core is using") == true)
        #expect(model.upgradingMailbox == nil)
    }

    @Test func withoutTheCLITheFolderIsUsedUnchecked() async {
        model.mailboxTool = nil
        await model.changeMailbox(to: early)
        #expect(switched.paths == [early.path])
    }
}
