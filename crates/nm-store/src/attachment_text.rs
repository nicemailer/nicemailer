//! The text of an attachment, for the AI to summarize it
//! (`specifications/ia.md`, section 5, `attachment.text`): plain text,
//! HTML, PDF, DOCX and a mail attached (EML). Nothing is run or opened:
//! the bytes are read by parsers, never handed to another app.
//!
//! # Hostile files
//!
//! An attachment comes from anyone. Text, HTML and mail are read by the
//! code that already reads every incoming mail (`html_text.rs`,
//! `ingest.rs`). PDF and DOCX are read by parsers of other projects
//! (`pdf-extract`, `zip`), which a crafted file may send into a panic, a
//! loop or a huge allocation: [`Kind::isolated`] says so, and the core runs
//! those in a process of its own, killed after a delay
//! (`nm-daemon/src/extract.rs`), so that a bad file costs a failed summary,
//! never the core. The limits below hold in both cases.

use std::io::Read;

/// What an attachment is, for reading its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Html,
    Pdf,
    Docx,
    /// A mail attached (`message/rfc822`, `.eml`).
    Mail,
}

/// The MIME type of Word's documents (Office Open XML).
const DOCX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

impl Kind {
    /// The kind of a file from its MIME type, else from the extension of
    /// its name: senders often send any file as
    /// `application/octet-stream`. `None`: a file whose text cannot be read
    /// (an image, an archive, a spreadsheet…).
    pub fn of(mime_type: &str, filename: Option<&str>) -> Option<Kind> {
        let mime = mime_type.trim().to_ascii_lowercase();
        match mime.as_str() {
            "application/pdf" => return Some(Kind::Pdf),
            DOCX_MIME => return Some(Kind::Docx),
            "message/rfc822" => return Some(Kind::Mail),
            "text/html" | "application/xhtml+xml" => return Some(Kind::Html),
            m if m.starts_with("text/") => return Some(Kind::Text),
            _ => {}
        }
        let ext = filename
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "pdf" => Some(Kind::Pdf),
            "docx" => Some(Kind::Docx),
            "eml" => Some(Kind::Mail),
            "html" | "htm" => Some(Kind::Html),
            "txt" | "text" | "md" | "markdown" | "csv" | "tsv" | "log" => Some(Kind::Text),
            _ => None,
        }
    }

    /// The kind's name, for the subprocess that reads it and the message
    /// sent to the model.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Html => "html",
            Kind::Pdf => "pdf",
            Kind::Docx => "docx",
            Kind::Mail => "mail",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Text, Kind::Html, Kind::Pdf, Kind::Docx, Kind::Mail].into_iter().find(|k| k.as_str() == s)
    }

    /// Whether the file is read by a parser of another project, to run in
    /// a process of its own (module documentation, "Hostile files").
    pub fn isolated(self) -> bool {
        matches!(self, Kind::Pdf | Kind::Docx)
    }
}

/// The largest file read. A summary of a bigger one would read its first
/// pages only anyway, and reading it whole would take long.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// The largest text of a DOCX once unzipped: a zip bomb (a few kilobytes
/// that unzip to gigabytes) stops there.
const MAX_XML_BYTES: u64 = 64 * 1024 * 1024;

/// The most characters kept. The prompt keeps far fewer
/// (`nm_ai::message::MAX_ATTACHMENT_CHARS`); this bound only keeps what
/// goes through the pipe of the subprocess small.
pub const MAX_TEXT_CHARS: usize = 100_000;

/// Why a file's text cannot be read; the message says it to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unreadable {
    #[error("this kind of file has no text the AI can read")]
    Unsupported,
    #[error("the file is too large to read ({0} MB, 50 MB at most)")]
    TooLarge(u64),
    #[error("the file is protected by a password")]
    Encrypted,
    #[error("no text in this file: a scan or an image?")]
    Empty,
    #[error("the file cannot be read: {0}")]
    Damaged(String),
}

/// The text of a file of this kind, cleaned up (see [`tidy`]) and cut at
/// [`MAX_TEXT_CHARS`].
pub fn extract(bytes: &[u8], kind: Kind) -> Result<String, Unreadable> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Unreadable::TooLarge(bytes.len() as u64 / (1024 * 1024)));
    }
    let raw = match kind {
        Kind::Text => decode(bytes),
        Kind::Html => crate::html_text::html_to_text(&decode(bytes)),
        Kind::Pdf => pdf(bytes)?,
        Kind::Docx => docx(bytes)?,
        Kind::Mail => mail(bytes)?,
    };
    let text = tidy(&raw);
    if text.is_empty() {
        return Err(Unreadable::Empty);
    }
    Ok(match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text,
    })
}

/// Text bytes as a string. An attachment's charset is not kept with it
/// (only its MIME type is), so it is guessed: a byte order mark says
/// UTF-8 or UTF-16; else valid UTF-8 is UTF-8; else Windows-1252, the
/// superset of Latin-1 that old Western mail software writes, which maps
/// every byte to a character and so never fails.
fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding.decode_without_bom_handling(&bytes[bom..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

fn pdf(bytes: &[u8]) -> Result<String, Unreadable> {
    // Rust note: a panic unwinds the stack up to `catch_unwind`, which
    // turns it into an `Err` instead of ending the thread. `pdf-extract`
    // panics on some damaged files rather than returning an error.
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    match result {
        Err(_) => Err(Unreadable::Damaged("the PDF reader failed on it".into())),
        // A PDF locked for reading. One locked only for editing has an
        // empty reading password, which `pdf-extract` tries itself.
        Ok(Err(pdf_extract::OutputError::PdfError(pdf_extract::Error::Decryption(_)))) => Err(Unreadable::Encrypted),
        Ok(Err(e)) => Err(Unreadable::Damaged(e.to_string())),
        Ok(Ok(text)) => Ok(text),
    }
}

/// A DOCX is a zip of XML files; the text is in `word/document.xml`.
fn docx(bytes: &[u8]) -> Result<String, Unreadable> {
    // A Word file locked by a password is not a zip but an OLE container:
    // it ends up here too, hence the two causes in the message.
    let damaged = || Unreadable::Damaged("it is not a DOCX file Word can open without a password".into());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| damaged())?;
    let file = zip.by_name("word/document.xml").map_err(|_| damaged())?;
    let mut xml = Vec::new();
    // Rust note: `take` stops reading after that many bytes, whatever the
    // zip claims the size is: the claim is the sender's.
    file.take(MAX_XML_BYTES + 1).read_to_end(&mut xml).map_err(|e| Unreadable::Damaged(e.to_string()))?;
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err(Unreadable::TooLarge(MAX_XML_BYTES / (1024 * 1024)));
    }
    Ok(docx_text(&String::from_utf8_lossy(&xml)))
}

/// The text of Word's XML: the runs of text (`<w:t>`), a line per
/// paragraph (`</w:p>`), the tabs and line breaks the user typed. Deleted
/// text of tracked changes (`<w:delText>`) and field codes
/// (`<w:instrText>`) are not text the reader sees: left out. The tab
/// stops of a paragraph's style (`<w:tabs><w:tab …/></w:tabs>`) are no
/// tab either.
///
/// A scan of the tags is enough here: the XML is Word's, flat, and only
/// these few elements matter.
fn docx_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    let (mut in_text, mut in_tabs) = (false, false);
    while let Some(lt) = rest.find('<') {
        if in_text {
            out.push_str(&decode_xml(&rest[..lt]));
        }
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        let closing = tag.starts_with('/');
        let empty = tag.ends_with('/');
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match (name, closing) {
            ("w:t", false) => in_text = !empty,
            ("w:t", true) => in_text = false,
            ("w:tabs", false) => in_tabs = !empty,
            ("w:tabs", true) => in_tabs = false,
            ("w:tab", false) if !in_tabs => out.push('\t'),
            ("w:br" | "w:cr", false) => out.push('\n'),
            ("w:p", true) => out.push('\n'),
            ("w:p", false) if empty => out.push('\n'),
            _ => {}
        }
    }
    out
}

/// The five XML entities and the numeric references.
fn decode_xml(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let c = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A mail attached: its headers, then its text, as the reader would show
/// it (`ingest::parse`, the code that reads every incoming mail).
fn mail(bytes: &[u8]) -> Result<String, Unreadable> {
    let m = crate::ingest::parse(bytes);
    if !m.parsed {
        return Err(Unreadable::Damaged("it is not a mail".into()));
    }
    let who = |a: &crate::ingest::ParsedAddress| match a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} <{}>", a.addr),
        None => a.addr.clone(),
    };
    let mut lines = Vec::new();
    if let Some(f) = &m.from {
        lines.push(format!("From: {}", who(f)));
    }
    if !m.to.is_empty() {
        lines.push(format!("To: {}", m.to.iter().map(who).collect::<Vec<_>>().join(", ")));
    }
    if let Some(at) = m.date_sent {
        // In UTC: the attached mail's own offset is not kept by the parse.
        lines.push(format!("Date: {}", nm_core::compose::rfc5322_date(at, 0)));
    }
    lines.push(format!("Subject: {}", m.subject));
    let names: Vec<&str> = m.attachments.iter().filter(|a| !a.is_inline).filter_map(|a| a.filename.as_deref()).collect();
    if !names.is_empty() {
        lines.push(format!("Attachments: {}", names.join(", ")));
    }
    Ok(format!("{}\n\n{}", lines.join("\n"), m.text.unwrap_or_default()))
}

/// The text as the model should read it: lines ending without spaces,
/// page breaks (a PDF's form feeds) as blank lines, at most one blank line
/// in a row, no control characters but tabs and line ends, nothing blank
/// around. PDFs give many blank lines and trailing spaces, which cost
/// tokens and say nothing.
pub fn tidy(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace(['\r', '\u{c}'], "\n");
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line: String = line.chars().filter(|c| !c.is_control() || *c == '\t').collect();
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_comes_from_the_type_then_the_name() {
        assert_eq!(Kind::of("application/pdf", None), Some(Kind::Pdf));
        assert_eq!(Kind::of("Application/PDF", Some("x.bin")), Some(Kind::Pdf));
        assert_eq!(Kind::of("application/octet-stream", Some("Devis.PDF")), Some(Kind::Pdf));
        assert_eq!(Kind::of("application/octet-stream", Some("lettre.docx")), Some(Kind::Docx));
        assert_eq!(Kind::of(DOCX_MIME, None), Some(Kind::Docx));
        assert_eq!(Kind::of("message/rfc822", None), Some(Kind::Mail));
        assert_eq!(Kind::of("text/html", Some("a.txt")), Some(Kind::Html));
        assert_eq!(Kind::of("text/csv", None), Some(Kind::Text));
        assert_eq!(Kind::of("application/octet-stream", Some("notes.md")), Some(Kind::Text));
        for (mime, name) in [("image/png", Some("a.png")), ("application/zip", Some("a.zip")), ("application/msword", Some("a.doc")),
                             ("application/octet-stream", None), ("application/octet-stream", Some("noext"))] {
            assert_eq!(Kind::of(mime, name), None, "{mime} {name:?}");
        }
        for k in [Kind::Text, Kind::Html, Kind::Pdf, Kind::Docx, Kind::Mail] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert!(Kind::Pdf.isolated() && Kind::Docx.isolated() && !Kind::Html.isolated());
    }

    #[test]
    fn text_is_decoded_whatever_its_charset() {
        assert_eq!(extract("Déjà vu\r\n".as_bytes(), Kind::Text).unwrap(), "Déjà vu");
        // Windows-1252: "é" is 0xE9, "€" 0x80.
        assert_eq!(extract(b"Caf\xe9 \x80", Kind::Text).unwrap(), "Café €");
        // UTF-16 with its byte order mark.
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Été".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(extract(&utf16, Kind::Text).unwrap(), "Été");
    }

    #[test]
    fn html_is_read_as_the_reader_shows_it() {
        let t = extract(b"<html><style>p{}</style><p>Bonjour&nbsp;Paul</p><p>Devis <b>joint</b></p></html>", Kind::Html).unwrap();
        assert!(t.contains("Bonjour\u{a0}Paul") || t.contains("Bonjour Paul"), "{t:?}");
        assert!(t.contains("Devis joint") && !t.contains("<b>") && !t.contains("p{}"), "{t:?}");
    }

    #[test]
    fn the_text_is_tidied_and_blank_files_refused() {
        assert_eq!(tidy("  \n\nA  \n\n\n\nB\u{c}C\u{7}\tD\n\n"), "A\n\nB\nC\tD");
        assert_eq!(extract(b"  \n\t\n", Kind::Text), Err(Unreadable::Empty));
    }

    #[test]
    fn a_long_text_is_cut() {
        let long = "é".repeat(MAX_TEXT_CHARS + 10);
        assert_eq!(extract(long.as_bytes(), Kind::Text).unwrap().chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn a_file_too_large_is_not_read() {
        let big = vec![b'a'; MAX_FILE_BYTES as usize + 1];
        assert!(matches!(extract(&big, Kind::Text), Err(Unreadable::TooLarge(50))));
    }

    /// A DOCX of one paragraph per string, in Word's XML.
    pub(crate) fn docx_of(xml_body: &str) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("[Content_Types].xml", opts).unwrap();
        w.write_all(b"<Types/>").unwrap();
        w.start_file("word/document.xml", opts).unwrap();
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{xml_body}</w:body></w:document>"
        );
        w.write_all(xml.as_bytes()).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn a_docx_gives_its_paragraphs() {
        let file = docx_of(
            "<w:p><w:pPr><w:tabs><w:tab w:val=\"left\" w:pos=\"720\"/></w:tabs></w:pPr>\
             <w:r><w:t>Devis n°</w:t></w:r><w:r><w:t xml:space=\"preserve\"> 42 &amp; suite</w:t></w:r></w:p>\
             <w:p/>\
             <w:p><w:r><w:t>Total</w:t><w:tab/><w:t>1&#160;200 €</w:t><w:br/><w:t>TTC</w:t></w:r></w:p>\
             <w:p><w:r><w:delText>supprimé</w:delText><w:instrText>PAGE</w:instrText></w:r></w:p>",
        );
        assert_eq!(extract(&file, Kind::Docx).unwrap(), "Devis n° 42 & suite\n\nTotal\t1\u{a0}200 €\nTTC");
    }

    #[test]
    fn a_damaged_docx_or_pdf_is_refused_with_a_reason() {
        assert!(matches!(extract(b"PK\x03\x04 not really", Kind::Docx), Err(Unreadable::Damaged(_))));
        // An OLE container: a Word file locked by a password.
        assert!(matches!(extract(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", Kind::Docx), Err(Unreadable::Damaged(_))));
        assert!(matches!(extract(b"%PDF-1.4\n garbage", Kind::Pdf), Err(Unreadable::Damaged(_))));
        assert!(matches!(extract(b"", Kind::Pdf), Err(Unreadable::Damaged(_))));
    }

    /// A one-page PDF that shows `text` in Helvetica, its offsets computed.
    pub(crate) fn pdf_of(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }

    #[test]
    fn a_pdf_gives_its_text() {
        let t = extract(&pdf_of("Facture 2026-114 : 1 200 EUR"), Kind::Pdf).unwrap();
        assert_eq!(t, "Facture 2026-114 : 1 200 EUR");
    }

    #[test]
    fn an_attached_mail_gives_its_headers_and_text() {
        let eml = b"From: Alice <alice@a.example>\r\nTo: bob@b.example\r\nSubject: Planning\r\nDate: Tue, 23 Jun 2020 19:20:56 +0200\r\n\r\nOn se voit jeudi.\r\n";
        let t = extract(eml, Kind::Mail).unwrap();
        assert_eq!(t, "From: Alice <alice@a.example>\nTo: bob@b.example\nDate: Tue, 23 Jun 2020 17:20:56 +0000\nSubject: Planning\n\nOn se voit jeudi.");
    }
}
