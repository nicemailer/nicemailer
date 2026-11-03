import Foundation
import Testing
@testable import NiceMailer

/// Presentation of attachments (`AttachmentDisplay`) and their place in the
/// reader's blocks (`ThreadDisplay`).
@MainActor
struct AttachmentDisplayTests {
    func file(_ name: String?, _ mime: String, size: Int = 1000, id: Int = 1) -> AttachmentView {
        AttachmentView(id: id, filename: name, mimeType: mime, size: size)
    }

    @Test func iconsFollowTheTypeThenTheExtension() {
        let s = AttachmentDisplay.symbol
        #expect(s("application/pdf", "devis.pdf") == "doc.richtext")
        #expect(s("image/jpeg", "IMG_1.jpg") == "photo")
        #expect(s("audio/mpeg", "a.mp3") == "waveform")
        #expect(s("video/mp4", "v.mp4") == "film")
        #expect(s("text/calendar", "invite.ics") == "calendar")
        #expect(s("text/vcard", nil) == "person.crop.rectangle")
        #expect(s("message/rfc822", nil) == "envelope")
        #expect(s("application/zip", "a.zip") == "doc.zipper")
        #expect(s("text/plain", "notes.txt") == "doc.plaintext")
        // Office files: by type, or by extension when sent as bytes.
        #expect(s("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "a.docx") == "doc.text")
        #expect(s("application/octet-stream", "budget.XLSX") == "tablecells")
        #expect(s("application/octet-stream", "slides.pptx") == "rectangle.on.rectangle")
        #expect(s("application/octet-stream", "script.sh") == "chevron.left.forwardslash.chevron.right")
        // Unknown: a plain document.
        #expect(s("application/octet-stream", "data.bin") == "doc")
        #expect(s("application/octet-stream", nil) == "doc")
    }

    @Test func namesSizesAndCuts() {
        #expect(AttachmentDisplay.name(file("devis.pdf", "application/pdf")) == "devis.pdf")
        #expect(AttachmentDisplay.name(file(nil, "application/pdf")) == L("Unnamed attachment"))
        #expect(AttachmentDisplay.name(file("  ", "application/pdf")) == L("Unnamed attachment"))
        #expect(!AttachmentDisplay.size(245_000).isEmpty)
        // Cut in the middle, the extension kept.
        let cut = AttachmentDisplay.middleTruncated("Rapport-annuel-du-conseil-2025.pdf", max: 20)
        #expect(cut.count == 20 && cut.hasSuffix("2025.pdf") && cut.contains("…"))
        #expect(AttachmentDisplay.middleTruncated("court.pdf", max: 20) == "court.pdf")
    }

    @Test func threeToFiveTilesPerRow() {
        // 24-character tiles of 7-point characters (168 points each).
        #expect(AttachmentDisplay.columns(width: 300, charWidth: 7) == 3)
        #expect(AttachmentDisplay.columns(width: 700, charWidth: 7) == 4)
        #expect(AttachmentDisplay.columns(width: 2000, charWidth: 7) == 5)
        #expect(AttachmentDisplay.columns(width: 800, charWidth: 0) == 3)
    }

    @Test func aSavedFileNeverOverwritesAnother() {
        let dir = URL(fileURLWithPath: "/d")
        let taken: Set<String> = ["/d/devis.pdf", "/d/devis (2).pdf", "/d/notes"]
        let free = { (name: String) in AttachmentDisplay.freeURL(in: dir, name: name) { taken.contains($0.path) }.path }
        #expect(free("devis.pdf") == "/d/devis (3).pdf")
        #expect(free("photo.jpg") == "/d/photo.jpg")
        #expect(free("notes") == "/d/notes (2)")
    }

    // MARK: Blocks of the reader

    func thread(expanded: Bool, files: [AttachmentView]) -> ThreadView {
        let m = MessageView(
            id: 7, folderId: 1, folderRole: "archive", subject: "S", fromName: "Alice", fromAddr: "a@x.org",
            to: [], cc: [], dateSent: nil, dateReceived: 1_700_000_000, bodyText: "Bonjour",
            isSeen: true, isFlagged: false, isAnswered: false, hasAttachments: !files.isEmpty, authResult: "unknown",
            bodyHtml: nil, hasRemoteContent: false, fromBodyFormat: nil,
            segments: [SegmentView(kind: "text", text: "Bonjour", depth: 0, hidden: false, inThread: false)],
            onlyQuotes: false, expanded: expanded, showRecipients: false, attachments: files)
        return ThreadView(id: 1, subject: "S", messages: [m], asOf: Position(epoch: "e", seq: 0))
    }

    @Test func anUnfoldedMailEndsWithItsTiles() {
        let files = [file("devis.pdf", "application/pdf", id: 1), file("photo.jpg", "image/jpeg", id: 2)]
        let blocks = ThreadDisplay.blocks(thread(expanded: true, files: files), .init())
        #expect(blocks.last == .attachments(messageId: 7, files: files))
        // No tiles block without attachments.
        let none = ThreadDisplay.blocks(thread(expanded: true, files: []), .init())
        #expect(!none.contains { if case .attachments = $0 { return true } else { return false } })
    }

    @Test func aFoldedMailShowsHowManyFilesItHas() {
        let files = [file("a.pdf", "application/pdf", id: 1), file("b.pdf", "application/pdf", id: 2)]
        let blocks = ThreadDisplay.blocks(thread(expanded: false, files: files), .init())
        guard case .header(7, true, _, _, let n) = blocks.first else { Issue.record("\(blocks)"); return }
        #expect(n == 2)
        #expect(!blocks.contains { if case .attachments = $0 { return true } else { return false } })
    }
}
