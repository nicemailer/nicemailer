//! Contact cards of the sender of a mail (`specifications/ia.md`, section 8,
//! "Fiche contact"): what the model found (the signature, the From name),
//! checked, and the vCard text (RFC 2426, vCard 3.0) of the card, for the
//! user's address book.
//!
//! Everything here is pure, as in [`crate::ics`]: no clock (the caller
//! passes the time of the revision stamp), no file (the app writes it).
//!
//! # Why vCard 3.0
//!
//! macOS Contacts reads and writes vCard 3.0; it imports 4.0 too, but drops
//! some of its properties, and other address books (Google Contacts,
//! Outlook) read 3.0 everywhere. The text escaping and the line folding are
//! those of iCalendar (RFC 2426, sections 4 and 2.6), so the helpers of
//! [`crate::ics`] are shared.

use crate::ics::{escape, file_stem, fold, utc_stamp};

/// What kind of number a phone is: the `TYPE` of its `TEL` line, which
/// Contacts shows as its label (mobile, work, home, work fax).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneKind {
    Mobile,
    Work,
    Home,
    Fax,
    Other,
}

impl PhoneKind {
    pub const ALL: [PhoneKind; 5] =
        [PhoneKind::Mobile, PhoneKind::Work, PhoneKind::Home, PhoneKind::Fax, PhoneKind::Other];

    /// The name the model and the clients use.
    pub fn as_str(self) -> &'static str {
        match self {
            PhoneKind::Mobile => "mobile",
            PhoneKind::Work => "work",
            PhoneKind::Home => "home",
            PhoneKind::Fax => "fax",
            PhoneKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<PhoneKind> {
        PhoneKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The `TYPE` parameters of the `TEL` line. A fax given in a signature
    /// is almost always the office's.
    fn vcard_types(self) -> &'static str {
        match self {
            PhoneKind::Mobile => "CELL,VOICE",
            PhoneKind::Work => "WORK,VOICE",
            PhoneKind::Home => "HOME,VOICE",
            PhoneKind::Fax => "WORK,FAX",
            PhoneKind::Other => "VOICE",
        }
    }
}

/// A phone number, as written in the mail ("+33 6 12 34 56 78"): the core
/// checks its characters, not its plan (the address book formats it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    pub kind: PhoneKind,
    pub number: String,
}

/// A postal address; every part may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Address {
    /// The street and number, and the building or the floor when given:
    /// several lines joined by a line break.
    pub street: String,
    pub locality: String,
    pub postcode: String,
    pub region: String,
    pub country: String,
}

impl Address {
    pub fn is_empty(&self) -> bool {
        [&self.street, &self.locality, &self.postcode, &self.region, &self.country].iter().all(|s| s.is_empty())
    }
}

/// The sender's card, as the model gave it and the core checked it. Every
/// text may be empty but one of the name, the company, an address or a
/// number is there (`answer::read`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// The name as the sender signs ("Alice Martin", "Dr Paul Durand").
    pub name: String,
    /// The first and the last name, as the address book sorts them.
    pub given: String,
    pub family: String,
    pub organization: String,
    /// The job title ("Sales manager").
    pub title: String,
    /// The From address first (the core puts it there), then the others
    /// of the signature; no address twice.
    pub emails: Vec<String>,
    pub phones: Vec<Phone>,
    pub address: Address,
    /// `http://` or `https://` links: the website, a profile page.
    pub urls: Vec<String>,
    /// The addresses, numbers and links the model gave that the core left
    /// out, not being one (`answer::read`), as written (cut at 80
    /// characters): shown in the panel, never put in the card.
    pub dropped: Vec<String>,
}

impl Contact {
    /// The name shown for the card (vCard's `FN`, required): the name, or
    /// the company for a card without one, or its first address.
    pub fn display_name(&self) -> &str {
        [&self.name, &self.organization]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(String::as_str)
            .or(self.emails.first().map(String::as_str))
            .unwrap_or("")
    }
}

/// The vCard text of a contact: one `VCARD`, lines ended by CRLF and folded
/// at 75 bytes (RFC 2426, section 2.6).
///
/// - `uid` identifies the card: the address book offers to update a card
///   it already has (`nicemailer-ai-<run>@nicemailer`).
/// - `stamp` is when the file is made, in Unix seconds (`REV`, UTC).
/// - `mail` is the link of the mail the card comes from
///   (`nicemailer://m/<id>`), put in the note: from the card, one click
///   goes back to the mail. Not in `URL`, which the address book would show
///   as the contact's website.
pub fn card(c: &Contact, uid: &str, stamp: i64, mail: Option<&str>) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCARD".into(),
        "VERSION:3.0".into(),
        "PRODID:-//NiceMailer//AI contact//EN".into(),
        format!("UID:{}", escape(uid)),
    ];
    // `N` is required, and it is what Contacts shows: a card with only an
    // `FN` shows as "No name". A name the model could not split goes whole
    // in the first name, which Contacts shows as written.
    let (family, given) = match (c.family.is_empty() && c.given.is_empty(), c.name.is_empty()) {
        (true, false) => (String::new(), c.name.clone()),
        _ => (c.family.clone(), c.given.clone()),
    };
    lines.push(format!("N:{};{};;;", escape(&family), escape(&given)));
    lines.push(format!("FN:{}", escape(c.display_name())));
    if !c.organization.is_empty() {
        // `ORG` has parts (company;department): its semicolons are escaped.
        lines.push(format!("ORG:{}", escape(&c.organization)));
    }
    if c.name.is_empty() && c.given.is_empty() && c.family.is_empty() && !c.organization.is_empty() {
        // A card of a company: Contacts shows and sorts it by the company
        // (its own extension, ignored by the other address books).
        lines.push("X-ABShowAs:COMPANY".into());
    }
    if !c.title.is_empty() {
        lines.push(format!("TITLE:{}", escape(&c.title)));
    }
    for (i, e) in c.emails.iter().enumerate() {
        // The From address is the preferred one: the address the sender
        // writes from.
        let pref = if i == 0 { ",PREF" } else { "" };
        lines.push(format!("EMAIL;TYPE=INTERNET{pref}:{}", escape(e)));
    }
    for p in &c.phones {
        lines.push(format!("TEL;TYPE={}:{}", p.kind.vcard_types(), escape(&p.number)));
    }
    if !c.address.is_empty() {
        let a = &c.address;
        // Post office box;extended address;street;locality;region;postcode;country.
        lines.push(format!(
            "ADR;TYPE=WORK:;;{};{};{};{};{}",
            escape(&a.street),
            escape(&a.locality),
            escape(&a.region),
            escape(&a.postcode),
            escape(&a.country)
        ));
    }
    for u in &c.urls {
        // A URI value: not escaped as a text.
        lines.push(format!("URL:{u}"));
    }
    if let Some(link) = mail {
        lines.push(format!("NOTE:{}", escape(link)));
    }
    lines.push(format!("REV:{}", utc_stamp(stamp)));
    lines.push("END:VCARD".into());
    let mut out = String::new();
    for l in &lines {
        out.push_str(&fold(l));
        out.push_str("\r\n");
    }
    out
}

/// The file name of a card: `Alice Martin.vcf`, made safe for a file name
/// as the calendar files are (`ics::file_stem`). The app adds " (2)" when
/// the file exists.
pub fn file_name(c: &Contact) -> String {
    let stem = file_stem(c.display_name());
    if stem.is_empty() { "Contact.vcf".into() } else { format!("{stem}.vcf") }
}

/// An address as the core accepts it: one `@`, a part before it, a domain
/// with a dot after it, no space, at most 254 characters (RFC 5321). Not a
/// full check of RFC 5322, which no address book needs; enough to refuse a
/// model that wrote "alice at acme dot fr" or a sentence.
pub fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    s.len() <= 254
        && !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control() || "<>,;\"".contains(c))
}

/// A phone number as the core accepts it: digits, spaces and the signs of
/// a written number (`( ) - . /`, and one `+` before the first digit, as in
/// "+33 6…" or "(+33)6…"), 4 to 20 digits, at most 40 characters. A number
/// with words ("06 12 34 56 78 (direct)") is refused: the address book
/// would dial the words.
pub fn is_phone(s: &str) -> bool {
    let digits = s.chars().filter(char::is_ascii_digit).count();
    // The `+` of the country code: once, and before any digit ("33+6" is
    // no number).
    let plus_ok = match s.find('+') {
        None => true,
        Some(at) => s.matches('+').count() == 1 && !s[..at].bytes().any(|b| b.is_ascii_digit()),
    };
    s.chars().count() <= 40
        && (4..=20).contains(&digits)
        && plus_ok
        && s.chars().all(|c| c.is_ascii_digit() || " ()-./+".contains(c))
}

/// A link as the core accepts it: `http://` or `https://`, then a host, no
/// space, at most 300 characters. "www.acme.fr" without its scheme is
/// refused: the prompt asks for the full link, and the card's link would
/// not open.
pub fn is_url(s: &str) -> bool {
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/'))
        && s.len() <= 300
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Contact {
        Contact {
            name: "Alice Martin".into(),
            given: "Alice".into(),
            family: "Martin".into(),
            organization: "Acme, Paris; Ventes".into(),
            title: "Directrice commerciale".into(),
            emails: vec!["alice@acme.fr".into(), "a.martin@gmail.com".into()],
            phones: vec![
                Phone { kind: PhoneKind::Mobile, number: "+33 6 12 34 56 78".into() },
                Phone { kind: PhoneKind::Fax, number: "01 23 45 67 89".into() },
            ],
            address: Address {
                street: "12 rue de la Paix\nBâtiment B".into(),
                locality: "Paris".into(),
                postcode: "75002".into(),
                region: String::new(),
                country: "France".into(),
            },
            urls: vec!["https://acme.fr".into()],
            dropped: vec!["www.acme.fr".into()],
        }
    }

    fn empty() -> Contact {
        Contact {
            name: String::new(),
            given: String::new(),
            family: String::new(),
            organization: String::new(),
            title: String::new(),
            emails: vec![],
            phones: vec![],
            address: Address::default(),
            urls: vec![],
            dropped: vec![],
        }
    }

    #[test]
    fn a_full_card_has_every_line_escaped_and_ends_with_crlf() {
        let v = card(&alice(), "nicemailer-ai-7@nicemailer", 1_763_000_000, Some("nicemailer://m/399914"));
        assert!(v.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"), "{v}");
        assert!(v.ends_with("END:VCARD\r\n"), "{v}");
        assert!(v.contains("\r\nUID:nicemailer-ai-7@nicemailer\r\n"), "{v}");
        assert!(v.contains("\r\nN:Martin;Alice;;;\r\nFN:Alice Martin\r\n"), "{v}");
        assert!(v.contains("\r\nORG:Acme\\, Paris\\; Ventes\r\n"), "{v}");
        assert!(v.contains("\r\nTITLE:Directrice commerciale\r\n"), "{v}");
        assert!(v.contains("\r\nEMAIL;TYPE=INTERNET,PREF:alice@acme.fr\r\nEMAIL;TYPE=INTERNET:a.martin@gmail.com\r\n"), "{v}");
        assert!(v.contains("\r\nTEL;TYPE=CELL,VOICE:+33 6 12 34 56 78\r\nTEL;TYPE=WORK,FAX:01 23 45 67 89\r\n"), "{v}");
        assert!(v.contains("\r\nADR;TYPE=WORK:;;12 rue de la Paix\\nBâtiment B;Paris;;75002;France\r\n"), "{v}");
        assert!(v.contains("\r\nURL:https://acme.fr\r\n"), "{v}");
        assert!(v.contains("\r\nNOTE:nicemailer://m/399914\r\n"), "{v}");
        // 1_763_000_000 is 2025-11-13 02:13:20 UTC.
        assert!(v.contains("\r\nREV:20251113T021320Z\r\n"), "{v}");
        assert!(!v.contains("X-ABShowAs"), "a person's card: {v}");
        assert!(!v.contains("www.acme.fr"), "what was left out is not in the card: {v}");
        assert_eq!(v.matches('\n').count(), v.matches("\r\n").count(), "no bare LF");
    }

    #[test]
    fn a_name_not_split_goes_whole_in_the_first_name() {
        let mut c = empty();
        c.name = "Alice Martin".into();
        c.emails = vec!["alice@acme.fr".into()];
        let v = card(&c, "u", 0, None);
        assert!(v.contains("\r\nN:;Alice Martin;;;\r\nFN:Alice Martin\r\n"), "{v}");
        // Empty parts are left out, the note without a mail too.
        for absent in ["ORG", "TITLE", "TEL", "ADR", "URL", "NOTE"] {
            assert!(!v.contains(&format!("\r\n{absent}")), "{absent}: {v}");
        }
    }

    #[test]
    fn a_company_without_a_name_is_shown_as_a_company() {
        let mut c = empty();
        c.organization = "Acme".into();
        c.emails = vec!["contact@acme.fr".into()];
        let v = card(&c, "u", 0, None);
        assert!(v.contains("\r\nN:;;;;\r\nFN:Acme\r\nORG:Acme\r\nX-ABShowAs:COMPANY\r\n"), "{v}");
        // Only an address: the card is named after it.
        let mut c = empty();
        c.emails = vec!["x@y.fr".into()];
        assert!(card(&c, "u", 0, None).contains("\r\nFN:x@y.fr\r\n"));
        assert_eq!(file_name(&c), "x@y.fr.vcf");
    }

    #[test]
    fn long_lines_are_folded_on_whole_characters() {
        let mut c = alice();
        c.title = "é".repeat(100);
        let v = card(&c, "u", 0, None);
        for line in v.split("\r\n") {
            assert!(line.len() <= 75, "{} bytes: {line}", line.len());
        }
        assert!(v.replace("\r\n ", "").contains(&format!("TITLE:{}\r\n", "é".repeat(100))), "{v}");
    }

    #[test]
    fn a_file_name_is_the_name_made_safe() {
        assert_eq!(file_name(&alice()), "Alice Martin.vcf");
        let mut c = empty();
        c.name = "Paul / Durand: \"PDG\"".into();
        assert_eq!(file_name(&c), "Paul Durand PDG.vcf");
        c.name = "..".into();
        assert_eq!(file_name(&c), "Contact.vcf");
    }

    #[test]
    fn addresses_numbers_and_links_are_checked() {
        for ok in ["alice@acme.fr", "a.b+tag@sub.example.co.uk"] {
            assert!(is_email(ok), "{ok}");
        }
        for bad in ["alice", "alice@acme", "@acme.fr", "alice@.fr", "alice@acme.", "a b@acme.fr", "a@b@c.fr", "<a@b.fr>", ""] {
            assert!(!is_email(bad), "{bad}");
        }
        for ok in ["+33 6 12 34 56 78", "(+33)6 01 47 26 04", "+33 (0)6 12 34 56 78", "06.12.34.56.78", "(555) 123-4567", "0123"] {
            assert!(is_phone(ok), "{ok}");
        }
        for bad in ["06 12 34 56 78 (direct)", "123", "33+6", "++33 6 12 34 56", "(+33)+6 12 34 56", "+33 6 12 34 56 78 ext 4", "", &"1".repeat(21)] {
            assert!(!is_phone(bad), "{bad}");
        }
        for ok in ["https://acme.fr", "http://acme.fr/equipe?x=1", "https://www.linkedin.com/in/alice"] {
            assert!(is_url(ok), "{ok}");
        }
        for bad in ["www.acme.fr", "acme.fr", "https://", "https:///x", "ftp://acme.fr", "https://acme.fr/a b", "javascript:alert(1)"] {
            assert!(!is_url(bad), "{bad}");
        }
    }

    #[test]
    fn phone_kinds_read_their_names() {
        for k in PhoneKind::ALL {
            assert_eq!(PhoneKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(PhoneKind::parse("cell"), None);
    }
}
