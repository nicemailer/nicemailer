//! People and robots (`specifications/expediteurs.md`): the clues a
//! message's headers give about who wrote it, the nature of the message
//! they make, and the verdict on an address drawn from the natures of its
//! mails.
//!
//! This module is pure, like `ingest.rs`: headers in, bits out; counts in,
//! a verdict out. No database. The rules were built on a real mailbox of
//! 400,000 mails (`expediteurs.md`, section 3); each list below says which
//! trap of the measures it avoids.
//!
//! The clues of a message are stored as bits in `message.sender_signals`,
//! with the bits of its nature (`NATURE_*`), so that the core can count the
//! mails of every address by nature in SQL (the catch-up of the mails
//! stored before this module, `senders.rs`) and decide whether a
//! mail is automatic without parsing it again.

use std::collections::HashSet;
use std::sync::OnceLock;

use nm_core::SenderKind;

// ----------------------------------------------------------------
// Clues (`expediteurs.md`, section 4)
// ----------------------------------------------------------------

/// `List-Unsubscribe`, outside a discussion list.
pub const LIST_UNSUBSCRIBE: i64 = 1 << 0;
/// `Feedback-ID`: a bulk sender's complaint loop (Gmail).
pub const FEEDBACK_ID: i64 = 1 << 1;
/// `Precedence: bulk` or `junk`, outside a discussion list.
pub const PRECEDENCE_BULK: i64 = 1 << 2;
/// `Auto-Submitted: auto-generated`.
pub const AUTO_GENERATED: i64 = 1 << 3;
/// `Auto-Submitted: auto-replied`, `X-Autoreply`, `X-Autorespond`.
pub const AUTO_REPLIED: i64 = 1 << 4;
/// A `noreply` local part, in any spelling.
pub const NOREPLY: i64 = 1 << 5;
/// `X-Mailer` / `User-Agent` of a sending platform or server software.
pub const ROBOT_MAILER: i64 = 1 << 6;
/// A header only sending platforms add (`X-SES-`, `X-Mandrill-`…).
pub const PLATFORM: i64 = 1 << 7;
/// A discussion list: `List-Post`, `X-BeenThere`, Google Groups.
pub const DISCUSSION: i64 = 1 << 8;
/// `X-Mailer` / `User-Agent` of a mail client or a browser.
pub const MAIL_CLIENT: i64 = 1 << 9;
/// `Message-ID` made by Gmail.
pub const GMAIL_ID: i64 = 1 << 10;
/// A reply: `In-Reply-To` or `References`.
pub const REPLY: i64 = 1 << 11;

// The nature of the message (`nature`), stored with the clues: exactly one
// of these bits is set on a message whose clues were read.

/// An automatic reply (out of office, calendar answer).
pub const NATURE_AUTO: i64 = 1 << 16;
/// A bulk mail, or a mail relayed by a platform with no sign of a person.
pub const NATURE_BULK: i64 = 1 << 17;
/// A mail of a discussion list.
pub const NATURE_LIST: i64 = 1 << 18;
/// A mail with a sign of a person.
pub const NATURE_HUMAN: i64 = 1 << 19;
/// No clue at all.
pub const NATURE_NONE: i64 = 1 << 20;

/// The bits of a message that was written by a machine itself, whoever its
/// address belongs to (`expediteurs.md`, section 6): its `sender_kind` is
/// `robot` whatever the computed verdict on its address. A verdict forced
/// by the user wins over these bits (see [`message_kind`]).
pub const MACHINE: i64 = NATURE_AUTO | NATURE_BULK;

/// The clues of a person that are not a reply: what rule 8 counts.
pub const CLIENT: i64 = MAIL_CLIENT | GMAIL_ID;

const BULK: i64 = LIST_UNSUBSCRIBE | FEEDBACK_ID | PRECEDENCE_BULK | NOREPLY | ROBOT_MAILER;
const HUMAN: i64 = MAIL_CLIENT | GMAIL_ID | REPLY;

/// The nature of a message, from its clues (`expediteurs.md`, section 4):
/// in this order, automatic reply, bulk, discussion list, person, platform
/// relay (bulk), nothing.
///
/// `Auto-Submitted: auto-generated` alone is an automatic reply: Exchange
/// puts it on the answers to calendar invitations, written for a person.
/// With another bulk clue, it is a bulk mail (a notification).
pub fn nature(clues: i64) -> i64 {
    if clues & AUTO_REPLIED != 0 {
        return NATURE_AUTO;
    }
    if clues & BULK != 0 {
        return NATURE_BULK;
    }
    if clues & AUTO_GENERATED != 0 {
        return NATURE_AUTO;
    }
    if clues & DISCUSSION != 0 {
        return NATURE_LIST;
    }
    // A person relayed by a platform (Mandrill for a company's staff) is
    // still a person: the platform header only counts without any sign of
    // one.
    if clues & HUMAN != 0 {
        return NATURE_HUMAN;
    }
    if clues & PLATFORM != 0 {
        return NATURE_BULK;
    }
    NATURE_NONE
}

/// The clues of a message and its nature, as stored in
/// `message.sender_signals`.
///
/// `headers`: every header field, name and raw value, in any order.
/// `from`: the sender's address, lowercase. `message_id`: without angle
/// brackets. `reply`: the message has `In-Reply-To` or `References`.
pub fn signals<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    from: Option<&str>,
    message_id: Option<&str>,
    reply: bool,
) -> i64 {
    let mut c = 0;
    // Read first, applied after: a discussion list cancels the unsubscribe
    // link and the precedence, whichever header comes first.
    let mut unsubscribe = false;
    let mut bulk_precedence = false;
    for (name, value) in headers {
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "list-unsubscribe" => unsubscribe = true,
            "feedback-id" => c |= FEEDBACK_ID,
            "precedence" => {
                let v = value.to_ascii_lowercase();
                if v == "bulk" || v == "junk" {
                    bulk_precedence = true;
                }
            }
            "auto-submitted" => {
                let v = value.to_ascii_lowercase();
                if v.starts_with("auto-replied") {
                    c |= AUTO_REPLIED;
                } else if v != "no" && !v.is_empty() {
                    c |= AUTO_GENERATED;
                }
            }
            "x-autoreply" | "x-autorespond" => c |= AUTO_REPLIED,
            // `List-Post: NO` is the RFC 2369 way for an announce list to
            // say nobody posts to it: a newsletter, not a discussion.
            "list-post" if !value.eq_ignore_ascii_case("no") => c |= DISCUSSION,
            "x-beenthere" => c |= DISCUSSION,
            "list-id" if value.to_ascii_lowercase().contains("googlegroups.com") => c |= DISCUSSION,
            "x-mailer" | "user-agent" => c |= mailer(value),
            _ if is_platform_header(&name) => c |= PLATFORM,
            _ => {}
        }
    }
    if c & DISCUSSION == 0 {
        if unsubscribe {
            c |= LIST_UNSUBSCRIBE;
        }
        if bulk_precedence {
            c |= PRECEDENCE_BULK;
        }
    }
    if from.and_then(|f| f.split('@').next()).is_some_and(is_noreply) {
        c |= NOREPLY;
    }
    if message_id.is_some_and(|id| id.to_ascii_lowercase().ends_with("@mail.gmail.com")) {
        c |= GMAIL_ID;
    }
    if reply {
        c |= REPLY;
    }
    c | nature(c)
}

/// Software of sending platforms and servers, as `X-Mailer` names them
/// (lowercase, matched anywhere). PHPMailer is not here: webmails of real
/// people use it (816 mails of one correspondent in the measures).
const ROBOT_MAILERS: &[&str] = &[
    "mailchimp", "sendinblue", "brevo", "activecampaign", "customer.io", "nlserver", "nms - 1:1", "sendy",
    "mailjet", "sendgrid", "hubspot", "salesforce", "marketo", "klaviyo", "mailgun", "sparkpost", "amazon ses",
    "emarsys", "selligent", "splio", "dolist", "cabestan", "actito", "mailpoet", "newsletter", "massmail",
    "mass mail", "emailing", "e-mailing", "campaign", "ediware", "python ai mailer", "framework::mail", "zendesk",
    "freshdesk", "helpscout", "intercom", "wordpress", "drupal", "joomla", "prestashop", "magento", "swiftmailer",
    "swift mailer", "symfony", "laravel", "nodemailer", "javamail", "java mail", "cdo for windows", "formmail",
    "jenkins", "gitlab", "redmine", "nagios", "zabbix", "sympa",
];

/// Mail clients and browsers (webmails), lowercase, matched anywhere.
const MAIL_CLIENTS: &[&str] = &[
    "apple mail", "iphone mail", "ipad mail", "thunderbird", "outlook", "entourage", "eudora", "mutt", "alpine",
    "roundcube", "evolution", "kmail", "sogo", "horde", "airmail", "spark", "postbox", "mailmate", "k-9",
    "fairemail", "em client", "windows live mail", "windows mail", "lotus notes", "zimbra", "rainloop",
    "squirrelmail", "claws", "sylpheed", "mozilla/", "seamonkey", "opera", "gnus", "mailspring", "canary",
    "newton", "bluemail", "blue mail", "mimestream", "superhuman", "polymail", "mailbird", "the bat", "pegasus",
    "becky", "mulberry", "internet mail service", "microsoft exchange", "open-xchange", "ox app suite", "kerio",
    "samsung", "android", "ios",
];

/// The clue of an `X-Mailer` or `User-Agent` value: a platform first (a
/// platform may name the mail client it imitates), then a mail client.
fn mailer(value: &str) -> i64 {
    let v = value.to_ascii_lowercase();
    if ROBOT_MAILERS.iter().any(|m| v.contains(m)) {
        ROBOT_MAILER
    } else if MAIL_CLIENTS.iter().any(|m| v.contains(m)) {
        MAIL_CLIENT
    } else {
        0
    }
}

/// Header prefixes only sending platforms add, lowercase. Precise
/// prefixes: security gateways add look-alikes that must not count, Trend
/// Micro (`x-tm-…`), Mimecast (`x-mc-unique`, hence `x-mc-user` and not
/// `x-mc-`), Proofpoint (`x-ems-…`). Google Groups (`x-google-group-…`) is
/// a discussion list, not a platform.
const PLATFORM_HEADERS: &[&str] = &[
    "x-ses-", "x-sg-", "x-mailgun-", "x-mc-user", "x-mandrill-", "x-pm-", "x-csa-", "x-msfbl", "x-sib-",
    "x-mailjet-", "x-campaign", "x-mailchimp", "x-hs-", "x-hubspot", "x-sfmc", "x-rpcampaign", "x-emarsys",
    "x-sendinblue", "x-mktomail", "x-marketo", "x-cmae", "x-cm-", "x-report-abuse", "x-complaints",
    "x-feedback", "x-postmark", "x-sparkpost", "x-msys", "x-zendesk", "x-freshdesk", "x-intercom", "x-github",
    "x-gitlab", "x-jira", "x-atlassian", "x-linkedin", "x-facebook", "x-mailer-sid", "x-job", "x-bulk",
];

fn is_platform_header(name: &str) -> bool {
    PLATFORM_HEADERS.iter().any(|p| name.starts_with(p))
}

/// The `noreply` words, separators removed (`no-reply`, `no_reply`,
/// `ne.pas.repondre` all become one of these).
const NOREPLY_WORDS: &[&str] = &[
    "noreply", "donotreply", "nepasrepondre", "pasdereponse", "noreponse", "noresponse", "mailerdaemon",
    "postmaster", "bounce",
];

/// Whether a local part is a `noreply` one, at its start or after a
/// separator: `avisage-ne-pas-repondre` (Chronopost) is one, as is
/// `no_reply` (Apple); `jeanbounce` is not.
pub fn is_noreply(local: &str) -> bool {
    let l = local.to_ascii_lowercase();
    let bytes = l.as_bytes();
    (0..bytes.len()).filter(|&i| i == 0 || matches!(bytes[i - 1], b'.' | b'_' | b'+' | b'-')).any(|i| {
        let squeezed: String = l[i..].chars().filter(|c| !matches!(c, '.' | '_' | '-')).collect();
        NOREPLY_WORDS.iter().any(|w| squeezed.starts_with(w))
    })
}

// ----------------------------------------------------------------
// Verdict on an address (`expediteurs.md`, section 5)
// ----------------------------------------------------------------

/// The mails of an address counted by nature (`address.sender_*`). The
/// automatic replies are not counted: an out-of-office message says nothing
/// about who writes the other mails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Mails counted (every nature but the automatic replies).
    pub count: i64,
    pub bulk: i64,
    pub list: i64,
    pub human: i64,
    /// Mails with the clue of a mail client or of Gmail (rule 8).
    pub client: i64,
}

impl Counts {
    /// Counts one more mail of these clues.
    pub fn add(&mut self, signals: i64) {
        if signals & NATURE_AUTO != 0 {
            return;
        }
        self.count += 1;
        self.bulk += i64::from(signals & NATURE_BULK != 0);
        self.list += i64::from(signals & NATURE_LIST != 0);
        self.human += i64::from(signals & NATURE_HUMAN != 0);
        self.client += i64::from(signals & CLIENT != 0);
    }
}

/// What the verdict on an address knows of it besides its counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    /// The user wrote to it (`address.written_to`).
    pub written_to: bool,
    /// It is one of the user's own addresses (`account.address`).
    pub own: bool,
}

/// The computed verdict on `addr` (`expediteurs.md`, section 5; rule 1,
/// the verdict forced by the user, is applied by the caller). The shares
/// are compared in integers (`2 * part >= whole` for "at least half") so
/// that a threshold never depends on a rounding.
pub fn verdict(addr: &str, c: Counts, f: Facts) -> SenderKind {
    use SenderKind::{Person, Robot};
    let at_least_half = |part: i64| 2 * part >= c.count;
    // 2. The user's own address.
    if f.own {
        return Person;
    }
    // 3. Nothing counted: only automatic replies, or no mail received.
    if c.count == 0 {
        return if f.written_to { Person } else { Robot };
    }
    // 4. A discussion list: people write there.
    if at_least_half(c.list) {
        return Person;
    }
    // 5. Written to: a person, unless it almost never writes like one. 10%
    // keeps a shop whose owner writes and sends a newsletter from the same
    // address (`expediteurs.md`, "Adresses mixtes").
    if f.written_to {
        return if 10 * c.human >= c.count || 2 * c.bulk < c.count { Person } else { Robot };
    }
    // 6. Mostly bulk.
    if at_least_half(c.bulk) {
        return Robot;
    }
    let (local, domain) = addr.rsplit_once('@').unwrap_or((addr, ""));
    // 7. A service local part.
    if is_role(local) {
        return Robot;
    }
    // 8. Mostly written with a mail client.
    if at_least_half(c.client) {
        return Person;
    }
    // 9. No bulk at all, and a public domain or mostly replies.
    if c.bulk == 0 && (is_public_domain(domain) || at_least_half(c.human)) {
        return Person;
    }
    // 10. Never written to, no sign of a person: a machine.
    Robot
}

/// The `sender_kind` of a message (`expediteurs.md`, section 6): `person`
/// for the user's own mail; the verdict on its address when the user forced
/// it (`forced`), for every mail of the address, automatic ones included:
/// the user's decision is on the sender, and all its mails must show it;
/// else `robot` for an automatic mail whatever its address, else the
/// computed verdict on its address.
pub fn message_kind(signals: Option<i64>, own_mail: bool, address: Option<SenderKind>, forced: bool) -> Option<SenderKind> {
    if own_mail {
        return Some(SenderKind::Person);
    }
    if forced {
        return address;
    }
    if signals.is_some_and(|s| s & MACHINE != 0) {
        return Some(SenderKind::Robot);
    }
    address
}

/// Service words of a local part (`info`, `contact`, `support`…). `true`
/// in the second place: the word may go on (`factures`, `contacts`,
/// `ticketing`); otherwise it must end the local part or be followed by a
/// separator or a digit (`info.fr`, `news2`), so that `infante.marie` is no
/// `info`.
const ROLE_WORDS: &[(&str, bool)] = &[
    ("info", false), ("infos", false), ("contact", true), ("news", false), ("newsletter", true),
    ("notification", true), ("notify", false), ("alert", true), ("support", false), ("service", true),
    ("serviceclient", true), ("hello", false), ("bonjour", false), ("team", false), ("equipe", false),
    ("billing", false), ("factur", true), ("invoice", true), ("account", true), ("compte", false),
    ("marketing", false), ("offre", true), ("promo", false), ("communication", false), ("admin", false),
    ("webmaster", false), ("mail", false), ("email", false), ("e-mail", false), ("order", true),
    ("commande", true), ("shop", false), ("boutique", false), ("store", false), ("sales", false), ("vente", true),
    ("digest", false), ("update", true), ("automated", false), ("system", false), ("robot", false),
    ("daemon", false), ("root", false), ("cron", false), ("monitoring", false), ("calendar", false),
    ("agenda", false), ("reply", false), ("reponse", false), ("reservation", true), ("booking", false),
    ("welcome", false), ("security", false), ("securite", false), ("verify", false), ("verification", false),
    ("feedback", false), ("survey", false), ("enquete", false), ("club", false), ("membre", true),
    ("member", true), ("crm", false), ("actu", true), ("event", true), ("evenement", true), ("rh", false),
    ("recrutement", false), ("job", false), ("jobs", false), ("career", true), ("press", false), ("presse", false),
    ("edition", true), ("abonnement", true), ("subscription", true), ("client", true), ("customer", true),
    ("care", false), ("help", false), ("aide", false), ("assistance", false), ("hotline", false),
    ("ticket", true), ("desk", false), ("registrar", false), ("hostmaster", false), ("abuse", false),
    ("noc", false), ("ops", false), ("devops", false), ("api", false), ("app", false), ("apps", false), ("cloud", false),
    ("domain", true), ("renew", true), ("expiration", false), ("paiement", false), ("payment", true),
    ("banque", false), ("bank", false), ("courrier", false), ("lettre", false), ("magazine", false),
    ("editorial", false), ("redaction", false),
];

/// Whether a local part is a service one (rule 7).
pub fn is_role(local: &str) -> bool {
    let l = local.to_ascii_lowercase();
    ROLE_WORDS.iter().any(|&(w, goes_on)| {
        l.strip_prefix(w).is_some_and(|rest| match rest.chars().next() {
            None => true,
            Some(c) if matches!(c, '.' | '_' | '+' | '-') || c.is_ascii_digit() => true,
            // `contacts`, `factures`: letters may follow some words.
            Some(_) => goes_on,
        })
    })
}

/// The public mail domains (`domaines-publics.md`), from the list kept in
/// the repository. Built once, at the first use.
///
/// Rust note: `OnceLock` holds a value computed at most once, by the first
/// thread that asks; the others wait for it, then share it.
fn public_domains() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        include_str!("../../../data/public-domains/public-domains.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    })
}

pub fn is_public_domain(domain: &str) -> bool {
    public_domains().contains(domain.trim().to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nm_core::SenderKind::{Person, Robot};

    fn sig(headers: &[(&str, &str)], from: &str) -> i64 {
        signals(headers.iter().copied(), Some(from), None, false)
    }

    #[test]
    fn a_newsletter_is_bulk() {
        let s = sig(&[("List-Unsubscribe", "<https://x>"), ("X-Mailer", "MailChimp Mailer - **CID1**")], "list@ben-evans.com");
        assert_ne!(s & LIST_UNSUBSCRIBE, 0);
        assert_ne!(s & ROBOT_MAILER, 0);
        assert_eq!(s & (NATURE_AUTO | NATURE_BULK | NATURE_LIST | NATURE_HUMAN | NATURE_NONE), NATURE_BULK);
    }

    #[test]
    fn a_discussion_list_cancels_the_unsubscribe_link() {
        // The Google Group of the measures: List-Unsubscribe on every mail.
        let s = sig(&[("List-Unsubscribe", "<mailto:x>"), ("List-Id", "<pommef.googlegroups.com>"), ("Precedence", "list")], "a@gmail.com");
        assert_eq!(s & LIST_UNSUBSCRIBE, 0);
        assert_ne!(s & NATURE_LIST, 0);
        let s = sig(&[("List-Post", "<mailto:entraide@lists.afup.org>"), ("Precedence", "bulk")], "a@b.fr");
        assert_eq!(s & PRECEDENCE_BULK, 0);
        assert_ne!(s & NATURE_LIST, 0);
        // An announce list says nobody posts to it.
        let s = sig(&[("List-Post", "NO"), ("List-Unsubscribe", "<x>")], "news@b.fr");
        assert_ne!(s & NATURE_BULK, 0);
    }

    #[test]
    fn security_gateways_are_not_platforms() {
        for h in ["X-TM-SNTS-SMTP", "X-MC-Unique", "X-EMS-Stamp", "X-Google-Group-Id"] {
            assert_eq!(sig(&[(h, "x")], "frederic.louisy@bpce.fr") & PLATFORM, 0, "{h}");
        }
        assert_ne!(sig(&[("X-MC-User", "x")], "a@b.fr") & PLATFORM, 0);
    }

    #[test]
    fn a_person_relayed_by_a_platform_stays_a_person() {
        let s = signals([("X-Mandrill-User", "md_1")], Some("m.molinari@feed-back.fr"), Some("CA+x@mail.gmail.com"), true);
        assert_ne!(s & NATURE_HUMAN, 0);
        // Without any sign of a person, the platform makes it bulk.
        assert_ne!(sig(&[("X-Mandrill-User", "md_1")], "m@feed-back.fr") & NATURE_BULK, 0);
    }

    #[test]
    fn auto_generated_alone_is_an_automatic_reply() {
        assert_ne!(sig(&[("Auto-Submitted", "auto-generated")], "a@bpce.fr") & NATURE_AUTO, 0);
        assert_ne!(sig(&[("Auto-Submitted", "auto-replied")], "a@bpce.fr") & NATURE_AUTO, 0);
        assert_ne!(sig(&[("Auto-Submitted", "auto-generated")], "noreply@x.fr") & NATURE_BULK, 0);
        assert_eq!(sig(&[("Auto-Submitted", "no")], "a@x.fr") & (NATURE_AUTO | AUTO_GENERATED), 0);
    }

    #[test]
    fn mailers_tell_clients_from_platforms() {
        assert_eq!(mailer("Apple Mail (2.3445.104.11)"), MAIL_CLIENT);
        assert_eq!(mailer("Microsoft Outlook 16.0"), MAIL_CLIENT);
        assert_eq!(mailer("Sendinblue"), ROBOT_MAILER);
        assert_eq!(mailer("nlserver, Build 6.7.0"), ROBOT_MAILER);
        // Webmails of real people.
        assert_eq!(mailer("PHPMailer 5.2.14 (https://github.com/PHPMailer/PHPMailer)"), 0);
    }

    #[test]
    fn noreply_in_every_spelling() {
        for l in ["noreply", "no-reply", "no_reply", "donotreply", "ne-pas-repondre", "avisage-ne-pas-repondre", "pasdereponse", "bounces+abc", "mailer-daemon", "noreply-sosh"] {
            assert!(is_noreply(l), "{l}");
        }
        for l in ["alice.dupont", "jeanbounce", "nora.reply"] {
            assert!(!is_noreply(l), "{l}");
        }
    }

    #[test]
    fn service_local_parts() {
        for l in ["info", "info.fr", "contact", "contacts", "factures", "news2", "support", "hello", "service-client"] {
            assert!(is_role(l), "{l}");
        }
        for l in ["alice", "infante.marie", "newton", "helloise", "supporter"] {
            assert!(!is_role(l), "{l}");
        }
    }

    fn counts(count: i64, bulk: i64, list: i64, human: i64, client: i64) -> Counts {
        Counts { count, bulk, list, human, client }
    }

    #[test]
    fn verdict_rules_in_order() {
        let none = Facts::default();
        let wrote = Facts { written_to: true, own: false };
        // 2. Own address.
        assert_eq!(verdict("me@x.fr", counts(10, 10, 0, 0, 0), Facts { written_to: false, own: true }), Person);
        // 3. Only automatic replies.
        assert_eq!(verdict("a@x.fr", Counts::default(), wrote), Person);
        assert_eq!(verdict("a@x.fr", Counts::default(), none), Robot);
        // 4. Discussion list.
        assert_eq!(verdict("a@x.fr", counts(10, 0, 6, 0, 0), none), Person);
        // 5. Written to: a mixed shop stays a person; a newsletter answered once is a robot.
        assert_eq!(verdict("beres@galerieberes.com", counts(455, 392, 0, 58, 50), wrote), Person);
        assert_eq!(verdict("daily@timetosignoff.fr", counts(5228, 5228, 0, 0, 0), wrote), Robot);
        assert_eq!(verdict("a@x.fr", counts(4, 0, 0, 0, 0), wrote), Person);
        // 6. Mostly bulk.
        assert_eq!(verdict("alice@la-boutique.com", counts(10, 5, 0, 5, 5), none), Robot);
        // 7. Service local part, even with a mail client.
        assert_eq!(verdict("info@ioncube.com", counts(7, 0, 0, 7, 7), none), Robot);
        // 8. Mail client.
        assert_eq!(verdict("j.dupont@acme.fr", counts(4, 1, 0, 2, 2), none), Person);
        // 9. Public domain or replies, no bulk.
        assert_eq!(verdict("catherine.potheret@free.fr", counts(2, 0, 0, 0, 0), none), Person);
        assert_eq!(verdict("gaelle.delepaut@sncf.fr", counts(12, 0, 0, 12, 0), none), Person);
        // 10. Nothing else: a machine.
        assert_eq!(verdict("voicemail@b3g-telecom.com", counts(1351, 0, 0, 0, 0), none), Robot);
        assert_eq!(verdict("x.y@acme.fr", counts(3, 0, 0, 0, 0), none), Robot);
    }

    #[test]
    fn message_kind_follows_the_message_then_the_address() {
        let bulk = NATURE_BULK | LIST_UNSUBSCRIBE;
        assert_eq!(message_kind(Some(bulk), true, Some(Robot), false), Some(Person));
        assert_eq!(message_kind(Some(bulk), false, Some(Person), false), Some(Robot));
        assert_eq!(message_kind(Some(NATURE_AUTO), false, Some(Person), false), Some(Robot));
        assert_eq!(message_kind(Some(NATURE_HUMAN), false, Some(Person), false), Some(Person));
        assert_eq!(message_kind(None, false, None, false), None);
        // Forced by the user: every received mail follows the address, the
        // automatic ones too; the user's own mails stay a person.
        assert_eq!(message_kind(Some(bulk), false, Some(Person), true), Some(Person));
        assert_eq!(message_kind(Some(NATURE_AUTO), false, Some(Person), true), Some(Person));
        assert_eq!(message_kind(Some(NATURE_HUMAN), false, Some(Robot), true), Some(Robot));
        assert_eq!(message_kind(Some(NATURE_HUMAN), true, Some(Robot), true), Some(Person));
    }

    #[test]
    fn public_domains_come_from_the_list() {
        assert!(is_public_domain("gmail.com"));
        assert!(is_public_domain("Free.FR"));
        assert!(!is_public_domain("acme.fr"));
    }
}
