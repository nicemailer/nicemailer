//! The IMAP client against the test Dovecot servers (`make imap-up`):
//! every command, on both servers, the complete one (CONDSTORE, QRESYNC,
//! MOVE, SPECIAL-USE…) and the minimal one (IMAP4rev1 UIDPLUS IDLE
//! LITERAL+ only), which exercises the fallbacks.
//!
//! The tests are `#[ignore]`: `make test` does not need the servers.
//! `make imap-test` runs them. Each test logs in as a new random user, who
//! starts with an empty mailbox: the tests are independent of each other
//! and of earlier runs, without any setup.

use std::path::PathBuf;
use std::time::Duration;

use nm_proto::imap::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Server {
    Full,
    Minimal,
}

const BOTH: [Server; 2] = [Server::Full, Server::Minimal];

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

fn spec(server: Server, security: Security) -> ServerSpec {
    let port = match (server, security) {
        (Server::Full, Security::Tls) => 31993,
        (Server::Full, _) => 31143,
        (Server::Minimal, Security::Tls) => 32993,
        (Server::Minimal, _) => 32143,
    };
    ServerSpec { host: "127.0.0.1".into(), port, security, ca_file: Some(ca()) }
}

/// A user name nobody used: an empty mailbox.
fn new_user() -> String {
    format!("t{:016x}", rand::random::<u64>())
}

/// A session over TLS, logged in as `user`, sync extensions on.
async fn open(server: Server, user: &str) -> Session {
    let mut s = Session::connect(&spec(server, Security::Tls), Timeouts::default()).await.unwrap();
    s.login(user, "nicemailer").await.unwrap();
    s.enable_sync().await.unwrap();
    s
}

/// A message: headers, then `lines` lines of text.
fn mail(id: &str, subject: &str, lines: usize) -> Vec<u8> {
    let mut m = format!(
        "From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: {subject}\r\n\
         Message-ID: <{id}@test.nicemailer>\r\nDate: Thu, 05 Nov 2026 10:00:00 +0000\r\n\r\n"
    );
    for i in 0..lines {
        m.push_str(&format!("line {i} of the body, with enough text to make it a real line\r\n"));
    }
    m.into_bytes()
}

fn uids(list: &[u32]) -> Uids {
    Uids::Set(UidSet::from_uids(list.iter().copied()))
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn tls_and_starttls_on_both_servers() {
    for server in BOTH {
        for security in [Security::Tls, Security::Starttls] {
            let mut s = Session::connect(&spec(server, security), Timeouts::default()).await.unwrap();
            assert!(s.is_tls(), "{server:?} {security:?}");
            s.login(&new_user(), "nicemailer").await.unwrap();
            assert!(s.capabilities().has("IMAP4REV1"));
            if server == Server::Full {
                // The extensions of the authenticated state are only known
                // after login (`plan-imap.md`, section 1).
                assert!(!s.capabilities_before_login().has("QRESYNC"));
                assert!(s.capabilities().has("QRESYNC"));
                assert_eq!(s.enable_sync().await.unwrap(), vec!["CONDSTORE", "QRESYNC"]);
            } else {
                assert!(s.enable_sync().await.unwrap().is_empty());
                assert!(!s.condstore());
            }
            s.id().await.unwrap();
            s.noop().await.unwrap();
            s.logout().await;
            assert!(!s.is_open());
        }
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn a_wrong_password_is_refused() {
    for server in BOTH {
        let mut s = Session::connect(&spec(server, Security::Tls), Timeouts::default()).await.unwrap();
        let err = s.login(&new_user(), "not-the-password").await.unwrap_err();
        assert!(matches!(err, ImapError::AuthFailed(_)), "{err:?}");
        assert!(!err.to_string().contains("not-the-password"));
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn a_certificate_of_an_unknown_authority_is_refused() {
    // Without ca_file, only the system's authorities count: the test
    // authority is not one of them.
    let mut spec = spec(Server::Full, Security::Tls);
    spec.ca_file = None;
    let Err(e) = Session::connect(&spec, Timeouts::default()).await else { panic!("accepted") };
    assert!(matches!(e, ImapError::Tls(_)), "{e:?}");
    // The same for STARTTLS: no fallback to clear text.
    let mut spec = self::spec(Server::Full, Security::Starttls);
    spec.ca_file = None;
    let Err(e) = Session::connect(&spec, Timeouts::default()).await else { panic!("accepted") };
    assert!(matches!(e, ImapError::Tls(_)), "{e:?}");
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn list_create_and_names_outside_ascii() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        let list = s.list().await.unwrap();
        let uses: Vec<_> = list.iter().filter_map(ListEntry::special_use).collect();
        for u in [SpecialUse::Sent, SpecialUse::Trash, SpecialUse::Drafts, SpecialUse::Junk] {
            assert!(uses.contains(&u), "{server:?} {u:?} in {list:?}");
        }
        assert!(list.iter().any(|e| e.name.is_inbox()));

        let name = MailboxName::new("Envoyés/Été 2026 & co");
        s.create(&name).await.unwrap();
        let err = s.create(&name).await.unwrap_err();
        assert!(matches!(err, ImapError::No { .. }), "creating twice: {err:?}");
        let list = s.list().await.unwrap();
        let found = list.iter().find(|e| e.name.name == "Envoyés/Été 2026 & co").expect("listed, decoded");
        assert_eq!(found.name.raw, name.raw, "the server keeps the name as sent");
        assert_eq!(found.delimiter, Some('/'));
        assert!(list.iter().any(|e| e.name.name == "Envoyés"), "the parent exists too");
        let st = s.status(&name).await.unwrap();
        assert_eq!(st.messages, Some(0));
        assert!(st.uid_validity.is_some());
        assert_eq!(st.highest_modseq.is_some(), server == Server::Full);

        if server == Server::Full {
            let with = s.list_with_status().await.unwrap();
            let inbox = with.iter().find(|(e, _)| e.name.is_inbox()).unwrap();
            assert_eq!(inbox.1.as_ref().unwrap().messages, Some(0));
            let ns = s.namespace().await.unwrap();
            assert_eq!(ns.personal.len(), 1);
        } else {
            assert_eq!(s.list_with_status().await.unwrap_err(), ImapError::Unsupported("LIST-STATUS"));
        }
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn append_fetch_and_search() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        let inbox = MailboxName::inbox();
        let messages = [mail("a", "First", 3), mail("b", "Second été", 5), mail("c", "Third", 1)];
        let date = 1_793_887_380; // 2026-11-05 14:03:00 UTC
        let mut appended = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            let flags: &[&str] = if i == 0 { &["\\Seen"] } else { &[] };
            let uid = s.append(&inbox, flags, Some(date + i as i64), m.clone()).await.unwrap().expect("APPENDUID (UIDPLUS)");
            appended.push(uid.uid);
        }
        let sel = s.select(&inbox, None).await.unwrap();
        assert_eq!(sel.exists, 3);
        assert_eq!(sel.uid_next, Some(appended[2] + 1));
        assert!(!sel.read_only);
        assert_eq!(sel.highest_modseq.is_some(), server == Server::Full);

        let items = FetchItems {
            flags: true,
            internal_date: true,
            size: true,
            modseq: server == Server::Full,
            header: true,
            ..FetchItems::default()
        };
        let got = s.uid_fetch(&Uids::From(1), &items).await.unwrap();
        assert_eq!(got.iter().map(|f| f.uid.unwrap()).collect::<Vec<_>>(), appended);
        for (i, f) in got.iter().enumerate() {
            assert_eq!(f.size, Some(messages[i].len() as u32));
            assert_eq!(f.internal_date, Some(date + i as i64));
            let header = String::from_utf8(f.header.clone().unwrap()).unwrap();
            assert!(header.contains("Message-ID:"), "{header}");
            assert!(header.ends_with("\r\n\r\n"));
            assert_eq!(f.modseq.is_some(), server == Server::Full);
        }
        assert!(got[0].flags.as_ref().unwrap().contains(&"\\Seen".to_string()));

        let bodies = s.uid_fetch(&uids(&appended), &FetchItems { body: Some(BodyRange::Whole), ..FetchItems::default() }).await.unwrap();
        for (i, f) in bodies.iter().enumerate() {
            assert!(f.body.as_ref().unwrap() == &messages[i], "the body comes back byte for byte");
        }
        let after = s.uid_fetch(&uids(&appended[1..2]), &FetchItems { flags: true, ..FetchItems::default() }).await.unwrap();
        assert!(!after[0].flags.as_ref().unwrap().contains(&"\\Seen".to_string()), "BODY.PEEK does not mark it read");

        assert_eq!(s.uid_search(&SearchKey::Unseen).await.unwrap(), appended[1..].to_vec());
        assert_eq!(s.uid_search(&SearchKey::All).await.unwrap(), appended);
        let by_id = SearchKey::Header("Message-ID".into(), "<b@test.nicemailer>".into());
        assert_eq!(s.uid_search(&by_id).await.unwrap(), vec![appended[1]]);
        let by_subject = SearchKey::Header("Subject".into(), "été".into());
        assert_eq!(s.uid_search(&by_subject).await.unwrap(), vec![appended[1]], "CHARSET UTF-8");
        let since = SearchKey::Since { year: 2026, month: 11, day: 5 };
        assert_eq!(s.uid_search(&since).await.unwrap().len(), 3);
        assert!(s.uid_search(&SearchKey::Uid(UidSet::default())).await.unwrap().is_empty());
        // UIDs that do not exist are simply absent (RFC 3501, 6.4.8).
        assert!(s.uid_fetch(&uids(&[9999]), &items).await.unwrap().is_empty());
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn a_large_message_comes_back_in_pieces_identical() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        // About 6.5 MB: above the 4 MiB of a whole fetch.
        let big = mail("big", "Large", 100_000);
        assert!(big.len() as u32 > WHOLE_BODY_MAX);
        let uid = s.append(&MailboxName::inbox(), &[], None, big.clone()).await.unwrap().unwrap().uid;
        s.select(&MailboxName::inbox(), None).await.unwrap();
        let size = s.uid_fetch(&uids(&[uid]), &FetchItems { size: true, ..FetchItems::default() }).await.unwrap()[0].size.unwrap();
        let mut got = Vec::with_capacity(big.len());
        let mut pieces = 0;
        let n = s
            .fetch_body(uid, size, &mut |b: &[u8]| -> std::result::Result<(), ()> {
                pieces += 1;
                got.extend_from_slice(b);
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, Some(big.len() as u64));
        assert_eq!(pieces, big.len().div_ceil(BODY_CHUNK as usize));
        assert!(got == big);
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn flags_and_keywords() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        let inbox = MailboxName::inbox();
        let a = s.append(&inbox, &[], None, mail("a", "A", 1)).await.unwrap().unwrap().uid;
        let b = s.append(&inbox, &[], None, mail("b", "B", 1)).await.unwrap().unwrap().uid;
        let sel = s.select(&inbox, None).await.unwrap();
        assert!(sel.permanent_flags.contains(&"\\*".to_string()), "keywords can be created");

        let r = s.uid_store(&uids(&[a, b]), StoreOp::Add, &["\\Flagged", "$NmWasTodo"], false, None).await.unwrap();
        assert_eq!(r.fetched.len(), 2);
        assert!(r.fetched.iter().all(|f| f.flags.as_ref().unwrap().contains(&"$NmWasTodo".to_string())));
        let r = s.uid_store(&uids(&[a]), StoreOp::Remove, &["$NmWasTodo"], true, None).await.unwrap();
        // .SILENT: no flags sent back. With CONDSTORE, the server still
        // sends the new MODSEQ (RFC 7162, 3.2), without the flags.
        assert!(r.fetched.iter().all(|f| f.flags.is_none()), ".SILENT: {r:?}");
        assert_eq!(r.fetched.is_empty(), server == Server::Minimal);
        let f = s.uid_fetch(&uids(&[a, b]), &FetchItems { flags: true, ..FetchItems::default() }).await.unwrap();
        assert_eq!(f[0].flags.as_ref().unwrap(), &vec!["\\Flagged".to_string()]);
        assert!(f[1].flags.as_ref().unwrap().contains(&"$NmWasTodo".to_string()));

        if server == Server::Full {
            // UNCHANGEDSINCE: a modseq older than the message's current one
            // means someone changed it in between: left alone, reported.
            let m = s.uid_fetch(&uids(&[b]), &FetchItems { modseq: true, ..FetchItems::default() }).await.unwrap()[0].modseq.unwrap();
            let r = s.uid_store(&uids(&[a, b]), StoreOp::Add, &["\\Seen"], true, Some(m - 1)).await.unwrap();
            assert!(r.modified.contains(b), "{r:?}");
        } else {
            let err = s.uid_store(&uids(&[a]), StoreOp::Add, &["\\Seen"], true, Some(1)).await.unwrap_err();
            assert_eq!(err, ImapError::Unsupported("CONDSTORE"));
        }
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn moves_with_and_without_move() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        let inbox = MailboxName::inbox();
        let archive = MailboxName::new("Archive");
        s.create(&archive).await.unwrap();
        let mut list = Vec::new();
        for i in 0..3 {
            list.push(s.append(&inbox, &[], None, mail(&format!("m{i}"), "M", 1)).await.unwrap().unwrap().uid);
        }
        s.select(&inbox, None).await.unwrap();
        let r = s.uid_move(&uids(&list[..2]), &archive).await.unwrap();
        assert!(r.expunged, "{server:?}: MOVE, or COPY + UID EXPUNGE with UIDPLUS");
        let pairs = r.copy_uids.unwrap().pairs;
        assert_eq!(pairs.iter().map(|p| p.0).collect::<Vec<_>>(), list[..2].to_vec());
        let notices = s.take_unsolicited();
        assert!(
            notices.iter().any(|n| matches!(n.event, Unsolicited::Expunge(_) | Unsolicited::Vanished { earlier: false, .. })),
            "{server:?}: the expunges are reported: {notices:?}"
        );
        assert_eq!(s.uid_search(&SearchKey::All).await.unwrap(), vec![list[2]]);
        let sel = s.select(&archive, None).await.unwrap();
        assert_eq!(sel.exists, 2);
        assert_eq!(s.uid_search(&SearchKey::All).await.unwrap(), pairs.iter().map(|p| p.1).collect::<Vec<_>>());

        // COPY keeps the original.
        let c = s.uid_copy(&uids(&[pairs[0].1]), &inbox).await.unwrap().unwrap();
        assert_eq!(c.pairs.len(), 1);
        assert_eq!(s.status(&inbox).await.unwrap().messages, Some(2));
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn qresync_reports_what_changed_since() {
    let mut s = open(Server::Full, &new_user()).await;
    let inbox = MailboxName::inbox();
    let mut list = Vec::new();
    for i in 0..3 {
        list.push(s.append(&inbox, &[], None, mail(&format!("q{i}"), "Q", 1)).await.unwrap().unwrap().uid);
    }
    let first = s.select(&inbox, None).await.unwrap();
    let (validity, modseq) = (first.uid_validity.unwrap(), first.highest_modseq.unwrap());

    // What another client could do: a flag on the first, the second gone.
    s.uid_store(&uids(&[list[0]]), StoreOp::Add, &["\\Seen"], true, None).await.unwrap();
    s.uid_store(&uids(&[list[1]]), StoreOp::Add, &["\\Deleted"], true, None).await.unwrap();
    s.uid_expunge(&uids(&[list[1]])).await.unwrap();
    s.take_unsolicited();

    // Changes since `modseq`, in one FETCH…
    let items = FetchItems { flags: true, changed_since: Some(modseq), vanished: true, ..FetchItems::default() };
    let (changed, vanished) = s.uid_fetch_vanished(&Uids::From(1), &items).await.unwrap();
    assert_eq!(changed.iter().map(|f| f.uid.unwrap()).collect::<Vec<_>>(), vec![list[0]]);
    assert_eq!(vanished.iter().collect::<Vec<_>>(), vec![list[1]]);

    // …and at SELECT time, the next time the mailbox is opened.
    s.examine(&MailboxName::new("Trash"), None).await.unwrap();
    let q = QresyncParams { uid_validity: validity, modseq, known_uids: Some(UidSet::from_uids(list.clone())) };
    let sel = s.select(&inbox, Some(&q)).await.unwrap();
    assert_eq!(sel.vanished.iter().collect::<Vec<_>>(), vec![list[1]]);
    assert_eq!(sel.changed.iter().map(|f| f.uid.unwrap()).collect::<Vec<_>>(), vec![list[0]]);
    assert!(sel.highest_modseq.unwrap() > modseq);
    s.logout().await;
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn examine_is_read_only() {
    for server in BOTH {
        let mut s = open(server, &new_user()).await;
        let uid = s.append(&MailboxName::inbox(), &[], None, mail("x", "X", 1)).await.unwrap().unwrap().uid;
        let sel = s.examine(&MailboxName::inbox(), None).await.unwrap();
        assert!(sel.read_only);
        assert!(s.selected().unwrap().1);
        // RFC 3501 lets the server refuse (NO) or ignore it; Dovecot
        // answers OK and changes nothing. Either way the flag is not set.
        let _ = s.uid_store(&uids(&[uid]), StoreOp::Add, &["\\Seen"], true, None).await;
        s.select(&MailboxName::inbox(), None).await.unwrap();
        let f = s.uid_fetch(&uids(&[uid]), &FetchItems { flags: true, ..FetchItems::default() }).await.unwrap();
        assert!(!f[0].flags.as_ref().unwrap().contains(&"\\Seen".to_string()), "{f:?}");
        s.logout().await;
    }
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn idle_sees_a_new_message() {
    for server in BOTH {
        let user = new_user();
        let mut watcher = open(server, &user).await;
        let mut other = open(server, &user).await;
        watcher.select(&MailboxName::inbox(), None).await.unwrap();
        let deliver = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            other.append(&MailboxName::inbox(), &[], None, mail("new", "New", 1)).await.unwrap();
        };
        let (end, ()) = tokio::join!(watcher.idle(Duration::from_secs(10), std::future::pending()), deliver);
        assert_eq!(end.unwrap(), IdleEnd::Changes, "{server:?}");
        let notices = watcher.take_unsolicited();
        assert!(notices.iter().any(|n| n.event == Unsolicited::Exists(1)), "{notices:?}");
        // IDLE can be started again on the same session.
        let end = watcher.idle(Duration::from_millis(300), std::future::pending()).await.unwrap();
        assert_eq!(end, IdleEnd::Timeout);
        watcher.noop().await.unwrap();
    }
}

/// A TCP relay to the full server that closes both sides once it has
/// relayed `cut_after` bytes from the server: a network cut, at a chosen
/// moment.
async fn cutting_proxy(cut_after: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let server = TcpStream::connect("127.0.0.1:31993").await.unwrap();
        let (mut cr, mut cw) = client.into_split();
        let (mut sr, mut sw) = server.into_split();
        let up = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut cr, &mut sw).await;
        });
        let mut relayed = 0;
        let mut buf = vec![0; 16 * 1024];
        loop {
            let n = match sr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let n = n.min(cut_after - relayed);
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
            relayed += n;
            if relayed >= cut_after {
                break;
            }
        }
        up.abort();
        // Dropping the halves closes both connections.
    });
    port
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn a_network_cut_during_a_body_closes_the_session_and_a_new_one_works() {
    let user = new_user();
    let big = mail("cut", "Cut", 20_000); // about 1.3 MB
    let mut s = open(Server::Full, &user).await;
    let uid = s.append(&MailboxName::inbox(), &[], None, big.clone()).await.unwrap().unwrap().uid;
    s.logout().await;

    // The login and the SELECT take a few kilobytes: a cut after 300 KB
    // falls in the middle of the body's literal.
    let port = cutting_proxy(300_000).await;
    let spec = ServerSpec { host: "127.0.0.1".into(), port, security: Security::Tls, ca_file: Some(ca()) };
    let mut s = Session::connect(&spec, Timeouts::default()).await.unwrap();
    s.login(&user, "nicemailer").await.unwrap();
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut got = 0;
    let err = s.fetch_body(uid, big.len() as u32, &mut |b: &[u8]| -> std::result::Result<(), ()> {
        got += b.len();
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(err.is_fatal(), "{err:?}");
    assert_eq!(got, 0, "nothing of an incomplete body reaches the sink");
    assert!(!s.is_open());

    let mut s = open(Server::Full, &user).await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut body = Vec::new();
    s.fetch_body(uid, big.len() as u32, &mut |b: &[u8]| -> std::result::Result<(), ()> {
        body.extend_from_slice(b);
        Ok(())
    })
    .await
    .unwrap()
    .unwrap();
    assert!(body == big);
}
