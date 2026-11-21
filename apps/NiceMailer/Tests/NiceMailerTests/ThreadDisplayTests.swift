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
        guard case .header(1, true, let title, let detail, _, _, _, _, _) = blocks[0] else { Issue.record("\(blocks[0])"); return }
        // The arrow (a conversation), then name · date · @domain (`ui.md`,
        // "Expéditeur").
        #expect(title == "▸ Alice · \(Format.geekMessageDate(1_700_000_000)) · @x.org")
        #expect(detail == "Peux-tu passer jeudi ?")
        // A message that only quotes says so instead of a snippet.
        let last = blocks.last { if case .header(3, _, _, _, _, _, _, _, _) = $0 { return true } else { return false } }
        if case .header(_, true, _, let d, _, _, _, _, _) = last { #expect(d == L("only quotes earlier messages")) }
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
        guard case .header(4, false, let title, _, _, let address, _, let foldable, _) = blocks[0] else { Issue.record("\(blocks)"); return }
        #expect(title == "zoe@x.org · \(Format.geekMessageDate(1_700_000_000))")
        #expect(address == nil && !foldable)
    }

    @Test func ctrlKAndCtrlJGoUpAndDownTheConversation() throws {
        // Messages 1 (folded), 2 (unfolded, the last unfolded), 3 (folded).
        let t = thread
        // Up from the last unfolded: 2 folds, 1 unfolds.
        let up = try #require(ThreadDisplay.step(t, .init(), by: -1))
        #expect(up.message == 1 && up.state.current == 1)
        #expect(up.state.folding == [2: false, 1: true])
        // No further up.
        #expect(ThreadDisplay.step(t, up.state, by: -1) == nil)
        // Down twice: 2, then 3; then the end.
        let down = try #require(ThreadDisplay.step(t, up.state, by: 1))
        #expect(down.message == 2 && down.state.folding == [1: false, 2: true])
        let last = try #require(ThreadDisplay.step(t, down.state, by: 1))
        #expect(last.message == 3 && last.state.folding[3] == true && last.state.folding[2] == false)
        #expect(ThreadDisplay.step(t, last.state, by: 1) == nil)
        // With T, nothing folds: only the current message moves.
        let all = try #require(ThreadDisplay.step(t, .init(showAll: true), by: -1))
        #expect(all.message == 2 && all.state.folding.isEmpty)
    }

    @Test func aSingleMailHasNowhereToGo() {
        let one = ThreadView(id: 1, subject: "S", messages: [thread.messages[0]], asOf: Position(epoch: "e", seq: 0))
        #expect(ThreadDisplay.step(one, .init(), by: -1) == nil)
        #expect(ThreadDisplay.step(one, .init(), by: 1) == nil)
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

    /// The recipients line, shared by the text and HTML readers so both
    /// show the same header: when the core says they are worth it, or
    /// always under `T`; To and Cc, by name.
    @Test func theRecipientsLineFollowsTheCoreOrT() {
        var m = message(1, from: "Alice", segments: [], expanded: true, recipients: true)
        #expect(ThreadDisplay.recipients(m, .init()) == L("To: %@", "me@x.org"))
        m = MessageView(
            id: 1, folderId: 1, folderRole: "archive", subject: "S", fromName: "Alice", fromAddr: "alice@x.org",
            to: [AddressView(addr: "h@x.org", name: "Hélène"), AddressView(addr: "e@x.org", name: "Elsa")],
            cc: [AddressView(addr: "c@x.org", name: nil)], dateSent: nil, dateReceived: 1_700_000_000,
            bodyText: "", isSeen: true, isFlagged: false, isAnswered: false, hasAttachments: false,
            authResult: "unknown", bodyHtml: "<p>x</p>", hasRemoteContent: false, fromBodyFormat: nil,
            segments: [], onlyQuotes: false, expanded: true, showRecipients: true, attachments: [])
        #expect(ThreadDisplay.recipients(m, .init())
                == L("To: %@", "Hélène, Elsa") + "   " + L("Cc: %@", "c@x.org"))
        // Not worth showing: nothing, unless `T` shows everything.
        let quiet = message(2, from: "Bob", segments: [], expanded: true, recipients: false)
        #expect(ThreadDisplay.recipients(quiet, .init()) == nil)
        #expect(ThreadDisplay.recipients(quiet, .init(showAll: true)) == L("To: %@", "me@x.org"))
        // The text reader's header carries the same line.
        let blocks = ThreadDisplay.blocks(ThreadView(id: 1, subject: "S", messages: [m], asOf: Position(epoch: "e", seq: 0)), .init())
        guard case .header(1, false, _, let detail, _, _, _, _, _) = blocks.first else { Issue.record("\(blocks)"); return }
        #expect(detail == ThreadDisplay.recipients(m, .init()))
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

    /// `l` links the message the reader is on, so that each message of a
    /// conversation can be linked from a note; else the latest received.
    @Test func theLinkNamesTheMessageTheReaderIsOn() {
        #expect(ThreadDisplay.linkMessage(thread, .init())?.id == 3)
        var s = ThreadDisplay.State()
        s.current = 1
        #expect(ThreadDisplay.linkMessage(thread, s)?.id == 1)
        // A current message of another thread (a state kept by id): the
        // latest.
        s.current = 99
        #expect(ThreadDisplay.linkMessage(thread, s)?.id == 3)
        // The user's own latest mail is not the default: the latest
        // received is, so that the link opens the conversation in its
        // folder rather than in Sent; chosen with ctrl-k / ctrl-j, it is.
        var mine = thread.messages
        mine[2] = MessageView(
            id: 3, folderId: 2, folderRole: "sent", subject: "S", fromName: "Me", fromAddr: "me@x.org", to: [], cc: [],
            dateSent: nil, dateReceived: 1_700_000_000, bodyText: "", isSeen: true, isFlagged: false, isAnswered: false,
            hasAttachments: false, authResult: "unknown", bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: [], onlyQuotes: false, expanded: true, showRecipients: false, attachments: [])
        let withSent = ThreadView(id: 9, subject: "S", messages: mine, asOf: Position(epoch: "e", seq: 0))
        #expect(ThreadDisplay.linkMessage(withSent, .init())?.id == 2)
        s.current = 3
        #expect(ThreadDisplay.linkMessage(withSent, s)?.id == 3)
    }

    /// A deep link opened on an older message: it is the current one and
    /// unfolded, the other choices kept; `T` needs no unfolding.
    @Test func aLinkOpenedOnAMessageUnfoldsItAndMakesItCurrent() {
        var s = ThreadDisplay.State()
        s.revealed = ["2:2"]
        let focused = ThreadDisplay.focus(s, on: 1)
        #expect(focused.current == 1)
        #expect(focused.folding[1] == true)
        #expect(focused.revealed == ["2:2"])
        let blocks = ThreadDisplay.blocks(thread, focused)
        #expect(blocks.contains { if case .header(1, false, _, _, _, _, _, _, _) = $0 { true } else { false } })
        // ctrl-j goes on from it.
        #expect(ThreadDisplay.step(thread, focused, by: 1)?.message == 2)
        var all = ThreadDisplay.State()
        all.showAll = true
        #expect(ThreadDisplay.focus(all, on: 1).folding.isEmpty)
    }
}
