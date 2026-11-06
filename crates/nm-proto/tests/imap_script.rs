//! The IMAP client against a scripted server: each test gives the exact
//! conversation, what the client must send (`C:`) and what the server
//! answers (`S:`), over an in-memory pipe. No network: these tests run with
//! `make test`, and cover what a real server rarely does on demand (a cut
//! in the middle of a literal, an unreadable response, silence).
//!
//! Tags are predictable (`N1`, `N2`…), so a script is the conversation as
//! it would appear in a log.

use std::time::Duration;

use nm_proto::imap::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// Short limits: a test that waits is a test that failed.
fn timeouts() -> Timeouts {
    Timeouts { connect: Duration::from_secs(2), io: Duration::from_secs(2) }
}

/// Plays `script` as the server, on one end of a pipe; returns the other
/// end and the server's task, which panics (failing the test when awaited)
/// if the client sends something else than expected.
///
/// Lines: `S: text` sends `text` and CRLF; `C: text` reads one line from
/// the client and compares; `C~ n` reads `n` raw bytes (a literal);
/// `CLOSE` drops the connection; `SLEEP ms` waits; `RAW: text` sends
/// `text` without CRLF.
fn server(script: &str) -> (tokio::io::DuplexStream, tokio::task::JoinHandle<()>) {
    let (client, srv) = tokio::io::duplex(1 << 16);
    let script: Vec<String> = script.lines().map(|l| l.trim_start().to_string()).filter(|l| !l.is_empty()).collect();
    let task = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(srv);
        let mut r = BufReader::new(r);
        for step in script {
            if let Some(text) = step.strip_prefix("S: ") {
                w.write_all(format!("{text}\r\n").as_bytes()).await.unwrap();
            } else if let Some(text) = step.strip_prefix("RAW: ") {
                w.write_all(text.as_bytes()).await.unwrap();
            } else if let Some(expected) = step.strip_prefix("C: ").or((step == "C:").then_some("")) {
                let mut line = Vec::new();
                r.read_until(b'\n', &mut line).await.unwrap();
                let got = String::from_utf8_lossy(&line);
                assert_eq!(got.trim_end_matches(['\r', '\n']), expected, "client sent something else");
            } else if let Some(n) = step.strip_prefix("C~ ") {
                let mut buf = vec![0; n.parse().unwrap()];
                r.read_exact(&mut buf).await.unwrap();
            } else if step == "CLOSE" {
                return;
            } else if let Some(ms) = step.strip_prefix("SLEEP ") {
                tokio::time::sleep(Duration::from_millis(ms.parse().unwrap())).await;
            } else {
                panic!("bad script line {step}");
            }
        }
        // Keep the connection open until the client is done.
        let mut rest = Vec::new();
        let _ = r.read_to_end(&mut rest).await;
    });
    (client, task)
}

const GREETING: &str = "S: * OK [CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR LITERAL+ IDLE UIDPLUS MOVE ENABLE CONDSTORE QRESYNC] ready";

/// A session logged in, on the scripted server: the script starts after
/// the login.
async fn logged_in(after_login: &str) -> (Session, tokio::task::JoinHandle<()>) {
    let script = format!(
        "{GREETING}
         C: N1 AUTHENTICATE PLAIN AGFsaWNlAHNlY3JldA==
         S: N1 OK [CAPABILITY IMAP4rev1 LITERAL+ IDLE UIDPLUS MOVE ENABLE CONDSTORE QRESYNC SPECIAL-USE LIST-EXTENDED LIST-STATUS NAMESPACE] done
         {after_login}"
    );
    let (c, task) = server(&script);
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.login("alice", "secret").await.unwrap();
    (s, task)
}

#[tokio::test]
async fn login_with_authenticate_plain_and_the_capabilities_after_it() {
    let (s, task) = logged_in("").await;
    // Before login: those of the greeting. After: those of the OK, which
    // replace them entirely.
    assert!(s.capabilities_before_login().has("AUTH=PLAIN"));
    assert!(!s.capabilities_before_login().has("SPECIAL-USE"));
    assert!(s.capabilities().has("SPECIAL-USE"));
    assert!(!s.capabilities().has("AUTH=PLAIN"));
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn authenticate_without_sasl_ir_waits_for_the_go_ahead() {
    let (c, task) = server(
        "S: * OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] hi
         C: N1 AUTHENTICATE PLAIN
         S: +
         C: AGFsaWNlAHNlY3JldA==
         S: N1 OK logged in
         C: N2 CAPABILITY
         S: * CAPABILITY IMAP4rev1 IDLE
         S: N2 OK done",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.login("alice", "secret").await.unwrap();
    assert!(s.capabilities().has("IDLE"), "capabilities read again after login");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn login_command_quotes_and_uses_literals_when_needed() {
    // No AUTH=PLAIN: LOGIN. The user name has non-ASCII letters (a
    // literal; without LITERAL+, the client waits for `+`), the password a
    // quote (a quoted string, the quote escaped).
    let (c, task) = server(
        "S: * OK [CAPABILITY IMAP4rev1] hi
         C: N1 LOGIN {7}
         S: + go on
         C~ 7
         C:  \"p\\\"w\"
         S: N1 OK [CAPABILITY IMAP4rev1 IDLE] welcome",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.login("jérôm", "p\"w").await.unwrap_or_else(|e| panic!("{e}"));
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_refused_password_is_an_auth_error_and_the_session_stays_usable() {
    let (c, task) = server(&format!(
        "{GREETING}
         C: N1 AUTHENTICATE PLAIN AGFsaWNlAHdyb25n
         S: N1 NO [AUTHENTICATIONFAILED] Authentication failed.
         C: N2 NOOP
         S: N2 OK done"
    ));
    let mut s = Session::over(c, timeouts()).await.unwrap();
    let err = s.login("alice", "wrong").await.unwrap_err();
    assert!(matches!(err, ImapError::AuthFailed(ref t) if t.contains("Authentication failed")), "{err:?}");
    assert!(!err.to_string().contains("wrong"), "the password never appears in an error");
    assert!(!err.is_fatal());
    s.noop().await.unwrap();
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn greetings_bye_and_preauth() {
    let (c, task) = server("S: * BYE too many connections");
    let Err(e) = Session::over(c, timeouts()).await else { panic!("accepted") };
    assert!(matches!(e, ImapError::Bye(ref t) if t == "too many connections"), "{e:?}");
    task.await.unwrap();

    let (c, task) = server("S: * PREAUTH [CAPABILITY IMAP4rev1 IDLE] already in");
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.login("alice", "secret").await.unwrap(); // nothing to send
    drop(s);
    task.await.unwrap();

    let (c, task) = server("S: HTTP/1.1 400 Bad Request");
    let Err(e) = Session::over(c, timeouts()).await else { panic!("accepted") };
    assert!(matches!(e, ImapError::Protocol(_)), "{e:?}");
    task.await.unwrap();
}

#[tokio::test]
async fn select_with_qresync_reports_what_changed() {
    let (mut s, task) = logged_in(
        "C: N2 ENABLE CONDSTORE QRESYNC
         S: * ENABLED CONDSTORE QRESYNC
         S: N2 OK enabled
         C: N3 SELECT INBOX (QRESYNC (67890007 20050715194045000 41:211))
         S: * OK [CLOSED]
         S: * 314 EXISTS
         S: * FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk)
         S: * OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk \\*)] ok
         S: * OK [UIDVALIDITY 67890007] UIDs valid
         S: * OK [UIDNEXT 567] next
         S: * OK [HIGHESTMODSEQ 20060128194045000] modseq
         S: * VANISHED (EARLIER) 41,43:116,118,120:211
         S: * 49 FETCH (UID 117 FLAGS (\\Seen \\Answered) MODSEQ (20060115194045000))
         S: * 50 FETCH (UID 119 FLAGS (\\Draft $Junk) MODSEQ (20060115194045001))
         S: N3 OK [READ-WRITE] mailbox selected",
    )
    .await;
    assert_eq!(s.enable_sync().await.unwrap(), vec!["CONDSTORE", "QRESYNC"]);
    let q = QresyncParams { uid_validity: 67890007, modseq: 20050715194045000, known_uids: Some(UidSet::from_ranges([(41, 211)])) };
    let sel = s.select(&MailboxName::inbox(), Some(&q)).await.unwrap();
    assert_eq!(sel.exists, 314);
    assert_eq!(sel.uid_validity, Some(67890007));
    assert_eq!(sel.uid_next, Some(567));
    assert_eq!(sel.highest_modseq, Some(20060128194045000));
    assert!(!sel.read_only);
    assert!(sel.permanent_flags.contains(&"\\*".to_string()));
    assert_eq!(sel.vanished.to_string(), "41,43:116,118,120:211");
    assert_eq!(sel.changed.len(), 2);
    assert_eq!(sel.changed[0].uid, Some(117));
    assert_eq!(sel.changed[1].flags.as_deref(), Some(&["\\Draft".to_string(), "$Junk".to_string()][..]));
    assert_eq!(s.selected().unwrap().0, &MailboxName::inbox());
    assert!(s.take_unsolicited().is_empty(), "everything belonged to the SELECT");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn unsolicited_responses_during_a_fetch_are_kept_with_their_mailbox() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: * 3 EXISTS
         S: N2 OK [READ-WRITE] done
         C: N3 UID FETCH 1:* (UID FLAGS RFC822.SIZE)
         S: * 1 FETCH (UID 4 FLAGS (\\Seen) RFC822.SIZE 120)
         S: * 4 EXISTS
         S: * 2 EXPUNGE
         S: * 2 FETCH (FLAGS (\\Flagged))
         S: * 3 FETCH (UID 9 FLAGS () RFC822.SIZE 33)
         S: * OK [ALERT] Mailbox almost full
         S: N3 OK fetched",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let items = FetchItems { flags: true, size: true, ..FetchItems::default() };
    let got = s.uid_fetch(&Uids::From(1), &items).await.unwrap();
    assert_eq!(got.iter().map(|f| f.uid.unwrap()).collect::<Vec<_>>(), vec![4, 9]);
    assert_eq!(got[1].flags.as_deref(), Some(&[][..]));
    let notices = s.take_unsolicited();
    let events: Vec<_> = notices.iter().map(|n| n.event.clone()).collect();
    assert_eq!(events[0], Unsolicited::Exists(4));
    assert_eq!(events[1], Unsolicited::Expunge(2));
    // A FETCH without UID is not an answer to UID FETCH: a notice.
    assert!(matches!(&events[2], Unsolicited::Fetch(f) if f.seq == 2 && f.uid.is_none()));
    assert_eq!(events[3], Unsolicited::Alert("Mailbox almost full".into()));
    assert!(notices.iter().all(|n| n.mailbox.as_ref().is_some_and(MailboxName::is_inbox)));
    assert!(s.take_unsolicited().is_empty(), "taken once");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn an_unreadable_response_fails_the_command_but_not_the_session() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 UID FETCH 5 (UID FLAGS)
         S: * 1 FETCH (UID 5 FLAGS (\\Seen)
         S: * 1 FETCH (UID 5 FLAGS (\\Seen))
         S: N3 OK fetched
         C: N4 NOOP
         S: N4 OK still fine",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let items = FetchItems { flags: true, ..FetchItems::default() };
    let err = s.uid_fetch(&Uids::Set(UidSet::from_uids([5])), &items).await.unwrap_err();
    assert!(matches!(err, ImapError::Malformed { command: "UID FETCH", .. }), "{err:?}");
    assert!(!err.is_fatal());
    s.noop().await.unwrap();
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_cut_in_the_middle_of_a_literal_closes_the_session() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 UID FETCH 5 (UID BODY.PEEK[])
         S: * 1 FETCH (UID 5 BODY[] {100}
         RAW: only thirty bytes of the body
         CLOSE",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let items = FetchItems { body: Some(BodyRange::Whole), ..FetchItems::default() };
    let err = s.uid_fetch(&Uids::Set(UidSet::from_uids([5])), &items).await.unwrap_err();
    assert_eq!(err, ImapError::Closed);
    assert!(!s.is_open());
    assert_eq!(s.noop().await.unwrap_err(), ImapError::Closed, "every later call fails at once");
    task.await.unwrap();
}

#[tokio::test]
async fn bye_then_close_gives_the_reason() {
    let (mut s, task) = logged_in(
        "C: N2 NOOP
         S: * BYE server shutting down
         CLOSE",
    )
    .await;
    let err = s.noop().await.unwrap_err();
    assert_eq!(err, ImapError::Bye("server shutting down".into()));
    task.await.unwrap();
}

#[tokio::test]
async fn a_silent_server_times_out_and_closes_the_session() {
    let (c, task) = server(&format!(
        "{GREETING}
         C: N1 NOOP
         SLEEP 1500"
    ));
    let t = Timeouts { connect: Duration::from_secs(1), io: Duration::from_millis(200) };
    let mut s = Session::over(c, t).await.unwrap();
    let started = std::time::Instant::now();
    let err = s.noop().await.unwrap_err();
    assert!(matches!(err, ImapError::Timeout(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!s.is_open());
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_response_to_another_tag_is_a_protocol_error() {
    let (mut s, task) = logged_in(
        "C: N2 NOOP
         S: N7 OK what?",
    )
    .await;
    let err = s.noop().await.unwrap_err();
    assert!(matches!(err, ImapError::Protocol(_)), "{err:?}");
    assert!(!s.is_open());
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_response_larger_than_the_limit_is_refused_and_the_session_goes_on() {
    let size = MAX_RESPONSE as usize + 1000;
    let (c, srv) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(srv);
        let mut r = BufReader::new(r);
        let mut line = String::new();
        w.write_all(b"* PREAUTH [CAPABILITY IMAP4rev1 IDLE] hi\r\n").await.unwrap();
        r.read_line(&mut line).await.unwrap();
        assert_eq!(line, "N1 SELECT INBOX\r\n");
        w.write_all(b"N1 OK [READ-WRITE] selected\r\n").await.unwrap();
        line.clear();
        r.read_line(&mut line).await.unwrap();
        assert_eq!(line, "N2 UID FETCH 5 (UID BODY.PEEK[])\r\n");
        w.write_all(format!("* 1 FETCH (UID 5 BODY[] {{{size}}}\r\n").as_bytes()).await.unwrap();
        let chunk = vec![b'x'; 1 << 16];
        let mut left = size;
        while left > 0 {
            let n = left.min(chunk.len());
            w.write_all(&chunk[..n]).await.unwrap();
            left -= n;
        }
        w.write_all(b")\r\nN2 OK done\r\n").await.unwrap();
        line.clear();
        r.read_line(&mut line).await.unwrap();
        assert_eq!(line, "N3 NOOP\r\n");
        w.write_all(b"N3 OK fine\r\n").await.unwrap();
    });
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let items = FetchItems { body: Some(BodyRange::Whole), ..FetchItems::default() };
    let err = s.uid_fetch(&Uids::Set(UidSet::from_uids([5])), &items).await.unwrap_err();
    assert!(matches!(&err, ImapError::Malformed { detail, .. } if detail.contains("larger")), "{err:?}");
    s.noop().await.unwrap();
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn idle_ends_on_news_not_on_keepalives() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 IDLE
         S: + idling
         S: * OK Still here
         SLEEP 50
         S: * 5 EXISTS
         C: DONE
         S: * 6 EXISTS
         S: N3 OK Idle completed",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let end = s.idle(Duration::from_secs(5), std::future::pending()).await.unwrap();
    assert_eq!(end, IdleEnd::Changes);
    let events: Vec<_> = s.take_unsolicited().into_iter().map(|n| n.event).collect();
    assert_eq!(events, vec![Unsolicited::Exists(5), Unsolicited::Exists(6)], "what came after DONE is kept too");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn idle_stops_on_request_and_on_time() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 IDLE
         S: + idling
         C: DONE
         S: N3 OK Idle completed
         C: N4 IDLE
         S: + idling
         C: DONE
         S: N4 OK Idle completed
         C: N5 NOOP
         S: N5 OK fine",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let stop = tokio::time::sleep(Duration::from_millis(50));
    assert_eq!(s.idle(Duration::from_secs(5), stop).await.unwrap(), IdleEnd::Stopped);
    assert_eq!(s.idle(Duration::from_millis(50), std::future::pending()).await.unwrap(), IdleEnd::Timeout);
    s.noop().await.unwrap();
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn move_with_move_gives_the_new_uids() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 UID MOVE 4:5,9 Archive
         S: * OK [COPYUID 432432 4:5,9 101:103] moved
         S: * 2 EXPUNGE
         S: * 2 EXPUNGE
         S: * 5 EXPUNGE
         S: N3 OK Move completed",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let r = s.uid_move(&Uids::Set(UidSet::from_uids([9, 4, 5])), &MailboxName::new("Archive")).await.unwrap();
    assert_eq!(r.copy_uids.unwrap().pairs, vec![(4, 101), (5, 102), (9, 103)]);
    assert!(r.expunged);
    assert_eq!(s.take_unsolicited().len(), 3, "the expunges are notices");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn move_without_move_copies_flags_and_expunges_only_those_uids() {
    let (c, task) = server(
        "S: * PREAUTH [CAPABILITY IMAP4rev1 UIDPLUS] hi
         C: N1 SELECT INBOX
         S: N1 OK [READ-WRITE] done
         C: N2 UID COPY 7 Envoy&AOk-s
         S: N2 OK [COPYUID 5 7 12] copied
         C: N3 UID STORE 7 +FLAGS.SILENT (\\Deleted)
         S: N3 OK stored
         C: N4 UID EXPUNGE 7
         S: * 3 EXPUNGE
         S: N4 OK expunged",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let r = s.uid_move(&Uids::Set(UidSet::from_uids([7])), &MailboxName::new("Envoyés")).await.unwrap();
    assert_eq!(r.copy_uids.unwrap().pairs, vec![(7, 12)]);
    assert!(r.expunged);
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn append_waits_for_the_go_ahead_without_literal_plus() {
    let (c, task) = server(
        "S: * PREAUTH [CAPABILITY IMAP4rev1 UIDPLUS] hi
         C: N1 APPEND Sent (\\Seen) \"05-Nov-2026 14:03:00 +0000\" {11}
         S: + Ready for literal data
         C~ 11
         C:
         S: N1 OK [APPENDUID 38505 3955] APPEND completed",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    let uid = s.append(&MailboxName::new("Sent"), &["\\Seen"], Some(1_793_887_380), b"Hello world".to_vec()).await.unwrap();
    assert_eq!(uid, Some(AppendUid { uid_validity: 38505, uid: 3955 }));
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn append_refused_before_the_literal_is_sent() {
    // A server may refuse the literal at once (too big): the message must
    // not be sent, and the session goes on.
    let (c, task) = server(
        "S: * PREAUTH [CAPABILITY IMAP4rev1] hi
         C: N1 APPEND Sent {11}
         S: N1 NO [TOOBIG] Message too large
         C: N2 NOOP
         S: N2 OK fine",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    let err = s.append(&MailboxName::new("Sent"), &[], None, b"Hello world".to_vec()).await.unwrap_err();
    assert!(matches!(err, ImapError::No { command: "APPEND", .. }), "{err:?}");
    s.noop().await.unwrap();
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn store_with_unchangedsince_reports_the_conflicts() {
    let (mut s, task) = logged_in(
        "C: N2 SELECT INBOX (CONDSTORE)
         S: N2 OK [READ-WRITE] done
         C: N3 UID STORE 1:3 (UNCHANGEDSINCE 320) +FLAGS ($NmWasTodo \\Seen)
         S: * 1 FETCH (UID 1 MODSEQ (321) FLAGS ($NmWasTodo \\Seen))
         S: N3 OK [MODIFIED 2:3] some were changed",
    )
    .await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let r = s
        .uid_store(&Uids::Set(UidSet::from_ranges([(1, 3)])), StoreOp::Add, &["$NmWasTodo", "\\Seen"], false, Some(320))
        .await
        .unwrap();
    assert_eq!(r.fetched.len(), 1);
    assert_eq!(r.fetched[0].modseq, Some(321));
    assert_eq!(r.modified.to_string(), "2:3");
    let err = s.uid_store(&Uids::From(1), StoreOp::Add, &["bad flag"], true, None).await.unwrap_err();
    assert!(matches!(err, ImapError::State(_)), "nothing sent for an invalid flag");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn list_decodes_names_and_reads_special_uses_and_status() {
    let (mut s, task) = logged_in(
        "C: N2 LIST \"\" \"*\" RETURN (SPECIAL-USE STATUS (MESSAGES UIDNEXT UIDVALIDITY UNSEEN HIGHESTMODSEQ))
         S: * LIST () \"/\" INBOX
         S: * STATUS INBOX (MESSAGES 12 UIDNEXT 40 UIDVALIDITY 7 UNSEEN 2 HIGHESTMODSEQ 99)
         S: * LIST (\\Sent \\HasNoChildren) \"/\" \"Envoy&AOk-s\"
         S: * STATUS \"Envoy&AOk-s\" (MESSAGES 3 UIDNEXT 4 UIDVALIDITY 7 UNSEEN 0 HIGHESTMODSEQ 5)
         S: * LIST (\\Noselect \\HasChildren) \"/\" Clients
         S: * LIST () \"/\" {10}
         S: Clients/A&
         S: N2 OK done",
    )
    .await;
    let got = s.list_with_status().await.unwrap();
    assert_eq!(got.len(), 4);
    assert!(got[0].0.name.is_inbox());
    assert_eq!(got[0].1.as_ref().unwrap().messages, Some(12));
    assert_eq!(got[1].0.name.name, "Envoyés");
    assert_eq!(got[1].0.name.raw, b"Envoy&AOk-s");
    assert_eq!(got[1].0.special_use(), Some(SpecialUse::Sent));
    assert_eq!(got[1].1.as_ref().unwrap().highest_modseq, Some(5));
    assert!(!got[2].0.selectable());
    assert_eq!(got[2].1, None);
    // Not valid modified UTF-7 (an `&` alone): shown as sent, kept as sent.
    assert_eq!(got[3].0.name.name, "Clients/A&");
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn bodies_above_four_mib_come_in_pieces() {
    let size = WHOLE_BODY_MAX as usize + 300_000;
    let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    // Every byte value, NUL included: RFC 3501 forbids NUL in a literal,
    // the client reads it as 0x80, as Dovecot sends it (`read_resp`).
    let expected: Vec<u8> = body.iter().map(|&b| if b == 0 { 0x80 } else { b }).collect();
    let (c, srv) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(srv);
        let mut r = BufReader::new(r);
        let mut line = String::new();
        w.write_all(b"* PREAUTH [CAPABILITY IMAP4rev1] hi\r\n").await.unwrap();
        r.read_line(&mut line).await.unwrap();
        w.write_all(b"N1 OK [READ-WRITE] selected\r\n").await.unwrap();
        let mut tag = 2;
        let mut offset = 0usize;
        loop {
            line.clear();
            r.read_line(&mut line).await.unwrap();
            let want = format!("N{tag} UID FETCH 8 (UID BODY.PEEK[]<{offset}.{BODY_CHUNK}>)\r\n");
            assert_eq!(line, want);
            let piece = &body[offset.min(size)..(offset + BODY_CHUNK as usize).min(size)];
            w.write_all(format!("* 1 FETCH (UID 8 BODY[]<{offset}> {{{}}}\r\n", piece.len()).as_bytes()).await.unwrap();
            w.write_all(piece).await.unwrap();
            w.write_all(format!(")\r\nN{tag} OK done\r\n").as_bytes()).await.unwrap();
            offset += piece.len();
            tag += 1;
            if piece.len() < BODY_CHUNK as usize {
                break;
            }
        }
    });
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut got = Vec::new();
    let mut pieces = 0;
    let n = s
        .fetch_body(8, size as u32, &mut |b: &[u8]| -> std::result::Result<(), ()> {
            pieces += 1;
            assert!(b.len() <= BODY_CHUNK as usize);
            got.extend_from_slice(b);
            Ok(())
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, Some(size as u64));
    assert_eq!(pieces, 5);
    assert!(got == expected);
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_body_that_disappears_is_none() {
    let (c, task) = server(
        "S: * PREAUTH [CAPABILITY IMAP4rev1] hi
         C: N1 SELECT INBOX
         S: N1 OK [READ-WRITE] done
         C: N2 UID FETCH 8 (UID BODY.PEEK[])
         S: N2 OK nothing",
    );
    let mut s = Session::over(c, timeouts()).await.unwrap();
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let r = s.fetch_body(8, 10, &mut |_: &[u8]| -> std::result::Result<(), ()> { panic!("no body") }).await.unwrap().unwrap();
    assert_eq!(r, None);
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn commands_out_of_state_send_nothing() {
    let (c, task) = server("S: * OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] hi");
    let mut s = Session::over(c, timeouts()).await.unwrap();
    assert!(matches!(s.list().await, Err(ImapError::State(_))));
    assert!(matches!(s.uid_fetch(&Uids::From(1), &FetchItems::default()).await, Err(ImapError::State(_))));
    assert!(matches!(s.idle(Duration::from_secs(1), std::future::pending()).await, Err(ImapError::State(_))));
    drop(s);
    task.await.unwrap();
}

#[tokio::test]
async fn a_read_only_session_sends_no_writing_command() {
    // The script expects EXAMINE alone: any writing command would be a line
    // the server does not expect, and the test would fail.
    let (mut s, task) = logged_in(
        "C: N2 EXAMINE INBOX (CONDSTORE)\nS: * 3 EXISTS\nS: * OK [UIDVALIDITY 7] ok\nS: * OK [UIDNEXT 4] ok\nS: N2 OK [READ-ONLY] done",
    )
    .await;
    s.forbid_writes();
    let inbox = MailboxName::inbox();
    assert!(matches!(s.select(&inbox, None).await, Err(ImapError::ReadOnly("SELECT"))));
    assert!(matches!(s.create(&MailboxName::new("X")).await, Err(ImapError::ReadOnly("CREATE"))));
    assert!(matches!(s.append(&inbox, &[], None, b"x".to_vec()).await, Err(ImapError::ReadOnly("APPEND"))));
    s.examine(&inbox, None).await.unwrap();
    let one = Uids::Set(UidSet::from_uids([1]));
    assert!(matches!(s.uid_store(&one, StoreOp::Add, &["\\Seen"], true, None).await, Err(ImapError::ReadOnly("STORE"))));
    assert!(matches!(s.uid_move(&one, &MailboxName::new("X")).await, Err(ImapError::ReadOnly("MOVE"))));
    assert!(matches!(s.uid_copy(&one, &MailboxName::new("X")).await, Err(ImapError::ReadOnly("COPY"))));
    assert!(matches!(s.uid_expunge(&one).await, Err(ImapError::ReadOnly("EXPUNGE"))));
    assert!(s.is_open(), "a refusal is no failure of the session");
    drop(s);
    task.await.unwrap();
}
