import Foundation
import Testing
@testable import NiceMailer

/// A mail without text says "no text", not "only quotes earlier messages",
/// which it does not. The core decides (`no_text`, `only_quotes` in
/// `clean.rs`); the app shows its decision (`ThreadDisplay`).
@MainActor
struct NoTextTests {
    func message(body: String?, segments: [SegmentView], onlyQuotes: Bool = false, noText: Bool = false) -> MessageView {
        MessageView(
            id: 7, folderId: 1, folderRole: "inbox", subject: "S", fromName: "Alice", fromAddr: "a@x.org",
            to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: body,
            isSeen: true, isFlagged: false, isAnswered: false, hasAttachments: false, authResult: "unknown",
            bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: segments, onlyQuotes: onlyQuotes, noText: noText, expanded: true, showRecipients: false,
            attachments: [])
    }

    func segment(_ kind: String, _ text: String, hidden: Bool = false) -> SegmentView {
        SegmentView(kind: kind, text: text, depth: 0, hidden: hidden, inThread: false)
    }

    func blocks(_ m: MessageView, _ state: ThreadDisplay.State = .init()) -> [ThreadDisplay.Block] {
        ThreadDisplay.blocks(ThreadView(id: 1, subject: "S", messages: [m], asOf: Position(epoch: "e", seq: 0)), state)
    }

    func notes(_ m: MessageView, _ state: ThreadDisplay.State = .init()) -> [String] {
        blocks(m, state).compactMap { if case .note(_, let text) = $0 { text } else { nil } }
    }

    func segmentTexts(_ m: MessageView) -> [String] {
        blocks(m).compactMap { if case .segment(_, _, _, let text, _) = $0 { text } else { nil } }
    }

    @Test func anEmptyBodySaysNoText() {
        let m = message(body: "", segments: [], noText: true)
        #expect(notes(m) == [L("no text")])
        #expect(ThreadDisplay.snippet(m) == L("no text"))
        // Not mistaken for a body not downloaded.
        #expect(segmentTexts(m).isEmpty)
    }

    @Test func aSignatureAloneSaysNoText() {
        let m = message(body: "-- \nBob", segments: [segment("signature", "-- \nBob", hidden: true)], noText: true)
        #expect(notes(m) == [L("no text")])
    }

    @Test func showingAllDropsTheNote() {
        let m = message(body: "-- \nBob", segments: [segment("signature", "-- \nBob", hidden: true)], noText: true)
        var all = ThreadDisplay.State()
        all.showAll = true
        #expect(notes(m, all).isEmpty)
    }

    @Test func aMailWithTextSaysNothing() {
        let m = message(body: "Bonjour", segments: [segment("text", "Bonjour")])
        #expect(notes(m).isEmpty)
        #expect(ThreadDisplay.snippet(m) == "Bonjour")
    }

    @Test func aMailThatOnlyQuotesSaysSo() {
        let m = message(body: "> Salut", segments: [segment("quote_tail", "> Salut", hidden: true)], onlyQuotes: true)
        #expect(notes(m) == [L("only quotes earlier messages")])
    }

    @Test func aBodyNotDownloadedIsNotEmpty() {
        let m = message(body: nil, segments: [])
        #expect(notes(m).isEmpty)
        #expect(segmentTexts(m) == [L("(body not downloaded)")])
    }

    /// The flag comes from the core's JSON (`no_text`).
    @Test func theFlagIsReadFromTheCore() throws {
        let json: [String: Any] = [
            "id": 7, "folder_id": 1, "folder_role": "inbox", "subject": "S", "from_name": NSNull(),
            "from_addr": "a@x.org", "to": [], "cc": [], "date_sent": NSNull(), "date_received": 0,
            "body_text": "", "is_seen": true, "is_flagged": false, "is_answered": false, "has_attachments": false,
            "auth_result": "pass", "body_html": NSNull(), "has_remote_content": false, "from_body_format": NSNull(),
            "segments": [], "only_quotes": false, "no_text": true, "expanded": true, "show_recipients": false,
            "attachments": [], "unsubscribe": NSNull(), "tracking_pixels": 0, "links": [],
        ]
        let m = try CoreClient.decoder.decode(MessageView.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(m.noText)
    }
}
