import Foundation
import Testing
@testable import NiceMailer

/// A mail without text says "no text", not "only quotes earlier messages",
/// which it does not (`ThreadDisplay.hasNoText`; the core's `only_quotes`
/// needs a quote, `clean.rs`).
@MainActor
struct NoTextTests {
    func message(body: String?, segments: [SegmentView], onlyQuotes: Bool = false) -> MessageView {
        MessageView(
            id: 7, folderId: 1, folderRole: "inbox", subject: "S", fromName: "Alice", fromAddr: "a@x.org",
            to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: body,
            isSeen: true, isFlagged: false, isAnswered: false, hasAttachments: false, authResult: "unknown",
            bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: segments, onlyQuotes: onlyQuotes, expanded: true, showRecipients: false, attachments: [])
    }

    func segment(_ kind: String, _ text: String, hidden: Bool = false) -> SegmentView {
        SegmentView(kind: kind, text: text, depth: 0, hidden: hidden, inThread: false)
    }

    func notes(_ m: MessageView) -> [String] {
        let t = ThreadView(id: 1, subject: "S", messages: [m], asOf: Position(epoch: "e", seq: 0))
        return ThreadDisplay.blocks(t, .init()).compactMap { if case .note(_, let text) = $0 { text } else { nil } }
    }

    @Test func anEmptyBodySaysNoText() {
        for m in [message(body: "", segments: []), message(body: "\n  \n", segments: [])] {
            #expect(ThreadDisplay.hasNoText(m))
            #expect(notes(m) == [L("no text")])
            #expect(ThreadDisplay.snippet(m) == L("no text"))
        }
    }

    @Test func aSignatureAloneSaysNoText() {
        let m = message(body: "-- \nBob", segments: [segment("signature", "-- \nBob", hidden: true)])
        #expect(ThreadDisplay.hasNoText(m))
        #expect(notes(m) == [L("no text")])
    }

    @Test func aMailWithTextSaysNothing() {
        let m = message(body: "Bonjour", segments: [segment("text", "Bonjour")])
        #expect(!ThreadDisplay.hasNoText(m))
        #expect(notes(m).isEmpty)
        #expect(ThreadDisplay.snippet(m) == "Bonjour")
    }

    @Test func aMailThatOnlyQuotesSaysSo() {
        let m = message(body: "> Salut", segments: [segment("quote_tail", "> Salut", hidden: true)], onlyQuotes: true)
        #expect(!ThreadDisplay.hasNoText(m))
        #expect(notes(m) == [L("only quotes earlier messages")])
    }

    @Test func aBodyNotDownloadedIsNotEmpty() {
        let m = message(body: nil, segments: [])
        #expect(!ThreadDisplay.hasNoText(m))
        #expect(notes(m).isEmpty)
    }
}
