import Foundation

/// How the geek theme presents attachments (`ui.md`, "Pièces jointes"):
/// tiles at the foot of each unfolded message, and rows of the `l` picker.
/// Pure functions of the attachment and of the reader's width, so they are
/// tested without any view (`Tests/NiceMailerTests/AttachmentDisplayTests.swift`).
enum AttachmentDisplay {
    /// SF Symbol of a file type, drawn in the theme's colours. Chosen from
    /// the MIME type first, then from the extension: senders often send
    /// Office files as `application/octet-stream`.
    static func symbol(mimeType: String, filename: String?) -> String {
        let mime = mimeType.lowercased()
        let ext = filename.flatMap { $0.split(separator: ".").count > 1 ? $0.split(separator: ".").last : nil }
            .map { $0.lowercased() } ?? ""
        // Swift note: `switch` on a tuple tests both values; `_` matches
        // anything, and the first matching case wins.
        switch (mime, ext) {
        case (_, "pdf"), ("application/pdf", _): return "doc.richtext"
        case (_, "ics"), ("text/calendar", _): return "calendar"
        case (_, "vcf"), ("text/vcard", _), ("text/x-vcard", _): return "person.crop.rectangle"
        case (_, "eml"), ("message/rfc822", _): return "envelope"
        case (_, "xls"), (_, "xlsx"), (_, "numbers"), (_, "csv"), (_, "ods"): return "tablecells"
        case (_, "ppt"), (_, "pptx"), (_, "key"), (_, "odp"): return "rectangle.on.rectangle"
        case (_, "doc"), (_, "docx"), (_, "pages"), (_, "odt"), (_, "rtf"): return "doc.text"
        case (_, "zip"), (_, "gz"), (_, "tgz"), (_, "7z"), (_, "rar"), (_, "tar"), (_, "dmg"),
             ("application/zip", _), ("application/gzip", _), ("application/x-7z-compressed", _):
            return "doc.zipper"
        case (_, "swift"), (_, "rs"), (_, "py"), (_, "js"), (_, "json"), (_, "xml"), (_, "sh"), (_, "html"):
            return "chevron.left.forwardslash.chevron.right"
        default: break
        }
        if mime.contains("spreadsheet") || mime.contains("excel") { return "tablecells" }
        if mime.contains("presentation") || mime.contains("powerpoint") { return "rectangle.on.rectangle" }
        if mime.contains("wordprocessing") || mime.contains("msword") || mime.contains("opendocument.text") {
            return "doc.text"
        }
        if mime.hasPrefix("image/") { return "photo" }
        if mime.hasPrefix("audio/") { return "waveform" }
        if mime.hasPrefix("video/") { return "film" }
        if mime.hasPrefix("text/") { return "doc.plaintext" }
        return "doc"
    }

    /// Whether a file is a graphic, left out by `sg` ("attachments but
    /// graphics": logos and signature images first of all, photos too).
    static func isGraphic(mimeType: String, filename: String?) -> Bool {
        if mimeType.lowercased().hasPrefix("image/") { return true }
        let ext = filename.map { ($0 as NSString).pathExtension.lowercased() } ?? ""
        return ["gif", "png", "jpg", "jpeg", "webp", "bmp", "svg", "heic", "heif", "tif", "tiff", "ico"].contains(ext)
    }

    /// The files `sa` / `sg` save from one thread: graphics left out unless
    /// `includingGraphics`, and a file attached again to a later message
    /// (same name, type and size) saved once.
    static func toSave(_ files: [AttachmentView], includingGraphics: Bool) -> [AttachmentView] {
        var seen = Set<String>()
        return files.filter { a in
            if !includingGraphics && isGraphic(mimeType: a.mimeType, filename: a.filename) { return false }
            return seen.insert("\(a.filename ?? "")|\(a.mimeType)|\(a.size)").inserted
        }
    }

    /// Name shown: the sender's, or a neutral one when it gave none.
    static func name(_ a: AttachmentView) -> String {
        let n = a.filename?.trimmingCharacters(in: .whitespaces) ?? ""
        return n.isEmpty ? L("Unnamed attachment") : n
    }

    /// Size in the interface's locale ("245 kB", "1,2 Mo").
    static func size(_ bytes: Int) -> String {
        Format.bytes(bytes)
    }

    /// `name` cut to `max` characters in the middle, keeping the end and its
    /// extension: "Rapport-ann…2025.pdf" says more than "Rapport-annuel-…".
    static func middleTruncated(_ name: String, max: Int) -> String {
        guard name.count > max, max > 3 else { return name }
        let tail = (max - 1) / 2
        let head = max - 1 - tail
        return String(name.prefix(head)) + "…" + String(name.suffix(tail))
    }

    /// Tiles per row: as many tiles of `tileChars` characters as fit in the
    /// reader, between 3 and 5 (`ui.md`).
    static func columns(width: Double, charWidth: Double, tileChars: Double = 24) -> Int {
        guard charWidth > 0, width.isFinite else { return 3 }
        return min(5, max(3, Int(width / (charWidth * tileChars))))
    }

    /// A file in `dir` named `name` that does not exist yet: "devis.pdf",
    /// then "devis (2).pdf", "devis (3).pdf"… An existing file is never
    /// overwritten, as with `export_message`.
    static func freeURL(in dir: URL, name: String, exists: (URL) -> Bool) -> URL {
        let first = dir.appendingPathComponent(name)
        if !exists(first) { return first }
        let ext = (name as NSString).pathExtension
        let stem = (name as NSString).deletingPathExtension
        var n = 2
        while true {
            let candidate = dir.appendingPathComponent(ext.isEmpty ? "\(stem) (\(n))" : "\(stem) (\(n)).\(ext)")
            if !exists(candidate) { return candidate }
            n += 1
        }
    }
}
