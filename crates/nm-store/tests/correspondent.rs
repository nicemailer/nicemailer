//! The person a conversation is with, as a list row names it
//! (`ThreadSummary::correspondent` and `last_is_mine`, `ui.md`, "Liste"):
//! real conversations imported from an mbox, then listed as a client does.
//!
//! Before, the row named the author of the latest mail, which was the
//! user themselves as soon as they had answered.

use std::sync::Mutex;

use nm_api::{ThreadFilter, ThreadSort, ThreadSummary};
use nm_core::{SenderKind, Source};
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

From n Mon Nov  2 10:00:00 2026
From: Shop <news@shop.example>
To: me@x.org
Subject: Weekly offers
Message-ID: <g1@shop.example>
List-Unsubscribe: <https://shop.example/unsubscribe>
Precedence: bulk

Offers.
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

#[test]
fn the_row_says_whether_a_person_or_a_robot_wrote_to_the_user() {
    // The icon of the sender column in Quarantine (`ui.md`, "Quarantine"):
    // the verdict of the mail that names the correspondent
    // (`message.sender_kind`, `expediteurs.md`, section 6).
    let s = store();
    // Alice, to whom the user wrote: a person, even when the user's answer
    // is the latest mail (the user's own verdict is never the one shown).
    assert_eq!(row(&s, "Answered", &[]).correspondent_kind, Some(SenderKind::Person));
    // A bulk mail with a list header: a robot.
    let news = row(&s, "Weekly offers", &[]);
    assert_eq!(news.correspondent.as_deref(), Some("Shop"));
    assert_eq!(news.correspondent_kind, Some(SenderKind::Robot));
    // The user wrote every mail: the recipient named is not judged.
    assert_eq!(row(&s, "Mine alone", &[]).correspondent_kind, None);
}

/// The thread of the conversation `subject`.
fn thread_of(store: &Store, subject: &str) -> i64 {
    store.conn().query_row("SELECT id FROM thread WHERE subject = ?1", [subject], |r| r.get(0)).unwrap()
}

/// What `g` adds to a group from the conversation `subject`.
fn added(store: &mut Store, subject: &str, own: &[String]) -> Result<String, nm_store::StoreError> {
    let t = thread_of(store, subject);
    store.set_group_member("Clients", None, Some(t), false, own, Source::App).map(|(_, addr, _)| addr)
}

#[test]
fn g_adds_the_person_the_conversation_is_with_never_the_user() {
    // The imported mails are all in Archive, the user's replies too: the
    // case where "the latest author outside Sent" was the user, and the
    // group then matched every mail of the mailbox.
    let mut s = store();
    assert_eq!(added(&mut s, "Answered", &[]).unwrap(), "alice@x.org");
    assert_eq!(added(&mut s, "Waiting on me", &[]).unwrap(), "alice@x.org");
    // The latest other author, as the sender column names.
    assert_eq!(added(&mut s, "Three people", &[]).unwrap(), "bob@x.org");
    // The user wrote alone: their first recipient, To before Cc.
    assert_eq!(added(&mut s, "Mine alone", &[]).unwrap(), "carol@x.org");
    // An address only `accounts.toml` declares counts as the user's.
    assert_eq!(added(&mut s, "Declared address", &["Work@X.org".into()]).unwrap(), "alice@x.org");
    let members: Vec<String> = s
        .conn()
        .prepare("SELECT a.addr FROM contact_group_member gm JOIN address a ON a.id = gm.address_id ORDER BY a.addr")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(members, ["alice@x.org", "bob@x.org", "carol@x.org"]);
}

#[test]
fn the_users_addresses_cannot_join_a_group() {
    let mut s = store();
    let own = vec!["work@x.org".to_string()];
    // The archive account's address, which the database holds, and one of
    // `accounts.toml`, given by the caller; in any case.
    for addr in ["me@x.org", "WORK@x.org"] {
        let err = s.set_group_member("Clients", Some(addr), None, false, &own, Source::App).unwrap_err();
        assert!(matches!(err, nm_store::StoreError::InvalidParams(_)), "{addr}: {err:?}");
    }
    // Nothing was created: neither the group nor a member.
    assert!(s.list_groups().unwrap().is_empty());
    // A conversation with nobody else (the user wrote to themselves):
    // nobody to add.
    let t = thread_of(&s, "Declared address");
    s.conn().execute("UPDATE message SET from_addr = 'me@x.org' WHERE thread_id = ?1", [t]).unwrap();
    s.conn()
        .execute(
            "DELETE FROM message_address WHERE message_id IN (SELECT id FROM message WHERE thread_id = ?1)
               AND address_id <> (SELECT id FROM address WHERE addr = 'work@x.org')",
            [t],
        )
        .unwrap();
    assert!(matches!(added(&mut s, "Declared address", &own), Err(nm_store::StoreError::NotFound(_))));
}

#[test]
fn reading_the_accounts_removes_the_users_addresses_from_the_groups() {
    let mut s = store();
    // Put in a group before the rule: the database of a user who pressed
    // `g` on their own reply, and an address `accounts.toml` declares
    // only later.
    s.set_group_member("Clients", Some("alice@x.org"), None, false, &[], Source::App).unwrap();
    s.set_group_member("Clients", Some("work@x.org"), None, false, &[], Source::App).unwrap();
    s.conn()
        .execute(
            "INSERT INTO contact_group_member (group_id, address_id, created_by, modified_by)
             SELECT g.id, a.id, 'app', 'app' FROM contact_group g, address a WHERE a.addr = 'me@x.org'",
            [],
        )
        .unwrap();
    assert_eq!(s.list_groups().unwrap()[0].member_count, 3);
    let own = vec!["work@x.org".to_string()];
    assert_eq!(s.remove_own_from_groups(&own, Source::ConfigSync).unwrap(), 2);
    let group = &s.list_groups().unwrap()[0];
    assert_eq!(group.member_count, 1);
    // The group's filter now matches the conversations of Alice only.
    let rows = s
        .list_everything(None, None, 50, ThreadSort::Received, &ThreadFilter { group_ids: vec![group.id], ..Default::default() }, &own)
        .unwrap()
        .0;
    assert!(!rows.is_empty() && rows.iter().all(|t| t.participants.iter().any(|p| p == "Alice")));
    // A tombstone, in the history; a second reading has nothing to do.
    let removed: i64 = s
        .conn()
        .query_row("SELECT count(*) FROM audit_log WHERE action = 'remove_member'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(s.remove_own_from_groups(&own, Source::ConfigSync).unwrap(), 0);
}

#[test]
fn the_labels_of_a_thread_name_its_tags_and_the_groups_of_its_person() {
    // The header of the open thread, and what `t` and `g` can remove
    // (`ui.md`, "Tags et groupes dans l'en-tête").
    let mut s = store();
    let answered = thread_of(&s, "Answered");
    // Nothing yet: no tag, no group, but the person is known.
    let l = s.thread_labels(answered, &[]).unwrap();
    assert_eq!((l.thread_id, l.correspondent.as_deref()), (answered, Some("alice@x.org")));
    assert!(l.tags.is_empty() && l.groups.is_empty());

    s.tag_thread(answered, "Urgent", false, Source::App).unwrap();
    s.tag_thread(answered, "Clients", false, Source::App).unwrap();
    // Two groups of Alice, created in this order (display order), and
    // one of Bob, who is not the person of this thread.
    added(&mut s, "Answered", &[]).unwrap();
    s.set_group_member("VIP", Some("alice@x.org"), None, false, &[], Source::App).unwrap();
    s.set_group_member("Team", Some("bob@x.org"), None, false, &[], Source::App).unwrap();
    let l = s.thread_labels(answered, &[]).unwrap();
    assert_eq!(l.tags, ["Clients", "Urgent"]);
    assert_eq!(l.groups, ["Clients", "VIP"]);
    // The same tags as the list row.
    assert_eq!(row(&s, "Answered", &[]).tags, l.tags);

    // A removed member, or a deleted group, is no longer named.
    s.set_group_member("VIP", None, Some(answered), true, &[], Source::App).unwrap();
    s.conn().execute("UPDATE contact_group SET date_deleted = unixepoch() WHERE name = 'Clients'", []).unwrap();
    assert!(s.thread_labels(answered, &[]).unwrap().groups.is_empty());
}

#[test]
fn a_thread_with_nobody_else_has_no_groups() {
    // The user wrote to themselves: no person, so no group, even though
    // the user's address was put in one before the rule.
    let mut s = store();
    let t = thread_of(&s, "Declared address");
    let own = vec!["work@x.org".to_string()];
    s.conn().execute("UPDATE message SET from_addr = 'me@x.org' WHERE thread_id = ?1", [t]).unwrap();
    s.conn()
        .execute(
            "DELETE FROM message_address WHERE message_id IN (SELECT id FROM message WHERE thread_id = ?1)
               AND address_id <> (SELECT id FROM address WHERE addr = 'work@x.org')",
            [t],
        )
        .unwrap();
    s.set_group_member("Clients", Some("work@x.org"), None, false, &[], Source::App).unwrap();
    let l = s.thread_labels(t, &own).unwrap();
    assert_eq!(l.correspondent, None);
    assert!(l.groups.is_empty());
}
