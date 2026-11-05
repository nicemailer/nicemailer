import Foundation
import Testing
@testable import NiceMailer

/// The links of the `l` panel (`AppModel.Link`): the links the core gives
/// for each message, merged into one list for the thread.
struct LinkTests {
    private func view(_ url: String, _ text: String? = nil) -> LinkView { LinkView(url: url, text: text) }

    @Test func theLinksOfAThreadAreMergedInOrderEachUrlOnce() {
        let merged = AppModel.Link.merge([
            [view("https://x.org/a", "Lire"), view("https://x.org/b")],
            // A reply quoting the first message repeats its links.
            [view("https://x.org/b", "Le plan"), view("https://x.org/a", "Autre texte"), view("https://x.org/c")],
        ])
        #expect(merged.map(\.url.absoluteString) == ["https://x.org/a", "https://x.org/b", "https://x.org/c"])
        // The first text given stays; an address without text takes one
        // found later.
        #expect(merged.map(\.text) == ["Lire", "Le plan", nil])
    }

    @Test func linksWithATextComeFirstThenPhonesThenLinksWithoutText() {
        let merged = AppModel.Link.merge([[
            view("https://instagram.example/lettre"),
            view("https://x.org/lire", "Lire en ligne"),
            view("tel:0123456789", "01 23 45 67 89"),
            view("https://x.org/plain"),
            view("https://x.org/facture", "Voir ma facture"),
        ]])
        #expect(merged.map(\.label) == [
            "Lire en ligne", "Voir ma facture", "01 23 45 67 89", "instagram.example", "x.org",
        ])
    }

    @Test func aPhoneNumberIsOnceWhateverItsForm() {
        // The same number, international in a quoted signature.
        let merged = AppModel.Link.merge([
            [view("tel:0612345678", "06 12 34 56 78")],
            [view("tel:+33612345678", "+33 6 12 34 56 78"), view("tel:0123456789", "01 23 45 67 89")],
        ])
        #expect(merged.map(\.label) == ["06 12 34 56 78", "01 23 45 67 89"])
        #expect(merged.allSatisfy { $0.isPhone && $0.icon == "phone" })
    }

    @Test func aPhoneRowShowsTheNumberOnce() {
        let written = AppModel.Link(url: URL(string: "tel:0612345678")!, text: "06 12 34 56 78")
        #expect(written.label == "06 12 34 56 78" && written.destination == nil)
        // A link of the HTML with a text of its own: the number on the right.
        let named = AppModel.Link(url: URL(string: "tel:+33612345678")!, text: "Nous appeler")
        #expect(named.label == "Nous appeler" && named.destination == "+33612345678")
        let bare = AppModel.Link(url: URL(string: "tel:+33612345678")!, text: nil)
        #expect(bare.label == "+33612345678" && bare.destination == nil)
        #expect(AppModel.Link(url: URL(string: "https://x.org")!, text: nil).icon == "link")
    }

    @Test func aUrlFoundationCannotReadIsLeftOutAndASpaceIsEncoded() {
        // A space a sender left in an `href` is encoded: the link opens.
        #expect(AppModel.Link.merge([[view(""), view("https://x.org/a b")]]).map(\.url.absoluteString)
            == ["https://x.org/a%20b"])
    }

    @Test func theRowShowsTheTextAndWhereTheLinkGoes() {
        let web = AppModel.Link(url: URL(string: "https://t.example.com/c?u=1")!, text: "Voir ma facture")
        // The full address on the right, to read before opening it.
        #expect(web.label == "Voir ma facture" && web.destination == "https://t.example.com/c?u=1")
        let mail = AppModel.Link(url: URL(string: "mailto:aide@x.org")!, text: "Support")
        #expect(mail.label == "Support" && mail.destination == "aide@x.org")
        // Without text: the site on the left, the full address on the right.
        let bare = AppModel.Link(url: URL(string: "https://www.facebook.com/lettre?utm=1")!, text: nil)
        #expect(bare.label == "www.facebook.com" && bare.destination == "https://www.facebook.com/lettre?utm=1")
        let bareMail = AppModel.Link(url: URL(string: "mailto:moi@x.org")!, text: nil)
        #expect(bareMail.label == "moi@x.org" && bareMail.destination == nil)
    }

    @MainActor @Test func theLinksPanelTakesTheWidthOfTheWindowUpToALimit() {
        // A wide window: 1,100 points at most.
        #expect(PickerPopup.width(.links, available: 1400, scale: 1) == 1100)
        // Narrower: the window less a margin of 40 on each side.
        #expect(PickerPopup.width(.links, available: 900, scale: 1) == 820)
        // Never narrower than the other pickers, never wider than the window.
        #expect(PickerPopup.width(.links, available: 560, scale: 1) == 520)
        #expect(PickerPopup.width(.links, available: 500, scale: 1) == 500)
        // A bigger text size widens the limit too.
        #expect(PickerPopup.width(.links, available: 3000, scale: 1.5) == 1650)
        // The other pickers keep their width.
        #expect(PickerPopup.width(.tag, available: 1400, scale: 1) == 520)
        #expect(PickerPopup.width(.search, available: 1400, scale: 1) == 680)
    }
}
