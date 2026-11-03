//! Simplified display of a thread through `get_thread` (`clean.rs`): a real
//! conversation imported from an mbox, then read back as a client does.

use std::sync::Mutex;

use nm_core::Source;
use nm_store::Store;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;

const MBOX: &str = "From a Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Chantier
Message-ID: <1@x>

Peux-tu passer jeudi sur le chantier pour valider les plans du rez-de-chaussee ?

--\x20
Alice Martin
Atelier Dupuis

From me Mon Nov  2 11:00:00 2026
From: me@x.org
To: alice@x.org
Cc: bob@x.org
Subject: Re: Chantier
Message-ID: <2@x>
In-Reply-To: <1@x>

Oui, jeudi 10 h.

Envoyé de mon iPhone

Le 2 nov. 2026 a 10:00, Alice <alice@x.org> a ecrit :
> Peux-tu passer jeudi sur le chantier pour valider les plans du rez-de-chaussee ?

From a Mon Nov  2 12:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Cc: bob@x.org
Subject: Re: Chantier
Message-ID: <3@x>
In-Reply-To: <2@x>

Le 2 nov. 2026 a 11:00, me@x.org a ecrit :
> Oui, jeudi 10 h.
";

fn thread() -> Vec<nm_api::MessageView> {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let path = dir.path().join("t.mbox");
    // The attribution lines use "a ecrit" without accent in the fixture, so
    // add the accented form expected by the rules.
    std::fs::write(&path, MBOX.replace("a ecrit :", "a écrit :")).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "t".into(),
        address: "me@x.org".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    let store = store.into_inner().unwrap();
    let id: i64 = store.conn().query_row("SELECT thread_id FROM message LIMIT 1", [], |r| r.get(0)).unwrap();
    store.get_thread(id).unwrap().1
}

fn kinds(m: &nm_api::MessageView) -> Vec<(&str, bool)> {
    m.segments.iter().map(|s| (s.kind.as_str(), s.hidden)).collect()
}

#[test]
fn each_message_shows_what_it_brings() {
    let t = thread();
    assert_eq!(t.len(), 3);
    assert_eq!(kinds(&t[0]), vec![("text", false), ("signature", true)]);
    assert_eq!(kinds(&t[1]), vec![("text", false), ("boilerplate", true), ("quote_tail", true)]);
    assert_eq!(t[1].segments[0].text, "Oui, jeudi 10 h.");
    // The quoted history is in the thread (first message).
    assert!(t[1].segments[2].in_thread);
    // The last message only quotes the previous one.
    assert!(t[2].only_quotes);
    assert_eq!(kinds(&t[2]), vec![("quote_tail", true)]);
}

#[test]
fn only_the_latest_and_unread_messages_are_unfolded() {
    let t = thread();
    // Imported mail is read: only the latest is open.
    assert_eq!(t.iter().map(|m| m.expanded).collect::<Vec<_>>(), vec![false, false, true]);
}

#[test]
fn recipients_are_shown_only_when_they_change() {
    let t = thread();
    // First: a single recipient; second: recipients changed (Alice + Bob);
    // third: the same recipients as the second... minus Alice, plus me.
    assert!(!t[0].show_recipients);
    assert!(t[1].show_recipients);
    assert!(t[2].show_recipients);
}
