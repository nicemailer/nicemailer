//! Fake mail generator for development: a believable mailbox spread across
//! every workflow folder, with conversations, newsletters, invoices,
//! Quarantine and spam.
//!
//! Deterministic: the same seed always produces the same database, so a bug
//! seen on one machine can be reproduced on another.
//!
//! The generated text is in French on purpose: it is the language of the
//! target user, and it exercises accent handling in search.
//!
//! The generator writes directly into the tables, as the future IMAP sync
//! would, and computes the denormalized aggregates in one pass at the end
//! (much faster than refreshing them after each insert).

use std::collections::HashMap;

use nm_core::{FolderRole, KW_WAS_TODO, KW_WAS_WAITING};
// `StdRng` is a pseudo-random generator that can be created from a seed.
// The `IndexedRandom` trait adds `choose` (pick one element of a slice) and
// `choose_weighted`; `Rng` adds `random`, `random_range` and `random_bool`.
//
// Rust note: a *trait* is like an interface (a Swift protocol). Methods
// provided by a trait can only be called when the trait is imported with
// `use`, which is why these imports are needed even though the names are
// never written below.
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rusqlite::{Transaction, params};

use crate::{Result, Store, StoreError};

/// Parameters of a seed run.
pub struct SeedOptions {
    /// Number of threads to create (about two messages per thread on average).
    pub threads: usize,
    /// Seed of the random generator.
    pub seed: u64,
    /// Reference date (Unix seconds): every message is dated before it.
    pub now: i64,
}

/// What was created, printed by the CLI at the end.
#[derive(Debug, Default)]
pub struct SeedReport {
    pub threads: usize,
    pub messages: usize,
    pub addresses: usize,
}

/// Durations in seconds. The `_` in numbers is only a visual separator.
const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

/// The fake owner of the mailbox.
const ME_NAME: &str = "Alex Moreau";
const ME_ADDR: &str = "alex@moreau-conseil.fr";

// Raw material for the generated mail. `&[&str]` is a slice of string
// slices: a constant list of texts embedded in the binary.

/// First names and last names, combined to build contacts.
const FIRST: &[&str] = &[
    "Camille", "Léa", "Hugo", "Louis", "Chloé", "Manon", "Lucas", "Emma", "Nathan", "Inès",
    "Jules", "Sarah", "Thomas", "Julie", "Antoine", "Claire", "Maxime", "Pauline", "Nicolas",
    "Élodie", "Karim", "Yasmine", "Mehdi", "Sofia", "Paul", "Alice", "Arthur", "Zoé", "Victor",
    "Margaux", "Romain", "Anaïs", "Bastien", "Noémie", "Samir", "Agathe",
];
const LAST: &[&str] = &[
    "Martin", "Bernard", "Dubois", "Durand", "Lefebvre", "Laurent", "Simon", "Michel", "Garcia",
    "Roux", "Fournier", "Girard", "Bonnet", "Lambert", "Fontaine", "Rousseau", "Mercier", "Blanc",
    "Guérin", "Benali", "Nguyen", "Perrin", "Morel", "Chevalier", "Faure", "André",
];
/// Domains of contacts: professional ones, and public mailbox providers.
const WORK_DOMAINS: &[&str] = &[
    "atelier-dupuis.fr", "studio-lumen.fr", "cabinet-arnaud.fr", "transports-vidal.fr",
    "agence-hublot.fr", "coop-lesgraines.fr", "mairie-sainte-aude.fr", "editions-du-quai.fr",
];
const PERSONAL_DOMAINS: &[&str] = &[
    "gmail.com", "orange.fr", "free.fr", "laposte.net", "outlook.fr", "proton.me", "icloud.com",
];

/// Senders accepted into Newsletter: (name, address).
const NEWSLETTERS: &[(&str, &str)] = &[
    ("La Gazette du Code", "newsletter@gazette-du-code.fr"),
    ("Hebdo Vélo", "bonjour@hebdovelo.fr"),
    ("Les Carnets du Jardin", "lettre@carnets-jardin.fr"),
    ("Ciné Quartier", "programme@cine-quartier.fr"),
    ("Le Fil Tech", "news@lefiltech.io"),
    ("Cuisine de Saison", "recettes@cuisine-saison.fr"),
    ("Rando Alpes", "infos@rando-alpes.fr"),
    ("Librairie des Halles", "nouveautes@librairie-des-halles.fr"),
    ("Marché Bio Express", "promo@marchebio-express.fr"),
    ("Digest Startups", "digest@digest-startups.fr"),
];
/// Senders accepted into Paperwork: (name, address, subject).
const PAPERWORK: &[(&str, &str, &str)] = &[
    ("Énergie Plus", "factures@energie-plus.fr", "Votre facture d'électricité"),
    ("Banque Horizon", "noreply@banque-horizon.fr", "Votre relevé de compte"),
    ("Mutuelle Sereine", "service@mutuelle-sereine.fr", "Remboursement de vos soins"),
    ("Nova Télécom", "facture@nova-telecom.fr", "Votre facture mobile"),
    ("Assur Prévoyance", "contrats@assur-prevoyance.fr", "Avis d'échéance"),
    ("Train Express", "billets@train-express.fr", "Votre billet électronique"),
    ("Livraison Rapide", "suivi@livraison-rapide.fr", "Votre colis est en route"),
    ("Syndic Les Tilleuls", "gestion@syndic-tilleuls.fr", "Appel de charges"),
    ("Cloudéo", "billing@cloudeo.net", "Invoice for your hosting plan"),
];
/// Domains of unknown senders, whose first message lands in Quarantine.
const PROSPECT_DOMAINS: &[&str] = &[
    "leadboost-agency.com", "growth-partners.io", "salon-pro-2026.fr", "recrutement-plus.fr",
    "seo-magique.com", "formation-express.fr", "conseil-digital360.fr",
];
/// Senders the user has banned: (name, address).
const BANNED: &[(&str, &str)] = &[
    ("Matelas Promo", "deals@matelas-promo.fr"),
    ("Vacances Flash", "offres@vacances-flash.fr"),
    ("Sondages Express", "enquete@sondages-express.com"),
];

/// Conversation topics, inserted into the templates below.
const TOPICS: &[&str] = &[
    "le devis pour la rénovation", "la réunion de jeudi", "le projet de site web",
    "les photos du week-end", "la facture de mars", "le dossier de subvention",
    "l'organisation du séminaire", "le contrat de location", "la relecture du rapport",
    "le planning de l'été", "l'anniversaire de Léa", "la commande de matériel",
    "le compte rendu du conseil", "les inscriptions au club", "le déménagement",
    "la maquette de l'application", "le budget 2027", "la formation Rust",
    "l'entretien de vendredi", "la visite du chantier", "la présentation client",
    "le stand du salon", "les congés de décembre", "la migration des serveurs",
];
/// Text templates. `{t}` is replaced by the topic, `{T}` by the topic with
/// a capital letter, `{f}` by the recipient's first name (see `fill`).
const SUBJECTS: &[&str] = &[
    "À propos de {t}", "{T}", "Question sur {t}", "Point sur {t}", "Rappel : {t}",
    "{T} : besoin de ton avis", "Suite à notre échange sur {t}",
];
const OPENERS: &[&str] = &["Bonjour {f},", "Salut {f},", "Hello {f},", "Bonjour,", "{f},"];
const FIRST_LINES: &[&str] = &[
    "Je reviens vers toi au sujet de {t}.",
    "Est-ce que tu as pu avancer sur {t} ?",
    "Je voulais te tenir au courant pour {t}.",
    "Petite question rapide concernant {t}.",
    "Comme convenu, je t'écris pour {t}.",
];
const MIDDLE_LINES: &[&str] = &[
    "De mon côté, tout est prêt, il ne manque plus que ta validation.",
    "Je pense qu'on peut viser la fin du mois, si ça te convient.",
    "J'ai regardé les chiffres hier soir, ça me semble raisonnable.",
    "Il faudrait qu'on en parle de vive voix, c'est plus simple.",
    "Je t'ai mis en pièce jointe la dernière version.",
    "Le fournisseur m'a confirmé les délais ce matin.",
    "On a encore deux ou trois points à trancher avant de lancer.",
    "Tu peux me dire ce que tu en penses d'ici vendredi ?",
    "Je n'ai pas encore eu de retour de l'équipe.",
    "Ça m'a pris plus de temps que prévu, désolé pour le retard.",
];
const REPLY_LINES: &[&str] = &[
    "Merci pour ton message.", "Parfait, merci !", "Bien reçu, je regarde ça.",
    "OK pour moi.", "Bonne nouvelle, merci.", "Je te réponds dans la journée.",
    "Désolé pour la réponse tardive.", "Très bien, on fait comme ça.",
];
const CLOSERS: &[&str] = &[
    "Bonne journée,", "À bientôt,", "Merci d'avance,", "Belle soirée,", "Bises,", "Cordialement,",
];
/// Possible attachments: (file name, MIME type).
const ATTACHMENTS: &[(&str, &str)] = &[
    ("devis.pdf", "application/pdf"),
    ("compte-rendu.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("photo.jpg", "image/jpeg"),
    ("budget.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("maquette.png", "image/png"),
];

/// A sender or recipient.
///
/// Rust note: `String` (unlike `&str`) owns its text. `Clone` is derived
/// because the same person appears in many mails; `.clone()` copies them
/// explicitly. Rust never copies heap data behind your back.
#[derive(Clone)]
struct Person {
    /// Display name, e.g. "Camille Martin".
    name: String,
    addr: String,
    /// First name, used in greetings ("Bonjour Camille,").
    first: String,
}

/// ASCII form of a first or last name, for use in an email address:
/// "Élodie" becomes "elodie".
fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' | 'É' => 'e',
            'à' | 'â' => 'a',
            'ï' | 'î' => 'i',
            'ô' => 'o',
            'ù' | 'û' => 'u',
            'ç' => 'c',
            c => c,
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_lowercase()
}

/// Fills a template: `{t}` → topic, `{T}` → capitalized topic, `{f}` →
/// first name.
fn fill(template: &str, topic: &str, first: &str) -> String {
    let mut cap = topic.to_string();
    if let Some(c) = cap.chars().next() {
        // Rust strings are UTF-8: a character like "é" takes 2 bytes, so the
        // rest of the string starts at `c.len_utf8()`, not at index 1.
        // Slicing in the middle of a character would panic.
        cap = c.to_uppercase().collect::<String>() + &topic[c.len_utf8()..];
    }
    template.replace("{t}", topic).replace("{T}", &cap).replace("{f}", first)
}

/// Preview shown in the thread list: the body on a single line, cut to 200
/// characters (characters, not bytes, so no accent is cut in half).
fn snippet(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(200).collect()
}

/// A message to insert, before it is written to the database.
struct Mail {
    /// Folder the message goes into.
    role: FolderRole,
    from: Person,
    to: Vec<Person>,
    cc: Vec<Person>,
    subject: String,
    body: String,
    /// Reception date, Unix seconds.
    date: i64,
    seen: bool,
    flagged: bool,
    answered: bool,
    draft: bool,
    attachment: Option<(&'static str, &'static str)>,
    /// Authentication result: "pass", "fail" or "none".
    auth: &'static str,
    /// Set when the message claims to come from a known sender but fails
    /// authentication (possible phishing).
    suspect: bool,
    /// IMAP keywords, e.g. the return marks `$NmWasTodo`.
    keywords: Vec<&'static str>,
}

impl Mail {
    /// A read message with a single recipient and default values; callers
    /// then adjust the fields they need.
    fn new(role: FolderRole, from: Person, to: Person, subject: String, body: String, date: i64) -> Mail {
        Mail {
            role,
            from,
            to: vec![to],
            cc: vec![],
            subject,
            body,
            date,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            attachment: None,
            auth: "pass",
            suspect: false,
            keywords: vec![],
        }
    }
}

/// State shared by all the generation functions.
///
/// Rust note: `<'a>` is a *lifetime* parameter. The generator holds a
/// reference to the transaction (`tx`), and the compiler must be sure the
/// transaction outlives the generator. `'a` names that relationship: the
/// generator cannot live longer than the transaction it borrows. Lifetimes
/// are checked at compile time and cost nothing at runtime.
struct Generator<'a> {
    rng: StdRng,
    now: i64,
    tx: &'a Transaction<'a>,
    account_id: i64,
    /// Folder id of each role.
    folders: HashMap<FolderRole, i64>,
    /// Next IMAP UID to assign in each folder (UIDs increase per folder).
    next_uid: HashMap<FolderRole, i64>,
    /// Addresses already inserted, by email address, to reuse their id.
    addresses: HashMap<String, i64>,
    me: Person,
    report: SeedReport,
}

// `'_` lets the compiler infer the lifetime: the methods below do not need
// to name it.
impl Generator<'_> {
    /// Id of the `address` row for this person, inserting it the first
    /// time. `target` is the screening decision on the sender (`"inbox"`,
    /// `"newsletter"`...), or `None` for an unknown sender.
    fn address_id(&mut self, p: &Person, target: Option<&str>) -> Result<i64> {
        if let Some(id) = self.addresses.get(&p.addr) {
            return Ok(*id);
        }
        self.tx.execute(
            "INSERT INTO address (addr, name, screening_target, date_screening_modified, auth_seen,
                                  created_by, modified_by)
             VALUES (?1, ?2, ?3, CASE WHEN ?3 IS NULL THEN NULL ELSE ?4 END, 1, 'core', 'core')",
            params![p.addr, p.name, target, self.now - 400 * DAY],
        )?;
        let id = self.tx.last_insert_rowid();
        self.addresses.insert(p.addr.clone(), id);
        self.report.addresses += 1;
        Ok(id)
    }

    /// Random age in seconds, between `min_days` and `max_days`, biased
    /// towards recent dates (a real mailbox has more recent mail).
    fn age(&mut self, min_days: f64, max_days: f64) -> i64 {
        // `u` is uniform in [0, 1); `u * u` is still in [0, 1) but closer to
        // 0 on average, hence the bias. A random hour is added so that
        // messages do not all arrive at midnight.
        let u: f64 = self.rng.random::<f64>();
        let days = min_days + (max_days - min_days) * u * u;
        (days * DAY as f64) as i64 + self.rng.random_range(0..HOUR)
    }

    /// Random `Message-ID` header value on the sender's domain. Real
    /// Message-IDs are used to link replies into threads.
    fn msgid(&mut self, addr: &str) -> String {
        let domain = addr.split('@').nth(1).unwrap_or("localhost");
        format!("{:016x}.{:08x}@{domain}", self.rng.random::<u64>(), self.rng.random::<u32>())
    }

    /// Inserts a whole thread: the `thread` row, then for each message its
    /// row in `message`, its body (the FTS triggers of schema.sql index it
    /// automatically), its references, recipients, attachment and keywords.
    ///
    /// `mails` must be sorted from oldest to newest. `targets` maps each
    /// known address to its screening decision.
    fn insert_thread(&mut self, subject: &str, mails: Vec<Mail>, targets: &HashMap<String, &'static str>) -> Result<()> {
        self.tx.execute(
            "INSERT INTO thread (account_id, subject) VALUES (?1, ?2)",
            params![self.account_id, subject],
        )?;
        let thread_id = self.tx.last_insert_rowid();
        // Message-IDs of the previous messages: each new message replies to
        // the last one (`In-Reply-To`) and references all of them
        // (`References`), as real mail clients do.
        let mut previous: Vec<String> = vec![];
        // `for m in mails` takes ownership of each `Mail` (the Vec is consumed).
        for m in mails {
            // Indexing a HashMap with `[...]` panics if the key is missing;
            // that is fine here since every role was inserted in `seed`.
            let folder_id = self.folders[&m.role];
            // `entry(...).or_insert(1)` returns a mutable reference to the
            // counter, creating it at 1 if needed. We read it, then increment.
            let uid = {
                let u = self.next_uid.entry(m.role).or_insert(1);
                *u += 1;
                *u - 1
            };
            // `copied()` turns `Option<&&str>` into `Option<&str>`.
            let from_target = targets.get(&m.from.addr).copied();
            let from_id = self.address_id(&m.from, from_target)?;
            let msgid = self.msgid(&m.from.addr);
            // Plausible size in bytes: body + headers, + attachment if any.
            let size = m.body.len() as i64 + 1800 + if m.attachment.is_some() { 120_000 } else { 0 };
            // The sending date is 40 s before reception. `body_state = 1`
            // means the body is downloaded.
            self.tx.execute(
                "INSERT INTO message (account_id, folder_id, uid, thread_id, message_id_hdr, in_reply_to,
                    subject, from_address_id, from_name, from_addr, date_sent, date_received, size, snippet,
                    is_seen, is_flagged, is_answered, is_draft, has_attachments, auth_result, auth_source,
                    auth_suspect, body_state, date_created, date_modified, created_by, modified_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         ?15, ?16, ?17, ?18, ?19, ?20, 'server', ?21, 1, ?12, ?12, 'imap', 'imap')",
                params![
                    self.account_id, folder_id, uid, thread_id, msgid, previous.last(),
                    m.subject, from_id, m.from.name, m.from.addr, m.date - 40, m.date, size,
                    snippet(&m.body), m.seen, m.flagged, m.answered, m.draft,
                    m.attachment.is_some(), m.auth, m.suspect,
                ],
            )?;
            let message_id = self.tx.last_insert_rowid();
            self.tx.execute(
                "INSERT INTO message_body (message_id, text_plain, parser_version, date_fetched)
                 VALUES (?1, ?2, 1, ?3)",
                params![message_id, m.body, m.date],
            )?;
            for (pos, r) in previous.iter().enumerate() {
                self.tx.execute(
                    "INSERT INTO message_reference (message_id, position, ref_msgid) VALUES (?1, ?2, ?3)",
                    params![message_id, pos as i64, r],
                )?;
            }
            for (role, list) in [("to", &m.to), ("cc", &m.cc)] {
                for (pos, p) in list.iter().enumerate() {
                    let target = targets.get(&p.addr).copied();
                    let aid = self.address_id(p, target)?;
                    self.tx.execute(
                        "INSERT INTO message_address (message_id, role, position, address_id) VALUES (?1, ?2, ?3, ?4)",
                        params![message_id, role, pos as i64, aid],
                    )?;
                }
            }
            if let Some((name, mime)) = m.attachment {
                self.tx.execute(
                    "INSERT INTO attachment (message_id, part_id, filename, mime_type, size) VALUES (?1, '2', ?2, ?3, 120000)",
                    params![message_id, name, mime],
                )?;
            }
            for kw in &m.keywords {
                self.tx.execute(
                    "INSERT INTO message_keyword (message_id, keyword) VALUES (?1, ?2)",
                    params![message_id, kw],
                )?;
            }
            previous.push(msgid);
            self.report.messages += 1;
        }
        self.report.threads += 1;
        Ok(())
    }

    /// Builds a body: greeting, first line (or a short reply line), one to
    /// three random middle lines, closing and signature.
    ///
    /// `unwrap()` on `choose` is safe: it only returns `None` for an empty
    /// list, and these constant lists are never empty.
    fn body(&mut self, opener_to: &str, topic: &str, signer: &str, reply: bool) -> String {
        let opener = fill(OPENERS.choose(&mut self.rng).unwrap(), topic, opener_to);
        let mut lines = vec![opener, String::new()];
        if reply {
            lines.push(REPLY_LINES.choose(&mut self.rng).unwrap().to_string());
        } else {
            lines.push(fill(FIRST_LINES.choose(&mut self.rng).unwrap(), topic, opener_to));
        }
        let n = self.rng.random_range(1..=3);
        for l in MIDDLE_LINES.choose_multiple(&mut self.rng, n) {
            lines.push(l.to_string());
        }
        lines.push(String::new());
        lines.push(CLOSERS.choose(&mut self.rng).unwrap().to_string());
        lines.push(signer.to_string());
        lines.join("\n")
    }
}

/// Builds a `Person`; the first name is the first word of the name.
fn person(name: &str, addr: &str) -> Person {
    Person {
        name: name.to_string(),
        addr: addr.to_string(),
        first: name.split(' ').next().unwrap_or(name).to_string(),
    }
}

/// Fills an empty database. Refuses a database that already contains an
/// account, so real data can never be mixed with fake data.
///
/// Everything happens in a single transaction: either the whole mailbox is
/// created, or nothing is. It is also much faster, because SQLite only
/// syncs to disk once, at commit.
pub fn seed(store: &mut Store, opts: &SeedOptions) -> Result<SeedReport> {
    let existing: i64 = store.conn.query_row("SELECT count(*) FROM account", [], |r| r.get(0))?;
    if existing > 0 {
        return Err(StoreError::InvalidParams("la base contient déjà des données".into()));
    }
    let tx = store.conn.transaction()?;
    // One account, with one folder per workflow role. The Inbox is named
    // "INBOX" as IMAP requires. `uidvalidity` is an arbitrary distinct value
    // per folder (`role as i64` gives the variant's position in the enum).
    tx.execute(
        "INSERT INTO account (config_key, created_by, modified_by) VALUES ('perso', 'app', 'app')",
        [],
    )?;
    let account_id = tx.last_insert_rowid();
    let mut folders = HashMap::new();
    for role in FolderRole::ALL {
        let path = if role == FolderRole::Inbox { "INBOX".to_string() } else { role.display_name().to_string() };
        tx.execute(
            "INSERT INTO folder (account_id, path, delimiter, role, uidvalidity, created_by, modified_by)
             VALUES (?1, ?2, '/', ?3, ?4, 'imap', 'imap')",
            params![account_id, path, role.as_str(), 1_700_000_000 + role as i64],
        )?;
        folders.insert(role, tx.last_insert_rowid());
    }

    let mut g = Generator {
        rng: StdRng::seed_from_u64(opts.seed),
        now: opts.now,
        tx: &tx,
        account_id,
        folders,
        next_uid: HashMap::new(),
        addresses: HashMap::new(),
        me: person(ME_NAME, ME_ADDR),
        report: SeedReport::default(),
    };
    // Screening decision of each known sender, by address. Unknown senders
    // (Quarantine, spam) have no entry.
    let mut targets: HashMap<String, &'static str> = HashMap::new();

    // Contacts: colleagues and friends, all accepted into the Inbox. Names
    // are combined with simple arithmetic rather than randomly, to get many
    // different pairs; the rare duplicate address is skipped.
    let mut contacts: Vec<Person> = vec![];
    for i in 0..140 {
        let first = FIRST[i % FIRST.len()];
        let last = LAST[(i * 7 + i / FIRST.len()) % LAST.len()];
        let domain = if i % 3 == 0 {
            PERSONAL_DOMAINS[i % PERSONAL_DOMAINS.len()]
        } else {
            WORK_DOMAINS[i % WORK_DOMAINS.len()]
        };
        let addr = format!("{}.{}@{}", ascii(first), ascii(last), domain);
        if contacts.iter().any(|c| c.addr == addr) {
            continue;
        }
        targets.insert(addr.clone(), "inbox");
        contacts.push(person(&format!("{first} {last}"), &addr));
    }
    for (_, addr) in NEWSLETTERS {
        targets.insert(addr.to_string(), "newsletter");
    }
    for (_, addr, _) in PAPERWORK {
        targets.insert(addr.to_string(), "paperwork");
    }
    for (_, addr) in BANNED {
        targets.insert(addr.to_string(), "banned");
    }
    // The user's own address. It is cloned first because `g.address_id`
    // borrows `g` mutably, and Rust forbids borrowing `g.me` at the same time.
    let me = g.me.clone();
    g.address_id(&me, None)?;

    // Pick the kind of each thread by percentage: 47 % conversations, 24 %
    // newsletters, 12 % paperwork, 8 % quarantine, 6 % junk, 2 % banned,
    // 1 % drafts. `0..=46` is an inclusive range.
    for _ in 0..opts.threads {
        let kind = g.rng.random_range(0..100);
        match kind {
            0..=46 => conversation(&mut g, &contacts, &targets)?,
            47..=70 => newsletter(&mut g, &targets)?,
            71..=82 => paperwork(&mut g, &targets)?,
            83..=90 => quarantine(&mut g, &targets)?,
            91..=96 => junk(&mut g, &targets)?,
            97..=98 => banned(&mut g, &targets)?,
            _ => draft(&mut g, &contacts, &targets)?,
        }
    }

    // Denormalized aggregates, in one pass over all messages, plus the IMAP
    // sync state the future sync engine expects (`uidnext`, last sync date,
    // start of screening in the Inbox).
    tx.execute_batch(
        "INSERT INTO thread_folder (thread_id, folder_id, date_last_received, message_count, unread_count)
         SELECT thread_id, folder_id, max(date_received), count(*), sum(is_seen = 0)
           FROM message WHERE thread_id IS NOT NULL GROUP BY thread_id, folder_id;
         UPDATE thread SET
            date_last_received = (SELECT max(date_received) FROM message m WHERE m.thread_id = thread.id),
            message_count      = (SELECT count(*) FROM message m WHERE m.thread_id = thread.id),
            unread_count       = (SELECT count(*) FROM message m WHERE m.thread_id = thread.id AND m.is_seen = 0);
         UPDATE folder SET
            total_count  = (SELECT count(*) FROM message m WHERE m.folder_id = folder.id),
            unread_count = (SELECT count(*) FROM message m WHERE m.folder_id = folder.id AND m.is_seen = 0),
            uidnext      = coalesce((SELECT max(uid) + 1 FROM message m WHERE m.folder_id = folder.id), 1),
            date_last_sync = unixepoch(),
            uid_screening_start = CASE WHEN role = 'inbox' THEN 1 ELSE NULL END;",
    )?;
    // `mem::take` moves the report out of `g`, leaving a default value in its
    // place. Then `g` must be dropped explicitly: it borrows `tx`, and
    // `commit` consumes `tx`, which is impossible while a borrow exists.
    let report = std::mem::take(&mut g.report);
    drop(g);
    tx.commit()?;
    Ok(report)
}

/// Exchange between the user and a contact (sometimes with a second contact
/// in copy). The contact's messages go to the thread's folder, the user's
/// own messages go to Sent, as in a real mailbox.
fn conversation(g: &mut Generator, contacts: &[Person], targets: &HashMap<String, &'static str>) -> Result<()> {
    // Folder of the thread, chosen with weights (50 % Archive, 20 % Inbox...).
    // `choose_weighted` returns a `Result`, `.map(|x| &x.0)` keeps the role,
    // and the leading `*` copies it out of the reference.
    let role = *[
        (FolderRole::Archive, 50),
        (FolderRole::Inbox, 20),
        (FolderRole::Todo, 8),
        (FolderRole::Waiting, 8),
        (FolderRole::Deleted, 4),
    ]
    .choose_weighted(&mut g.rng, |x| x.1)
    .map(|x| &x.0)
    .unwrap();
    let contact = contacts.choose(&mut g.rng).unwrap().clone();
    let cc: Vec<Person> = if g.rng.random_bool(0.2) {
        vec![contacts.choose(&mut g.rng).unwrap().clone()]
    } else {
        vec![]
    };
    let topic = *TOPICS.choose(&mut g.rng).unwrap();
    let subject = fill(SUBJECTS.choose(&mut g.rng).unwrap(), topic, "");
    // Number of messages, weighted: mostly short exchanges.
    let count = *[(1, 25), (2, 30), (3, 20), (4, 12), (5, 7), (6, 4), (8, 2)]
        .choose_weighted(&mut g.rng, |x| x.1)
        .map(|x| &x.0)
        .unwrap();

    // Age of the latest message, depending on the folder: the Inbox holds
    // recent mail, the Archive spans two years.
    let last_age = match role {
        FolderRole::Inbox => g.age(0.0, 12.0),
        FolderRole::Todo => g.age(1.0, 45.0),
        FolderRole::Waiting => g.age(2.0, 30.0),
        FolderRole::Deleted => g.age(0.0, 90.0),
        _ => g.age(3.0, 730.0),
    };
    // Who writes last: the contact in the Inbox and Todo (the user owes an
    // answer), the user in Waiting (the user is waiting for an answer).
    let me_last = match role {
        FolderRole::Waiting => true,
        FolderRole::Inbox | FolderRole::Todo => false,
        _ => g.rng.random_bool(0.4),
    };
    // A quarter of the longer Inbox threads simulate a thread that was in
    // Todo or Waiting and came back because the contact answered: the
    // earlier messages carry the return keyword.
    let returned = role == FolderRole::Inbox && count >= 3 && g.rng.random_bool(0.25);
    let returned_kw = if g.rng.random_bool(0.5) { KW_WAS_TODO } else { KW_WAS_WAITING };

    // Dates are generated backwards from the latest message, with gaps of
    // one hour to three days, then reversed to be in chronological order.
    let mut dates = vec![g.now - last_age];
    for _ in 1..count {
        let gap = g.rng.random_range(HOUR..3 * DAY);
        dates.push(dates.last().unwrap() - gap);
    }
    dates.reverse();

    let mut mails: Vec<Mail> = vec![];
    // `enumerate` yields `(index, item)` pairs.
    for (i, date) in dates.iter().enumerate() {
        // The two sides alternate, counting from the last message, so that
        // the last author is the one chosen in `me_last`.
        let from_me = if me_last { (count - 1 - i) % 2 == 0 } else { (count - 1 - i) % 2 == 1 };
        let reply = i > 0;
        let subj = if reply { format!("Re: {subject}") } else { subject.clone() };
        let (from, to, folder) = if from_me {
            (g.me.clone(), contact.clone(), FolderRole::Sent)
        } else {
            (contact.clone(), g.me.clone(), role)
        };
        let body = g.body(&to.first, topic, &from.first, reply);
        let mut m = Mail::new(folder, from, to, subj, body, *date);
        m.cc = cc.clone();
        let is_last = i == count - 1;
        // Only received messages get read state, flags and keywords: the
        // user's own sent mail is always read.
        if !from_me {
            // In the Inbox, the latest message is often still unread.
            m.seen = match role {
                FolderRole::Inbox => !(is_last && g.rng.random_bool(0.6)),
                _ => true,
            };
            // Every received message except the last was answered by the user.
            m.answered = !is_last;
            if is_last {
                m.flagged = match role {
                    FolderRole::Todo => g.rng.random_bool(0.3),
                    FolderRole::Inbox => g.rng.random_bool(0.08),
                    FolderRole::Archive => g.rng.random_bool(0.02),
                    _ => false,
                };
            }
            if returned && !is_last {
                m.keywords.push(returned_kw);
            }
        }
        if g.rng.random_bool(0.1) {
            m.attachment = ATTACHMENTS.choose(&mut g.rng).copied();
        }
        mails.push(m);
    }
    g.insert_thread(&subject, mails, targets)
}

/// One issue of a newsletter, from an accepted sender. Old issues are read.
fn newsletter(g: &mut Generator, targets: &HashMap<String, &'static str>) -> Result<()> {
    let (name, addr) = *NEWSLETTERS.choose(&mut g.rng).unwrap();
    let role = *[(FolderRole::Newsletter, 90), (FolderRole::Archive, 4), (FolderRole::Deleted, 6)]
        .choose_weighted(&mut g.rng, |x| x.1)
        .map(|x| &x.0)
        .unwrap();
    let age = g.age(0.0, 365.0);
    let n = g.rng.random_range(1..200);
    let topics: Vec<&str> = TOPICS.choose_multiple(&mut g.rng, 3).copied().collect();
    let subject = format!("{name} n° {n} : {}", topics[0]);
    let body = format!(
        "Bonjour,\n\nAu sommaire cette semaine :\n\n- {}\n- {}\n- {}\n\nBonne lecture !\nL'équipe {name}\n\nSe désabonner : https://{}/desabonnement",
        topics[0], topics[1], topics[2], addr.split('@').nth(1).unwrap()
    );
    let mut m = Mail::new(role, person(name, addr), g.me.clone(), subject.clone(), body, g.now - age);
    m.seen = age > 14 * DAY || g.rng.random_bool(0.3);
    g.insert_thread(&subject, vec![m], targets)
}

/// An invoice, statement or notice from an accepted sender, often with a
/// PDF attachment.
fn paperwork(g: &mut Generator, targets: &HashMap<String, &'static str>) -> Result<()> {
    let (name, addr, subj) = *PAPERWORK.choose(&mut g.rng).unwrap();
    let role = if g.rng.random_bool(0.9) { FolderRole::Paperwork } else { FolderRole::Archive };
    let age = g.age(0.0, 700.0);
    let amount = g.rng.random_range(900..25_000) as f64 / 100.0;
    let number = g.rng.random_range(100_000..999_999);
    let subject = format!("{subj} n° {number}");
    let body = format!(
        "Bonjour {},\n\nVotre document n° {number} d'un montant de {amount:.2} € est disponible dans votre espace client.\n\nLe prélèvement interviendra sous 10 jours.\n\n{name}\nCe message est envoyé automatiquement, merci de ne pas y répondre.",
        ME_NAME
    );
    let mut m = Mail::new(role, person(name, addr), g.me.clone(), subject.clone(), body, g.now - age);
    m.seen = age > 5 * DAY;
    if g.rng.random_bool(0.7) {
        m.attachment = Some(("facture.pdf", "application/pdf"));
    }
    g.insert_thread(&subject, vec![m], targets)
}

/// First message from an unknown sender, waiting in Quarantine for the user
/// to accept or reject the sender. Some fail authentication, so the UI's
/// warning can be tested.
fn quarantine(g: &mut Generator, targets: &HashMap<String, &'static str>) -> Result<()> {
    let first = *FIRST.choose(&mut g.rng).unwrap();
    let last = *LAST.choose(&mut g.rng).unwrap();
    let domain = *PROSPECT_DOMAINS.choose(&mut g.rng).unwrap();
    let from = person(&format!("{first} {last}"), &format!("{}@{domain}", ascii(first)));
    let topic = *TOPICS.choose(&mut g.rng).unwrap();
    let subject = [
        format!("Proposition de collaboration pour {topic}"),
        format!("{first}, une question rapide"),
        "Invitation : salon professionnel 2026".to_string(),
        "Opportunité de poste qui pourrait vous intéresser".to_string(),
        format!("Votre avis sur {topic} ?"),
    ]
    .choose(&mut g.rng)
    .unwrap()
    .clone();
    let body = format!(
        "Bonjour {},\n\nJe me permets de vous contacter car j'ai vu votre travail sur {topic}.\nAuriez-vous 15 minutes la semaine prochaine pour en discuter ?\n\nBien à vous,\n{first} {last}",
        g.me.first
    );
    let mut m = Mail::new(FolderRole::Quarantine, from, g.me.clone(), subject.clone(), body, g.now - g.age(0.0, 20.0));
    m.seen = g.rng.random_bool(0.2);
    m.auth = *["pass", "pass", "pass", "none", "fail"].choose(&mut g.rng).unwrap();
    m.suspect = m.auth == "fail";
    g.insert_thread(&subject, vec![m], targets)
}

/// Spam detected by the server.
fn junk(g: &mut Generator, targets: &HashMap<String, &'static str>) -> Result<()> {
    // Spam uses a new address for each message.
    let n: u32 = g.rng.random();
    let from = person("Service Client", &format!("promo{n:x}@offres-{:x}.biz", n % 997));
    let subject = [
        "Félicitations ! Vous avez gagné un iPhone",
        "Dernier rappel : votre colis est bloqué",
        "Votre compte sera suspendu sous 24 h",
        "Gagnez 5 000 € par mois depuis chez vous",
        "Offre exclusive réservée aux membres",
    ]
    .choose(&mut g.rng)
    .unwrap()
    .to_string();
    let body = "Cliquez ici pour réclamer votre récompense avant minuit.\nOffre limitée, ne manquez pas cette chance unique !".to_string();
    let mut m = Mail::new(FolderRole::Junk, from, g.me.clone(), subject.clone(), body, g.now - g.age(0.0, 30.0));
    m.seen = g.rng.random_bool(0.5);
    m.auth = *["none", "fail", "pass"].choose(&mut g.rng).unwrap();
    g.insert_thread(&subject, vec![m], targets)
}

/// Message from a sender the user banned.
fn banned(g: &mut Generator, targets: &HashMap<String, &'static str>) -> Result<()> {
    let (name, addr) = *BANNED.choose(&mut g.rng).unwrap();
    let subject = format!("{name} : -{} % ce week-end seulement", g.rng.random_range(20..70));
    let body = format!("Profitez de nos offres exceptionnelles.\n\n{name}");
    let m = Mail::new(FolderRole::Banned, person(name, addr), g.me.clone(), subject.clone(), body, g.now - g.age(0.0, 120.0));
    g.insert_thread(&subject, vec![m], targets)
}

/// An unfinished message written by the user.
fn draft(g: &mut Generator, contacts: &[Person], targets: &HashMap<String, &'static str>) -> Result<()> {
    let to = contacts.choose(&mut g.rng).unwrap().clone();
    let topic = *TOPICS.choose(&mut g.rng).unwrap();
    let subject = fill(SUBJECTS.choose(&mut g.rng).unwrap(), topic, "");
    let body = format!("{}\n\n{}", fill("Bonjour {f},", topic, &to.first), fill(FIRST_LINES[0], topic, ""));
    let mut m = Mail::new(FolderRole::Drafts, g.me.clone(), to, subject.clone(), body, g.now - g.age(0.0, 10.0));
    m.draft = true;
    g.insert_thread(&subject, vec![m], targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder counters computed in bulk must match the messages actually
    /// inserted, and a second seed on the same database must fail.
    /// `unwrap()` is acceptable in tests: a failure simply fails the test.
    #[test]
    fn seed_produces_consistent_counters() {
        let mut store = Store::open_in_memory().unwrap();
        let report = seed(&mut store, &SeedOptions { threads: 300, seed: 1, now: 1_790_000_000 }).unwrap();
        assert_eq!(report.threads, 300);
        assert!(report.messages > 300);
        let (total, sum): (i64, i64) = store
            .conn
            .query_row(
                "SELECT (SELECT count(*) FROM message), (SELECT sum(total_count) FROM folder)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(total, sum);
        assert!(seed(&mut store, &SeedOptions { threads: 1, seed: 1, now: 0 }).is_err());
    }
}
