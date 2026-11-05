import Foundation
import Testing
@testable import NiceMailer

/// Deep links to a mail (`DeepLink`): `nicemailer://m/<id>`.
struct DeepLinkTests {
    @Test func aMessageHasAShortLinkMadeOfItsId() {
        #expect(DeepLink.url(messageId: 48213).absoluteString == "nicemailer://m/48213")
    }

    @Test func theIdIsReadBackFromTheLink() {
        #expect(DeepLink.messageId(from: URL(string: "nicemailer://m/48213")!) == 48213)
        // What a note app or a hand may do to it.
        #expect(DeepLink.messageId(from: URL(string: "NiceMailer://M/48213/")!) == 48213)
        #expect(DeepLink.messageId(from: DeepLink.url(messageId: 7)) == 7)
    }

    @Test func otherURLsAreNotMessageLinks() {
        for text in [
            "https://m/48213", "nicemailer://t/48213", "nicemailer://m/", "nicemailer://m/abc",
            "nicemailer://m/12a", "nicemailer://m/-3", "nicemailer://m/0", "nicemailer://m/1/2",
            "nicemailer://m/99999999999999999999999",
        ] {
            #expect(DeepLink.messageId(from: URL(string: text)!) == nil, "\(text)")
        }
    }

    @Test func theMarkdownLinkKeepsASubjectWithBracketsWhole() {
        #expect(DeepLink.markdown(subject: "Devis toiture", messageId: 3) == "[Devis toiture](nicemailer://m/3)")
        #expect(DeepLink.markdown(subject: "[Spam] Offre \\ 50%", messageId: 3)
            == #"[\[Spam\] Offre \\ 50%](nicemailer://m/3)"#)
        #expect(DeepLink.markdown(subject: "  Deux\nlignes ", messageId: 3) == "[Deux lignes](nicemailer://m/3)")
        #expect(DeepLink.markdown(subject: " ", messageId: 3) == "[\(L("(no subject)"))](nicemailer://m/3)")
    }
}
