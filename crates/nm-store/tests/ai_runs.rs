//! The history of AI runs (`specifications/ia.md`, section 9; tests
//! required in section 15, "Historique").

use nm_core::Source;
use nm_store::Store;
use nm_store::ai_runs::{ABANDONED, Decision, NewRun, NewSuggestion, RunDone, RunEnd};

fn new_run<'a>(hash: &'a [u8; 32]) -> NewRun<'a> {
    NewRun {
        created_by: Source::App,
        prompt_id: "proofread",
        variant: None,
        prompt_source: "builtin",
        prompt_hash: hash,
        provider: "claude-code",
        model: "claude-sonnet-5-5",
        scope: "draft",
        target: r#"{"draft": 7}"#,
        lang: "fr",
        input: "TASK=proofread\n…\n<segment n=\"1\">teh text</segment>",
    }
}

fn done() -> RunDone<'static> {
    RunDone {
        output: "<segment n=\"1\">the text</segment>",
        input_tokens: Some(640),
        output_tokens: Some(12),
        cost_usd: Some(0.0041),
        billed: false,
        duration_ms: Some(2_240),
    }
}

#[test]
fn a_run_goes_from_running_to_done_to_decided() {
    let mut s = Store::open_in_memory().unwrap();
    let hash = [7u8; 32];
    let id = s.ai_run_start(&new_run(&hash)).unwrap();
    let r = s.ai_run(id).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.created_by.as_str(), r.prompt_hash.as_slice()), ("running", "app", &hash[..]));
    assert!(r.input.unwrap().contains("teh text"));

    s.ai_run_done(id, &done()).unwrap();
    let r = s.ai_run(id).unwrap().unwrap();
    assert_eq!(r.state, "done");
    assert_eq!((r.input_tokens, r.output_tokens, r.duration_ms), (Some(640), Some(12), Some(2_240)));
    assert_eq!(r.cost_usd, Some(0.0041));
    assert!(!r.billed, "a subscription call is not billed");
    assert_eq!(r.decision, None);

    s.ai_run_decide(id, Decision::Accepted).unwrap();
    assert_eq!(s.ai_run(id).unwrap().unwrap().decision.as_deref(), Some("accepted"));
}

#[test]
fn a_run_ends_once_and_is_decided_once() {
    let mut s = Store::open_in_memory().unwrap();
    let hash = [0u8; 32];
    let id = s.ai_run_start(&new_run(&hash)).unwrap();
    // Not done yet: no decision.
    assert!(s.ai_run_decide(id, Decision::Accepted).is_err());
    s.ai_run_done(id, &done()).unwrap();
    // A second answer for the same run is refused, not written over the first.
    assert!(s.ai_run_done(id, &RunDone { output: "other", ..done() }).is_err());
    assert!(s.ai_run_end(id, RunEnd::Failed, "late", None).is_err());
    assert_eq!(s.ai_run(id).unwrap().unwrap().output.as_deref(), Some(done().output));
    s.ai_run_decide(id, Decision::Rejected).unwrap();
    assert!(s.ai_run_decide(id, Decision::Accepted).is_err());
    assert_eq!(s.ai_run(id).unwrap().unwrap().decision.as_deref(), Some("rejected"));
    // An unknown run is "not found", not "not running".
    let e = s.ai_run_done(999, &done()).unwrap_err().to_string();
    assert!(e.contains("introuvable"), "{e}");
}

#[test]
fn failed_and_cancelled_runs_keep_the_raw_answer() {
    let mut s = Store::open_in_memory().unwrap();
    let hash = [0u8; 32];
    let a = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_run_end(a, RunEnd::Failed, "ai_invalid_output: 2 segments in the answer, 1 sent", Some("<segment…")).unwrap();
    let r = s.ai_run(a).unwrap().unwrap();
    assert_eq!(r.state, "failed");
    assert!(r.error.unwrap().starts_with("ai_invalid_output"));
    assert_eq!(r.output.as_deref(), Some("<segment…"));
    // A failed run cannot be decided.
    assert!(s.ai_run_decide(a, Decision::Accepted).is_err());

    let b = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_run_end(b, RunEnd::Cancelled, "cancelled", None).unwrap();
    assert_eq!(s.ai_run(b).unwrap().unwrap().state, "cancelled");
}

#[test]
fn runs_left_running_by_a_stop_fail_at_start() {
    let mut s = Store::open_in_memory().unwrap();
    let hash = [0u8; 32];
    let left = s.ai_run_start(&new_run(&hash)).unwrap();
    let finished = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_run_done(finished, &done()).unwrap();
    assert_eq!(s.ai_runs_abandoned().unwrap(), 1);
    let r = s.ai_run(left).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.error.as_deref()), ("failed", Some(ABANDONED)));
    assert_eq!(s.ai_run(finished).unwrap().unwrap().state, "done");
    assert_eq!(s.ai_runs_abandoned().unwrap(), 0);
}

#[test]
fn the_purge_forgets_the_texts_and_keeps_the_rest() {
    let mut s = Store::open_in_memory().unwrap();
    let hash = [3u8; 32];
    let old = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_run_done(old, &done()).unwrap();
    s.ai_run_decide(old, Decision::Accepted).unwrap();
    let running = s.ai_run_start(&new_run(&hash)).unwrap();
    s.conn().execute("UPDATE ai_run SET date_created = 1000", []).unwrap();
    let recent = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_run_done(recent, &done()).unwrap();

    // Before 2000: the old one only; the running one waits for its end.
    assert_eq!(s.ai_runs_purge(2_000).unwrap(), 1);
    let r = s.ai_run(old).unwrap().unwrap();
    assert_eq!((r.input, r.output), (None, None));
    assert!(r.date_purged.is_some());
    assert_eq!((r.prompt_id.as_str(), r.decision.as_deref(), r.cost_usd), ("proofread", Some("accepted"), Some(0.0041)));
    assert!(s.ai_run(running).unwrap().unwrap().input.is_some());
    assert!(s.ai_run(recent).unwrap().unwrap().output.is_some());
    // A purged row is not purged again.
    assert_eq!(s.ai_runs_purge(2_000).unwrap(), 0);
}

#[test]
fn suggestions_follow_their_run_and_their_conversation() {
    let mut s = Store::open_in_memory().unwrap();
    s.conn()
        .execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('a', 'imap', 'core', 'core')", [])
        .unwrap();
    for id in [1, 2] {
        s.conn()
            .execute(
                "INSERT INTO thread (id, account_id) VALUES (?1, 1)",
                [id],
            )
            .unwrap();
    }
    let hash = [0u8; 32];
    let run = s.ai_run_start(&new_run(&hash)).unwrap();
    s.ai_suggestions_add(
        run,
        &[
            NewSuggestion { thread_id: 1, target: "newsletter", reason: "A store's offers." },
            NewSuggestion { thread_id: 2, target: "inbox", reason: "A person asks." },
        ],
    )
    .unwrap();
    let count = |s: &Store| -> i64 { s.conn().query_row("SELECT count(*) FROM ai_suggestion", [], |r| r.get(0)).unwrap() };
    assert_eq!(count(&s), 2);
    // A suggestion for a conversation that no longer exists goes with it.
    s.conn().execute("DELETE FROM thread WHERE id = 1", []).unwrap();
    assert_eq!(count(&s), 1);
    // A batch is written whole or not at all: a duplicate refuses it all.
    let dup = [
        NewSuggestion { thread_id: 2, target: "junk", reason: "x" },
        NewSuggestion { thread_id: 2, target: "junk", reason: "x" },
    ];
    let run2 = s.ai_run_start(&new_run(&hash)).unwrap();
    assert!(s.ai_suggestions_add(run2, &dup).is_err());
    assert_eq!(count(&s), 1);
}
