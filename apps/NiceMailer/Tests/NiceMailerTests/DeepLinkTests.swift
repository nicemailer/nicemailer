import Foundation
import Testing
@testable import NiceMailer

/// Deep links to a mail (`DeepLink`): `nicemailer://m/<key>`, and the old
/// `nicemailer://m/<id>`.
struct DeepLinkTests {
    /// A message as the core gives it, with or without its link.
    private func message(id: Int, link: String?) -> MessageView {
        let json: [String: Any] = [
            "id": id, "folder_id": 1, "folder_role": NSNull(), "subject": "s", "from_name": NSNull(), "from_addr": "a@b.fr",
            "to": [], "cc": [], "date_sent": NSNull(), "date_received": 0, "body_text": "x", "is_seen": true,
            "is_flagged": false, "is_answered": false, "has_attachments": false, "auth_result": "pass",
            "body_html": NSNull(), "has_remote_content": false, "from_body_format": NSNull(), "segments": [],
            "only_quotes": false, "no_text": false, "expanded": true, "show_recipients": false, "attachments": [], "tracking_pixels": 0, "links": [],
            "link": link ?? NSNull(),
        ]
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try! decoder.decode(MessageView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    @Test func theLinkOfAMessageIsTheOneTheCoreWrote() {
        #expect(DeepLink.url(of: message(id: 48213, link: "nicemailer://m/3f9a0c2b7e41")).absoluteString
            == "nicemailer://m/3f9a0c2b7e41")
        // No link from the core (a test, an older core): the id.
        #expect(DeepLink.url(of: message(id: 48213, link: nil)).absoluteString == "nicemailer://m/48213")
        // A link this app would not read back is not copied.
        #expect(DeepLink.url(of: message(id: 7, link: "https://example.org")).absoluteString == "nicemailer://m/7")
        #expect(DeepLink.url(messageId: 48213).absoluteString == "nicemailer://m/48213")
    }

    @Test func aKeyIsReadBackFromTheLinkInAnyCase() {
        #expect(DeepLink.target(from: URL(string: "nicemailer://m/3f9a0c2b7e41")!) == .key("3f9a0c2b7e41"))
        // What a note app or a hand may do to it.
        #expect(DeepLink.target(from: URL(string: "NiceMailer://M/3F9A0C2B7E41/")!) == .key("3f9a0c2b7e41"))
        // Twelve digits are a key too: no id is that long.
        #expect(DeepLink.target(from: URL(string: "nicemailer://m/123456789012")!) == .key("123456789012"))
        #expect(DeepLink.Target.key("3f9a0c2b7e41").params["link_key"] as? String == "3f9a0c2b7e41")
    }

    @Test func anOldLinkStillGivesItsId() {
        #expect(DeepLink.target(from: URL(string: "nicemailer://m/48213")!) == .id(48213))
        #expect(DeepLink.target(from: URL(string: "NiceMailer://M/48213/")!) == .id(48213))
        #expect(DeepLink.target(from: DeepLink.url(messageId: 7)) == .id(7))
        #expect(DeepLink.target(from: URL(string: "nicemailer://m/99999999999")!) == .id(99_999_999_999))
        #expect(DeepLink.Target.id(7).params["message_id"] as? Int == 7)
    }

    @Test func otherURLsAreNotMessageLinks() {
        for text in [
            "https://m/48213", "nicemailer://t/48213", "nicemailer://m/", "nicemailer://m/abc",
            "nicemailer://m/12a", "nicemailer://m/-3", "nicemailer://m/0", "nicemailer://m/1/2",
            "nicemailer://m/99999999999999999999999",
            // Not 12 hexadecimal characters.
            "nicemailer://m/3f9a0c2b7e4", "nicemailer://m/3f9a0c2b7e410", "nicemailer://m/3f9a0c2b7e4g",
        ] {
            #expect(DeepLink.target(from: URL(string: text)!) == nil, "\(text)")
        }
    }

    @Test func theMarkdownLinkKeepsASubjectWithBracketsWhole() {
        let url = URL(string: "nicemailer://m/3f9a0c2b7e41")!
        #expect(DeepLink.markdown(subject: "Devis toiture", url: url) == "[Devis toiture](nicemailer://m/3f9a0c2b7e41)")
        #expect(DeepLink.markdown(subject: "[Spam] Offre \\ 50%", url: url)
            == #"[\[Spam\] Offre \\ 50%](nicemailer://m/3f9a0c2b7e41)"#)
        #expect(DeepLink.markdown(subject: "  Deux\nlignes ", url: url) == "[Deux lignes](nicemailer://m/3f9a0c2b7e41)")
        #expect(DeepLink.markdown(subject: " ", url: url) == "[\(L("(no subject)"))](nicemailer://m/3f9a0c2b7e41)")
    }
}
