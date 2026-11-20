import AppKit
import Contacts
import Foundation
import Testing
@testable import NiceMailer

/// The senders' cards in `l` (`AddressBook.swift`): the rows made from a
/// card, and the lookup driven by the model, against a book of the test
/// (`FixtureAddressBook`): the user's cards are never read.
@MainActor
struct AddressBookTests {
    let core = FakeCore(handler: AddressBookTests.answer)
    let model = AppModel()
    let book = FixtureAddressBook(access: .notDetermined, cards: [
        "alice@example.org": [ContactCard(id: "A1:ABPerson", name: "Alice Martin", phones: [
            .init(label: "mobile", number: "+33 6 12 34 56 78"),
            .init(label: "work", number: "01 44 55 66 77 p12"),
        ])],
        "bob@example.org": [ContactCard(id: "B1", name: "Bob Durand", phones: [.init(label: nil, number: "06 98 76 54 32")])],
        "carol@example.org": [ContactCard(id: "C1", name: "Carol", phones: [.init(label: "mobile", number: "06 11 11 11 11")])],
    ])
    let controller: GeekController

    init() async {
        model.openFile = { _ in }
        model.addressBook = book
        let defaults = TestDefaults.make()
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize())
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        await model.attach(core)
        _ = await eventually { model.openThread != nil }
    }

    /// Thread 1: Alice writes (Carol in copy), the user answers (Sent),
    /// Bob writes, Alice again with her work number in her signature.
    static func answer(_ method: String, _ params: [String: Any]) -> Any {
        guard method == "get_thread" else { return Fixture.answer(method, params) }
        var thread = Fixture.threadView(1)
        let base = (thread["messages"] as! [[String: Any]])[0]
        func message(_ id: Int, _ from: String, role: String, links: [[String: Any]] = []) -> [String: Any] {
            var m = base
            m["id"] = id
            m["from_addr"] = from
            m["folder_role"] = role
            m["cc"] = [["addr": "carol@example.org", "name": "Carol"]]
            m["links"] = links
            return m
        }
        thread["messages"] = [
            message(10, "Alice@Example.org", role: "inbox"),
            message(11, "me@example.com", role: "sent"),
            message(12, "bob@example.org", role: "inbox"),
            message(13, "alice@example.org", role: "inbox", links: [
                ["url": "tel:0144556677", "text": "01 44 55 66 77"],
                ["url": "https://example.org/plans", "text": "Les plans"],
            ]),
        ]
        return thread
    }

    /// One key: a character, `ret`, `esc`, or `ctrl-X` (`ctrl-C` with
    /// Shift, as the keyboard sends it).
    func press(_ key: String) {
        var (chars, code) = key == "ret" ? ("\r", KeyCode.returnKey) : key == "esc" ? ("\u{1b}", KeyCode.escape) : (key, 0)
        var mods: NSEvent.ModifierFlags = []
        if key.hasPrefix("ctrl-") {
            chars = String(key.dropFirst(5))
            mods = chars == chars.lowercased() ? .control : [.control, .shift]
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        #expect(controller.handle(event))
    }

    func type(_ text: String) {
        for c in text { press(String(c)) }
    }

    // MARK: Rows of a card

    @Test func aNumberBecomesATelURLWithoutItsExtension() {
        #expect(ContactLinks.telURL("+33 6 12 34 56 78")?.absoluteString == "tel:+33612345678")
        #expect(ContactLinks.telURL("(01) 44-55.66.77")?.absoluteString == "tel:0144556677")
        #expect(ContactLinks.telURL("01 44 55 66 77 p12")?.absoluteString == "tel:0144556677")
        #expect(ContactLinks.telURL("01 44 55 66 77, 12")?.absoluteString == "tel:0144556677")
        #expect(ContactLinks.telURL("12") == nil)
        #expect(ContactLinks.telURL("ext") == nil)
    }

    @Test func eachCardGivesItsNumbersThenItself() {
        let cards = [
            ContactCard(id: "A1:ABPerson", name: "Alice Martin", phones: [
                .init(label: "mobile", number: "+33 6 12 34 56 78"), .init(label: nil, number: "06 12 34 56 78"),
            ]),
            // A switchboard shared with Alice's card: listed once.
            ContactCard(id: "Z", name: "Atelier", phones: [.init(label: "work", number: "06.12.34.56.78")]),
        ]
        let links = ContactLinks.links(cards)
        #expect(links.map(\.title) == ["Alice Martin — mobile", L("Card of %@", "Alice Martin"), L("Card of %@", "Atelier")])
        #expect(links.map(\.destination) == ["+33 6 12 34 56 78", L("Contacts"), L("Contacts")])
        #expect(links.map(\.icon) == ["phone", "person.crop.circle", "person.crop.circle"])
        #expect(links[1].url.absoluteString == "addressbook://A1:ABPerson")
        // An identifier Contacts would not read gives no card row.
        #expect(ContactLinks.cardURL("a b/c") == nil)
        #expect(ContactLinks.cardURL("") == nil)
    }

    // MARK: Lookup

    @Test func theSendersAreTheAuthorsOfTheReceivedMessagesLatestFirst() {
        // Not the user (Sent), not Carol (in copy); Alice once, whatever
        // the case of her address.
        #expect(model.threadSenders == ["alice@example.org", "bob@example.org"])
    }

    @Test func theBookIsOnlyAskedFromL() async {
        // Opening the thread did not ask macOS's question.
        #expect(book.access == .notDetermined)
        #expect(model.contactLinks.isEmpty)
        press("l")
        _ = await eventually { !model.contactLinks.isEmpty }
        #expect(book.access == .authorized)
        #expect(book.lookups.last == ["alice@example.org", "bob@example.org"])
        let rows = controller.pickerEntries(.links).map(\.label)
        #expect(rows == [
            L("Link to this mail"),
            "Alice Martin — mobile", "Alice Martin — work", L("Card of %@", "Alice Martin"),
            "Bob Durand", L("Card of %@", "Bob Durand"),
            // The number of Alice's signature is her work number: named
            // above, not listed again.
            "Les plans",
        ])
        // Return on a number calls it; on a card, opens Contacts.
        var opened: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        type("mobile"); press("ret")
        press("l"); type("A1:AB"); press("ret")
        #expect(opened == ["tel:+33612345678", "addressbook://A1:ABPerson"])
    }

    @Test func onceAllowedTheCardsComeWithTheThread() async {
        book.setAccess(.authorized)
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        model.selectedThreadId = 1
        _ = await eventually { !model.contactLinks.isEmpty }
        #expect(model.contactLinks.first?.title == "Alice Martin — mobile")
    }

    @Test func aRefusalIsSaidOnceAndTheLinksStay() async {
        book.setAccess(.denied)
        press("l")
        _ = await eventually { model.flash != nil }
        #expect(model.flash?.contains("Contacts") == true)
        #expect(controller.pickerEntries(.links).map(\.label).contains("Les plans"))
        #expect(model.contactLinks.isEmpty)
        model.flash = nil
        press("esc"); press("l")
        try? await Task.sleep(for: .milliseconds(50))
        #expect(model.flash == nil)
    }

    @Test func aBareExecutableSaysWhereContactsAreRead() async {
        book.setAccess(.unavailable)
        press("l")
        _ = await eventually { model.flash != nil }
        #expect(model.flash == L("Contacts are only read by the installed app (make open)"))
    }

    // MARK: Sender in Contacts

    /// A thread of messages (id, sender, folder role), each unfolded.
    func thread(_ senders: [(Int, String, String)]) -> ThreadView {
        let messages = senders.map { id, from, role in
            MessageView(
                id: id, folderId: 1, folderRole: role, subject: "S", fromName: "N", fromAddr: from,
                to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: "", isSeen: true,
                isFlagged: false, isAnswered: false, hasAttachments: false, authResult: "pass", bodyHtml: nil,
                hasRemoteContent: false, fromBodyFormat: nil, segments: [], onlyQuotes: false, expanded: true,
                showRecipients: false, attachments: [])
        }
        return ThreadView(id: 9, subject: "S", messages: messages, asOf: Position(epoch: "e", seq: 0))
    }

    @Test func eachReceivedSenderIsKnownOrNotOnceTheCardsAreRead() {
        let t = thread([(10, "Alice@Example.org", "inbox"), (11, "me@example.com", "sent"), (12, "dave@example.org", "inbox")])
        var cards: [ContactCard] = []
        cards.add(ContactCard(id: "A", name: "Alice", phones: []), foundFor: "alice@example.org")
        // Not read: nothing known, no icon (not a wrong "not in Contacts").
        #expect(SenderBook.statuses(t, cards: nil).isEmpty)
        // The user's own message gets none; the case of an address does
        // not matter.
        #expect(SenderBook.statuses(t, cards: cards) == [10: .known, 12: .unknown])
        #expect(SenderBook.statuses(t, cards: []) == [10: .unknown, 12: .unknown])
        #expect(SenderBook.card(of: "ALICE@example.org", in: cards)?.id == "A")
        #expect(SenderBook.card(of: "dave@example.org", in: cards) == nil)
    }

    @Test func aCardFoundForTwoAddressesIsListedOnceWithBoth() {
        var cards: [ContactCard] = []
        let card = ContactCard(id: "A", name: "Alice", phones: [])
        cards.add(card, foundFor: "alice@work.org")
        cards.add(card, foundFor: "Alice@Home.org")
        cards.add(card, foundFor: "alice@work.org")
        #expect(cards.count == 1)
        #expect(cards[0].emails == ["alice@work.org", "alice@home.org"])
    }

    @Test func theHeaderOfAnUnfoldedMessageCarriesItsIcon() {
        let t = thread([(10, "alice@example.org", "inbox"), (12, "dave@example.org", "inbox")])
        let blocks = ThreadDisplay.blocks(t, ThreadDisplay.State(showAll: true), book: [10: .known, 12: .unknown])
        let books = blocks.compactMap { b -> SenderBook? in
            if case .header(_, _, _, _, _, _, _, _, let book) = b { return book } else { return nil }
        }
        #expect(books == [.known, .unknown])
        // Folded, the line is short: no icon.
        let folded = ThreadDisplay.blocks(t, ThreadDisplay.State(folding: [10: false]), book: [10: .known])
        guard case .header(10, true, _, _, _, _, _, _, let book) = folded.first else { Issue.record("\(folded)"); return }
        #expect(book == nil)
    }

    @Test func theSenderActedOnIsTheCurrentMessageOrTheLatestReceivedBeforeIt() {
        let t = thread([(10, "alice@example.org", "inbox"), (11, "me@example.com", "sent"), (12, "bob@example.org", "inbox"),
                        (13, "me@example.com", "sent")])
        // No current message: the last unfolded one, the user's own reply:
        // Bob, who wrote before it.
        #expect(ThreadDisplay.senderMessage(t, ThreadDisplay.State())?.id == 12)
        #expect(ThreadDisplay.senderMessage(t, ThreadDisplay.State(current: 10))?.id == 10)
        #expect(ThreadDisplay.senderMessage(t, ThreadDisplay.State(current: 11))?.id == 10)
        // Only the user before it: the next mail received.
        let mine = thread([(11, "me@example.com", "sent"), (12, "bob@example.org", "inbox")])
        #expect(ThreadDisplay.senderMessage(mine, ThreadDisplay.State(current: 11))?.id == 12)
        // The user alone: their own address.
        let alone = thread([(11, "me@example.com", "sent")])
        #expect(ThreadDisplay.senderMessage(alone, ThreadDisplay.State())?.id == 11)
    }

    @Test func theModelKnowsTheSendersOnceTheBookIsRead() async {
        // Not asked yet: nothing known.
        #expect(model.senderBook.isEmpty)
        book.setAccess(.authorized)
        await model.loadSenderCards(asking: false)
        #expect(model.senderBook == [10: .known, 12: .known, 13: .known])
        #expect(model.senderCard("ALICE@example.org")?.id == "A1:ABPerson")
    }

    @Test func ctrlCOpensTheCardOfAKnownSender() async {
        book.setAccess(.authorized)
        await model.loadSenderCards(asking: false)
        var opened: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        // The current message: the last unfolded, Alice's (13).
        press("ctrl-c")
        _ = await eventually { !opened.isEmpty }
        #expect(opened == ["addressbook://A1:ABPerson"])
        #expect(model.flash == L("Card of %@ opened in Contacts", "Alice Martin"))
        #expect(controller.pasteboard.string(forType: .string) == nil)
    }

    @Test func ctrlCCopiesTheAddressOfAnUnknownSender() async {
        book.setAccess(.authorized)
        book.setCards([:])
        await model.loadSenderCards(asking: false)
        #expect(model.senderBook[13] == .unknown)
        var opened: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        press("ctrl-c")
        _ = await eventually { model.flash != nil }
        #expect(controller.pasteboard.string(forType: .string) == "alice@example.org")
        #expect(model.flash == L("Copied: %@", "alice@example.org"))
        #expect(opened.isEmpty)
    }

    @Test func ctrlCAsksForContactsTheFirstTime() async {
        // Never asked: ctrl-c asks, as `l` does, then opens the card.
        var opened: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        press("ctrl-c")
        _ = await eventually { !opened.isEmpty }
        #expect(book.access == .authorized)
        #expect(opened == ["addressbook://A1:ABPerson"])
    }

    @Test func ctrlCCopiesWhenTheBookCannotBeRead() async {
        book.setAccess(.denied)
        press("ctrl-c")
        _ = await eventually { controller.pasteboard.string(forType: .string) != nil }
        #expect(controller.pasteboard.string(forType: .string) == "alice@example.org")
    }

    @Test func aClickOnTheIconDoesWhatCtrlCDoesForItsMessage() async {
        book.setAccess(.authorized)
        await model.loadSenderCards(asking: false)
        var opened: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        #expect(controller.readerLink(URL(string: "nm://book/12")!, command: false))
        _ = await eventually { !opened.isEmpty }
        #expect(opened == ["addressbook://B1"])
    }

    @Test func aCardAddedInContactsShowsWithoutReopeningTheMail() async {
        book.setAccess(.authorized)
        book.setCards([:])
        await model.loadSenderCards(asking: false)
        #expect(model.senderBook[12] == .unknown)
        book.setCards(["bob@example.org": [ContactCard(id: "B1", name: "Bob Durand", phones: [])]])
        NotificationCenter.default.post(name: .CNContactStoreDidChange, object: nil)
        _ = await eventually { model.senderBook[12] == .known }
        #expect(model.senderBook[12] == .known)
        #expect(model.senderBook[13] == .unknown)
    }

    @Test func ctrlShiftCListsTheMailsOfTheSender() async {
        // The contacts of `Fixture` are c1@example.org… : thread 1's
        // latest mail comes from c2 for this test.
        let core = FakeCore { method, params in
            var answer = AddressBookTests.answer(method, params)
            if method == "get_thread", var t = answer as? [String: Any], var messages = t["messages"] as? [[String: Any]] {
                messages[3]["from_addr"] = "C2@example.org"
                t["messages"] = messages
                answer = t
            }
            return answer
        }
        await model.attach(core)
        model.selectedThreadId = 2
        _ = await eventually { model.openThread?.id == 2 }
        model.selectedThreadId = 1
        _ = await eventually { model.openThread?.messages.last?.fromAddr == "C2@example.org" }
        #expect(model.openThread?.messages.last?.fromAddr == "C2@example.org")
        controller.reading = true
        press("ctrl-C")
        _ = await eventually { model.listFilter != nil }
        #expect(model.listFilter == .contact(ContactView(id: 2, addr: "c2@example.org", name: nil, screeningTarget: nil,
                                                         groups: [], messageCount: 2, senderKind: "robot",
                                                         senderKindForced: true)))
        #expect(!controller.reading)
        // The core was asked for that address.
        #expect(core.calls.contains { $0.method == "list_contacts" && $0.params["query"] as? String == "c2@example.org" })
    }

    @Test func ctrlShiftCSaysWhenTheCoreHasNoSuchContact() async {
        // The thread's senders are not among the contacts of `Fixture`.
        press("ctrl-C")
        _ = await eventually { model.flash != nil }
        #expect(model.flash == L("No contact %@", "alice@example.org"))
        #expect(model.listFilter == nil)
    }
}
