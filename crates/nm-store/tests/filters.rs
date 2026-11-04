//! Integration tests of the text and address criteria of `ThreadFilter`
//! (`ui.md`, "Sélecteur d'attributs" and "Négation"): Text… and Subject…
//! never look at the addresses, `contains` (the `/` search) looks
//! everywhere, values starting with `@` are domains, and every criterion
//! can exclude.
//!
//! The mailbox is the owner's use case: mails about the domain acme.fr at
//! a registrar, mixed with mails sent to or by acme.fr.

use std::sync::Mutex;

use nm_api::{ThreadFilter, ThreadSort};
use nm_core::Source;
use nm_store::Store;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;

const ME: &str = "me@hey.example";

/// (from, to, subject, body) of each mail, one thread each.
const MAILS: &[(&str, &str, &str, &str)] = &[
    // 0: the registrar talks about the domain, in the subject and the body.
    ("Gandi <registrar@gandi.example>", ME, "Renouvellement du domaine acme.fr", "Votre domaine acme.fr expire bientot."),
    // 1: someone at acme.fr: the domain is only in the address.
    ("Jean Martin <jean@acme.fr>", ME, "Reunion de mardi", "Bonjour, a mardi."),
    // 2: the owner writes to a subdomain of acme.fr about the domain.
    ("Me <me@hey.example>", "Support <support@mail.acme.fr>", "Question acme.fr", "Le transfert est bloque."),
    // 3: a look-alike domain, with an invoice.
    ("Sales <sales@notacme.fr>", ME, "Transfert acme.fr", "Votre facture du transfert."),
    // 4: the domain only in the body.
    ("Bob <bob@b.example>", ME, "Hello", "Le site acme.fr est en panne."),
];

/// A store with `MAILS` imported, and every folder (the threads land in
/// Inbox, Sent or Quarantine depending on the sender).
fn store() -> (Store, Vec<i64>) {
    store_of(MAILS, &[])
}

/// A store with `mails` imported, one thread each, except that the mail at
/// index `b` of each `(a, b)` of `replies` answers the mail at index `a`.
fn store_of(mails: &[(&str, &str, &str, &str)], replies: &[(usize, usize)]) -> (Store, Vec<i64>) {
    let dir = tempfile::tempdir().unwrap();
    let mut mbox = String::new();
    for (i, (from, to, subject, body)) in mails.iter().enumerate() {
        let reply = replies
            .iter()
            .find(|(_, b)| *b == i)
            .map(|(a, _)| format!("In-Reply-To: <m{a}@test.example>\nReferences: <m{a}@test.example>\n"))
            .unwrap_or_default();
        mbox.push_str(&format!(
            "From sender@x Wed Jun 24 09:0{i}:00 2020\nFrom: {from}\nTo: {to}\nSubject: {subject}\n\
             Message-ID: <m{i}@test.example>\n{reply}Date: Wed, 24 Jun 2020 09:0{i}:00 +0000\n\n{body}\n\n"
        ));
    }
    let path = dir.path().join("filters.mbox");
    std::fs::write(&path, mbox).unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "hey".into(),
        address: ME.into(),
        mark_correspondents: true,
        source: Source::Cli,
        batch_size: 10,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    let store = store.into_inner().unwrap();
    let folders = store.list_folders().unwrap().iter().map(|f| f.id).collect();
    (store, folders)
}

/// The indexes in `MAILS` of the threads matching `filter`, sorted.
fn found(env: &(Store, Vec<i64>), filter: ThreadFilter) -> Vec<usize> {
    let (store, folders) = env;
    let (items, next) = store.list_threads_in(folders, None, 100, ThreadSort::Received, &filter).unwrap();
    assert!(next.is_none());
    let mut out: Vec<usize> =
        items.iter().map(|t| MAILS.iter().position(|m| m.2 == t.subject).expect("a subject of MAILS")).collect();
    out.sort();
    out
}

fn words(s: &str) -> Option<String> {
    Some(s.to_string())
}

#[test]
fn every_mail_is_listed_without_filter() {
    let env = store();
    assert_eq!(found(&env, ThreadFilter::default()), vec![0, 1, 2, 3, 4]);
}

#[test]
fn text_looks_at_the_subject_and_the_body_never_the_addresses() {
    let env = store();
    // `contains` (the `/` search) also finds mail 1, by its sender.
    assert_eq!(found(&env, ThreadFilter { contains: words("acme"), ..Default::default() }), vec![0, 1, 2, 3, 4]);
    assert_eq!(found(&env, ThreadFilter { text: words("acme"), ..Default::default() }), vec![0, 2, 3, 4]);
    // A name found only in an address: nothing in the text.
    assert!(found(&env, ThreadFilter { text: words("martin"), ..Default::default() }).is_empty());
    assert_eq!(found(&env, ThreadFilter { contains: words("martin"), ..Default::default() }), vec![1]);
    // Every word is a prefix, accents and case ignored, ANDed.
    assert_eq!(found(&env, ThreadFilter { text: words("TRANSF acme"), ..Default::default() }), vec![2, 3]);
}

#[test]
fn subject_looks_at_the_subject_only() {
    let env = store();
    // Mail 4 has acme.fr in its body only.
    assert_eq!(found(&env, ThreadFilter { subject: words("acme.fr"), ..Default::default() }), vec![0, 2, 3]);
    assert!(found(&env, ThreadFilter { subject: words("panne"), ..Default::default() }).is_empty());
    assert_eq!(found(&env, ThreadFilter { text: words("panne"), ..Default::default() }), vec![4]);
}

#[test]
fn a_dashed_word_is_excluded() {
    let env = store();
    assert_eq!(found(&env, ThreadFilter { text: words("acme -facture"), ..Default::default() }), vec![0, 2, 4]);
    // Only excluded words: every thread without them.
    assert_eq!(found(&env, ThreadFilter { text: words("-facture -panne"), ..Default::default() }), vec![0, 1, 2]);
    assert_eq!(found(&env, ThreadFilter { subject: words("-acme"), ..Default::default() }), vec![1, 4]);
    // In `contains`, the excluded word is looked for everywhere, addresses
    // included.
    assert_eq!(found(&env, ThreadFilter { contains: words("-acme"), ..Default::default() }), Vec::<usize>::new());
    // A dash inside a word is part of it; a lone dash is no word; FTS
    // syntax typed by the user is taken literally.
    assert_eq!(found(&env, ThreadFilter { text: words("acme -"), ..Default::default() }), vec![0, 2, 3, 4]);
    assert!(found(&env, ThreadFilter { text: words("\"subject:\" OR NOT"), ..Default::default() }).is_empty());
}

#[test]
fn a_value_starting_with_at_is_a_domain_with_its_subdomains() {
    let env = store();
    // Not "notacme.fr" (mail 3).
    assert_eq!(found(&env, ThreadFilter { from: vec!["@acme.fr".into()], ..Default::default() }), vec![1]);
    assert_eq!(found(&env, ThreadFilter { to: vec!["@acme.fr".into()], ..Default::default() }), vec![2]);
    // Case ignored, spaces trimmed.
    assert_eq!(found(&env, ThreadFilter { from: vec![" @ACME.fr ".into()], ..Default::default() }), vec![1]);
    // Without "@", a part of the address or the name: "notacme.fr" too.
    assert_eq!(found(&env, ThreadFilter { from: vec!["acme.fr".into()], ..Default::default() }), vec![1, 3]);
    // A lone "@" is a part of every address.
    assert_eq!(found(&env, ThreadFilter { from: vec!["@".into()], ..Default::default() }), vec![0, 1, 2, 3, 4]);
    // `%` and `_` stay literal in a domain.
    assert!(found(&env, ThreadFilter { from: vec!["@%".into()], ..Default::default() }).is_empty());
}

#[test]
fn excluded_senders_and_recipients_leave_their_threads_out() {
    let env = store();
    assert_eq!(found(&env, ThreadFilter { not_from: vec!["@acme.fr".into()], ..Default::default() }), vec![0, 2, 3, 4]);
    assert_eq!(found(&env, ThreadFilter { not_to: vec!["@acme.fr".into()], ..Default::default() }), vec![0, 1, 3, 4]);
    // Excluded values are ANDed: none of them.
    assert_eq!(
        found(&env, ThreadFilter { not_from: vec!["gandi".into(), "bob".into()], ..Default::default() }),
        vec![1, 2, 3]
    );
    // Wanted values OR-ed, then the excluded ones taken out.
    assert_eq!(
        found(
            &env,
            ThreadFilter { from: vec!["acme".into(), "gandi".into()], not_from: vec!["@acme.fr".into()], ..Default::default() }
        ),
        vec![0, 3]
    );
}

#[test]
fn the_registrar_example_of_the_spec() {
    // Mails talking about acme.fr, but not those sent to or by acme.fr.
    let env = store();
    let filter = ThreadFilter {
        text: words("acme.fr"),
        not_from: vec!["@acme.fr".into()],
        not_to: vec!["@acme.fr".into()],
        ..Default::default()
    };
    assert_eq!(found(&env, filter), vec![0, 3, 4]);
}

#[test]
fn the_new_fields_are_optional_in_json() {
    // An older client sends only the fields it knows.
    let f: ThreadFilter = serde_json::from_str(r#"{"contains":"acme","from":["x"]}"#).unwrap();
    assert!(f.not_from.is_empty() && f.not_to.is_empty() && f.text.is_none() && f.subject.is_none());
    let f: ThreadFilter = serde_json::from_str(r#"{"not_from":["@acme.fr"],"text":"a -b","subject":"c"}"#).unwrap();
    assert_eq!(f.not_from, vec!["@acme.fr".to_string()]);
    assert_eq!(f.text.as_deref(), Some("a -b"));
}

#[test]
fn in_everything_the_criteria_see_every_listed_folder() {
    // acme.fr writes (Inbox), the owner replies (Sent): in "Everything" the
    // thread is shown in Sent, the folder of its latest mail, where nothing
    // comes from acme.fr. The criteria still see the Inbox.
    let mails = &[
        ("Jean <jean@acme.fr>", ME, "Devis", "Voici le devis."),
        ("Me <me@hey.example>", "Jean <jean@acme.fr>", "Re: Devis", "Merci."),
        ("Bob <bob@b.example>", ME, "Autre", "Rien."),
    ];
    let (store, folders) = store_of(mails, &[(0, 1)]);
    let list = |filter: ThreadFilter| -> Vec<String> {
        let (items, _) = store.list_threads_in(&folders, None, 100, ThreadSort::Received, &filter).unwrap();
        items.into_iter().map(|t| t.subject).collect()
    };
    let all = list(ThreadFilter::default());
    assert_eq!(all.len(), 2, "{all:?}");
    let devis = all.iter().find(|s| s.contains("Devis")).unwrap().clone();
    assert_eq!(list(ThreadFilter { not_from: vec!["@acme.fr".into()], ..Default::default() }), vec!["Autre".to_string()]);
    assert_eq!(list(ThreadFilter { from: vec!["jean".into()], ..Default::default() }), vec![devis]);
}
