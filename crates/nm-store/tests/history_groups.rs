//! `list_history_groups`: the audit log as the History screen shows it
//! (`tracabilite.md`, section 6). The lines are written by hand: the rules
//! are about what the log holds, whichever command wrote it.

use nm_api::HistoryGroup;
use nm_store::Store;
use nm_store::library::HistoryGroupQuery;
use rusqlite::params;

const READ: &str = r#"{"before":{"is_seen":0},"after":{"is_seen":1}}"#;
const UNREAD: &str = r#"{"before":{"is_seen":1},"after":{"is_seen":0}}"#;
const FLAG: &str = r#"{"before":{"is_flagged":0},"after":{"is_flagged":1}}"#;

fn line(s: &Store, date: i64, source: &str, entity: &str, id: i64, action: &str, detail: &str) {
    s.conn()
        .execute(
            "INSERT INTO audit_log (date_created, created_by, entity, entity_id, action, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![date, source, entity, id, action, detail],
        )
        .unwrap();
}

fn mail(s: &Store, date: i64, id: i64, action: &str, detail: &str) {
    line(s, date, "app", "message", id, action, detail);
}

fn groups(s: &Store, query: &HistoryGroupQuery) -> Vec<HistoryGroup> {
    s.list_history_groups(None, 100, query).unwrap().0
}

fn everything() -> HistoryGroupQuery {
    HistoryGroupQuery { include_read: true, ..Default::default() }
}

/// (kind, count), newest first.
fn shape(list: &[HistoryGroup]) -> Vec<(String, i64)> {
    list.iter().map(|g| (g.kind.clone(), g.count)).collect()
}

#[test]
fn one_action_on_several_mails_is_one_group() {
    let s = Store::open_in_memory().unwrap();
    for id in 1..=5 {
        mail(&s, 1_000, id, "set_flags", READ);
    }
    let list = groups(&s, &everything());
    assert_eq!(shape(&list), [("read".to_string(), 5)]);
    assert_eq!((list[0].entity_id, list[0].date), (5, 1_000), "the group shows its newest line");
    assert_eq!(list[0].last_id - list[0].first_id, 4);
}

#[test]
fn another_source_action_or_target_starts_another_group() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", READ);
    line(&s, 1_000, "cli", "message", 2, "set_flags", READ);
    mail(&s, 1_000, 3, "set_flags", UNREAD);
    mail(&s, 1_000, 4, "set_tags", r#"{"add":"a","tag":"a"}"#);
    mail(&s, 1_000, 5, "set_tags", r#"{"add":"b","tag":"b"}"#);
    assert_eq!(groups(&s, &everything()).len(), 5);
}

#[test]
fn the_same_action_later_is_another_group() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    mail(&s, 1_002, 2, "set_flags", FLAG);
    mail(&s, 1_003, 3, "set_flags", FLAG);
    mail(&s, 1_010, 4, "set_flags", FLAG);
    assert_eq!(shape(&groups(&s, &everything())), [("flagged".to_string(), 1), ("flagged".to_string(), 3)]);
}

#[test]
fn a_line_between_two_runs_keeps_them_apart() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    mail(&s, 1_000, 2, "set_flags", READ);
    mail(&s, 1_000, 3, "set_flags", FLAG);
    let expected = [("flagged".to_string(), 1), ("read".to_string(), 1), ("flagged".to_string(), 1)];
    assert_eq!(shape(&groups(&s, &everything())), expected);
}

#[test]
fn the_mails_marked_as_read_are_given_only_when_asked_for() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    for id in 2..=4 {
        mail(&s, 1_001, id, "set_flags", READ);
    }
    mail(&s, 1_002, 5, "set_flags", UNREAD);
    let hidden = groups(&s, &HistoryGroupQuery::default());
    assert_eq!(shape(&hidden), [("unread".to_string(), 1), ("flagged".to_string(), 1)]);
    assert_eq!(groups(&s, &everything()).len(), 3);
}

#[test]
fn the_detail_of_a_group_gives_each_of_its_lines() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 900, 9, "set_flags", FLAG);
    for id in 1..=3 {
        mail(&s, 1_000, id, "set_flags", READ);
    }
    mail(&s, 1_100, 8, "set_flags", FLAG);
    let group = groups(&s, &everything()).into_iter().find(|g| g.count == 3).unwrap();
    let query = HistoryGroupQuery { range: Some((group.first_id, group.last_id)), ..Default::default() };
    let lines = groups(&s, &query);
    assert_eq!(lines.iter().map(|g| g.entity_id).collect::<Vec<_>>(), [3, 2, 1]);
    assert!(lines.iter().all(|g| g.count == 1 && g.kind == "read"));
}

#[test]
fn every_group_comes_once_across_the_pages() {
    let s = Store::open_in_memory().unwrap();
    for n in 0..30_i64 {
        // Runs of 1 to 3 lines, each its own action.
        for id in 0..(n % 3 + 1) {
            mail(&s, 1_000 + n * 10, n * 10 + id, "set_flags", FLAG);
        }
        // Hidden lines between them: a page must go past them.
        mail(&s, 1_000 + n * 10 + 5, n * 10 + 9, "set_flags", READ);
    }
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (items, next) = s.list_history_groups(cursor.as_deref(), 4, &HistoryGroupQuery::default()).unwrap();
        assert!(items.len() <= 4);
        seen.extend(items);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len(), 30);
    assert!(seen.windows(2).all(|w| w[0].date > w[1].date), "newest first, no group twice");
    assert_eq!(seen.iter().map(|g| g.count).sum::<i64>(), (0..30).map(|n| n % 3 + 1).sum::<i64>());
}

#[test]
fn each_action_says_what_was_done_with_what() {
    let s = Store::open_in_memory().unwrap();
    let cases = [
        ("message", "set_flags", READ, "read", None),
        ("message", "set_flags", UNREAD, "unread", None),
        ("message", "set_flags", FLAG, "flagged", None),
        ("message", "set_flags", r#"{"before":{"is_flagged":1},"after":{"is_flagged":0}}"#, "unflagged", None),
        ("message", "set_tags", r#"{"add":"work","tag":"work"}"#, "tag_added", Some("work")),
        ("message", "set_tags", r#"{"remove":"work","tag":"work"}"#, "tag_removed", Some("work")),
        ("message", "screen", r#"{"folder":"quarantine"}"#, "screen", Some("quarantine")),
        ("message", "move", r#"{"after":{"folder_id":999}}"#, "move", Some("?")),
        ("address", "screening", r#"{"before":null,"after":"newsletter"}"#, "decision", Some("newsletter")),
        ("address", "screening", r#"{"before":"inbox","after":null}"#, "decision_removed", None),
        ("contact_group", "add_member", r#"{"addr":"a@x.example"}"#, "add_member", Some("a@x.example")),
        ("account", "import_mbox", r#"{"imported":12}"#, "import_mbox", Some("12")),
        ("account", "reorder", "{}", "reorder", None),
    ];
    for (i, (entity, action, detail, ..)) in cases.iter().enumerate() {
        line(&s, 1_000 + i as i64 * 10, "app", entity, i as i64, action, detail);
    }
    let list = groups(&s, &everything());
    for (g, (_, _, _, kind, value)) in list.iter().rev().zip(cases.iter()) {
        assert_eq!((g.kind.as_str(), g.value.as_deref()), (*kind, *value));
    }
}

#[test]
fn an_item_deleted_since_has_no_title_unless_its_line_kept_its_name() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 123_456, "move", r#"{"after":{"folder_id":1}}"#);
    line(&s, 1_010, "app", "tag", 77, "delete", r#"{"before":{"name":"old"}}"#);
    let list = groups(&s, &everything());
    assert_eq!(list[0].title.as_deref(), Some("old"));
    assert_eq!(list[1].title, None);
}

/// The worst case of the screen: a long run of hidden lines before the first
/// row to show. Timed by hand with `--nocapture`.
#[test]
fn a_long_run_of_hidden_lines_is_read_through() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    s.conn().execute_batch("BEGIN").unwrap();
    for id in 0..50_000_i64 {
        s.conn().execute(
            "INSERT INTO audit_log (date_created, created_by, entity, entity_id, action, detail)
             VALUES (2000, 'app', 'message', ?1, 'set_flags', ?2)",
            params![id, READ],
        )
        .unwrap();
    }
    s.conn().execute_batch("COMMIT").unwrap();
    let start = std::time::Instant::now();
    let list = groups(&s, &HistoryGroupQuery::default());
    eprintln!("50,000 hidden lines read in {:?}", start.elapsed());
    assert_eq!(shape(&list), [("flagged".to_string(), 1)]);
}

fn of(sources: &[&str]) -> HistoryGroupQuery {
    HistoryGroupQuery { include_read: true, sources: sources.iter().map(|s| s.to_string()).collect(), ..Default::default() }
}

#[test]
fn a_filter_on_who_keeps_only_their_actions() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    line(&s, 1_010, "core", "message", 2, "screen", r#"{"folder":"inbox"}"#);
    line(&s, 1_020, "cli", "message", 3, "set_flags", FLAG);
    line(&s, 1_030, "tui", "message", 4, "set_flags", UNREAD);
    line(&s, 1_040, "mcp", "message", 5, "set_flags", FLAG);
    let who = |q: &HistoryGroupQuery| groups(&s, q).into_iter().map(|g| g.source).collect::<Vec<_>>();
    assert_eq!(who(&of(&["app"])), ["app"]);
    assert_eq!(who(&of(&["core"])), ["core"]);
    assert_eq!(who(&of(&["cli", "tui"])), ["tui", "cli"], "several sources at once");
    assert_eq!(who(&of(&["mcp"])), ["mcp"]);
    assert_eq!(who(&of(&[])).len(), 5, "no filter: everyone");
}

#[test]
fn a_filter_on_who_does_not_join_two_runs_around_another_source() {
    let s = Store::open_in_memory().unwrap();
    mail(&s, 1_000, 1, "set_flags", FLAG);
    line(&s, 1_000, "mcp", "message", 2, "set_flags", FLAG);
    mail(&s, 1_000, 3, "set_flags", FLAG);
    // Unfiltered, three rows; the agent's line kept the two of the app apart,
    // and hiding it must not make them one action.
    assert_eq!(shape(&groups(&s, &of(&["app"]))), [("flagged".to_string(), 1), ("flagged".to_string(), 1)]);
}

#[test]
fn a_filter_on_who_still_hides_read_mails_and_pages() {
    let s = Store::open_in_memory().unwrap();
    for n in 0..10_i64 {
        line(&s, 1_000 + n * 10, "mcp", "message", n, "set_flags", FLAG);
        mail(&s, 1_000 + n * 10 + 5, 100 + n, "set_flags", FLAG);
    }
    line(&s, 2_000, "mcp", "message", 50, "set_flags", READ);
    let query = HistoryGroupQuery { sources: vec!["mcp".into()], ..Default::default() };
    let (first, next) = s.list_history_groups(None, 4, &query).unwrap();
    assert_eq!(first.len(), 4);
    assert!(first.iter().all(|g| g.source == "mcp" && g.kind == "flagged"), "the agent's read mail stays hidden");
    let (rest, _) = s.list_history_groups(next.as_deref(), 100, &query).unwrap();
    assert_eq!(first.len() + rest.len(), 10);
}

#[test]
fn an_unknown_source_is_refused() {
    let s = Store::open_in_memory().unwrap();
    assert!(s.list_history_groups(None, 10, &of(&["someone"])).is_err());
}
