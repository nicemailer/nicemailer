import AppKit
import PDFKit
import Testing
@testable import NiceMailer

/// The PDF of `sp` (`MailPDF`): what it says, in which fonts, cut into
/// pages, and the file it is saved to.
@Suite struct MailPDFTests {
    static let rawHash = String(repeating: "0f", count: 32)
    static let fileHash = String(repeating: "ab", count: 32)

    /// A record as the core gives it for a mail read from its raw file.
    static func record(body: String? = "Madame, Monsieur,\n\nJe vous mets en demeure.\n> Votre mail du 2 juin",
                       raw: Bool = true, subject: String = "Mise en demeure – loyer",
                       sender: [String: Any] = ["addr": "sec@a.example", "name": "Secrétariat"],
                       replyTo: [[String: Any]] = [["addr": "avocat@c.example", "name": "Cabinet"]]) throws -> MessageRecord {
        let json: [String: Any] = [
            "message_id": "1@a.example", "subject": subject,
            "from": [["addr": "alice@a.example", "name": "Alice Martin"]],
            "sender": sender, "reply_to": replyTo,
            "to": [["addr": "me@x.example", "name": "Moi"], ["addr": "bob@b.example", "name": NSNull()]],
            "cc": [["addr": "carol@c.example", "name": "Carol Petit"]],
            "bcc": [["addr": "archive@x.example", "name": NSNull()]],
            "date_header": "Tue, 23 Jun 2020 21:20:56 +0200", "date_sent": 1_592_940_056, "date_received": 1_592_940_061,
            "in_reply_to": "0@x.example", "references": ["00@x.example", "0@x.example"],
            "account": "perso", "folder": "INBOX", "auth_result": "pass", "body": body ?? NSNull(),
            "attachments": [["part_id": "2", "filename": "bail.pdf", "mime_type": "application/pdf", "size": 250_880,
                             "inline": false, "sha256": raw ? fileHash as Any : NSNull()]],
            "headers": raw ? [["name": "Received", "value": "from mx.a.example by mx.x.example; Tue, 23 Jun 2020 19:20:57 +0000"],
                              ["name": "DKIM-Signature", "value": "v=1; a=rsa-sha256; d=a.example; s=sel; b=abc"]] : [],
            "raw_size": 4096, "raw_sha256": raw ? rawHash as Any : NSNull(),
        ]
        let data = try JSONSerialization.data(withJSONObject: json)
        return try CoreClient.decoder.decode(MessageRecord.self, from: data)
    }

    static let paris = TimeZone(identifier: "Europe/Paris")!
    static let dates = MailPDF.Dates(locale: Locale(identifier: "en_GB"), timeZone: paris)

    /// The text of every page of `data`, read back by PDFKit.
    static func pages(_ data: Data) throws -> [String] {
        let doc = try #require(PDFDocument(data: data))
        return (0..<doc.pageCount).map { doc.page(at: $0)?.string ?? "" }
    }

    @Test func theEvidenceHasEveryPartyBothDatesAndTheTechnicalRecord() throws {
        let data = MailPDF.render(try Self.record(), kind: .evidence,
                                  locale: Locale(identifier: "en_GB"), timeZone: Self.paris)
        if let out = ProcessInfo.processInfo.environment["NM_PDF_SAMPLE"] { try data.write(to: URL(fileURLWithPath: out)) }
        let pages = try Self.pages(data)
        let page = pages.joined(separator: "\n")
        for expected in ["Mise en demeure – loyer", "Alice Martin", "<alice@a.example>", "Secrétariat", "<sec@a.example>",
                         "Cabinet", "<avocat@c.example>", "Moi", "<me@x.example>", "bob@b.example", "Carol Petit",
                         "<carol@c.example>", "archive@x.example", "Je vous mets en demeure.", "Votre mail du 2 juin",
                         "bail.pdf", "SHA-256 \(Self.fileHash)", "<1@a.example>", "<00@x.example>",
                         "Tue, 23 Jun 2020 21:20:56 +0200", "from mx.a.example by mx.x.example",
                         "d=a.example", Self.rawHash, "Page 1 / \(pages.count)"] {
            #expect(page.contains(expected), "missing: \(expected)")
        }
        // Both dates to the second, in the time zone of the Mac.
        #expect(page.contains("21:20:56"))
        #expect(page.contains("21:21:01"))
        // Left out on purpose: the account and folder, the authentication
        // verdict, the export date.
        for absent in ["perso", "INBOX", "NiceMailer saves", "s e;"] {
            #expect(!page.contains(absent), "unexpected: \(absent)")
        }
        #expect(!page.contains("2026"))
        // PDF information: title, author, the hash as subject.
        let doc = try #require(PDFDocument(data: data))
        let attrs = doc.documentAttributes ?? [:]
        #expect(attrs[PDFDocumentAttribute.titleAttribute] as? String == "Mise en demeure – loyer")
        #expect(attrs[PDFDocumentAttribute.authorAttribute] as? String == "Alice Martin <alice@a.example>")
        #expect(attrs[PDFDocumentAttribute.creatorAttribute] as? String == "NiceMailer")
        #expect(attrs[PDFDocumentAttribute.subjectAttribute] as? String == "SHA-256 \(Self.rawHash)")
    }

    @Test func thePlainPDFHasTheMailButNoTechnicalInformation() throws {
        let data = MailPDF.render(try Self.record(), kind: .plain,
                                  locale: Locale(identifier: "en_GB"), timeZone: Self.paris)
        let pages = try Self.pages(data)
        let all = pages.joined(separator: "\n")
        // The mail, its parties, its dates and its attachments are there.
        for expected in ["Mise en demeure – loyer", "Alice Martin", "<alice@a.example>", "Secrétariat", "Cabinet",
                         "<me@x.example>", "bob@b.example", "Carol Petit", "archive@x.example", "21:20:56", "21:21:01",
                         "Je vous mets en demeure.", "bail.pdf", "application/pdf"] {
            #expect(all.contains(expected), "missing: \(expected)")
        }
        // No technical record, no hash, no export line: the page number alone.
        for absent in [L("Technical information"), Self.rawHash, Self.fileHash, "SHA-256", "<1@a.example>",
                       "Tue, 23 Jun 2020 21:20:56 +0200", "mx.a.example", "NiceMailer"] {
            #expect(!all.contains(absent), "unexpected: \(absent)")
        }
        for (i, page) in pages.enumerated() {
            #expect(page.contains(L("Page %lld / %lld", i + 1, pages.count)))
        }
        // The document information keeps the title and the author, not the hash.
        let attrs = try #require(PDFDocument(data: data)).documentAttributes ?? [:]
        #expect(attrs[PDFDocumentAttribute.titleAttribute] as? String == "Mise en demeure – loyer")
        #expect(attrs[PDFDocumentAttribute.subjectAttribute] == nil)
    }

    @Test func aLongPlainMailIsNumberedOnEveryPage() throws {
        let body = (1...400).map { "Line \($0) of a long mail, written to fill several pages of the document." }
            .joined(separator: "\n")
        let pages = try Self.pages(MailPDF.render(try Self.record(body: body), kind: .plain))
        #expect(pages.count >= 5)
        for (i, page) in pages.enumerated() {
            #expect(page.contains(L("Page %lld / %lld", i + 1, pages.count)))
            #expect(!page.contains("SHA-256"))
        }
    }

    @Test func theSubjectIsHelveticaBoldAndTheBodyHelvetica() throws {
        let text = MailPDF.document(try Self.record(), kind: .evidence, dates: Self.dates)
        let font = { (i: Int) in text.attribute(.font, at: i, effectiveRange: nil) as? NSFont }
        #expect(font(0)?.fontName == "Helvetica-Bold")
        let body = (text.string as NSString).range(of: "Je vous mets en demeure.")
        #expect(font(body.location)?.fontName == "Helvetica")
        // The hashes in Menlo, where 0 and O cannot be confused.
        let hash = (text.string as NSString).range(of: Self.rawHash)
        #expect(font(hash.location)?.fontName == "Menlo-Regular")
        // Fixed colours: black on white even in dark mode.
        let color = text.attribute(.foregroundColor, at: body.location, effectiveRange: nil) as? NSColor
        #expect(color?.colorSpace == .sRGB)
    }

    @Test func theSenderAndReplyToAreLeftOutWhenTheyRepeatTheAuthor() throws {
        let r = try Self.record(sender: ["addr": "alice@a.example", "name": "Alice Martin"],
                                replyTo: [["addr": "alice@a.example", "name": "Alice"]])
        let text = MailPDF.document(r, kind: .plain, dates: Self.dates).string
        #expect(!text.contains(L("Sent by")))
        #expect(!text.contains(L("Reply-To") + "\t"))
    }

    @Test func aLongMailRunsOverSeveralNumberedPagesEachWithTheHash() throws {
        let body = (1...400).map { "Line \($0) of a long mail, written to fill several pages of the document." }
            .joined(separator: "\n")
        let pages = try Self.pages(MailPDF.render(try Self.record(body: body), kind: .evidence, locale: Locale(identifier: "en_GB"),
                                                  timeZone: Self.paris))
        #expect(pages.count >= 5)
        for (i, page) in pages.enumerated() {
            #expect(page.contains("Page \(i + 1) / \(pages.count)"))
            #expect(page.contains(Self.rawHash))
        }
        // Nothing lost between pages.
        let all = pages.joined(separator: "\n")
        #expect(all.contains("Line 1 of a long mail") && all.contains("Line 400 of a long mail"))
        #expect(all.contains("Line 217 of a long mail"))
    }

    @Test func aLabelNeverEndsAPageWithoutItsValue() {
        // Two labels then their value: if the page ends after the labels,
        // the run moves to the next page, from the first label.
        let font = MailPDF.helvetica(10)
        let text = NSMutableAttributedString(attributedString: MailPDF.paragraph("body", font: font, color: .black, after: 0))
        text.append(MailPDF.paragraph("Label A", font: font, color: .black, after: 0, keepWithNext: true))
        text.append(MailPDF.paragraph("Label B", font: font, color: .black, after: 0, keepWithNext: true))
        text.append(MailPDF.paragraph("value", font: font, color: .black, after: 0))
        let storage = NSTextStorage(attributedString: text)
        let labels = (text.string as NSString).range(of: "Label A").location
        let afterLabels = (text.string as NSString).range(of: "value").location
        #expect(MailPDF.orphanStart(storage, NSRange(location: 0, length: afterLabels)) == labels)
        // Ending on the value, or a page of labels alone: nothing moves.
        #expect(MailPDF.orphanStart(storage, NSRange(location: 0, length: text.length)) == nil)
        #expect(MailPDF.orphanStart(storage, NSRange(location: labels, length: afterLabels - labels)) == nil)
    }

    @Test func withoutTheRawMessageThePDFSaysItCannotBeChecked() throws {
        let page = try Self.pages(MailPDF.render(try Self.record(raw: false), kind: .evidence))[0]
        #expect(page.contains(L("Original message not available")))
        #expect(!page.contains(Self.rawHash))
        let text = MailPDF.document(try Self.record(raw: false), kind: .evidence, dates: Self.dates).string
        #expect(text.contains(L("The original message is not stored on this Mac: this document was made from the mail database, and cannot be checked against the original.")))
        #expect(text.contains(L("SHA-256 not available")))
    }

    @Test func aMailWithoutBodySaysSo() throws {
        let text = MailPDF.document(try Self.record(body: nil), kind: .plain, dates: Self.dates).string
        #expect(text.contains(L("(body not downloaded)")))
    }

    @Test func charactersHelveticaLacksAreKept() throws {
        let page = try Self.pages(MailPDF.render(try Self.record(body: "会议纪要 Ελληνικά Ünïcødé"), kind: .plain))[0]
        #expect(page.contains("会议纪要") && page.contains("Ελληνικά") && page.contains("Ünïcødé"))
    }

    @Test func theFileIsNamedAsTheOtherExports() {
        #expect(MailPDF.fileName(subject: "Devis: v2/3", received: 1_592_940_056) == "2020-06-23 Devis v2 3.pdf")
        #expect(MailPDF.fileName(subject: "  ", received: 0) == "1970-01-01 message.pdf")
        #expect(MailPDF.fileName(subject: "..hidden", received: 0) == "1970-01-01 hidden.pdf")
        #expect(MailPDF.fileName(subject: String(repeating: "a", count: 200), received: 0).count == "1970-01-01 ".count + 80 + 4)
    }

    @Test func aFileIsNeverOverwritten() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nicemailer-pdf-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let a = try MailPDF.write(Data("1".utf8), in: dir, name: "m.pdf")
        let b = try MailPDF.write(Data("2".utf8), in: dir, name: "m.pdf")
        #expect(a.lastPathComponent == "m.pdf" && b.lastPathComponent == "m (2).pdf")
        #expect(try String(contentsOf: a, encoding: .utf8) == "1")
    }

    @Test func sizesAreRoundedAndExact() {
        #expect(MailPDF.byteCount(512) == L("%@ bytes", NumberFormatter.localizedString(from: 512, number: .decimal)))
        #expect(MailPDF.byteCount(250_880).contains(NumberFormatter.localizedString(from: 250_880, number: .decimal)))
    }
}
