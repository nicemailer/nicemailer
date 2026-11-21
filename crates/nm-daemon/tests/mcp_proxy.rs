//! `nicemailer-mcp` end to end, against the real core (`mcp.md`, section
//! 12, "Proxy"): the server of `nm-mcp` runs in the test, on a pipe, as
//! the MCP client (Claude Desktop) would run it; the core is the binary of
//! this crate, on a temporary data directory.
//!
//! What is checked: `initialize` without a core, the first tool opening
//! the connection; every tool's answer against its output schema, without
//! empty fields, dates in ISO 8601; a conversation without its HTML, its
//! quotes hidden unless asked, its texts bounded by `[mcp]`; a folder by
//! role across accounts; a core in maintenance, then available; a core
//! restarted between two calls; the drafts of the agent and the refusals
//! on the user's; cancellation.
//!
//! Rust note: `#[tokio::test(flavor = "multi_thread")]` runs the test in a
//! runtime with several threads, so that the blocking helpers of `common`
//! (the socket client of the setup) do not stop the server running beside.

mod common;

use std::path::Path;
use std::time::Duration;

use common::fixtures::{self, message};
use common::{Client, start_with};
use nm_mcp::core::{CoreLink, Waits};
use nm_mcp::tools::Tools;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};

/// The MCP client of the tests: writes requests to the server, reads its
/// lines, keeps the notifications.
struct Agent {
    input: WriteHalf<DuplexStream>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    next_id: u64,
    notifications: Vec<Value>,
}

/// Short waits: a test must not wait 30 s for a core that never comes.
fn waits() -> Waits {
    Waits { start: Duration::from_millis(300), maintenance: Duration::from_secs(15), maintenance_step: Duration::from_millis(100) }
}

impl Agent {
    /// A server on the data directory `dir`, initialized with `version`.
    async fn start(dir: &Path, version: &str) -> Agent {
        let (client, server) = tokio::io::duplex(1 << 22);
        let (server_read, server_write) = tokio::io::split(server);
        let tools = Tools::new(CoreLink::new(Some(dir.to_path_buf()), waits()));
        tokio::spawn(nm_mcp::server::serve(BufReader::new(server_read), server_write, tools));
        let (read, input) = tokio::io::split(client);
        let mut a = Agent { input, lines: BufReader::new(read).lines(), next_id: 1, notifications: Vec::new() };
        let r = a
            .request("initialize", json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}))
            .await;
        assert!(r.get("result").is_some(), "{r}");
        a.notify("notifications/initialized", json!({})).await;
        a
    }

    async fn send(&mut self, v: Value) {
        self.input.write_all(format!("{v}\n").as_bytes()).await.unwrap();
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params})).await;
    }

    /// Sends a request; its id.
    async fn ask(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        id
    }

    /// The response to `id`, the notifications read meanwhile kept.
    async fn response(&mut self, id: u64) -> Value {
        loop {
            let line = tokio::time::timeout(Duration::from_secs(30), self.lines.next_line())
                .await
                .expect("no answer of the server within 30 s")
                .unwrap()
                .expect("the server closed its output");
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON on the output: {line}: {e}"));
            assert_eq!(v["jsonrpc"], "2.0", "{v}");
            if v.get("id").is_none() {
                self.notifications.push(v);
                continue;
            }
            assert_eq!(v["id"], id, "an answer to another request: {v}");
            return v;
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.ask(method, params).await;
        self.response(id).await
    }

    /// Calls a tool: its structured content and whether it is an error;
    /// checks that the text content is the same JSON.
    async fn tool(&mut self, name: &str, arguments: Value) -> (Value, bool) {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments})).await;
        let result = r.get("result").unwrap_or_else(|| panic!("{name}: a protocol error: {r}")).clone();
        let error = result["isError"] == true;
        let text = result["content"][0]["text"].as_str().unwrap().to_string();
        if error {
            return (json!(text), true);
        }
        let structured = result["structuredContent"].clone();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), structured, "{name}: the text and the structured content differ");
        (structured, false)
    }

    /// [`Agent::tool`] that must succeed.
    async fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (v, error) = self.tool(name, arguments.clone()).await;
        assert!(!error, "{name} {arguments}: {v}");
        v
    }

    /// A tool that the core refuses: the reason it gives the agent, in the
    /// second text block of the result (`docs/mcp.md`, "Errors").
    async fn refused(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments})).await;
        assert_eq!(r["result"]["isError"], true, "{name}: {r}");
        let block = r["result"]["content"][1]["text"].as_str().unwrap_or_else(|| panic!("{name}: no reason: {r}"));
        serde_json::from_str(block).unwrap()
    }

    /// [`Agent::tool`] that must fail: its message.
    async fn fails(&mut self, name: &str, arguments: Value) -> String {
        let (v, error) = self.tool(name, arguments.clone()).await;
        assert!(error, "{name} {arguments} should fail: {v}");
        v.as_str().unwrap().to_string()
    }
}

/// A core with the fake network (an archive account can write drafts),
/// the test mails imported into the account `hey`.
fn prepared(dir: &Path, config: &str) -> common::Core {
    std::fs::write(dir.join("config.toml"), config).unwrap();
    let core = start_with(dir, &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut app = Client::connect_as(dir, "app");
    fixtures::import(dir, &mut app, "hey");
    core
}

/// Checks `v` against the parts of JSON Schema the tools use: `type`
/// (one or several), `properties`, `items`, `enum`, `required`,
/// `additionalProperties: false`.
fn conforms(v: &Value, schema: &Value, at: &str) {
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => panic!("bad type in the schema at {at}"),
        };
        let ok = types.iter().any(|t| match *t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => false,
        });
        assert!(ok, "{at}: {v} is not of type {t}");
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        assert!(e.contains(v), "{at}: {v} is not one of {e:?}");
    }
    if let (Some(o), Some(props)) = (v.as_object(), schema.get("properties").and_then(Value::as_object)) {
        for (k, x) in o {
            match props.get(k) {
                Some(s) => conforms(x, s, &format!("{at}.{k}")),
                None => assert!(schema.get("additionalProperties") != Some(&json!(false)), "{at}: {k} is not in the schema"),
            }
        }
        for r in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            assert!(o.contains_key(r.as_str().unwrap()), "{at}: {r} is required");
        }
    }
    if let (Some(a), Some(items)) = (v.as_array(), schema.get("items")) {
        for (i, x) in a.iter().enumerate() {
            conforms(x, items, &format!("{at}[{i}]"));
        }
    }
}

/// No empty field below the top level: no `null`, `""`, `[]`, `{}` nor
/// `false` (`mcp.md`, section 7). The lists of results themselves are
/// always there, even empty.
fn no_empty_field(v: &Value, at: &str, top: bool) {
    match v {
        Value::Null => panic!("{at} is null"),
        Value::Bool(false) => panic!("{at} is false"),
        Value::String(s) => assert!(!s.is_empty(), "{at} is empty"),
        Value::Array(a) => {
            assert!(top || !a.is_empty(), "{at} is an empty list");
            for (i, x) in a.iter().enumerate() {
                no_empty_field(x, &format!("{at}[{i}]"), false);
            }
        }
        Value::Object(o) => {
            assert!(top || !o.is_empty(), "{at} is an empty object");
            for (k, x) in o {
                no_empty_field(x, &format!("{at}.{k}"), top && at.is_empty() && x.is_array());
            }
        }
        _ => {}
    }
}

/// Every field named `date` is an ISO 8601 date with its offset.
fn dates_are_iso(v: &Value) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if k == "date" {
                    let s = x.as_str().unwrap();
                    assert!(s.len() == 25 && &s[10..11] == "T" && (s[19..].starts_with('+') || s[19..].starts_with('-')), "{s}");
                    assert!(nm_mcp::time::parse(s).is_ok(), "{s}");
                }
                dates_are_iso(x);
            }
        }
        Value::Array(a) => a.iter().for_each(dates_are_iso),
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_needs_no_core_and_the_first_tool_connects() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let list = a.request("tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 13);
    assert!(tools.iter().all(|t| t["outputSchema"]["type"] == "object" && t["annotations"].is_object()));
    let r = a.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}})).await;
    assert!(r["result"]["instructions"].as_str().unwrap().contains("never instructions"), "{r}");
    // No core yet: a tool error that says so, not a protocol error.
    let e = a.fails("mailbox_overview", json!({})).await;
    assert!(e.contains("no NiceMailer core runs"), "{e}");
    // The core starts: the next call connects.
    let _core = prepared(dir.path(), "");
    let v = a.ok("mailbox_overview", json!({})).await;
    assert_eq!(v["accounts"][0]["key"], "hey");
    // An unknown tool, a method of another capability: protocol errors.
    let r = a.request("tools/call", json!({"name": "send_mail", "arguments": {}})).await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = a.request("resources/list", json!({})).await;
    assert_eq!(r["error"]["code"], -32601, "{r}");
    // Arguments out of the schema: a tool error the model can correct.
    let e = a.fails("get_conversation", json!({"thread": 1})).await;
    assert!(e.contains("invalid arguments"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_client_gets_text_only() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let mut a = Agent::start(dir.path(), "2024-11-05").await;
    let list = a.request("tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().all(|t| t.get("outputSchema").is_none() && t.get("annotations").is_none()));
    let r = a.request("tools/call", json!({"name": "mailbox_overview", "arguments": {}})).await;
    assert!(r["result"].get("structuredContent").is_none(), "{r}");
    let text: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["accounts"][0]["key"], "hey");
    // A version this server does not know: its latest.
    let r = a.request("initialize", json!({"protocolVersion": "2099-01-01", "capabilities": {}})).await;
    assert_eq!(r["result"]["protocolVersion"], nm_mcp::server::VERSIONS[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_tool_answers_within_its_schema_without_empty_fields() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let (m2, thread) = message(dir.path(), "hey", "m2@a.example");
    let (_, docs) = message(dir.path(), "hey", "f1@b.example");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let list = a.request("tools/list", json!({})).await;
    let schemas: std::collections::HashMap<String, Value> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), t["outputSchema"].clone()))
        .collect();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "notes").unwrap();
    let attachment = {
        let t = a.ok("get_conversation", json!({"thread_id": docs})).await;
        t["messages"][0]["attachments"].as_array().unwrap().iter().find(|x| x["filename"] == "facture.pdf").unwrap()["id"].clone()
    };
    let mut calls = vec![
        ("mailbox_overview", json!({})),
        ("list_conversations", json!({"folder": "archive"})),
        ("list_conversations", json!({"folder": "inbox"})),
        ("list_conversations", json!({"folder": "everything", "filter": {"from": "alice", "after": "2020-01-01"}, "limit": 1})),
        ("search_messages", json!({"query": "devis"})),
        ("get_conversation", json!({"thread_id": thread})),
        ("get_conversation", json!({"thread_id": thread, "include_quoted": true})),
        ("get_message", json!({"message_id": m2})),
        ("get_message", json!({"link": link_of(dir.path(), m2)})),
        ("read_attachment", json!({"attachment_id": attachment})),
        ("find_contacts", json!({"query": "ali"})),
        ("agent_history", json!({})),
    ];
    let draft = a.ok("create_draft", json!({"kind": "reply_all", "reply_to_message_id": m2, "body": "Oui, jeudi."})).await;
    let id = draft["draft_id"].clone();
    calls.extend([
        ("get_draft", json!({"draft_id": id})),
        ("update_draft", json!({"draft_id": id, "subject": "Re: Devis, jeudi"})),
        ("attach_file", json!({"draft_id": id, "path": file.to_str().unwrap()})),
    ]);
    let mut seen = std::collections::HashSet::new();
    for (name, args) in calls {
        let v = a.ok(name, args).await;
        conforms(&v, &schemas[name], name);
        no_empty_field(&v, "", true);
        dates_are_iso(&v);
        seen.insert(name);
    }
    let v = a.ok("discard_draft", json!({"draft_id": id})).await;
    conforms(&v, &schemas["discard_draft"], "discard_draft");
    conforms(&draft, &schemas["create_draft"], "create_draft");
    no_empty_field(&draft, "", true);
    seen.extend(["discard_draft", "create_draft"]);
    assert_eq!(seen.len(), schemas.len(), "a tool is not tried: {:?}", schemas.keys().filter(|k| !seen.contains(k.as_str())).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_has_no_html_its_quotes_hidden_and_bounded_texts() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "[mcp]\nmessage_chars = 1000\n");
    let (m2, thread) = message(dir.path(), "hey", "m2@a.example");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let c = a.ok("get_conversation", json!({"thread_id": thread})).await;
    let text = c.to_string();
    assert!(!text.contains("IGNORE PREVIOUS") && !text.contains("<p>"), "{text}");
    let reply = c["messages"].as_array().unwrap().iter().find(|m| m["id"] == m2).unwrap().clone();
    assert_eq!(reply["text"], "Alors, jeudi ?");
    assert_eq!(reply["quote_hidden"], true);
    assert_eq!(reply["from"], "Alice <alice@a.example>");
    let with = a.ok("get_conversation", json!({"thread_id": thread, "include_quoted": true})).await;
    let reply = with["messages"].as_array().unwrap().iter().find(|m| m["id"] == m2).unwrap().clone();
    assert!(reply["text"].as_str().unwrap().contains("> Tu viens jeudi ?"), "{reply}");
    // The mail of 3,000 characters, bounded at 1,000; read whole by
    // get_message.
    let (report, report_thread) = message(dir.path(), "hey", "r1@d.example");
    let c = a.ok("get_conversation", json!({"thread_id": report_thread})).await;
    assert_eq!(c["truncated"], true);
    assert_eq!(c["messages"][0]["text"].as_str().unwrap().chars().count(), 1_000);
    let m = a.ok("get_message", json!({"message_id": report})).await;
    assert!(m["text"].as_str().unwrap().contains(&fixtures::report_text()), "{m}");
    assert!(m.get("next_offset").is_none());
    assert_eq!(m["thread_id"], report_thread);
}

/// The link of a message, as the core writes it (`docs/deep-links.md`).
fn link_of(dir: &Path, message: i64) -> String {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let key: i64 = db.query_row("SELECT link_key FROM message WHERE id = ?1", [message], |r| r.get(0)).unwrap();
    nm_api::deep_link::url(message, Some(key))
}

/// Each message carries its link to the mail, and `get_message` reads a
/// link the user pasted (from a note) as well as an id.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_carries_its_link_and_is_read_from_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let (m2, thread) = message(dir.path(), "hey", "m2@a.example");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let link = link_of(dir.path(), m2);
    assert_eq!(link.len(), "nicemailer://m/".len() + 12, "a key, not the id: {link}");
    let c = a.ok("get_conversation", json!({"thread_id": thread})).await;
    let reply = c["messages"].as_array().unwrap().iter().find(|m| m["id"] == m2).unwrap().clone();
    assert_eq!(reply["link"], link);

    let by_id = a.ok("get_message", json!({"message_id": m2})).await;
    assert_eq!(by_id["link"], link);
    // As pasted, in another case and with a final slash: the same answer.
    let by_link = a.ok("get_message", json!({"link": format!(" {}/ ", link.to_uppercase())})).await;
    assert_eq!(by_link, by_id);
    // An old link, by id, still works.
    assert_eq!(a.ok("get_message", json!({"link": format!("nicemailer://m/{m2}")})).await, by_id);

    let e = a.fails("get_message", json!({"link": "https://example.org/m/1"})).await;
    assert!(e.contains("not a link to a NiceMailer mail"), "{e}");
    let e = a.fails("get_message", json!({"link": "nicemailer://m/000000000000"})).await;
    assert!(e.contains("not found") || e.contains("000000000000"), "{e}");
    for both_or_none in [json!({"message_id": m2, "link": link}), json!({})] {
        let e = a.fails("get_message", both_or_none).await;
        assert!(e.contains("one of the two"), "{e}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_role_without_an_account_lists_that_folder_of_every_account() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let mut app = Client::connect_as(dir.path(), "app");
    fixtures::import(dir.path(), &mut app, "work");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let accounts = |v: &Value| -> std::collections::BTreeSet<String> {
        v["conversations"].as_array().unwrap().iter().map(|c| c["account"].as_str().unwrap().to_string()).collect()
    };
    let both = a.ok("list_conversations", json!({"folder": "archive"})).await;
    assert_eq!(accounts(&both), ["hey".to_string(), "work".to_string()].into());
    let one = a.ok("list_conversations", json!({"folder": "archive", "account": "work"})).await;
    assert_eq!(accounts(&one), ["work".to_string()].into());
    let e = a.fails("list_conversations", json!({"folder": "archive", "account": "nope"})).await;
    assert!(e.contains("hey") && e.contains("work"), "{e}");
    // Pages: every conversation once.
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({"folder": "archive", "limit": 2});
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let p = a.ok("list_conversations", args).await;
        ids.extend(p["conversations"].as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()));
        match p["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    let all = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!((ids.len(), all), (both["conversations"].as_array().unwrap().len(), ids.len()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_core_restarted_between_two_calls_is_found_again() {
    let dir = tempfile::tempdir().unwrap();
    let core = prepared(dir.path(), "");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    a.ok("mailbox_overview", json!({})).await;
    drop(core);
    // The socket file is left by a killed core: the next one replaces it.
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let _ = Client::connect(dir.path());
    let v = a.ok("mailbox_overview", json!({})).await;
    assert_eq!(v["accounts"][0]["key"], "hey");
}

/// The app prepares an update (`prepare_update`): the core answers
/// `MAINTENANCE` to the proxy, which waits with progress, then answers once
/// the app cancels it (`mise-a-jour.md`, section 5).
#[tokio::test(flavor = "multi_thread")]
async fn a_core_in_maintenance_is_waited_for_with_progress() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let dir2 = dir.path().to_path_buf();
    let mut app = tokio::task::spawn_blocking(move || {
        let mut app = Client::connect_as(&dir2, "app");
        app.call("prepare_update", json!({})).unwrap();
        app
    })
    .await
    .unwrap();
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let id = a.ask("tools/call", json!({"name": "mailbox_overview", "arguments": {}, "_meta": {"progressToken": "p1"}})).await;
    // Longer than the start wait (300 ms): only the maintenance waits so.
    tokio::time::sleep(Duration::from_millis(900)).await;
    // The app cancels the update: the core serves its clients again.
    tokio::task::spawn_blocking(move || app.call("cancel_update", json!({})).unwrap()).await.unwrap();
    let r = a.response(id).await;
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(r["result"]["structuredContent"]["accounts"][0]["key"], "hey");
    let progress: Vec<&Value> = a.notifications.iter().filter(|n| n["method"] == "notifications/progress").collect();
    assert!(!progress.is_empty());
    assert!(progress.iter().all(|n| n["params"]["progressToken"] == "p1"));
    assert!(progress[0]["params"]["message"].as_str().unwrap().contains("updated"), "{:?}", progress[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_drafts_its_replies_and_leaves_the_users_alone() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path(), "");
    let (m1, _) = message(dir.path(), "hey", "m1@a.example");
    let (m2, _) = message(dir.path(), "hey", "m2@a.example");
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    let drafts = |dir: &Path| -> i64 {
        let db = nm_store::rusqlite::Connection::open(dir.join("nicemailer.db")).unwrap();
        db.query_row("SELECT count(*) FROM draft", [], |r| r.get(0)).unwrap()
    };

    // A bad address: refused before anything is written.
    let e = a.fails("create_draft", json!({"kind": "new", "to": "alice@a.example, not an address", "body": "x"})).await;
    assert!(e.contains("not an address"), "{e}");
    assert_eq!(drafts(dir.path()), 0);

    // A reply: the agent's text above the quotation, an English quote line.
    let d = a.ok("create_draft", json!({"kind": "reply_all", "reply_to_message_id": m2, "body": "Oui, jeudi."})).await;
    assert_eq!(d["to"], json!(["Alice <alice@a.example>"]));
    assert!(d.get("body").is_none());
    let id = d["draft_id"].clone();
    let full = a.ok("get_draft", json!({"draft_id": id})).await;
    let body = full["body"].as_str().unwrap();
    assert!(body.starts_with("Oui, jeudi.\n\nOn 24 Jun 2020 at "), "{body}");
    assert!(body.contains("Alice <alice@a.example> wrote:") && body.contains("Alors, jeudi ?"), "{body}");
    assert_eq!(fixtures::draft_sources(dir.path(), id.as_i64().unwrap()), ("mcp".into(), "mcp".into()));
    // Changed, a file attached: still its own.
    let u = a.ok("update_draft", json!({"draft_id": id, "body": "Oui, jeudi à 10 h.", "cc": "Bob <bob@b.example>"})).await;
    assert_eq!(u["cc"], json!(["Bob <bob@b.example>"]));
    let file = dir.path().join("plan.pdf");
    std::fs::write(&file, fixtures::pdf_of("Plan")).unwrap();
    let f = a.ok("attach_file", json!({"draft_id": id, "path": file.to_str().unwrap()})).await;
    assert_eq!(f["attachments"][0]["filename"], "plan.pdf");
    assert_eq!(f["attachments"][0]["mime_type"], "application/pdf");
    // The staged copy was taken by the core; the agent's file is left.
    assert!(file.exists());
    assert_eq!(std::fs::read_dir(dir.path().join("incoming.noindex")).unwrap().count(), 0);
    let e = a.fails("attach_file", json!({"draft_id": id, "path": "relative/file.txt"})).await;
    assert!(e.contains("absolute"), "{e}");
    let e = a.fails("attach_file", json!({"draft_id": id, "path": "/nonexistent/file.txt"})).await;
    assert!(e.contains("/nonexistent/file.txt"), "{e}");

    // The user's reply to the other mail: the agent may read it only.
    let mut app = Client::connect_as(dir.path(), "app");
    let user = app.call("create_draft", json!({"reply_to": m1, "quote_header": "Alice a écrit :"})).unwrap()["draft"].clone();
    let read = a.ok("get_draft", json!({"draft_id": user["id"]})).await;
    assert!(read["body"].as_str().unwrap().contains("Alice a écrit"), "{read}");
    let e = a.fails("update_draft", json!({"draft_id": user["id"], "body": "pwned"})).await;
    assert!(e.contains("written by the user"), "{e}");
    let e = a.fails("discard_draft", json!({"draft_id": user["id"]})).await;
    assert!(e.contains("written by the user"), "{e}");
    let e = a.fails("create_draft", json!({"kind": "reply_sender", "reply_to_message_id": m1, "body": "x"})).await;
    assert!(e.contains("already started a reply"), "{e}");
    // The reason comes with the sentence, for the agent to decide on it.
    let why = a.refused("update_draft", json!({"draft_id": user["id"], "body": "pwned"})).await;
    assert_eq!(why, json!({"reason": "draft_by_user", "draft": user["id"]}));
    assert_eq!(app.call("open_draft", json!({"id": user["id"]})).unwrap()["body"], user["body"]);

    // Its own draft, switched to the formatted mode by the user: its text
    // stays the user's, the subject may still change.
    let d = app.call("open_draft", json!({"id": id})).unwrap();
    app.call("update_draft", json!({"id": id, "revision": d["revision"], "body": "<p>Oui, <b>jeudi</b>.</p>", "body_format": "html"}))
        .unwrap();
    let e = a.fails("update_draft", json!({"draft_id": id, "body": "plain"})).await;
    assert!(e.contains("formatted"), "{e}");
    let s = a.ok("update_draft", json!({"draft_id": id, "subject": "Jeudi"})).await;
    assert_eq!((s["subject"].clone(), s["format"].clone()), (json!("Jeudi"), json!("html")));
    assert_eq!(app.call("open_draft", json!({"id": id})).unwrap()["body"], "<p>Oui, <b>jeudi</b>.</p>");

    // Its own, discarded.
    let gone = a.ok("discard_draft", json!({"draft_id": id})).await;
    assert_eq!(gone["state"], "discarded");
    let e = a.fails("get_draft", json!({"draft_id": id})).await;
    assert!(e.starts_with("not found"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_call_gets_no_answer_and_the_server_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Agent::start(dir.path(), "2025-11-25").await;
    // No core: the call waits for one (300 ms).
    let id = a.ask("tools/call", json!({"name": "mailbox_overview", "arguments": {}})).await;
    a.notify("notifications/cancelled", json!({"requestId": id, "reason": "test"})).await;
    let ping = a.ask("ping", json!({})).await;
    assert_eq!(a.response(ping).await["result"], json!({}));
    // Past the wait, still nothing for the cancelled call: the next line
    // is the answer to the next request.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let ping = a.ask("ping", json!({})).await;
    assert_eq!(a.response(ping).await["id"], ping);
}
