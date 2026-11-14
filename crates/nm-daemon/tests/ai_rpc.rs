//! The AI assistance through the contract, on the real core (`ia.md`,
//! section 10; `src/ai.rs`): a reply proofread from start to end, the
//! checks made before a run starts, the answers refused, a draft changed
//! during a run, cancelling, and a `claude` binary that announces tools.
//!
//! The provider is the fake one (`NICEMAILER_FAKE_AI`, a table of answers
//! by task), or a fake `claude` binary (`ai.claude_path`, a shell script)
//! when the test is about the process.

mod common;

use std::path::Path;

use common::{Client, start_with};
use serde_json::{Value, json};

const MBOX: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Devis
Message-ID: <m1@a.example>

Tu viens jeudi ?
";

const QUOTE: &str = "Le 23 juin 2020 à 19:20, Alice <alice@a.example> a écrit :";

/// A core with the AI on, the fake network (an archive account can
/// answer) and, when given, fake AI answers.
fn core(dir: &Path, answers: Option<Value>, ai: &str) -> common::Core {
    std::fs::write(dir.join("config.toml"), format!("[ai]\nenabled = true\n{ai}")).unwrap();
    let mut env = vec![("NICEMAILER_FAKE_NETWORK", "1".to_string())];
    if let Some(a) = answers {
        let path = dir.join("answers.json");
        std::fs::write(&path, a.to_string()).unwrap();
        env.push(("NICEMAILER_FAKE_AI", path.display().to_string()));
    }
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    start_with(dir, &env)
}

/// Imports the mail, starts a reply and types `text` above the quotation;
/// returns the draft.
fn reply(dir: &Path, c: &mut Client, text: &str) -> Value {
    let mbox = dir.join("test.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    c.call("subscribe", json!({})).unwrap();
    c.call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"})).unwrap();
    c.event("ImportFinished", 30);
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let m: i64 = db.query_row("SELECT id FROM message WHERE message_id_hdr = 'm1@a.example'", [], |r| r.get(0)).unwrap();
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": QUOTE})).unwrap()["draft"].clone();
    let body = format!("{text}{}", d["body"].as_str().unwrap());
    let r = c.call("update_draft", json!({"id": d["id"], "revision": d["revision"], "body": body})).unwrap();
    let mut d = d;
    d["revision"] = r["revision"].clone();
    d["body"] = json!(body);
    d
}

fn run(c: &mut Client, d: &Value, prompt: &str) -> Result<Value, Value> {
    c.call("ai_run", json!({"prompt": prompt, "draft_id": d["id"], "revision": d["revision"], "lang": "fr"}))
}

/// The row of a run, from the database.
fn row(dir: &Path, run_id: i64) -> (String, Option<String>, Option<String>, Option<String>) {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.query_row("SELECT state, decision, error, output FROM ai_run WHERE id = ?1", [run_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
}

fn runs(dir: &Path) -> i64 {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.query_row("SELECT count(*) FROM ai_run", [], |r| r.get(0)).unwrap()
}

#[test]
fn a_reply_is_proofread_and_only_the_users_words_change() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"proofread": "<segment n=\"1\">Oui, je viens jeudi à 14h.</segment>"});
    let _core = core(dir.path(), Some(answers), "");
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "oui je vien jeudi a 14h.\n");

    // The compose menu: proofread can run, writing from notes cannot yet.
    let menu = c.call("ai_list_prompts", json!({"screen": "compose", "lang": "fr"})).unwrap();
    let find = |id: &str| menu.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap().clone();
    assert_eq!((find("proofread")["key"].clone(), find("proofread")["label"].clone()), (json!("c"), json!("Corriger")));
    assert_eq!(find("proofread")["available"], true);
    assert_eq!(find("write-from-notes")["available"], false);

    c.clear_events();
    let r = run(&mut c, &d, "proofread").unwrap();
    assert_eq!(r["streamed"], true);
    let id = r["run_id"].as_i64().unwrap();
    // The text comes in fragments, without the segment tags (the fake cuts
    // the answer word by word, tags included), then the end.
    let mut shown = String::new();
    let done = loop {
        let v = c.next_event();
        match v["method"].as_str() {
            Some("AiDelta") => {
                assert_eq!(v["params"]["run_id"], id);
                assert!(v["params"].get("seq").is_none(), "fragments are transient");
                shown.push_str(v["params"]["text"].as_str().unwrap());
            }
            Some("AiDone") => break v["params"].clone(),
            Some("AiFailed") => panic!("{v}"),
            _ => {}
        }
    };
    assert_eq!(done["run_id"], id);
    assert_eq!(shown, "Oui, je viens jeudi à 14h.");

    let v = c.call("ai_get_run", json!({"run_id": id})).unwrap();
    assert_eq!(v["state"], "done");
    let p = &v["proposal"];
    assert_eq!(p["kind"], "replace");
    assert_eq!(p["unchanged"], false);
    let seg = &p["segments"][0];
    assert_eq!(seg["old"], "oui je vien jeudi a 14h.");
    assert_eq!(seg["new"], "Oui, je viens jeudi à 14h.");
    assert_eq!(seg["start"], 0);
    let removed: Vec<&str> = seg["diff"].as_array().unwrap().iter().filter(|x| x["kind"] == "removed").map(|x| x["text"].as_str().unwrap()).collect();
    assert!(removed.contains(&"oui ") && removed.iter().any(|t| t.contains("vien")), "{removed:?}");

    let applied = c.call("ai_apply", json!({"run_id": id, "revision": d["revision"]})).unwrap();
    let body = applied["draft"]["body"].as_str().unwrap();
    let expected = d["body"].as_str().unwrap().replacen("oui je vien jeudi a 14h.", "Oui, je viens jeudi à 14h.", 1);
    assert_eq!(body, expected, "the quotation and its header are untouched");
    assert!(body.contains(QUOTE) && body.contains("> Tu viens jeudi ?"));

    let (state, decision, _, output) = row(dir.path(), id);
    assert_eq!((state.as_str(), decision.as_deref()), ("done", Some("accepted")));
    assert!(output.unwrap().contains("Oui, je viens"));
    // Applied once: the proposal is gone.
    assert_eq!(c.call("ai_apply", json!({"run_id": id, "revision": applied["draft"]["revision"]})).unwrap_err()["code"], 1001);
}

#[test]
fn nothing_runs_while_the_ai_is_off() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "").unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "Bonjour.\n");
    let e = run(&mut c, &d, "proofread").unwrap_err();
    assert_eq!(e["code"], 1017, "{e}");
    let s = c.call("ai_status", json!({})).unwrap();
    assert_eq!((s["enabled"].clone(), s["ready"].clone()), (json!(false), json!(false)));
    assert!(s["reason"].as_str().unwrap().contains("ai.enabled"));
    assert_eq!(runs(dir.path()), 0);
}

#[test]
fn what_cannot_work_is_refused_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let _core = core(dir.path(), Some(json!({})), "");
    let mut c = Client::connect_as(dir.path(), "app");
    // Nothing typed above the quotation: no text of the user's.
    let d = reply(dir.path(), &mut c, "");
    let e = run(&mut c, &d, "proofread").unwrap_err();
    assert_eq!((e["code"].clone(), e["data"]["code"].clone()), (json!(1018), json!("ai_bad_input")), "{e}");
    // A prompt not wired yet.
    let e = run(&mut c, &d, "write-from-notes").unwrap_err();
    assert_eq!(e["data"]["code"], "ai_bad_input");
    // An unknown prompt, a variant missing, a stale revision.
    assert_eq!(run(&mut c, &d, "nothing").unwrap_err()["code"], 1001);
    assert_eq!(run(&mut c, &d, "rewrite").unwrap_err()["code"], -32602);
    let stale = json!({"id": d["id"], "revision": d["revision"].as_i64().unwrap() - 1});
    assert_eq!(run(&mut c, &stale, "proofread").unwrap_err()["code"], 1012);
    // None of these left a row.
    assert_eq!(runs(dir.path()), 0);
}

#[test]
fn an_answer_with_the_wrong_segments_fails_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"proofread": "<segment n=\"1\">A</segment><segment n=\"2\">B</segment>"});
    let _core = core(dir.path(), Some(answers), "");
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "Un seul segment.\n");
    c.clear_events();
    let id = run(&mut c, &d, "proofread").unwrap()["run_id"].as_i64().unwrap();
    let f = c.event("AiFailed", 10);
    assert_eq!((f["run_id"].as_i64(), f["code"].as_str()), (Some(id), Some("ai_invalid_output")));
    let (state, _, error, output) = row(dir.path(), id);
    assert_eq!(state, "failed");
    assert!(error.unwrap().starts_with("ai_invalid_output"));
    assert!(output.unwrap().contains("<segment n=\"2\">"), "the raw answer is kept to understand the refusal");
    let v = c.call("ai_get_run", json!({"run_id": id})).unwrap();
    assert_eq!((v["error_code"].clone(), v["proposal"].clone()), (json!("ai_invalid_output"), Value::Null));
}

#[test]
fn a_draft_changed_during_the_run_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let _core = core(dir.path(), Some(json!({"proofread": "<segment n=\"1\">Bonjour.</segment>"})), "");
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "bonjour.\n");
    c.clear_events();
    let id = run(&mut c, &d, "proofread").unwrap()["run_id"].as_i64().unwrap();
    c.event("AiDone", 10);
    // The user typed meanwhile.
    let typed = format!("bonjour. Et merci !\n{}", &d["body"].as_str().unwrap()["bonjour.\n".len()..]);
    let r = c.call("update_draft", json!({"id": d["id"], "revision": d["revision"], "body": typed})).unwrap();
    let e = c.call("ai_apply", json!({"run_id": id, "revision": r["revision"]})).unwrap_err();
    assert_eq!(e["code"], 1012, "{e}");
    // The proposal is still there, to dismiss.
    c.call("ai_dismiss", json!({"run_id": id})).unwrap();
    assert_eq!(row(dir.path(), id).1.as_deref(), Some("rejected"));
    let draft = c.call("open_draft", json!({"id": d["id"]})).unwrap();
    assert!(draft["body"].as_str().unwrap().starts_with("bonjour. Et merci !"));
}

#[test]
fn a_subject_is_chosen_among_the_suggestions() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"subject-ideas": "{\"subjects\": [\"Jeudi 14h\", \"Confirmation pour jeudi\"]}"});
    let _core = core(dir.path(), Some(answers), "");
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "Je viens jeudi à 14h.\n");
    c.clear_events();
    let r = run(&mut c, &d, "subject-ideas").unwrap();
    assert_eq!(r["streamed"], false);
    let id = r["run_id"].as_i64().unwrap();
    c.event("AiDone", 10);
    let v = c.call("ai_get_run", json!({"run_id": id})).unwrap();
    assert_eq!(v["proposal"], json!({"kind": "subjects", "items": ["Jeudi 14h", "Confirmation pour jeudi"]}));
    assert_eq!(c.call("ai_apply", json!({"run_id": id, "revision": d["revision"]})).unwrap_err()["code"], -32602);
    let a = c.call("ai_apply", json!({"run_id": id, "revision": d["revision"], "choice": 1})).unwrap();
    assert_eq!(a["draft"]["subject"], "Confirmation pour jeudi");
    assert_eq!(a["draft"]["body"], d["body"], "the body is untouched");
}

/// A fake `claude` that announces `tools` and, when safe, hangs after the
/// probe: long enough to be cancelled.
fn fake_claude(dir: &Path, tools: &str) -> String {
    let script = format!(
        r#"#!/bin/sh
IFS= read -r line
echo '{{"type":"system","subtype":"init","tools":{tools},"mcp_servers":[]}}'
IFS= read -r line
echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"total_cost_usd":0}}'
IFS= read -r line
echo $$ > "{d}/claude.pid"
sleep 60
"#,
        d = dir.display()
    );
    let path = dir.join("claude");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("claude_path = \"{}\"\n", path.display())
}

#[test]
fn a_run_is_cancelled_and_its_process_killed() {
    let dir = tempfile::tempdir().unwrap();
    let ai = fake_claude(dir.path(), "[]");
    let _core = core(dir.path(), None, &ai);
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "Bonjour.\n");
    c.clear_events();
    let id = run(&mut c, &d, "proofread").unwrap()["run_id"].as_i64().unwrap();
    // Wait until the fake got the message and hangs.
    let pid_file = dir.path().join("claude.pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let pid: i32 = loop {
        if let Some(p) = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse().ok()) {
            break p;
        }
        assert!(std::time::Instant::now() < deadline, "the fake claude never got the message");
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    c.call("ai_cancel", json!({"run_id": id})).unwrap();
    let f = c.event("AiFailed", 10);
    assert_eq!((f["run_id"].as_i64(), f["code"].as_str()), (Some(id), Some("ai_cancelled")));
    assert_eq!(row(dir.path(), id).0, "cancelled");
    // The process is gone: `kill(pid, 0)` checks it exists.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(std::time::Instant::now() < deadline, "the claude process {pid} still runs");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Cancelled once.
    assert_eq!(c.call("ai_cancel", json!({"run_id": id})).unwrap_err()["code"], 1001);
}

#[test]
fn a_claude_binary_with_tools_never_gets_the_mail() {
    let dir = tempfile::tempdir().unwrap();
    let ai = fake_claude(dir.path(), "[\"Bash\",\"Write\"]");
    let _core = core(dir.path(), None, &ai);
    let mut c = Client::connect_as(dir.path(), "app");
    let d = reply(dir.path(), &mut c, "Bonjour.\n");
    c.clear_events();
    let id = run(&mut c, &d, "proofread").unwrap()["run_id"].as_i64().unwrap();
    let f = c.event("AiFailed", 10);
    assert_eq!((f["run_id"].as_i64(), f["code"].as_str()), (Some(id), Some("ai_provider_unsafe")));
    assert!(f["message"].as_str().unwrap().contains("Bash"));
    // The fake never read a third line: the mail was not written to it.
    assert!(!dir.path().join("claude.pid").exists());
}

// ---------------------------------------------------------------------------
// Prompts on received mail (the reading and list screens)
// ---------------------------------------------------------------------------

const CONVERSATION: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Visite
Message-ID: <v1@a.example>
Date: Tue, 23 Jun 2020 19:20:56 +0200

Tu peux passer jeudi a 14h pour la visite ?

From a@x Wed Jun 24 08:00:00 2020
From: Me <me@hey.example>
To: alice@a.example
Subject: Re: Visite
Message-ID: <v2@hey.example>
In-Reply-To: <v1@a.example>
References: <v1@a.example>
Date: Wed, 24 Jun 2020 08:00:00 +0200

Oui, je viendrai avec les cles.

> Tu peux passer jeudi a 14h pour la visite ?

From a@x Wed Jun 24 09:00:00 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Re: Visite
Message-ID: <v3@a.example>
In-Reply-To: <v2@hey.example>
References: <v1@a.example> <v2@hey.example>
Date: Wed, 24 Jun 2020 09:00:00 +0200

Parfait, pense a la piece d'identite.

From a@x Thu Jun 25 10:00:00 2020
From: Ma Banque <support@mabanque.example>
To: me@hey.example
Subject: Votre compte
Message-ID: <p1@mabanque.example>
Date: Thu, 25 Jun 2020 10:00:00 +0200
MIME-Version: 1.0
Content-Type: text/html; charset=utf-8

<p>Confirmez vos informations : <a href=\"http://evil.example/login\">www.mabanque.fr</a></p>
";

/// A core with the AI on and fake answers, the conversation and the
/// phishing mail imported; the ids of the conversation's thread and of
/// the phishing mail's thread.
fn mail_core(dir: &Path, answers: Value) -> (common::Core, Client, i64, i64) {
    let core = core(dir, Some(answers), "");
    let mut c = Client::connect_as(dir, "app");
    let mbox = dir.join("mail.mbox");
    std::fs::write(&mbox, CONVERSATION).unwrap();
    c.call("subscribe", json!({})).unwrap();
    c.call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"})).unwrap();
    c.event("ImportFinished", 30);
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let thread = |id: &str| -> i64 { db.query_row("SELECT thread_id FROM message WHERE message_id_hdr = ?1", [id], |r| r.get(0)).unwrap() };
    let (visit, bank) = (thread("v1@a.example"), thread("p1@mabanque.example"));
    assert_eq!(visit, thread("v3@a.example"), "one conversation");
    (core, c, visit, bank)
}

/// Runs `prompt` on a thread and waits for its proposal.
fn run_mail(c: &mut Client, prompt: &str, thread: i64, variant: Option<&str>) -> (i64, Value) {
    c.clear_events();
    let r = c.call("ai_run", json!({"prompt": prompt, "variant": variant, "thread_id": thread, "lang": "fr"})).unwrap();
    let id = r["run_id"].as_i64().unwrap();
    loop {
        let e = c.next_event();
        if e["params"]["run_id"].as_i64() != Some(id) {
            continue;
        }
        match e["method"].as_str() {
            Some("AiDone") => break,
            Some("AiFailed") => panic!("{e}"),
            _ => {}
        }
    }
    let v = c.call("ai_get_run", json!({"run_id": id})).unwrap();
    (id, v["proposal"].clone())
}

/// What the model was sent, from the history.
fn sent(dir: &Path, run_id: i64) -> String {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.query_row("SELECT input FROM ai_run WHERE id = ?1", [run_id], |r| r.get(0)).unwrap()
}

#[test]
fn a_conversation_is_summarized_in_the_apps_language() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"summarize-thread": "{\"summary\": \"Alice attend jeudi 14h.\", \"actions\": [{\"task\": \"Apporter la pièce d'identité\", \"owner\": \"you\", \"due\": \"jeudi\"}]}"});
    let (_core, mut c, visit, _) = mail_core(dir.path(), answers);
    let menu = c.call("ai_list_prompts", json!({"screen": "reading", "folder": "inbox", "lang": "fr"})).unwrap();
    let available: Vec<&str> = menu.as_array().unwrap().iter().filter(|p| p["available"] == true).map(|p| p["id"].as_str().unwrap()).collect();
    for id in ["summarize-thread", "quick-replies", "translate-message", "security-check", "dates"] {
        assert!(available.contains(&id), "{id} in {available:?}");
    }
    let (id, p) = run_mail(&mut c, "summarize-thread", visit, None);
    assert_eq!(p, json!({"kind": "summary", "summary": "Alice attend jeudi 14h.",
        "actions": [{"task": "Apporter la pièce d'identité", "owner": "you", "due": "jeudi"}]}));
    let input = sent(dir.path(), id);
    // The three mails, oldest first, mine marked; the quoted history of my
    // reply left out; the user's address; the app's language.
    let (a, b, z) = (input.find("Tu peux passer").unwrap(), input.find("je viendrai").unwrap(), input.find("Parfait").unwrap());
    assert!(a < b && b < z, "{input}");
    assert_eq!(input.matches("Tu peux passer").count(), 1, "the quote of the reply is left out:\n{input}");
    assert!(input.contains("Me <me@hey.example> (the user)"), "{input}");
    assert!(input.contains("<user>\nThe user writes from: me@hey.example"), "{input}");
    assert!(input.contains("Answer in French."), "{input}");
    // Dates at the mail's own offset: June is summer time (+0200 in Paris,
    // the zone the tests run in when TZ says so; any zone gives the same
    // hour as the mail's own Date header here).
    if std::env::var("TZ").as_deref() == Ok("Europe/Paris") {
        assert!(input.contains("Tue, 23 Jun 2020 19:20:56 +0200"), "{input}");
    }
    // Closing a summary is no decision on it.
    c.call("ai_dismiss", json!({"run_id": id})).unwrap();
    assert_eq!(row(dir.path(), id).1, None);
    // Nothing to apply.
    assert_eq!(c.call("ai_apply", json!({"run_id": id})).unwrap_err()["code"], 1001);
}

#[test]
fn a_mail_is_translated_with_the_variants_language() {
    let dir = tempfile::tempdir().unwrap();
    let (_core, mut c, visit, _) = mail_core(dir.path(), json!({"translate-message": "Perfect, remember your ID."}));
    let (id, p) = run_mail(&mut c, "translate-message", visit, Some("e"));
    assert_eq!(p, json!({"kind": "text", "text": "Perfect, remember your ID."}));
    let input = sent(dir.path(), id);
    // The latest mail received, not mine, and the variant's language only.
    assert!(input.contains("<email>\nParfait, pense a la piece d'identite."), "{input}");
    assert!(!input.contains("je viendrai"), "{input}");
    assert!(input.contains("Translate it into English.") && !input.contains("Answer in French"), "{input}");
}

#[test]
fn a_quick_reply_is_chosen_and_given_back() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"quick-replies": "{\"choices\": [\"C'est noté !\", \"Je ne pourrai pas.\", \"On peut décaler ?\"]}"});
    let (_core, mut c, visit, _) = mail_core(dir.path(), answers);
    let (id, p) = run_mail(&mut c, "quick-replies", visit, None);
    assert_eq!(p["kind"], "choices");
    assert_eq!(p["then"], "reply");
    assert_eq!(c.call("ai_apply", json!({"run_id": id})).unwrap_err()["code"], -32602, "a choice is needed");
    let a = c.call("ai_apply", json!({"run_id": id, "choice": 2})).unwrap();
    assert_eq!((a["text"].clone(), a["draft"].clone()), (json!("On peut décaler ?"), Value::Null));
    assert_eq!(row(dir.path(), id).1.as_deref(), Some("accepted"));
}

#[test]
fn a_low_verdict_is_raised_by_a_link_that_lies() {
    let dir = tempfile::tempdir().unwrap();
    let answers = json!({"security-check": "{\"risk\": \"low\", \"reasons\": [\"Looks like a normal notice.\"]}"});
    let (_core, mut c, _, bank) = mail_core(dir.path(), answers);
    let (id, p) = run_mail(&mut c, "security-check", bank, None);
    assert_eq!(p["kind"], "verdict");
    assert_eq!(p["risk"], "medium", "{p}");
    assert_eq!(p["raised"], true);
    assert_eq!(p["reasons"][0], "a link shows www.mabanque.fr but opens evil.example");
    // The model saw it too, with the headers and the sender's history.
    let input = sent(dir.path(), id);
    assert!(input.contains("- a link shows www.mabanque.fr but opens evil.example"), "{input}");
    assert!(input.contains("<sender>\nAddress: support@mabanque.example"), "{input}");
    assert!(input.contains("From: Ma Banque <support@mabanque.example>"), "{input}");
}

#[test]
fn a_mail_prompt_needs_its_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let (_core, mut c, visit, _) = mail_core(dir.path(), json!({}));
    let e = c.call("ai_run", json!({"prompt": "dates", "lang": "fr"})).unwrap_err();
    assert_eq!(e["code"], -32602);
    let e = c.call("ai_run", json!({"prompt": "dates", "thread_id": visit, "message_id": 999999, "lang": "fr"})).unwrap_err();
    assert_eq!(e["code"], 1001);
    // Quarantine sorting is not wired yet.
    let e = c.call("ai_run", json!({"prompt": "triage-quarantine", "thread_id": visit, "lang": "fr"})).unwrap_err();
    assert_eq!(e["data"]["code"], "ai_bad_input");
}
