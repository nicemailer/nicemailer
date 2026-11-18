import AppKit

/// The PDF of one mail (`ui.md`, menu `s`; `docs/geek-ui.md`, "Saving a
/// mail as PDF"), in two kinds (`Kind`):
///
/// - `sp`, for every day: the subject, the parties, the dates, the body and
///   the list of attachments; only the page number at the foot of a page.
/// - `si`, to be produced in a dispute: the same, complete enough that
///   nobody can say something was left out, as described below.
///
/// The evidence kind:
///
/// - Page 1 starts with the subject in bold, then every party: authors,
///   `Sender`, `Reply-To`, `To`, `Cc`, `Bcc`, each with its name as written
///   in this mail and its address; the date of sending (the `Date` header)
///   and of reception by the server, to the second, with the time zone.
/// - The body follows, whole, quotes and signature included.
/// - Then the attachments, each with its size and the SHA-256 of its
///   content, and the technical record: Message-ID, `References`, the `Date`
///   header as written, the trace headers (`Received`, `Authentication-
///   Results`, `DKIM-Signature`…), the size and SHA-256 of the raw message.
/// - Every page ends with the SHA-256 of the raw message and "Page n / N":
///   a page taken out of the document still says which
///   mail it belongs to, and that pages are missing.
///
/// The hash is what makes the PDF verifiable: the `.eml` (`se`) holds the
/// original bytes and the sending domain's DKIM signature; anyone can check
/// that it is the mail described here (`shasum -a 256 file.eml`).
///
/// Everything is set in Helvetica, the hashes and identifiers in Menlo,
/// where 0 and O, 1 and l cannot be confused. Laid out by TextKit
/// (`NSLayoutManager`, one text container per page) and drawn into a PDF
/// context by Core Graphics, which embeds the subsets of the fonts used:
/// characters Helvetica lacks (Chinese, emoji) are drawn with a fallback
/// font rather than lost.
enum MailPDF {
    // MARK: Page geometry, in PostScript points (1/72 inch)

    /// A4, the paper of French courts and most of the world.
    static let pageSize = CGSize(width: 595.28, height: 841.89)
    /// 2 cm on the sides, a little more at the bottom for the footer.
    static let margins = NSEdgeInsets(top: 56, left: 57, bottom: 72, right: 57)
    /// Width of the label column of the header block ("From", "Cc"…).
    static let labelWidth: CGFloat = 78

    static var contentSize: CGSize {
        CGSize(width: pageSize.width - margins.left - margins.right,
               height: pageSize.height - margins.top - margins.bottom)
    }

    // MARK: Fonts and colours

    /// Helvetica, with Helvetica Neue then the system font as fallbacks:
    /// all ship with macOS, but a font missing must not stop an export.
    static func helvetica(_ size: CGFloat, bold: Bool = false, italic: Bool = false) -> NSFont {
        let name = bold ? (italic ? "Helvetica-BoldOblique" : "Helvetica-Bold") : (italic ? "Helvetica-Oblique" : "Helvetica")
        return NSFont(name: name, size: size)
            ?? NSFont(name: bold ? "HelveticaNeue-Bold" : "HelveticaNeue", size: size)
            ?? (bold ? NSFont.boldSystemFont(ofSize: size) : NSFont.systemFont(ofSize: size))
    }

    static func mono(_ size: CGFloat) -> NSFont {
        NSFont(name: "Menlo-Regular", size: size) ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }

    /// Fixed colours, never the system's dynamic ones: a PDF made while the
    /// Mac is in dark mode must still be black on white.
    static let ink = NSColor(srgbRed: 0.08, green: 0.08, blue: 0.09, alpha: 1)
    static let muted = NSColor(srgbRed: 0.42, green: 0.42, blue: 0.45, alpha: 1)
    static let faint = NSColor(srgbRed: 0.60, green: 0.60, blue: 0.63, alpha: 1)
    static let rule = NSColor(srgbRed: 0.80, green: 0.80, blue: 0.82, alpha: 1)
    static let warning = NSColor(srgbRed: 0.62, green: 0.18, blue: 0.10, alpha: 1)

    // MARK: Document

    /// The two PDFs of a mail.
    enum Kind {
        /// `sp`: to read, file or send. No technical information, no
        /// hashes; the foot of a page holds its number alone.
        case plain
        /// `si`: to produce as evidence. The hashes of the attachments,
        /// the technical record, and at the foot of every page the export
        /// date and the SHA-256 of the original message.
        case evidence
    }

    /// The PDF of `record`. `locale` and `timeZone` write the dates (the
    /// interface's locale, `uiLocale`, and the Mac's time zone; fixed in
    /// tests).
    static func render(_ record: MessageRecord, kind: Kind,
                       locale: Locale = uiLocale, timeZone: TimeZone = .current) -> Data {
        let dates = Dates(locale: locale, timeZone: timeZone)
        let text = document(record, kind: kind, dates: dates)
        let footer = kind == .evidence ? footerHash(record) : nil
        return draw(text, footer: footer, info: info(record, kind: kind))
    }

    /// The whole text of the PDF, before it is cut into pages: a pure
    /// function of the record, tested on its own.
    static func document(_ r: MessageRecord, kind: Kind, dates: Dates) -> NSAttributedString {
        let out = NSMutableAttributedString()

        // The subject, in bold: the title of the document.
        let subject = r.subject.trimmingCharacters(in: .whitespacesAndNewlines)
        out.append(paragraph(subject.isEmpty ? L("(no subject)") : subject,
                             font: helvetica(17, bold: true), color: ink, after: 14, lineHeight: 1.15))

        // The parties and the dates, in a block closed by a rule.
        let block = NSTextBlock()
        // The full width of the page: without it the block is as wide as
        // its text, and the rule stops where the longest row does.
        block.setValue(100, type: .percentageValueType, for: .width)
        block.setWidth(0.6, type: .absoluteValueType, for: .border, edge: .maxY)
        block.setWidth(10, type: .absoluteValueType, for: .padding, edge: .maxY)
        block.setBorderColor(rule, for: .maxY)
        var rows: [(String, NSAttributedString)] = []
        let authorLabel = r.from.count > 1 ? L("Authors") : L("From")
        rows.append((authorLabel, r.from.isEmpty ? note(L("(no sender)")) : addresses(r.from)))
        if let s = r.sender, !r.from.contains(where: { $0.addr == s.addr }) {
            rows.append((L("Sent by"), addresses([s])))
        }
        if !r.replyTo.isEmpty, r.replyTo.map(\.addr) != r.from.map(\.addr) {
            rows.append((L("Reply-To"), addresses(r.replyTo)))
        }
        rows.append((L("To"), r.to.isEmpty ? note(L("(no recipient)")) : addresses(r.to)))
        if !r.cc.isEmpty { rows.append((L("Cc"), addresses(r.cc))) }
        if !r.bcc.isEmpty { rows.append((L("Bcc"), addresses(r.bcc))) }
        rows.append((L("Sent on"), r.dateSent.map { plain(dates.long($0)) } ?? note(L("no Date header"))))
        rows.append((L("Received on"), plain(dates.long(r.dateReceived))))
        if !r.attachments.isEmpty {
            rows.append((L("Attachments"), plain(attachmentSummary(r.attachments))))
        }
        for (i, row) in rows.enumerated() {
            out.append(field(row.0, row.1, block: block, last: i == rows.count - 1))
        }

        // The body, whole.
        out.append(paragraph("", font: helvetica(6), color: ink, after: 8))
        if let body = r.body, !body.isEmpty {
            out.append(bodyText(body))
        } else {
            out.append(paragraph(r.body == nil ? L("(body not downloaded)") : L("(empty body)"),
                                 font: helvetica(10.5, italic: true), color: muted, after: 0))
        }

        // Attachments, with their fingerprints in the evidence.
        if !r.attachments.isEmpty {
            out.append(heading(L("Attachments")))
            for a in r.attachments {
                var line = a.filename ?? L("(no name)")
                if a.inline { line = L("%@ (image shown in the body)", line) }
                out.append(paragraph(line, font: helvetica(9.5, bold: true), color: ink, after: 1, keepWithNext: true))
                let details = "\(a.mimeType) · \(byteCount(a.size))"
                guard kind == .evidence else {
                    out.append(paragraph(details, font: helvetica(8.5), color: muted, after: 7))
                    continue
                }
                out.append(paragraph(details, font: helvetica(8.5), color: muted, after: 1, keepWithNext: true))
                out.append(paragraph(a.sha256.map { "SHA-256 \($0)" } ?? L("SHA-256 not available"),
                                     font: mono(7.5), color: muted, after: 7))
            }
        }

        if kind == .evidence { out.append(technical(r, dates: dates)) }
        return out
    }

    /// The technical record: what identifies the mail and traces its path.
    static func technical(_ r: MessageRecord, dates: Dates) -> NSAttributedString {
        let out = NSMutableAttributedString()
        out.append(heading(L("Technical information")))
        var items: [(String, String, Bool)] = []  // label, value, monospaced
        items.append(("Message-ID", r.messageId.map { "<\($0)>" } ?? L("none"), true))
        if let irt = r.inReplyTo { items.append(("In-Reply-To", "<\(irt)>", true)) }
        if !r.references.isEmpty {
            items.append(("References", r.references.map { "<\($0)>" }.joined(separator: "\u{2028}"), true))
        }
        items.append((L("Date header, as written"), r.dateHeader ?? L("none"), false))
        items.append((L("Received by the server"), dates.long(r.dateReceived), false))
        if let size = r.rawSize { items.append((L("Size of the original message"), byteCount(size), false)) }
        items.append((L("SHA-256 of the original message"), r.rawSha256 ?? L("not available"), r.rawSha256 != nil))
        for (label, value, monospaced) in items {
            out.append(paragraph(label, font: helvetica(7.5, bold: true), color: muted, after: 1, keepWithNext: true))
            out.append(paragraph(value, font: monospaced ? mono(8) : helvetica(8.5), color: ink, after: 6))
        }

        if !r.headers.isEmpty {
            out.append(paragraph(L("Trace headers, as written"), font: helvetica(9.5, bold: true), color: ink,
                                 before: 8, after: 5, keepWithNext: true))
            for h in r.headers {
                let s = NSMutableAttributedString(string: h.name + ": ",
                                                  attributes: [.font: helvetica(7.5, bold: true), .foregroundColor: ink])
                s.append(NSAttributedString(string: h.value, attributes: [.font: helvetica(7.5), .foregroundColor: muted]))
                s.append(NSAttributedString(string: "\n"))
                s.addAttribute(.paragraphStyle, value: style(after: 4, lineHeight: 1.15), range: NSRange(location: 0, length: s.length))
                out.append(s)
            }
        }

        let explanation = r.rawSha256 == nil
            ? L("The original message is not stored on this Mac: this document was made from the mail database, and cannot be checked against the original.")
            : L("This document reproduces the mail as the server delivered it. Its original, with the signature of the sending domain, is the .eml file; its SHA-256 must be the one above (shasum -a 256 file.eml). The SHA-256 of each attachment is that of the file sent.")
        out.append(paragraph(explanation, font: helvetica(7.5, italic: true),
                             color: r.rawSha256 == nil ? warning : muted, before: 10, after: 0, lineHeight: 1.25))
        return out
    }

    /// The line at the foot of each page of the evidence, beside its
    /// number: the SHA-256 of the original message.
    static func footerHash(_ r: MessageRecord) -> String {
        r.rawSha256.map { "SHA-256 \($0)" } ?? L("Original message not available")
    }

    /// The document information of the PDF (File › Get Info, Spotlight);
    /// the hash only in the evidence.
    static func info(_ r: MessageRecord, kind: Kind) -> [CFString: Any] {
        var info: [CFString: Any] = [
            kCGPDFContextTitle: r.subject.isEmpty ? L("(no subject)") : r.subject,
            kCGPDFContextCreator: "NiceMailer",
        ]
        if let from = r.from.first { info[kCGPDFContextAuthor] = address(from) }
        if let id = r.messageId { info[kCGPDFContextKeywords] = "Message-ID <\(id)>" }
        if kind == .evidence, let hash = r.rawSha256 { info[kCGPDFContextSubject] = "SHA-256 \(hash)" }
        return info
    }

    // MARK: Pieces of text

    /// "Name <addr>", or the bare address.
    static func address(_ a: AddressView) -> String {
        if let n = a.name, !n.isEmpty { return "\(n) <\(a.addr)>" }
        return a.addr
    }

    /// One address per line (a line separator, so that the wrapped lines of
    /// the field keep its indent): the name in ink, the address muted.
    static func addresses(_ list: [AddressView]) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for (i, a) in list.enumerated() {
            if i > 0 { out.append(NSAttributedString(string: "\u{2028}")) }
            if let n = a.name, !n.isEmpty {
                out.append(NSAttributedString(string: n, attributes: [.font: helvetica(10, bold: true), .foregroundColor: ink]))
                out.append(NSAttributedString(string: " <\(a.addr)>", attributes: [.font: helvetica(10), .foregroundColor: muted]))
            } else {
                out.append(NSAttributedString(string: a.addr, attributes: [.font: helvetica(10), .foregroundColor: ink]))
            }
        }
        return out
    }

    static func plain(_ s: String) -> NSAttributedString {
        NSAttributedString(string: s, attributes: [.font: helvetica(10), .foregroundColor: ink])
    }

    static func note(_ s: String) -> NSAttributedString {
        NSAttributedString(string: s, attributes: [.font: helvetica(10, italic: true), .foregroundColor: muted])
    }

    /// A row of the header block: the label, a tab, the value, whose
    /// wrapped lines start under the value (`headIndent`).
    static func field(_ label: String, _ value: NSAttributedString, block: NSTextBlock, last: Bool) -> NSAttributedString {
        let s = NSMutableAttributedString(string: label + "\t", attributes: [.font: helvetica(8.5), .foregroundColor: muted])
        s.append(value)
        s.append(NSAttributedString(string: "\n", attributes: [.font: helvetica(10)]))
        let p = NSMutableParagraphStyle()
        p.tabStops = [NSTextTab(textAlignment: .left, location: labelWidth)]
        p.headIndent = labelWidth
        p.paragraphSpacing = last ? 0 : 5
        p.lineHeightMultiple = 1.12
        // Swift note: `textBlocks` takes the blocks the paragraph sits in;
        // consecutive paragraphs with the same block form one box, whose
        // bottom border is the rule under the header.
        p.textBlocks = [block]
        s.addAttribute(.paragraphStyle, value: p, range: NSRange(location: 0, length: s.length))
        return s
    }

    static func heading(_ s: String) -> NSAttributedString {
        paragraph(s, font: helvetica(11, bold: true), color: ink, before: 20, after: 8, keepWithNext: true)
    }

    /// Marks a paragraph that must not end a page: a heading or a label
    /// alone at the foot of a page, its value on the next, reads as a
    /// missing value (`draw` moves it to the next page).
    static let keepWithNext = NSAttributedString.Key("NiceMailerKeepWithNext")

    static func style(before: CGFloat = 0, after: CGFloat, lineHeight: CGFloat = 1) -> NSParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.paragraphSpacingBefore = before
        p.paragraphSpacing = after
        p.lineHeightMultiple = lineHeight
        return p
    }

    static func paragraph(_ s: String, font: NSFont, color: NSColor, before: CGFloat = 0, after: CGFloat,
                          lineHeight: CGFloat = 1.1, keepWithNext keep: Bool = false) -> NSAttributedString {
        var attributes: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: color, .paragraphStyle: style(before: before, after: after, lineHeight: lineHeight),
        ]
        if keep { attributes[keepWithNext] = true }
        return NSAttributedString(string: s + "\n", attributes: attributes)
    }

    /// The body: 10.5 points, airy lines; quoted lines (`>`) muted, as in
    /// the reader. Nothing is removed or reordered.
    static func bodyText(_ body: String) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let p = style(after: 0, lineHeight: 1.32)
        let lines = body.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        for line in lines {
            let quoted = line.hasPrefix(">")
            out.append(NSAttributedString(string: line + "\n", attributes: [
                .font: helvetica(10.5), .foregroundColor: quoted ? muted : ink, .paragraphStyle: p,
            ]))
        }
        return out
    }

    /// "2 files, 1.2 MB": the summary line of the header block.
    static func attachmentSummary(_ list: [RecordAttachment]) -> String {
        let total = list.reduce(0) { $0 + $1.size }
        return L("%lld files, %@", list.count, byteCount(total))
    }

    /// "251 kB (250,880 bytes)": rounded for the reader, exact for the
    /// record, both in the interface's locale (`%lld` is written with the
    /// locale's grouping by `L`).
    static func byteCount(_ n: Int) -> String {
        n < 1000 ? L("%lld bytes", n) : L("%@ (%lld bytes)", Format.bytes(n), n)
    }

    /// Dates to the second, with the time zone: "Tuesday 23 June 2020 at
    /// 21:20:56 GMT+02:00", the hour on the clock of the Mac's settings
    /// ("j": 12- or 24-hour). A `DateFormatter` is costly to make: one per
    /// document.
    struct Dates {
        let formatter: DateFormatter

        init(locale: Locale, timeZone: TimeZone) {
            let f = DateFormatter()
            f.locale = locale
            f.timeZone = timeZone
            f.setLocalizedDateFormatFromTemplate("EEEEdMMMMyjmmssZZZZ")
            formatter = f
        }

        func long(_ ts: Int) -> String { long(Date(timeIntervalSince1970: TimeInterval(ts))) }
        func long(_ d: Date) -> String { formatter.string(from: d) }
    }

    // MARK: Pages

    /// Cuts `text` into pages and draws them into a PDF.
    /// `footer`: the hash line of the evidence; `nil` puts the page number
    /// alone, centred.
    static func draw(_ text: NSAttributedString, footer: String?,
                     info: [CFString: Any]) -> Data {
        // Swift note: TextKit 1. The storage holds the text, the layout
        // manager turns it into lines, and each text container is the
        // rectangle of one page: the layout manager fills them in order,
        // so adding containers until the text is used up paginates it.
        let storage = NSTextStorage(attributedString: text)
        let layout = NSLayoutManager()
        storage.addLayoutManager(layout)
        var pages: [NSTextContainer] = []
        while pages.count < maxPages {
            let c = NSTextContainer(size: contentSize)
            c.lineFragmentPadding = 0
            layout.addTextContainer(c)
            pages.append(c)
            var range = layout.glyphRange(for: c)
            if NSMaxRange(range) < layout.numberOfGlyphs, let cut = orphanStart(storage, layout.characterRange(forGlyphRange: range, actualGlyphRange: nil)) {
                // A page break before the run of headings and labels that
                // ends the page: they start the next one, with their value.
                storage.insert(NSAttributedString(string: "\u{0C}", attributes: storage.attributes(at: cut, effectiveRange: nil)), at: cut)
                range = layout.glyphRange(for: c)
            }
            // Done when the text is used up, or when a page took nothing
            // (never expected; it would otherwise loop to `maxPages`).
            if NSMaxRange(range) >= layout.numberOfGlyphs || range.length == 0 { break }
        }

        let data = NSMutableData()
        var box = CGRect(origin: .zero, size: pageSize)
        guard let consumer = CGDataConsumer(data: data as CFMutableData),
              let ctx = CGContext(consumer: consumer, mediaBox: &box, info as CFDictionary) else { return Data() }
        let total = pages.count
        for (i, container) in pages.enumerated() {
            ctx.beginPDFPage(nil)
            // PDF puts the origin at the bottom left, TextKit at the top
            // left: the context is flipped, and AppKit told so.
            ctx.translateBy(x: 0, y: pageSize.height)
            ctx.scaleBy(x: 1, y: -1)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: true)
            let range = layout.glyphRange(for: container)
            let origin = CGPoint(x: margins.left, y: margins.top)
            layout.drawBackground(forGlyphRange: range, at: origin)
            layout.drawGlyphs(forGlyphRange: range, at: origin)
            if let footer {
                drawFooter(footer, page: i + 1, of: total)
            } else {
                drawPageNumber(page: i + 1, of: total)
            }
            NSGraphicsContext.restoreGraphicsState()
            ctx.endPDFPage()
        }
        ctx.closePDF()
        return data as Data
    }

    /// Where the run of `keepWithNext` paragraphs that ends the page
    /// `chars` starts, if one does and it does not fill the whole page
    /// (moving it would then only move the problem).
    static func orphanStart(_ storage: NSTextStorage, _ chars: NSRange) -> Int? {
        let text = storage.string as NSString
        var start: Int?
        var end = NSMaxRange(chars)
        while end > chars.location {
            let para = text.paragraphRange(for: NSRange(location: end - 1, length: 0))
            guard storage.attribute(keepWithNext, at: max(para.location, chars.location), effectiveRange: nil) != nil else { break }
            start = max(para.location, chars.location)
            end = para.location
        }
        guard let start, start > chars.location else { return nil }
        return start
    }

    /// Pages at most: a mail of a million lines makes a PDF of this many
    /// pages, the rest cut, rather than one nobody can open.
    static let maxPages = 2000

    /// "Page n / N", centred at the foot of a page of the plain PDF.
    static func drawPageNumber(page: Int, of total: Int) {
        let number = NSAttributedString(string: L("Page %lld / %lld", page, total),
                                        attributes: [.font: helvetica(8), .foregroundColor: muted])
        let x = margins.left + (contentSize.width - number.size().width) / 2
        number.draw(at: CGPoint(x: x, y: pageSize.height - margins.bottom + 28))
    }

    /// The foot of a page of the evidence: a rule, then the hash line on
    /// the left and the page number on the right.
    static func drawFooter(_ hash: String, page: Int, of total: Int) {
        let y = pageSize.height - margins.bottom + 22
        let width = contentSize.width
        rule.setFill()
        NSRect(x: margins.left, y: y, width: width, height: 0.5).fill()
        NSAttributedString(string: hash, attributes: [.font: mono(6.5), .foregroundColor: muted])
            .draw(at: CGPoint(x: margins.left, y: y + 7))
        let number = L("Page %lld / %lld", page, total)
        let right = NSAttributedString(string: number, attributes: [.font: helvetica(7.5, bold: true), .foregroundColor: ink])
        right.draw(at: CGPoint(x: margins.left + width - right.size().width, y: y + 6))
    }

    // MARK: File

    /// "2020-06-23 Devis toiture.pdf": the name of the other exports
    /// (`nm-store`, `library.rs`, `export_message`): the reception date in
    /// UTC, then the subject without `/`, `:`, `\` or control characters,
    /// 80 characters at most, "message" when nothing is left.
    static func fileName(subject: String, received: Int) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "yyyy-MM-dd"
        let day = f.string(from: Date(timeIntervalSince1970: TimeInterval(received)))
        let cleaned = String(subject.map { c in
            c == "/" || c == ":" || c == "\\" || c.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) } ? " " : c
        })
        var words = cleaned.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        while words.hasPrefix(".") { words.removeFirst() }
        let stem = String(words.prefix(80)).trimmingCharacters(in: .whitespaces)
        return "\(day) \(stem.isEmpty ? "message" : stem).pdf"
    }

    /// Writes `data` into `dir` under `name`, or "name (2).pdf"… if a file
    /// has it: never overwrites (`.withoutOverwriting` makes the check and
    /// the creation one step, so two exports at once cannot collide).
    static func write(_ data: Data, in dir: URL, name: String) throws -> URL {
        var taken: Set<String> = []
        for _ in 0..<1000 {
            let url = AttachmentDisplay.freeURL(in: dir, name: name) {
                taken.contains($0.path) || FileManager.default.fileExists(atPath: $0.path)
            }
            do {
                try data.write(to: url, options: .withoutOverwriting)
                return url
            } catch let e as CocoaError where e.code == .fileWriteFileExists {
                taken.insert(url.path)
            }
        }
        throw CocoaError(.fileWriteFileExists)
    }
}
