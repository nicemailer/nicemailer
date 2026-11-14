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
    // The text comes in fragments, then the end.
    let delta = c.event("AiDelta", 10);
    assert_eq!(delta["run_id"], id);
    assert!(delta.get("seq").is_none(), "fragments are transient");
    assert_eq!(c.event("AiDone", 10)["run_id"], id);

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
