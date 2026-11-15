import Contacts
import Foundation

// The Mac's address book (Contacts), looked up for the senders of the open
// thread: their phone numbers and their card join the links of `l`
// (`docs/geek-ui.md`, "Links of the mail"; `ui.md`).
//
// Nothing of the address book is copied into NiceMailer's database: the
// book belongs to the user and changes on its own, so it is read on demand
// and kept only for the session (`AppModel.senderCards`).
//
// - `AddressBook`: what the app needs from a book, so that tests and the UI
//   scenarios use a book of their own (`FixtureAddressBook`) and never the
//   user's;
// - `SystemAddressBook`: the Contacts framework;
// - `ContactLinks`: the rows of `l`, and whether each sender of the open
//   thread is in the book (`SenderBook`, the icon at the end of its
//   header line), as pure functions.

/// Whether the app may read the address book.
enum AddressBookAccess: Equatable, Sendable {
    /// macOS has not asked the user yet: asked on the first `l`.
    case notDetermined
    case authorized
    /// Refused (or restricted by the Mac's administrator): changed only
    /// in System Settings.
    case denied
    /// The app cannot ask: a bare executable (`make app`) has no
    /// `Info.plist`, hence no reason to show in macOS's question, and
    /// asking without one stops the app. Only `NiceMailer.app` asks
    /// (`make bundle`).
    case unavailable
}

/// A card of the address book that holds the address of a sender.
struct ContactCard: Equatable, Sendable, Identifiable {
    /// The book's identifier of the card (`CNContact.identifier`).
    let id: String
    /// "Alice Martin", or the company for a company card.
    let name: String
    /// The phone numbers, in the card's order.
    let phones: [Phone]
    /// The addresses of the lookup this card was found for, in lowercase:
    /// filled by `cards(matching:)`, so that each sender of a thread can be
    /// told apart (one card may hold several of them). Empty for a card
    /// built by hand.
    var emails: [String] = []

    struct Phone: Equatable, Sendable {
        /// "mobile", "work"…, translated by macOS; nil without a label.
        let label: String?
        /// As written in the card: "+33 6 12 34 56 78".
        let number: String
    }
}

/// What the app needs from an address book.
///
/// Swift note: a protocol marked `Sendable` can be used from any task;
/// lookups run away from the main thread.
protocol AddressBook: Sendable {
    var access: AddressBookAccess { get }
    /// Asks the user (macOS's own question, once); true if granted.
    func requestAccess() async -> Bool
    /// The cards holding one of `emails`, each once, in the order of the
    /// addresses (the cards of the first address first); each card lists
    /// the addresses of `emails` it holds (`ContactCard.emails`).
    func cards(matching emails: [String]) async -> [ContactCard]
}

extension Array where Element == ContactCard {
    /// Adds `card`, found for `email`: a card already found for another
    /// address only records this one too (a card is listed once).
    ///
    /// Swift note: an extension with a `where` clause adds methods to
    /// arrays of cards only; `mutating` lets the method change the array
    /// it is called on.
    mutating func add(_ card: ContactCard, foundFor email: String) {
        let email = email.lowercased()
        if let i = firstIndex(where: { $0.id == card.id }) {
            if !self[i].emails.contains(email) { self[i].emails.append(email) }
        } else {
            var found = card
            found.emails = [email]
            append(found)
        }
    }
}

/// The Mac's address book, through the Contacts framework.
///
/// Swift note: `@unchecked Sendable` tells the compiler this class may be
/// shared between tasks although it cannot check it: `CNContactStore` is
/// documented as safe to use from any thread, and the class holds nothing
/// else.
final class SystemAddressBook: AddressBook, @unchecked Sendable {
    private let store = CNContactStore()

    /// Whether this process can ask macOS at all (see `unavailable`).
    static var canAsk: Bool {
        Bundle.main.object(forInfoDictionaryKey: "NSContactsUsageDescription") != nil
    }

    var access: AddressBookAccess {
        guard Self.canAsk else { return .unavailable }
        switch CNContactStore.authorizationStatus(for: .contacts) {
        case .notDetermined: return .notDetermined
        // `.limited` (macOS 15): the user chose some cards; they are the
        // only ones a lookup finds.
        case .denied, .restricted: return .denied
        default: return .authorized
        }
    }

    func requestAccess() async -> Bool {
        guard Self.canAsk else { return false }
        return (try? await store.requestAccess(for: .contacts)) ?? false
    }

    func cards(matching emails: [String]) async -> [ContactCard] {
        guard access == .authorized else { return [] }
        let store = self.store
        // The lookups are synchronous and may take a moment on a large
        // book: done on a background thread.
        return await Task.detached {
            let keys: [CNKeyDescriptor] = [
                CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
                CNContactOrganizationNameKey as CNKeyDescriptor,
                CNContactPhoneNumbersKey as CNKeyDescriptor,
            ]
            var out: [ContactCard] = []
            for email in emails {
                // An indexed search on the card's addresses, whatever
                // their case.
                let predicate = CNContact.predicateForContacts(matchingEmailAddress: email)
                guard let found = try? store.unifiedContacts(matching: predicate, keysToFetch: keys) else { continue }
                for contact in found {
                    // A card already found for another address of the
                    // thread: it only records this one too.
                    if let known = out.first(where: { $0.id == contact.identifier }) {
                        out.add(known, foundFor: email)
                        continue
                    }
                    let name = CNContactFormatter.string(from: contact, style: .fullName)
                        ?? (contact.organizationName.isEmpty ? email : contact.organizationName)
                    let phones = contact.phoneNumbers.map { labeled in
                        ContactCard.Phone(label: labeled.label.map { CNLabeledValue<CNPhoneNumber>.localizedString(forLabel: $0) },
                                          number: labeled.value.stringValue)
                    }
                    out.add(ContactCard(id: contact.identifier, name: name, phones: phones), foundFor: email)
                }
            }
            return out
        }.value
    }
}

/// A book given by the code: the unit tests and the UI scenarios, which
/// must never read the user's cards. Access starts as given; `grant`
/// says what macOS's question would answer.
final class FixtureAddressBook: AddressBook, @unchecked Sendable {
    private let lock = NSLock()
    private var state: AddressBookAccess
    private let grant: Bool
    private var byEmail: [String: [ContactCard]]
    /// The addresses looked up, for the tests.
    private(set) var lookups: [[String]] = []

    init(access: AddressBookAccess = .authorized, grant: Bool = true, cards: [String: [ContactCard]]) {
        state = access
        self.grant = grant
        byEmail = cards
    }

    var access: AddressBookAccess { lock.withLock { state } }

    func setAccess(_ access: AddressBookAccess) { lock.withLock { state = access } }

    /// Changes the cards, as the user would in Contacts (then macOS posts
    /// `CNContactStoreDidChange`, which the caller posts itself).
    func setCards(_ cards: [String: [ContactCard]]) { lock.withLock { byEmail = cards } }

    func requestAccess() async -> Bool {
        lock.withLock {
            guard state == .notDetermined else { return state == .authorized }
            state = grant ? .authorized : .denied
            return grant
        }
    }

    func cards(matching emails: [String]) async -> [ContactCard] {
        lock.withLock {
            lookups.append(emails)
            guard state == .authorized else { return [] }
            var out: [ContactCard] = []
            for email in emails {
                for card in byEmail[email.lowercased()] ?? [] { out.add(card, foundFor: email) }
            }
            return out
        }
    }

    /// The book of the UI scenarios (`UITests/fixtures/sample.mbox`):
    /// Alice and Bob, the senders of "Chantier". Alice's work number is
    /// also in the signature of one of her mails, as written there. The
    /// newsletter's sender is not in it.
    static func sample() -> FixtureAddressBook {
        FixtureAddressBook(access: .notDetermined, cards: [
            "alice@example.org": [ContactCard(id: "card-alice", name: "Alice Martin", phones: [
                .init(label: "mobile", number: "+33 6 12 34 56 78"),
                .init(label: "work", number: "01 44 55 66 77"),
            ])],
            "bob@example.org": [ContactCard(id: "card-bob", name: "Bob Durand", phones: [
                .init(label: nil, number: "06 98 76 54 32"),
            ])],
        ])
    }
}

/// The rows the cards of the senders add to `l`, as pure functions.
enum ContactLinks {
    /// The links of `cards`: for each card its phone numbers, then the
    /// card itself (opened in Contacts). A number shared by two cards (a
    /// switchboard) is listed once, under the first.
    static func links(_ cards: [ContactCard]) -> [AppModel.Link] {
        var out: [AppModel.Link] = []
        var seen = Set<String>()
        for card in cards {
            for phone in card.phones {
                guard let url = telURL(phone.number) else { continue }
                var link = AppModel.Link(url: url, text: phoneText(card.name, phone.label))
                link.shownNumber = phone.number
                guard seen.insert(link.key).inserted else { continue }
                out.append(link)
            }
            if let url = cardURL(card.id) {
                out.append(AppModel.Link(url: url, text: L("Card of %@", card.name)))
            }
        }
        return out
    }

    /// "Alice Martin — mobile", or the name alone without a label.
    static func phoneText(_ name: String, _ label: String?) -> String {
        guard let label, !label.isEmpty else { return name }
        return "\(name) — \(label)"
    }

    /// The `tel:` URL of a number as a card writes it: the digits, with
    /// the `+` of an international number. Nil with fewer than three
    /// digits. An extension ("p123", ", 12") is dropped: the phone app
    /// would not dial it.
    static func telURL(_ number: String) -> URL? {
        let trimmed = number.trimmingCharacters(in: .whitespaces)
        // The extension starts at the first letter or comma after a digit.
        let main = trimmed.prefix { !$0.isLetter && $0 != "," && $0 != ";" }
        let digits = main.filter { $0.isASCII && $0.isNumber }
        guard digits.count >= 3 else { return nil }
        let plus = main.first == "+" ? "+" : ""
        return URL(string: "tel:\(plus)\(digits)")
    }

    /// The URL that opens a card in Contacts: `addressbook://<identifier>`,
    /// the identifier as the book gives it ("410FE041-…-3DBAC:ABPerson"):
    /// Contacts reads it unencoded. An identifier with other characters
    /// than letters, digits, `-` and `:` gives no URL (no card row) rather
    /// than one Contacts would not understand.
    static func cardURL(_ id: String) -> URL? {
        guard !id.isEmpty, id.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == ":") })
        else { return nil }
        return URL(string: "addressbook://\(id)")
    }
}

/// Whether the sender of a message is in the address book: the icon at the
/// end of its header line in the reader (`docs/geek-ui.md`, "Sender in
/// Contacts"; `ui.md`, "Expéditeur"). `ctrl-c` opens the card of a known
/// sender, and copies the address of an unknown one.
enum SenderBook: Hashable, Sendable {
    case known
    case unknown

    /// The SF Symbol drawn: an address book for a sender in it; a person
    /// with a question mark otherwise (SF Symbols has no struck-out book).
    var symbol: String {
        switch self {
        case .known: "book.closed.fill"
        case .unknown: "person.crop.circle.badge.questionmark"
        }
    }

    /// The tooltip, and what VoiceOver reads.
    var label: String {
        switch self {
        case .known: L("In Contacts (ctrl-c opens the card)")
        case .unknown: L("Not in Contacts (ctrl-c copies the address)")
        }
    }

    /// What UI scripts read (`reader_books`).
    var name: String { self == .known ? "known" : "unknown" }

    /// The status of each message of `thread` whose sender was looked up,
    /// by message id. `cards`: the cards read for this thread; nil when
    /// they were not (book not allowed, or not read yet): then nothing is
    /// known and no icon is drawn, rather than a wrong "not in Contacts".
    /// The user's own messages (Sent, Drafts) get none: their sender is
    /// not looked up (`AppModel.threadSenders`).
    static func statuses(_ thread: ThreadView, cards: [ContactCard]?) -> [Int: SenderBook] {
        guard let cards else { return [:] }
        let known = Set(cards.flatMap(\.emails))
        var out: [Int: SenderBook] = [:]
        for m in thread.messages where m.folderRole != "sent" && m.folderRole != "drafts" {
            guard let addr = m.fromAddr?.lowercased(), !addr.isEmpty else { continue }
            out[m.id] = known.contains(addr) ? .known : .unknown
        }
        return out
    }

    /// The card of `address` among `cards`, whatever its case.
    static func card(of address: String, in cards: [ContactCard]) -> ContactCard? {
        let addr = address.lowercased()
        return cards.first { $0.emails.contains(addr) }
    }
}
