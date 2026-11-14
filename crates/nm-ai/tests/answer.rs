//! Checking the model's answers (`specifications/ia.md`, section 6; tests
//! required in section 15, "Validation des sorties" and "Plancher").

use nm_ai::answer::{self, ActionItem, Answer, Finding, Risk, Sent, Suggestion, Verdict};
use nm_ai::prompt::Output;
use nm_ai::{AiError, AiOutcome, Usage};
use nm_core::FolderRole;
use serde_json::{Value, json};

fn text(t: &str) -> AiOutcome {
    AiOutcome {
        text: t.into(),
        structured: None,
        usage: Usage::default(),
        cost_usd: None,
        billed: false,
        duration_ms: None,
    }
}

fn structured(v: Value) -> AiOutcome {
    AiOutcome {
        structured: Some(v.clone()),
        ..text(&v.to_string())
    }
}

fn read(output: Output, o: &AiOutcome, sent: &Sent) -> Result<Answer, AiError> {
    answer::read(output, o, sent)
}

/// The reason of an `ai_invalid_output`.
fn refused(r: Result<Answer, AiError>) -> String {
    match r {
        Err(AiError::InvalidOutput(m)) => m,
        other => panic!("expected ai_invalid_output, got {other:?}"),
    }
}

const NONE: Sent = Sent {
    segments: 0,
    threads: &[],
    targets: &[],
};

#[test]
fn segments_are_read_in_order() {
    let sent = Sent {
        segments: 2,
        ..NONE
    };
    let a = read(
        Output::Replace,
        &text("<segment n=\"1\">\nBonjour Paul,\n</segment>\n<segment n=\"2\">À jeudi</segment>"),
        &sent,
    );
    assert_eq!(
        a.unwrap(),
        Answer::Segments(vec!["Bonjour Paul,".into(), "À jeudi".into()])
    );
}

#[test]
fn the_recorded_proofreading_reads() {
    // The answer of phase 0 (`tests/fixtures/claude-code/streamed-text.jsonl`).
    let line = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/claude-code/streamed-text.jsonl"
    ))
    .unwrap()
    .lines()
    .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    .rfind(|d| d["type"] == "result" && d["subtype"] == "success")
    .unwrap();
    let a = read(
        Output::Replace,
        &text(line["result"].as_str().unwrap()),
        &Sent {
            segments: 1,
            ..NONE
        },
    )
    .unwrap();
    let Answer::Segments(s) = a else { panic!() };
    assert!(s[0].starts_with("Bonjour Paul,\n\nJe te confirme"));
}

#[test]
fn bad_segments_are_refused() {
    let one = Sent {
        segments: 1,
        ..NONE
    };
    let two = Sent {
        segments: 2,
        ..NONE
    };
    let cases = [
        (
            "<segment n=\"1\">a</segment>",
            two,
            "1 segments in the answer, 2 sent",
        ),
        (
            "<segment n=\"1\">a</segment><segment n=\"2\">b</segment>",
            one,
            "2 segments in the answer, 1 sent",
        ),
        (
            "<segment n=\"2\">b</segment><segment n=\"1\">a</segment>",
            two,
            "out of order",
        ),
        (
            "Here is the text:\n<segment n=\"1\">a</segment>",
            one,
            "text outside",
        ),
        ("<segment n=\"1\">a</segment> Done!", one, "text outside"),
        ("<segment n=\"1\">a", one, "not closed"),
        (
            "<segment n=\"1\">a <segment n=\"2\">b</segment>",
            one,
            "contains another",
        ),
        ("Bonjour Paul", one, "text outside"),
        ("", one, "0 segments"),
        // A self-correction is read only between complete sets.
        (
            "<segment n=\"1\">a</segment>\nWait.\n<segment n=\"1\">b</segment> Done!",
            one,
            "text outside",
        ),
        (
            "<segment n=\"1\">a</segment><segment n=\"2\">b</segment>\nWait.\n<segment n=\"1\">c</segment>",
            two,
            "1 segments in the answer, 2 sent",
        ),
        (
            "<segment n=\"1\">a</segment>\nWait.\n<segment n=\"2\">b</segment>",
            one,
            "text outside",
        ),
    ];
    for (answer, sent, why) in cases {
        let m = refused(read(Output::Replace, &text(answer), &sent));
        assert!(m.contains(why), "{answer:?}: {m}");
    }
}

#[test]
fn a_model_correcting_itself_gives_its_last_set_of_segments() {
    // A real answer (proofreading, November 2026): every segment, a comment,
    // then every segment again, as the model meant them.
    let sent = Sent {
        segments: 2,
        ..NONE
    };
    let answer = "<segment n=\"1\">Je suis en body.</segment>\n<segment n=\"2\">Les sauts devant-derrière.</segment>\n\n\
                  Wait — I must correct that: I changed \"babygros\", which alters the writer's wording. \
                  Here is the proper version:\n\n\
                  <segment n=\"1\">Je suis en babygros.</segment>\n<segment n=\"2\">Les sauts devant derrière.</segment>";
    assert_eq!(
        read(Output::Replace, &text(answer), &sent).unwrap(),
        Answer::Segments(vec![
            "Je suis en babygros.".into(),
            "Les sauts devant derrière.".into()
        ])
    );
    // Two sets with nothing between them: the last one too.
    let sent = Sent {
        segments: 1,
        ..NONE
    };
    assert_eq!(
        read(
            Output::Replace,
            &text("<segment n=\"1\">a</segment><segment n=\"1\">b</segment>"),
            &sent
        )
        .unwrap(),
        Answer::Segments(vec!["b".into()])
    );
}

#[test]
fn a_fenced_answer_is_unfenced() {
    let sent = Sent {
        segments: 1,
        ..NONE
    };
    let a = read(
        Output::Replace,
        &text("```xml\n<segment n=\"1\">a</segment>\n```"),
        &sent,
    )
    .unwrap();
    assert_eq!(a, Answer::Segments(vec!["a".into()]));
    assert_eq!(
        read(Output::Panel, &text("```\nplain\n```"), &NONE).unwrap(),
        Answer::Text("plain".into())
    );
    let s = read(
        Output::Subjects,
        &text("```json\n{\"subjects\": [\"A\"]}\n```"),
        &NONE,
    )
    .unwrap();
    assert_eq!(s, Answer::Subjects(vec!["A".into()]));
    // A fence inside the text is the text's.
    assert_eq!(answer::unfence("Use ``` for code."), "Use ``` for code.");
}

#[test]
fn text_answers_must_not_be_empty() {
    for o in [Output::Insert, Output::Reply, Output::Panel] {
        assert!(refused(read(o, &text("  \n"), &NONE)).contains("empty"));
        assert_eq!(
            read(o, &text(" Hi \n"), &NONE).unwrap(),
            Answer::Text("Hi".into())
        );
    }
}

#[test]
fn lists_of_texts() {
    let ok = read(
        Output::Choices,
        &structured(json!({"choices": [" Yes ", "No"]})),
        &NONE,
    )
    .unwrap();
    assert_eq!(ok, Answer::Choices(vec!["Yes".into(), "No".into()]));
    let too_many = json!({"subjects": ["a", "b", "c", "d", "e", "f"]});
    let bad = [
        (json!({"subjects": []}), "0 items"),
        (too_many, "6 items"),
        (json!({"subjects": ["a", "A"]}), "same item twice"),
        (json!({"subjects": ["a", " "]}), "empty"),
        (json!({"subjects": ["a", 3]}), "not a string"),
        (json!({"subject_lines": ["a"]}), "unexpected key"),
        (json!({"subjects": ["a"], "note": "x"}), "unexpected key"),
        (json!({"subjects": ["x".repeat(2_001)]}), "longer than"),
        (json!(["a"]), "not a JSON object"),
    ];
    for (v, why) in bad {
        let m = refused(read(Output::Subjects, &structured(v.clone()), &NONE));
        assert!(m.contains(why), "{v}: {m}");
    }
    assert!(
        refused(read(
            Output::Subjects,
            &text("Sure! Here are three subjects."),
            &NONE
        ))
        .contains("not JSON")
    );
}

#[test]
fn the_recorded_subjects_read() {
    let v = json!({"subjects": ["Confirmation de notre rendez-vous chez le notaire, jeudi 14h",
        "Rendez-vous notaire jeudi 14h : signature à confirmer",
        "Signature chez le notaire jeudi 14h, pense à ta pièce d'identité"]});
    let Answer::Subjects(s) = read(Output::Subjects, &structured(v), &NONE).unwrap() else {
        panic!()
    };
    assert_eq!(s.len(), 3);
}

#[test]
fn summaries() {
    let v = json!({"summary": "Paul confirms Thursday.", "actions": [
        {"task": "Bring the ID", "owner": "you", "due": "2026-11-19"},
        {"task": "Book the room", "owner": "", "due": ""}]});
    assert_eq!(
        read(Output::Summary, &structured(v), &NONE).unwrap(),
        Answer::Summary {
            summary: "Paul confirms Thursday.".into(),
            actions: vec![
                ActionItem {
                    task: "Bring the ID".into(),
                    owner: "you".into(),
                    due: "2026-11-19".into()
                },
                ActionItem {
                    task: "Book the room".into(),
                    owner: "".into(),
                    due: "".into()
                },
            ],
        }
    );
    let nine: Vec<Value> = (0..9)
        .map(|i| json!({"task": format!("t{i}"), "owner": "", "due": ""}))
        .collect();
    let bad = [
        (json!({"summary": "", "actions": []}), "summary is empty"),
        (json!({"summary": "s"}), "actions is missing"),
        (json!({"summary": "s", "actions": nine}), "9 action items"),
        (
            json!({"summary": "s", "actions": [{"task": "", "owner": "", "due": ""}]}),
            "task is empty",
        ),
        (
            json!({"summary": "s", "actions": [{"task": "t", "owner": null, "due": ""}]}),
            "owner is missing",
        ),
        (
            json!({"summary": "s", "actions": [{"task": "t", "owner": "", "due": "", "priority": 1}]}),
            "unexpected key",
        ),
        (
            json!({"summary": "s", "actions": ["Call Sam"]}),
            "not an object",
        ),
    ];
    for (v, why) in bad {
        let m = refused(read(Output::Summary, &structured(v.clone()), &NONE));
        assert!(m.contains(why), "{v}: {m}");
    }
}

#[test]
fn verdicts() {
    let v = read(
        Output::Verdict,
        &structured(json!({"risk": "high", "reasons": ["Asks for a password"]})),
        &NONE,
    )
    .unwrap();
    assert_eq!(
        v,
        Answer::Verdict(Verdict {
            risk: Risk::High,
            reasons: vec!["Asks for a password".into()],
            raised: false
        })
    );
    let bad = [
        (
            json!({"risk": "suspicious", "reasons": ["x"]}),
            "expected low, medium or high",
        ),
        (json!({"risk": "HIGH", "reasons": ["x"]}), "expected low"),
        (json!({"risk": "low", "reasons": []}), "0 reasons"),
        (
            json!({"risk": "low", "reasons": "looks fine"}),
            "not a list",
        ),
        (
            json!({"risk": "low", "reasons": ["x".repeat(301)]}),
            "longer than",
        ),
    ];
    for (v, why) in bad {
        let m = refused(read(Output::Verdict, &structured(v.clone()), &NONE));
        assert!(m.contains(why), "{v}: {m}");
    }
}

#[test]
fn targets_are_checked_against_what_was_sent() {
    let sent = Sent {
        segments: 0,
        threads: &[10, 11, 12],
        targets: &[FolderRole::Inbox, FolderRole::Newsletter, FolderRole::Junk],
    };
    let v = json!({"items": [
        {"thread": 10, "target": "newsletter", "reason": "A store's weekly offers."},
        {"thread": 12, "target": "inbox", "reason": "A person asks a question."}]});
    // Conversation 11 was left out: no suggestion for it, the rest stands.
    assert_eq!(
        read(Output::Targets, &structured(v), &sent).unwrap(),
        Answer::Targets(vec![
            Suggestion {
                thread: 10,
                target: FolderRole::Newsletter,
                reason: "A store's weekly offers.".into()
            },
            Suggestion {
                thread: 12,
                target: FolderRole::Inbox,
                reason: "A person asks a question.".into()
            },
        ])
    );
    let item = |thread: Value, target: &str| json!({"items": [{"thread": thread, "target": target, "reason": "r"}]});
    let bad = [
        (item(json!(99), "inbox"), "conversation 99 was not sent"),
        (item(json!(10), "banned"), "\"banned\" was not offered"),
        (item(json!(10), "spam"), "\"spam\" was not offered"),
        (item(json!("10"), "inbox"), "not an integer"),
        (
            json!({"items": [{"thread": 10, "target": "inbox", "reason": "r"},
                          {"thread": 10, "target": "junk", "reason": "r"}]}),
            "two suggestions",
        ),
        (
            json!({"items": [{"thread": 10, "target": "inbox", "reason": " "}]}),
            "reason is empty",
        ),
        (
            json!({"items": [{"thread": 10, "target": "inbox", "reason": "r", "confidence": 0.9}]}),
            "unexpected key",
        ),
    ];
    for (v, why) in bad {
        let m = refused(read(Output::Targets, &structured(v.clone()), &sent));
        assert!(m.contains(why), "{v}: {m}");
    }
}

#[test]
fn a_strong_finding_raises_a_low_verdict() {
    let low = Verdict {
        risk: Risk::Low,
        reasons: vec!["Looks like a normal newsletter.".into()],
        raised: false,
    };
    let weak = Finding {
        text: "Return-Path of another domain".into(),
        strong: false,
    };
    let strong = Finding {
        text: "A link shows mybank.com but opens evil.example".into(),
        strong: true,
    };

    // No strong sign: unchanged.
    assert_eq!(
        answer::apply_floor(low.clone(), std::slice::from_ref(&weak)),
        low
    );

    let v = answer::apply_floor(low.clone(), &[weak, strong.clone()]);
    assert_eq!(v.risk, Risk::Medium);
    assert!(v.raised);
    assert_eq!(
        v.reasons,
        [
            strong.text.clone(),
            "Looks like a normal newsletter.".into()
        ]
    );

    // Higher verdicts stay as the model gave them.
    let high = Verdict {
        risk: Risk::High,
        reasons: vec!["x".into()],
        raised: false,
    };
    assert_eq!(
        answer::apply_floor(high.clone(), std::slice::from_ref(&strong)),
        high
    );

    // The finding is not repeated, and reasons stay at most 5.
    let many = Verdict {
        risk: Risk::Low,
        reasons: (0..5)
            .map(|i| format!("r{i}"))
            .chain([strong.text.clone()])
            .collect(),
        raised: false,
    };
    let v = answer::apply_floor(many, std::slice::from_ref(&strong));
    assert_eq!(v.reasons.len(), 5);
    assert_eq!(v.reasons.iter().filter(|r| **r == strong.text).count(), 1);
    assert_eq!(v.reasons[0], strong.text);
}

#[test]
fn a_structured_answer_given_as_text_is_parsed() {
    // A provider without structured output (or the fake) gives the JSON as text.
    let a = read(Output::Choices, &text("{\"choices\": [\"OK\"]}"), &NONE).unwrap();
    assert_eq!(a, Answer::Choices(vec!["OK".into()]));
}
