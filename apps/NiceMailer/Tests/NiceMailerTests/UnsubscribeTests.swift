import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The unsubscribe button and ⌘U (`ui.md`, "Désabonnement"), against the
/// fake core: one click done by the core, failed, a page, an address, a
/// link of the body.
@MainActor
struct UnsubscribeTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()
    let controller: InterfaceController
    /// What the app opened (the tests never launch the browser or Mail).
    final class Opened { var urls: [URL] = [] }
    let opened = Opened()

    init() async {
        let defaults = TestDefaults.make()
        controller = InterfaceController(model: model, defaults: defaults, textSize: TextSize())
        let opened = opened
        model.openFile = { opened.urls.append($0) }
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func open(_ thread: Int) async {
        model.selectedThreadId = thread
        _ = await eventually { model.openThread?.id == thread }
    }

    func commandU() {
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "u", charactersIgnoringModifiers: "u", isARepeat: false, keyCode: 32)!
        #expect(controller.handle(e))
    }

    var calls: [Int] { core.calls("unsubscribe").compactMap { $0["message_id"] as? Int } }

    @Test func oneClickIsDoneByTheCoreAndNothingOpens() async {
        await open(5)
        #expect(model.unsubscribeOffer?.method == "one_click")
        commandU()
        #expect(await eventually { model.flash == L("Unsubscribed from %@", "Sender 5") })
        #expect(calls == [50])
        #expect(opened.urls.isEmpty)
        // The thread is read again, to show "unsubscribed on".
        #expect(await eventually { core.calls("get_thread").filter { $0["thread_id"] as? Int == 5 }.count >= 2 })
    }

    @Test func aFailedOneClickOpensThePage() async {
        await open(3)
        commandU()
        #expect(await eventually { opened.urls.map(\.absoluteString) == ["https://x.org/u3"] })
        #expect(model.flash?.contains("500") == true)
    }

    @Test func aPageAnAddressAndABodyLinkAreOpened() async {
        await open(7)
        commandU()
        #expect(await eventually { opened.urls.last?.absoluteString == "https://x.org/u7" })
        #expect(model.flash == L("Unsubscribe page opened"))
        await open(8)
        commandU()
        #expect(await eventually { opened.urls.last?.absoluteString == "mailto:leave@x.org" })
        #expect(model.flash == L("Unsubscribe mail opened"))
        await open(2)
        commandU()
        #expect(await eventually { opened.urls.last?.absoluteString == "https://x.org/b2" })
    }

    @Test func withoutALinkCommandUSaysSoAndCallsNothing() async {
        await open(1)
        #expect(model.unsubscribeOffer == nil)
        commandU()
        #expect(model.flash == L("No unsubscribe link in this mail"))
        #expect(calls.isEmpty)
    }

    @Test func commandUWaitsWhileAPanelIsOpen() async {
        await open(5)
        controller.overlay = .help
        let e = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "u", charactersIgnoringModifiers: "u", isARepeat: false, keyCode: 32)!
        #expect(!controller.handle(e))
        #expect(calls.isEmpty)
    }

    // MARK: The button (pure)

    func message(role: String, unsubscribe: UnsubscribeView?) -> MessageView {
        MessageView(
            id: 1, folderId: 1, folderRole: role, subject: "S", fromName: "L", fromAddr: "l@x.org", to: [], cc: [],
            dateSent: nil, dateReceived: 0, bodyText: "", isSeen: true, isFlagged: false, isAnswered: false,
            hasAttachments: false, authResult: "unknown", bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: [], onlyQuotes: false, expanded: true, showRecipients: false, attachments: [],
            unsubscribe: unsubscribe)
    }

    func view(_ method: String, _ source: String, at: Int? = nil) -> UnsubscribeView {
        UnsubscribeView(method: method, source: source, url: "https://x.org/u", mailto: nil, senderUnsubscribedAt: at)
    }

    @Test func noButtonInTheSpamFolders() {
        let u = view("one_click", "header")
        for role in ["junk", "quarantine", "banned"] {
            #expect(UnsubscribeDisplay.offer(message(role: role, unsubscribe: u)) == nil, "\(role)")
        }
        for role in ["inbox", "newsletter", "archive"] {
            #expect(UnsubscribeDisplay.offer(message(role: role, unsubscribe: u)) == u, "\(role)")
        }
        #expect(UnsubscribeDisplay.offer(message(role: "inbox", unsubscribe: nil)) == nil)
        #expect(UnsubscribeDisplay.offer(nil) == nil)
    }

    /// `unsubscribe.in_banned` opens Banned, and Banned alone.
    @Test func theSettingOpensBannedOnly() {
        let u = view("one_click", "header")
        #expect(UnsubscribeDisplay.offer(message(role: "banned", unsubscribe: u), inBanned: true) == u)
        for role in ["junk", "quarantine"] {
            #expect(UnsubscribeDisplay.offer(message(role: role, unsubscribe: u), inBanned: true) == nil, "\(role)")
        }
        #expect(UnsubscribeDisplay.offer(message(role: "inbox", unsubscribe: u), inBanned: true) == u)
    }

    /// The app reads the setting from `config_status`; a core that does not
    /// give it keeps Banned closed.
    @Test func theSettingIsReadFromTheCore() async {
        for (answer, expected) in [(true, true), (false, false)] {
            let core = FakeCore { method, params in
                method == "config_status"
                    ? ["ui": ["title_bar": "hidden"], "unsubscribe": ["in_banned": answer]] as [String: Any]
                    : Fixture.answer(method, params)
            }
            let model = AppModel()
            await model.attach(core)
            #expect(await eventually { model.unsubscribeInBanned == expected }, "\(answer)")
        }
        let older = FakeCore { method, params in
            method == "config_status" ? ["ui": ["title_bar": "hidden"]] as [String: Any] : Fixture.answer(method, params)
        }
        let model = AppModel()
        await model.attach(older)
        await model.loadUISettings()
        #expect(!model.unsubscribeInBanned)
    }

    @Test func theButtonSaysWhereTheLinkComesFromAndWhatAClickDoes() {
        // Header, one click: nothing opens, no symbol.
        #expect(UnsubscribeDisplay.label(view("one_click", "header")).symbol == nil)
        // Header, a page or an address of the list.
        #expect(UnsubscribeDisplay.label(view("browser", "header")).symbol == "arrow.up.right.square")
        #expect(UnsubscribeDisplay.label(view("mail", "header")).symbol == "envelope")
        // A link of the body: another symbol, and a tooltip that says so.
        let body = UnsubscribeDisplay.label(view("browser", "body"))
        #expect(body.symbol == "link")
        #expect(body.help != UnsubscribeDisplay.label(view("browser", "header")).help)
        #expect(!UnsubscribeDisplay.done(1_790_000_000).isEmpty)
    }

    /// "unsubscribed 09:57" read as a time: the line gives the day, even
    /// for today, and the year only before this one.
    @Test func theUnsubscribeDateIsADayNeverATime() {
        let utc = TimeZone(identifier: "UTC")!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = utc
        let f = Format.Dates(locale: Locale(identifier: "en_GB"), timeZone: utc)
        let at = 1_795_600_620            // 2026-11-25 09:57 UTC
        let sameDay = Date(timeIntervalSince1970: TimeInterval(at + 3600))
        let today = UnsubscribeDisplay.done(at, in: f, now: sameDay, calendar: calendar)
        #expect(today.contains("25"))
        #expect(today.contains("Nov"))
        #expect(!today.contains("09:57") && !today.contains("9:57"))
        let nextYear = Date(timeIntervalSince1970: TimeInterval(at + 400 * 86_400))
        #expect(UnsubscribeDisplay.done(at, in: f, now: nextYear, calendar: calendar).contains("2026"))
    }
}
