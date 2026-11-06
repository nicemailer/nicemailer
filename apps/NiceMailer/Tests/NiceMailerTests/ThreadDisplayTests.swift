import Foundation
import Testing
@testable import NiceMailer

/// Blocks of the geek reader (`ThreadDisplay`), built from segments as the
/// core sends them.
@MainActor
struct ThreadDisplayTests {
    func segment(_ kind: String, _ text: String, hidden: Bool, depth: Int = 0, inThread: Bool = true) -> SegmentView {
        SegmentView(kind: kind, text: text, depth: depth, hidden: hidden, inThread: inThread)
    }

    func message(_ id: Int, from: String, segments: [SegmentView], expanded: Bool, onlyQuotes: Bool = false,
                 recipients: Bool = false, files: [AttachmentView] = []) -> MessageView {
        MessageView(
            id: id, folderId: 1, folderRole: "archive", subject: "S", fromName: from, fromAddr: "\(from)@x.org",
            to: [AddressView(addr: "me@x.org", name: nil)], cc: [], dateSent: nil, dateReceived: 1_700_000_000,
            bodyText: segments.map(\.text).joined(separator: "\n"), isSeen: true, isFlagged: false, isAnswered: false,
            hasAttachments: false, authResult: "unknown", bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: segments, onlyQuotes: onlyQuotes, expanded: expanded, showRecipients: recipients,
            attachments: files)
    }

    var thread: ThreadView {
        ThreadView(id: 9, subject: "S", messages: [
            message(1, from: "Alice", segments: [
                segment("text", "Peux-tu passer jeudi ?", hidden: false),
                segment("signature", "Alice Martin\nAtelier", hidden: true),
            ], expanded: false),
            message(2, from: "Bob", segments: [
                segment("quote", "jeudi ?", hidden: false, depth: 1),
                segment("text", "Oui, 10 h.", hidden: false),
                segment("quote_tail", "Le 2 nov., Alice a écrit :\nPeux-tu passer jeudi ?", hidden: true),
            ], expanded: true, recipients: true),
            message(3, from: "Carol", segments: [
                segment("quote_tail", "old", hidden: true, inThread: false),
            ], expanded: false, onlyQuotes: true),
        ], asOf: Position(epoch: "e", seq: 0))
    }

    @Test func foldedMessagesAreOneLineWithASnippet() {
        let blocks = ThreadDisplay.blocks(thread, .init())
        #expect(ThreadDisplay.foldedCount(blocks) == 2)
        guard case .header(1, true, let title, let detail, _, _, _, _) = blocks[0] else { Issue.record("\(blocks[0])"); return }
        // The arrow (a conversation), then name · date · @domain (`ui.md`,
        // "Expéditeur").
        #expect(title == "▸ Alice · \(Format.geekMessageDate(1_700_000_000)) · @x.org")
        #expect(detail == "Peux-tu passer jeudi ?")
        // A message that only quotes says so instead of a snippet.
        let last = blocks.last { if case .header(3, _, _, _, _, _, _, _) = $0 { return true } else { return false } }
        if case .header(_, true, _, let d, _, _, _, _) = last { #expect(d == L("only quotes earlier messages")) }
    }

    @Test func unfoldedMessagesShowVisibleSegmentsAndHiddenLines() {
        let blocks = ThreadDisplay.blocks(thread, .init())
        #expect(blocks.contains(.segment(messageId: 2, index: 0, kind: "quote", text: "jeudi ?", depth: 1)))
        #expect(blocks.contains(.segment(messageId: 2, index: 1, kind: "text", text: "Oui, 10 h.", depth: 0)))
        #expect(blocks.contains(.hidden(messageId: 2, index: 2, label: L("··· %lld quoted lines", 2))))
        // One line: arrow, name · date, then the address in full; the
        // recipients below, only when the core says they changed.
        #expect(blocks.contains(.header(messageId: 2, folded: false, title: "▾ Bob · \(Format.geekMessageDate(1_700_000_000))",
                                        detail: L("To: %@", "me@x.org"), address: "Bob@x.org", alert: false, foldable: true)))
    }

    @Test func aSingleMailHasNoArrowAndASenderWithoutNameItsAddressOnce() {
        var m = message(4, from: "x", segments: [segment("text", "Hi", hidden: false)], expanded: false)
        m = MessageView(id: 4, folderId: 1, folderRole: "archive", subject: "S", fromName: nil, fromAddr: "zoe@x.org",
                        to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: "Hi", isSeen: true,
                        isFlagged: false, isAnswered: false, hasAttachments: false, authResult: "unknown", bodyHtml: nil,
                        hasRemoteContent: false, fromBodyFormat: nil, segments: m.segments, onlyQuotes: false,
                        expanded: false, showRecipients: false, attachments: [])
        let blocks = ThreadDisplay.blocks(ThreadView(id: 1, subject: "S", messages: [m], asOf: Position(epoch: "e", seq: 0)), .init())
        // A single mail: unfolded although the core says folded, no arrow,
        // no folding; the address is the name, not repeated.
        guard case .header(4, false, let title, _, _, let address, _, let foldable) = blocks[0] else { Issue.record("\(blocks)"); return }
        #expect(title == "zoe@x.org · \(Format.geekMessageDate(1_700_000_000))")
        #expect(address == nil && !foldable)
    }

    @Test func theDomainsASendersNameShows() {
        #expect(ThreadDisplay.domain("Alice@Atelier.FR") == "atelier.fr")
        #expect(ThreadDisplay.domain("nobody") == nil)
        #expect(ThreadDisplay.namedDomains("service@paypal.com") == ["paypal.com"])
        #expect(ThreadDisplay.namedDomains("PayPal.com Service") == ["paypal.com"])
        #expect(ThreadDisplay.namedDomains("Le Monde.fr") == ["monde.fr"])
        #expect(ThreadDisplay.namedDomains("Alice Martin").isEmpty)
        #expect(ThreadDisplay.namedDomains("M. Dupont, Sr.").isEmpty)
        #expect(ThreadDisplay.namedDomains("v1.2 release").isEmpty)
    }

    @Test func aSenderPassingForAnotherIsInTheAlertColour() {
        func sender(_ name: String?, _ address: String, auth: String = "pass") -> MessageView {
            MessageView(id: 1, folderId: 1, folderRole: "inbox", subject: "S", fromName: name, fromAddr: address,
                        to: [], cc: [], dateSent: nil, dateReceived: 1, bodyText: "x", isSeen: true, isFlagged: false,
                        isAnswered: false, hasAttachments: false, authResult: auth, bodyHtml: nil, hasRemoteContent: false,
                        fromBodyFormat: nil, segments: [], onlyQuotes: false, expanded: true, showRecipients: false,
                        attachments: [])
        }
        // Another address or domain in the name.
        #expect(ThreadDisplay.senderAlert(sender("service@paypal.com", "x7@promo-mail.ru")))
        #expect(ThreadDisplay.senderAlert(sender("PayPal.com", "x7@promo-mail.ru")))
        // The same domain, or one it ends with.
        #expect(!ThreadDisplay.senderAlert(sender("service@paypal.com", "service@paypal.com")))
        #expect(!ThreadDisplay.senderAlert(sender("Le Monde.fr", "news@info.lemonde.fr")))
        #expect(!ThreadDisplay.senderAlert(sender("Alice Martin", "alice@atelier.fr")))
        #expect(!ThreadDisplay.senderAlert(sender(nil, "alice@atelier.fr")))
        // A failed authentication, whatever the name.
        #expect(ThreadDisplay.senderAlert(sender("Alice Martin", "alice@atelier.fr", auth: "fail")))
        #expect(!ThreadDisplay.senderAlert(sender("Alice Martin", "alice@atelier.fr", auth: "none")))
    }

    @Test func showAllUnfoldsAndRevealsEverything() {
        let blocks = ThreadDisplay.blocks(thread, .init(showAll: true))
        #expect(ThreadDisplay.foldedCount(blocks) == 0)
        #expect(!blocks.contains { if case .hidden = $0 { return true } else { return false } })
        #expect(blocks.contains(.segment(messageId: 1, index: 1, kind: "signature", text: "Alice Martin\nAtelier", depth: 0)))
    }

    @Test func clicksUnfoldAMessageAndRevealOnePart() {
        var state = ThreadDisplay.State()
        state.folding[1] = true
        var blocks = ThreadDisplay.blocks(thread, state)
        #expect(blocks.contains(.segment(messageId: 1, index: 0, kind: "text", text: "Peux-tu passer jeudi ?", depth: 0)))
        #expect(blocks.contains(.hidden(messageId: 1, index: 1, label: L("··· signature"))))
        state.revealed.insert("1:1")
        blocks = ThreadDisplay.blocks(thread, state)
        #expect(blocks.contains(.segment(messageId: 1, index: 1, kind: "signature", text: "Alice Martin\nAtelier", depth: 0)))
        // Folding the latest message.
        state.folding[2] = false
        #expect(ThreadDisplay.foldedCount(ThreadDisplay.blocks(thread, state)) == 2)
    }

    @Test func aTailMissingFromTheThreadSaysSo() {
        let s = segment("quote_tail", "a\nb\nc", hidden: true, inThread: false)
        #expect(ThreadDisplay.label(s) == L("··· %lld quoted lines, not in this conversation", 3))
    }

    @Test func skipGoesToTheTextAfterTheNextQuote() {
        let quotes = [(100.0, 180.0), (300.0, 340.0)]
        let texts = [0.0, 180.0, 340.0]
        #expect(ThreadDisplay.skipTarget(from: 0, quotes: quotes, texts: texts) == 180)
        #expect(ThreadDisplay.skipTarget(from: 150, quotes: quotes, texts: texts) == 180)
        #expect(ThreadDisplay.skipTarget(from: 180, quotes: quotes, texts: texts) == 340)
        #expect(ThreadDisplay.skipTarget(from: 400, quotes: quotes, texts: texts) == nil)
    }
}
