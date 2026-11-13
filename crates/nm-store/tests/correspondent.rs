//! The person a conversation is with, as a list row names it
//! (`ThreadSummary::correspondent` and `last_is_mine`, `ui.md`, "Liste"):
//! real conversations imported from an mbox, then listed as a client does.
//!
//! Before, the row named the author of the latest mail, which was the
//! user themselves as soon as they had answered.

use std::sync::Mutex;

use nm_api::{ThreadFilter, ThreadSort, ThreadSummary};
use nm_core::Source;
use nm_store::Store;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;

/// One conversation per subject. `me@x.org` is the address of the
/// imported (archive) account, which the database holds; `work@x.org` is
/// one only `accounts.toml` would declare, given by the caller.
const MBOX: &str = "From a Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Answered
Message-ID: <a1@x>

Question ?

From me Mon Nov  2 11:00:00 2026
From: me@x.org
To: alice@x.org
Subject: Re: Answered
Message-ID: <a2@x>
In-Reply-To: <a1@x>

Answer.

From a Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Waiting on me
Message-ID: <b1@x>

Question ?

From me Mon Nov  2 11:00:00 2026
From: Me <me@x.org>
To: alice@x.org
Subject: Re: Waiting on me
Message-ID: <b2@x>
In-Reply-To: <b1@x>

Answer.

From a Mon Nov  2 12:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Re: Waiting on me
Message-ID: <b3@x>
In-Reply-To: <b2@x>

Another question ?

From me Mon Nov  2 10:00:00 2026
From: me@x.org
To: Carol <carol@x.org>, dan@x.org
Cc: erin@x.org
Subject: Mine alone
Message-ID: <c1@x>

Hello.

From me Mon Nov  2 10:00:00 2026
From: me@x.org
To: frank@x.org
Subject: No name
Message-ID: <d1@x>

Hello.

From a Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: me@x.org, bob@x.org
Subject: Three people
Message-ID: <e1@x>

Question ?

From b Mon Nov  2 11:00:00 2026
From: Bob <bob@x.org>
To: alice@x.org, me@x.org
Subject: Re: Three people
Message-ID: <e2@x>
In-Reply-To: <e1@x>

Bob's answer.

From me Mon Nov  2 12:00:00 2026
From: me@x.org
To: alice@x.org, bob@x.org
Subject: Re: Three people
Message-ID: <e3@x>
In-Reply-To: <e2@x>

My answer.

From a Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: work@x.org
Subject: Declared address
Message-ID: <f1@x>

Question ?

From w Mon Nov  2 11:00:00 2026
From: Work <work@x.org>
To: alice@x.org
Subject: Re: Declared address
Message-ID: <f2@x>
In-Reply-To: <f1@x>

Answer.
";

/// The store, with the mbox imported into the archive account `me@x.org`.
fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let path = dir.path().join("t.mbox");
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
    store.into_inner().unwrap()
}

/// The row of the conversation `subject` in Everything, the addresses
/// `own` given as the daemon gives those of `accounts.toml`.
fn row(store: &Store, subject: &str, own: &[String]) -> ThreadSummary {
    let (rows, _) = store.list_everything(None, None, 50, ThreadSort::Received, &ThreadFilter::default(), own).unwrap();
    rows.into_iter().find(|t| t.subject == subject).unwrap_or_else(|| panic!("no row {subject}"))
}

/// What the sender column shows: the correspondent, marked when the user
/// spoke last.
fn shown(t: &ThreadSummary) -> (Option<&str>, bool) {
    (t.correspondent.as_deref(), t.last_is_mine)
}

#[test]
fn an_answered_mail_names_the_person_answered() {
    let s = store();
    let t = row(&s, "Answered", &[]);
    // The case that started it: the latest mail is the user's answer.
    assert_eq!(shown(&t), (Some("Alice"), true));
    // `participants` is unchanged, for the clients that show them all.
    assert_eq!(t.participants, ["Alice", "me@x.org"]);
}

#[test]
fn a_conversation_that_waits_on_the_user_is_not_marked() {
    let s = store();
    assert_eq!(shown(&row(&s, "Waiting on me", &[])), (Some("Alice"), false));
}

#[test]
fn a_mail_the_user_wrote_alone_names_its_first_recipient() {
    let s = store();
    // To before Cc, in header order, by name when the header gives one.
    assert_eq!(shown(&row(&s, "Mine alone", &[])), (Some("Carol"), true));
    // Else by address.
    assert_eq!(shown(&row(&s, "No name", &[])), (Some("frank@x.org"), true));
}

#[test]
fn with_several_people_the_latest_other_author_is_named() {
    let s = store();
    // Alice asked, Bob answered, then the user: Bob spoke last before them.
    assert_eq!(shown(&row(&s, "Three people", &[])), (Some("Bob"), true));
}

#[test]
fn an_address_only_accounts_toml_declares_counts_as_the_users() {
    let s = store();
    // Unknown to the database: the mail of `work@x.org` is someone else's,
    // and the row names it, as before the change.
    assert_eq!(shown(&row(&s, "Declared address", &[])), (Some("Work"), false));
    // Given by the caller (the daemon reads `accounts.toml`), in any case.
    assert_eq!(shown(&row(&s, "Declared address", &["Work@X.org".into()])), (Some("Alice"), true));
}

#[test]
fn every_list_names_the_correspondent() {
    let s = store();
    let own: Vec<String> = vec![];
    let f = ThreadFilter::default();
    let answered = |rows: Vec<ThreadSummary>| rows.into_iter().find(|t| t.subject == "Answered").map(|t| shown(&t).0.map(str::to_string));
    let folders: Vec<i64> = s.list_folders().unwrap().iter().map(|f| f.id).collect();
    let expected = Some(Some("Alice".to_string()));
    assert_eq!(answered(s.list_threads_in(&folders, None, 50, ThreadSort::Received, &f, &own).unwrap().0), expected);
    assert_eq!(answered(s.list_all_mail(None, None, 50, ThreadSort::Received, &f, &own).unwrap().0), expected);
    // Imported mail is in Archive, out of Work: filed in the Inbox, the
    // conversation is listed there too.
    let inbox = s.list_folders().unwrap().into_iter().find(|f| f.role == Some(nm_core::FolderRole::Inbox)).unwrap().id;
    let answered_id = s.conn().query_row("SELECT id FROM thread WHERE subject = 'Answered'", [], |r| r.get::<_, i64>(0)).unwrap();
    s.conn().execute("UPDATE message SET folder_id = ?1 WHERE thread_id = ?2", [inbox, answered_id]).unwrap();
    s.conn().execute("UPDATE thread SET folder = 'i' WHERE id = ?1", [answered_id]).unwrap();
    assert_eq!(answered(s.list_work(None, None, 50, ThreadSort::Modified, &f, &own).unwrap().0), expected);
}
