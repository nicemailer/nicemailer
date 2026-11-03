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
        guard case .header(1, true, let title, let detail, _) = blocks[0] else { Issue.record("\(blocks[0])"); return }
        #expect(title.hasPrefix("▸ Alice"))
        #expect(detail == "Peux-tu passer jeudi ?")
        // A message that only quotes says so instead of a snippet.
        let last = blocks.last { if case .header(3, _, _, _, _) = $0 { return true } else { return false } }
        if case .header(_, true, _, let d, _) = last { #expect(d == L("only quotes earlier messages")) }
    }

    @Test func unfoldedMessagesShowVisibleSegmentsAndHiddenLines() {
        let blocks = ThreadDisplay.blocks(thread, .init())
        #expect(blocks.contains(.segment(messageId: 2, index: 0, kind: "quote", text: "jeudi ?", depth: 1)))
        #expect(blocks.contains(.segment(messageId: 2, index: 1, kind: "text", text: "Oui, 10 h.", depth: 0)))
        #expect(blocks.contains(.hidden(messageId: 2, index: 2, label: L("··· %lld quoted lines", 2))))
        // Recipients only when the core says they changed.
        #expect(blocks.contains { if case .header(2, false, _, let d?, _) = $0 { return d.contains("me@x.org") } else { return false } })
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
