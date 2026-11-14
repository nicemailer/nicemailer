import AppKit
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
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
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

    /// One key: a character, `ret` or `esc`.
    func press(_ key: String) {
        let (chars, code) = key == "ret" ? ("\r", KeyCode.returnKey) : key == "esc" ? ("\u{1b}", KeyCode.escape) : (key, 0)
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
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
}
