//! Integration tests of `links.rs`: the links `get_thread` gives with each
//! message, for the `l` panel.
//!
//! The case that broke: a newsletter whose text part is botched (entities
//! left in it) gets the core's conversion of its HTML as text (parser
//! version 7, `ingest.rs`), and that conversion keeps the words of a link
//! but not its address. The links must come from the HTML.

use std::sync::Mutex;

use nm_api::LinkView;
use nm_core::Source;
use nm_store::Store;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;

/// Three mails: a newsletter with a botched text part and an HTML part; an
/// HTML-only mail; a plain text mail with addresses and a phone number
/// written out.
const MBOX: &str = "From x Mon Nov  2 10:00:00 2026
From: Lettre <news@x.org>
To: me@x.org
Subject: Lettre
Message-ID: <1@x>
MIME-Version: 1.0
Content-Type: multipart/alternative; boundary=\"A\"

--A
Content-Type: text/plain; charset=utf-8

Votre facture&nbsp;est disponible.Voir ma facture
--A
Content-Type: text/html; charset=utf-8

<p>Votre facture&nbsp;est disponible.</p><p><a href=\"https://t.x.org/c?u=1&amp;k=2\">Voir ma facture</a></p>
<p><a href=\"https://x.org/unsubscribe\">Se d&eacute;sabonner</a></p>
--A--

From x Mon Nov  2 11:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Html
Message-ID: <2@x>
MIME-Version: 1.0
Content-Type: text/html; charset=utf-8

<p>Le rapport est <a href=\"https://x.org/rapport\">ici</a>, <a href=\"mailto:alice@x.org\">écris-moi</a>.</p>

From x Mon Nov  2 12:00:00 2026
From: Bob <bob@x.org>
To: me@x.org
Subject: Texte
Message-ID: <3@x>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Le plan : <https://x.org/plan>, et encore https://x.org/plan.
Bob, 06 12 34 56 78 (commande 2026-000123)
";

fn imported() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let path = dir.path().join("l.mbox");
    std::fs::write(&path, MBOX).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "t".into(),
        address: "me@x.org".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: nm_store::import::LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    (dir, store.into_inner().unwrap())
}

/// The links `get_thread` gives with the mail `subject`, and its text.
fn links(store: &Store, subject: &str) -> (Vec<LinkView>, String) {
    let thread: i64 = store
        .conn()
        .query_row("SELECT thread_id FROM message WHERE subject = ?1", [subject], |r| r.get(0))
        .unwrap();
    let m = store.get_thread(thread).unwrap().1.remove(0);
    (m.links, m.body_text.unwrap_or_default())
}

fn link(url: &str, text: &str) -> LinkView {
    LinkView { url: url.into(), text: Some(text.into()), target: None, other_texts: vec![] }
}

#[test]
fn a_botched_text_part_replaced_by_the_html_keeps_its_links() {
    let (_dir, store) = imported();
    let (links, text) = links(&store, "Lettre");
    // The text is the conversion of the HTML: no address left in it.
    assert!(text.contains("Voir ma facture") && !text.contains("https://"), "{text}");
    assert_eq!(
        links,
        vec![link("https://t.x.org/c?u=1&k=2", "Voir ma facture"), link("https://x.org/unsubscribe", "Se désabonner")]
    );
}

#[test]
fn an_html_only_mail_gives_its_web_and_mail_links() {
    let (_dir, store) = imported();
    assert_eq!(
        links(&store, "Html").0,
        vec![link("https://x.org/rapport", "ici"), link("mailto:alice@x.org", "écris-moi")]
    );
}

#[test]
fn a_text_mail_gives_its_written_addresses_once_and_its_phone_number() {
    let (_dir, store) = imported();
    assert_eq!(
        links(&store, "Texte").0,
        vec![
            LinkView { url: "https://x.org/plan".into(), text: None, target: None, other_texts: vec![] },
            // A phone number: its digits in the URL, as written in the text.
            link("tel:0612345678", "06 12 34 56 78"),
        ]
    );
}
