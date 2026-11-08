//! The volume test of `plan-imap.md`, phase 8: a generated mailbox of
//! `NM_VOLUME` mails (100,000 by default) on the complete test Dovecot,
//! synchronized from nothing by the real core. Measures the initial
//! synchronization (headers, then every body), the core's memory, and how
//! fast the lists answer while it runs.
//!
//! Not part of `make imap-test` (it takes minutes): `make volume-test`,
//! after `make imap-up`. It prints its measures; it fails only if the
//! synchronization is not complete and verified, or a list takes more than
//! a second.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Client, start};
use nm_proto::imap::{MailboxName, Security, ServerSpec, Session, Timeouts};
use serde_json::json;

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

/// The core's resident memory, in MB (`ps`).
fn rss_mb(pid: u32) -> u64 {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        / 1024
}

fn dir_mb(p: &std::path::Path) -> u64 {
    let out = std::process::Command::new("/usr/bin/du")
        .args(["-sk"])
        .arg(p)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}

#[test]
#[ignore = "minutes long: make imap-up, then make volume-test"]
fn a_mailbox_of_a_hundred_thousand_mails() {
    let n: usize = std::env::var("NM_VOLUME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    // `NM_VOLUME_USER`: a mailbox filled by an earlier run (filling 100,000
    // mails takes the test Dovecot about 20 minutes); else a new one.
    let reuse = std::env::var("NM_VOLUME_USER").ok();
    let user = reuse
        .clone()
        .unwrap_or_else(|| format!("v{:016x}", rand::random::<u64>()));
    println!("volume: user {user}");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // The mailbox: 60 % in the Inbox, 30 % in Archive, 10 % in Sent; one
    // mail in three answers an earlier one; bodies of 1 to 6 kB.
    let fill = Instant::now();
    if reuse.is_none() {
        rt.block_on(async {
        let spec = ServerSpec { host: "127.0.0.1".into(), port: 31993, security: Security::Tls, ca_file: Some(ca()) };
        let mut s = Session::connect(&spec, Timeouts::default()).await.unwrap();
        s.login(&user, "nicemailer").await.unwrap();
        let _ = s.create(&MailboxName::new("Archive")).await;
        let words = ["devis", "facture", "chantier", "réunion", "projet", "livraison", "contrat", "planning"];
        for i in 0..n {
            let folder = match i % 10 {
                0 => "Sent",
                1..=3 => "Archive",
                _ => "INBOX",
            };
            let reply = if i % 3 == 0 && i > 0 { format!("In-Reply-To: <m{}@vol>\r\nReferences: <m{}@vol>\r\n", i / 2, i / 2) } else { String::new() };
            let body: String = (0..(20 + i % 80)).map(|k| format!("{} {} {k}\r\n", words[(i + k) % 8], words[(i * 7 + k) % 8])).collect();
            let mail = format!(
                "From: Person {} <p{}@ex{}.example>\r\nTo: {user}@example.com\r\nSubject: {} {i}\r\nMessage-ID: <m{i}@vol>\r\n{reply}\
                 Date: Thu, 05 Nov 2026 10:00:00 +0000\r\n\r\n{body}",
                i % 500,
                i % 500,
                i % 40,
                words[i % 8]
            );
            let flags: &[&str] = if i % 2 == 0 { &["\\Seen"] } else { &[] };
            s.append(&MailboxName::new(folder), flags, None, mail.into_bytes()).await.unwrap();
        }
        s.logout().await;
    });
        println!(
            "volume: {n} mails delivered in {:.0} s",
            fill.elapsed().as_secs_f64()
        );
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("accounts.toml"),
        format!(
            "[accounts.vol]\nemail = \"{user}@example.com\"\n[accounts.vol.imap]\nhost = \"127.0.0.1\"\nport = 31993\nusername = \"{user}\"\nca_file = \"{}\"\n",
            ca().display()
        ),
    )
    .unwrap();
    let core = start(dir.path());
    let pid = core.pid();
    let mut c = Client::connect(dir.path());
    let started = Instant::now();
    c.call(
        "set_password",
        json!({"account": "vol", "password": "nicemailer"}),
    )
    .unwrap();

    let mut headers_done: Option<f64> = None;
    let mut peak_rss = 0;
    let mut lists: Vec<f64> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3600);
    loop {
        let a = c.call("sync_status", json!({"account": "vol"})).unwrap()["accounts"][0].clone();
        peak_rss = peak_rss.max(rss_mb(pid));
        let messages: u64 = a["folders"]
            .as_array()
            .map(|f| f.iter().map(|x| x["messages"].as_u64().unwrap_or(0)).sum())
            .unwrap_or(0);
        if headers_done.is_none() && messages as usize >= n {
            headers_done = Some(started.elapsed().as_secs_f64());
        }
        // A list of the Inbox, as the app asks while the sync runs.
        if let Some(inbox) = a["folders"]
            .as_array()
            .and_then(|f| f.iter().find(|x| x["role"] == "inbox"))
            .map(|x| x["folder_id"].clone())
        {
            let t = Instant::now();
            c.call("list_threads", json!({"folder_id": inbox, "limit": 60}))
                .unwrap();
            lists.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        if headers_done.is_some()
            && a["state"] == "idle"
            && a["bodies_missing"] == 0
            && a["actions_pending"] == 0
        {
            break;
        }
        assert!(Instant::now() < deadline, "not done within an hour: {a}");
        std::thread::sleep(Duration::from_millis(250));
    }
    let total = started.elapsed().as_secs_f64();
    lists.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = lists.iter().sum::<f64>() / lists.len().max(1) as f64;
    let p99 = lists.get(lists.len() * 99 / 100).copied().unwrap_or(0.0);
    let max = lists.last().copied().unwrap_or(0.0);
    println!(
        "volume: headers of {n} mails in {:.0} s, everything (bodies included) in {total:.0} s",
        headers_done.unwrap()
    );
    println!("volume: core memory at most {peak_rss} MB");
    println!(
        "volume: list of the Inbox during the sync: {} samples, {avg:.0} ms average, {p99:.0} ms p99, {max:.0} ms at worst",
        lists.len()
    );
    println!(
        "volume: database {} MB, raw store {} MB",
        dir_mb(&dir.path().join("nicemailer.db")) + dir_mb(&dir.path().join("nicemailer.db-wal")),
        dir_mb(&dir.path().join("raw.noindex")),
    );
    let verify_started = Instant::now();
    let r = c
        .call("sync_verify", json!({"account": "vol", "skip_hash": true}))
        .unwrap();
    println!(
        "volume: sync verify {:.1} s",
        verify_started.elapsed().as_secs_f64()
    );
    assert_eq!(r["ok"], true, "{r:#}");
    assert!(max < 1000.0, "a list took {max:.0} ms during the sync");
}
