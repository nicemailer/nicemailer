//! The prompt files: the built-in ones, the user's, their merge and every
//! rule of `specifications/ia.md`, sections 4 and 5 (tests required in
//! section 15, "Prompts").

use std::path::PathBuf;

use nm_ai::catalog::{self, BUILTIN, Loaded, Problem, UserFile};
use nm_ai::prompt::{Input, Lang, On, Output, Scope, Screen, Source, Then, Unmarked};
use nm_core::FolderRole;

/// A minimal valid user prompt, with extra lines appended.
fn minimal(id: &str, key: &str, extra: &str) -> String {
    format!(
        r#"version = 1
id = "{id}"
scope = "message"
key = "{key}"
label = "Test"
input = ["message.text"]
output = "panel"
system = "Do the task."
{extra}"#
    )
}

/// A prompt of a passage selected in a draft, `on` and `then` given.
fn passage_prompt(on: &str, then: &str) -> String {
    format!(
        r#"version = 1
id = "mine"
scope = "draft"
key = "x"
label = "Test"
input = ["draft.own_text", "draft.context"]
output = "choices"
{then}
on = "{on}"
system = "Do the task."
"#
    )
}

fn user(files: &[(&str, &str)]) -> Loaded {
    let files: Vec<UserFile> = files
        .iter()
        .map(|(name, text)| UserFile {
            path: PathBuf::from(format!("/prompts/{name}")),
            text: text.to_string(),
        })
        .collect();
    catalog::load(&files)
}

/// The single problem of a load with one user file.
fn problem_of(text: &str) -> Problem {
    let l = user(&[("p.toml", text)]);
    assert_eq!(l.problems.len(), 1, "{:?}", l.problems);
    l.problems[0].clone()
}

#[test]
fn every_builtin_prompt_loads_cleanly() {
    let l = catalog::load(&[]);
    assert_eq!(
        l.problems,
        vec![],
        "built-in prompts must load without problem"
    );
    assert_eq!(
        l.warnings,
        vec![],
        "built-in prompts must load without warning"
    );
    assert_eq!(l.catalog.prompts().len(), BUILTIN.len());
    for (name, _) in BUILTIN {
        let id = name.trim_end_matches(".toml");
        let p = l
            .catalog
            .get(id)
            .unwrap_or_else(|| panic!("{name}: its id must be its file name"));
        assert_eq!(p.source, Source::Builtin);
        // Every built-in prompt is in English and French (`CLAUDE.md`: the
        // app supports both).
        assert!(
            p.label.0.contains_key("en") && p.label.0.contains_key("fr"),
            "{id}"
        );
        for v in &p.variants {
            assert!(
                v.label.0.contains_key("en") && v.label.0.contains_key("fr"),
                "{id}/{}",
                v.key
            );
        }
    }
}

#[test]
fn builtin_system_prompts_build_for_every_variant() {
    let l = catalog::load(&[]);
    for p in l.catalog.prompts() {
        let keys: Vec<Option<char>> = if p.variants.is_empty() {
            vec![None]
        } else {
            p.variants.iter().map(|v| Some(v.key)).collect()
        };
        for k in keys {
            let s = p.system(k, "fr").unwrap();
            assert!(s.starts_with(&format!("TASK={}", p.id)), "{}", p.id);
            assert!(!s.contains("{{variant}}"), "{}", p.id);
            assert!(s.ends_with(nm_ai::prompt::GUARD), "{}", p.id);
        }
    }
}

#[test]
fn the_menus_of_the_spec() {
    let l = catalog::load(&[]);
    let keys = |screen, folder| -> Vec<(char, String)> {
        l.catalog
            .menu(screen, folder)
            .iter()
            .map(|p| (p.key, p.id.clone()))
            .collect()
    };
    // The most used first (the first row is the one Return chooses):
    // proofreading in a reply, the summary of a mail read, sorting in the
    // Quarantine.
    let compose = keys(Screen::Compose, None);
    assert_eq!(
        compose,
        [
            ('c', "proofread".into()),
            ('i', "improve".into()),
            ('r', "rewrite".into()),
            ('l', "translate-reply".into()),
            ('t', "translate-draft".into()),
            ('w', "write-from-notes".into()),
            ('o', "subject-ideas".into()),
        ]
    );
    let ids = |screen, folder| -> Vec<String> {
        keys(screen, Some(folder))
            .into_iter()
            .map(|k| k.1)
            .collect()
    };
    // One mail, in an ordinary folder: its prompts only, the conversation's
    // first; filing it alone is not offered.
    assert_eq!(
        ids(Screen::Mail, FolderRole::Inbox),
        [
            "summarize-thread",
            "quick-replies",
            "translate-message",
            "security-check",
            "dates",
            "contact"
        ]
    );
    // One mail in Quarantine: the sorting of the whole folder first, as it
    // takes the folder when nothing is marked, then the mail's prompts,
    // the security check included.
    assert_eq!(
        ids(Screen::Mail, FolderRole::Quarantine),
        [
            "triage-quarantine",
            "summarize-thread",
            "quick-replies",
            "translate-message",
            "security-check",
            "dates",
            "contact"
        ]
    );
    // Several mails marked: filing them, nothing that needs one mail.
    assert_eq!(
        ids(Screen::Selection, FolderRole::Quarantine),
        ["triage-quarantine"]
    );
    assert_eq!(
        ids(Screen::Selection, FolderRole::Archive),
        ["suggest-folder"]
    );
    // The letters of one palette are distinct: `s` sorts, `c` checks.
    let k: Vec<char> = keys(Screen::Mail, Some(FolderRole::Quarantine))
        .into_iter()
        .map(|k| k.0)
        .collect();
    assert_eq!(k, ['s', 'r', 'p', 't', 'c', 'd', 'v']);
    let q = l
        .catalog
        .menu(Screen::Selection, Some(FolderRole::Quarantine));
    assert_eq!(
        (q[0].id.as_str(), q[0].unmarked),
        ("triage-quarantine", Unmarked::Folder)
    );
    let a = l.catalog.menu(Screen::Selection, Some(FolderRole::Archive));
    assert_eq!(
        (a[0].id.as_str(), a[0].unmarked),
        ("suggest-folder", Unmarked::Cursor)
    );
    // The screens' names in the contract.
    for screen in Screen::ALL {
        assert_eq!(Screen::parse(screen.as_str()), Some(screen));
    }
    assert_eq!(Screen::parse("reading"), None);
}

#[test]
fn builtin_defaults_follow_the_spec() {
    let l = catalog::load(&[]);
    let p = |id| l.catalog.get(id).unwrap();
    // Writing follows the text's language, the rest the app's (section 7).
    assert_eq!(p("proofread").lang, Lang::Content);
    assert_eq!(p("summarize-thread").lang, Lang::App);
    // Translation (section 7): a mail read, into the app's language, with
    // no language to choose; a reply, into the language of the mail
    // received, on a reply only; a mail written, into a language chosen
    // among those of the app.
    assert_eq!(p("translate-message").lang, Lang::App);
    assert!(p("translate-message").variants.is_empty());
    assert_eq!(p("translate-reply").lang, Lang::Replied);
    assert!(p("translate-reply").replies_only());
    assert!(p("translate-reply").inputs.contains(&Input::DraftReplied));
    assert!(!p("translate-draft").replies_only());
    assert_eq!(p("translate-draft").lang, Lang::Variant);
    let languages: Vec<char> = p("translate-draft").variants.iter().map(|v| v.key).collect();
    assert_eq!(languages.len(), 19, "the languages of the app: {languages:?}");
    assert_eq!(p("proofread").inputs, [Input::DraftOwnText]);
    assert_eq!(p("proofread").output, Output::Replace);
    assert_eq!(
        p("triage-quarantine").folders.as_deref(),
        Some(&[FolderRole::Quarantine][..])
    );
}

#[test]
fn a_user_file_replaces_adds_and_disables() {
    let replace = minimal("dates", "d", "").replace("Do the task.", "My own dates prompt.");
    let add = minimal("my-tone", "x", "");
    let disable =
        "version = 1\nid = \"quick-replies\"\nenabled = false\nscope = \"whatever, not checked\"\n";
    let l = user(&[
        ("dates.toml", &replace),
        ("tone.toml", &add),
        ("off.toml", disable),
    ]);
    assert_eq!(l.problems, vec![]);
    let c = &l.catalog;
    assert_eq!(c.get("dates").unwrap().text, "My own dates prompt.");
    assert!(matches!(c.get("dates").unwrap().source, Source::User(_)));
    assert!(c.get("my-tone").is_some());
    assert!(c.get("quick-replies").is_none());
    // Order: built-in ones (or their replacement) in place, the added at the end.
    let ids: Vec<&str> = c.prompts().iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.last(), Some(&"my-tone"));
    assert!(
        ids.iter().position(|i| *i == "dates")
            < ids.iter().position(|i| *i == "summarize-attachment")
    );
}

#[test]
fn an_invalid_file_is_refused_alone() {
    let l = user(&[
        ("bad.toml", "version = 1\nid = \"bad\"\n"),
        ("good.toml", &minimal("good", "x", "")),
    ]);
    assert_eq!(l.problems.len(), 1);
    assert_eq!(l.problems[0].file, "/prompts/bad.toml");
    assert!(l.catalog.get("good").is_some());
    assert!(l.catalog.get("proofread").is_some());
}

#[test]
fn the_same_id_twice_is_refused_in_the_second_file() {
    let l = user(&[
        ("a.toml", &minimal("mine", "x", "")),
        ("b.toml", &minimal("mine", "y", "")),
    ]);
    assert_eq!(l.problems.len(), 1);
    assert_eq!(l.problems[0].file, "/prompts/b.toml");
    assert!(l.problems[0].message.contains("/prompts/a.toml"));
    assert_eq!(l.catalog.get("mine").unwrap().key, 'x');
}

#[test]
fn disabling_an_unknown_id_is_a_warning() {
    let l = user(&[(
        "off.toml",
        "version = 1\nid = \"nothing\"\nenabled = false\n",
    )]);
    assert_eq!(l.problems, vec![]);
    assert_eq!(l.warnings.len(), 1);
    assert_eq!(l.warnings[0].line, Some(3));
}

#[test]
fn unknown_keys_are_warnings_with_their_line() {
    let l = user(&[("p.toml", &minimal("mine", "x", "colour = \"blue\""))]);
    assert_eq!(l.problems, vec![]);
    assert_eq!(l.warnings.len(), 1);
    assert_eq!(
        (l.warnings[0].key.as_deref(), l.warnings[0].line),
        (Some("colour"), Some(9))
    );
}

#[test]
fn a_key_taken_in_the_same_screen_is_refused_on_the_users_file() {
    // `t` is translate-message's in the reading screen; a thread prompt is
    // shown there too.
    let clash = minimal("mine", "t", "").replace("scope = \"message\"", "scope = \"thread\"");
    let p = problem_of(&clash);
    assert_eq!(p.file, "/prompts/p.toml");
    assert_eq!(p.key.as_deref(), Some("key"));
    assert!(p.message.contains("translate-message"), "{}", p.message);
    // The same letter in another screen is fine: `t` in the attachment menu.
    let other = minimal("mine", "t", "")
        .replace("scope = \"message\"", "scope = \"attachment\"")
        .replace("message.text", "attachment.text");
    assert_eq!(user(&[("p.toml", &other)]).problems, vec![]);
}

#[test]
fn a_shared_key_needs_folders_that_do_not_overlap() {
    let sel = |folders: &str| {
        format!(
            "version = 1\nid = \"mine\"\nscope = \"selection\"\nkey = \"s\"\nlabel = \"x\"\n\
             input = [\"selection.items\", \"targets\"]\noutput = \"targets\"\nsystem = \"t\"\n{folders}"
        )
    };
    // `s` is used in the Quarantine and in ten other folders: Drafts is free.
    assert_eq!(
        user(&[("p.toml", &sel("folders = [\"drafts\"]"))]).problems,
        vec![]
    );
    assert!(
        problem_of(&sel("folders = [\"drafts\", \"archive\"]"))
            .message
            .contains("suggest-folder")
    );
    assert!(problem_of(&sel("")).message.contains("already"));
}

#[test]
fn each_rule_is_an_error_with_its_line() {
    // (text, key, line, part of the message)
    let cases: Vec<(String, &str, Option<u32>, &str)> = vec![
        (
            minimal("mine", "x", "").replace("version = 1", "version = 2"),
            "version",
            Some(1),
            "version = 1",
        ),
        (minimal("Mine!", "x", ""), "id", Some(2), "a-z"),
        (
            minimal("mine", "x", "").replace("\"message\"", "\"mailbox\""),
            "scope",
            Some(3),
            "unknown value",
        ),
        (
            minimal("mine", "xy", ""),
            "key",
            Some(4),
            "one lower-case letter",
        ),
        (
            minimal("mine", "X", ""),
            "key",
            Some(4),
            "one lower-case letter",
        ),
        (
            minimal("mine", "x", "").replace("label = \"Test\"", "label = { english = \"x\" }"),
            "label.english",
            Some(5),
            "two-letter",
        ),
        (
            minimal("mine", "x", "").replace("[\"message.text\"]", "[\"draft.own_text\"]"),
            "input",
            Some(6),
            "no meaning in the scope message",
        ),
        (
            minimal("mine", "x", "").replace("[\"message.text\"]", "[\"message.body\"]"),
            "input",
            Some(6),
            "unknown input",
        ),
        (
            minimal("mine", "x", "")
                .replace("[\"message.text\"]", "[\"message.text\", \"message.text\"]"),
            "input",
            Some(6),
            "twice",
        ),
        (
            minimal("mine", "x", "").replace("[\"message.text\"]", "[]"),
            "input",
            Some(6),
            "at least one",
        ),
        (
            minimal("mine", "x", "").replace("\"panel\"", "\"replace\""),
            "output",
            Some(7),
            "cannot be applied in the scope message",
        ),
        (
            minimal("mine", "x", "").replace("\"panel\"", "\"choices\""),
            "then",
            Some(7),
            "needs then",
        ),
        (
            minimal("mine", "x", "then = \"reply\""),
            "then",
            Some(9),
            "only for output",
        ),
        (
            minimal("mine", "x", "").replace("\"panel\"", "\"choices\"") + "then = \"insert\"",
            "then",
            Some(9),
            "cannot be applied",
        ),
        (
            minimal("mine", "x", "max_output_tokens = 0"),
            "max_output_tokens",
            Some(9),
            "between",
        ),
        (
            minimal("mine", "x", "folders = [\"attic\"]"),
            "folders",
            Some(9),
            "unknown folder role",
        ),
        (
            minimal("mine", "x", "folders = []"),
            "folders",
            Some(9),
            "at least one",
        ),
        (
            minimal("mine", "x", "unmarked = \"folder\""),
            "unmarked",
            Some(9),
            "only for scope",
        ),
        (
            minimal("mine", "x", "lang = \"variant\""),
            "lang",
            Some(9),
            "needs variants",
        ),
        (
            minimal("mine", "x", "").replace("Do the task.", "Do {{variant}}."),
            "system",
            Some(8),
            "no variant",
        ),
        (
            minimal("mine", "x", "").replace("Do the task.", "  "),
            "system",
            Some(8),
            "empty",
        ),
        (
            minimal("mine", "x", "").replace("system = \"Do the task.\"\n", ""),
            "system",
            None,
            "required",
        ),
        (
            "version = 1\nid = \"mine\"\nscope = [\n".to_string(),
            "",
            Some(3),
            "",
        ),
        (
            minimal("mine", "x", "on = \"both\""),
            "on",
            Some(9),
            "only for scope = \"draft\"",
        ),
        (
            passage_prompt("both", "then = \"replace\""),
            "input",
            Some(6),
            "draft.context marks the passage",
        ),
        (
            passage_prompt("selection", "then = \"replace\"")
                .replace(", \"draft.context\"", "")
                .replace("on = \"selection\"", "on = \"text\""),
            "then",
            Some(8),
            "needs on = \"selection\"",
        ),
        (
            passage_prompt("everywhere", "then = \"replace\""),
            "on",
            Some(9),
            "text, selection or both",
        ),
    ];
    for (text, key, line, message) in cases {
        let p = problem_of(&text);
        if !key.is_empty() {
            assert_eq!(p.key.as_deref(), Some(key), "{text}\n→ {p}");
        }
        assert_eq!(p.line, line, "{text}\n→ {p}");
        assert!(p.message.contains(message), "{text}\n→ {p}");
    }
}

#[test]
fn a_passage_selected_has_its_own_palette() {
    let l = user(&[]);
    let ids = |screen| -> Vec<String> {
        l.catalog
            .menu(screen, None)
            .iter()
            .map(|p| p.id.clone())
            .collect()
    };
    // Text selected: its other wordings first (⌘I then Return), then what
    // works on either; nothing that needs the whole text.
    assert_eq!(
        ids(Screen::ComposeSelection),
        [
            "alternatives",
            "proofread",
            "improve",
            "rewrite",
            "translate-reply",
            "translate-draft"
        ]
    );
    // Nothing selected: no prompt of a passage.
    assert_eq!(
        ids(Screen::Compose),
        [
            "proofread",
            "improve",
            "rewrite",
            "translate-reply",
            "translate-draft",
            "write-from-notes",
            "subject-ideas"
        ]
    );
    let alt = l.catalog.get("alternatives").unwrap();
    assert_eq!((alt.on, alt.then), (On::Selection, Some(Then::Replace)));
    // A user's prompt of a passage loads.
    let mine = user(&[(
        "mine.toml",
        &passage_prompt("selection", "then = \"replace\""),
    )]);
    assert_eq!(mine.problems, vec![]);
    assert!(
        mine.catalog
            .menu(Screen::ComposeSelection, None)
            .iter()
            .any(|p| p.id == "mine")
    );
    assert!(
        !mine
            .catalog
            .menu(Screen::Compose, None)
            .iter()
            .any(|p| p.id == "mine")
    );
}

#[test]
fn whole_folder_selection_needs_folders() {
    let text = "version = 1\nid = \"mine\"\nscope = \"selection\"\nkey = \"z\"\nlabel = \"x\"\n\
                input = [\"selection.items\", \"targets\"]\noutput = \"targets\"\nunmarked = \"folder\"\nsystem = \"t\"\n";
    let p = problem_of(text);
    assert_eq!((p.key.as_deref(), p.line), (Some("unmarked"), Some(8)));
    assert!(p.message.contains("needs folders"));
    let ok = text.replace("unmarked", "folders = [\"drafts\"]\nunmarked");
    let l = user(&[("p.toml", &ok)]);
    assert_eq!(l.problems, vec![]);
    assert_eq!(l.catalog.get("mine").unwrap().unmarked, Unmarked::Folder);
}

#[test]
fn targets_and_replace_need_their_inputs() {
    let targets = "version = 1\nid = \"mine\"\nscope = \"selection\"\nkey = \"z\"\nlabel = \"x\"\n\
                   input = [\"selection.items\"]\noutput = \"targets\"\nsystem = \"t\"\n";
    assert!(
        problem_of(targets)
            .message
            .contains("selection.items and targets")
    );
    let replace = "version = 1\nid = \"mine\"\nscope = \"draft\"\nkey = \"z\"\nlabel = \"x\"\n\
                   input = [\"draft.full\"]\noutput = \"replace\"\nsystem = \"t\"\n";
    assert!(problem_of(replace).message.contains("draft.own_text"));
}

#[test]
fn variants_are_checked() {
    let base = |variants: &str| {
        format!(
            "{}{variants}",
            minimal("mine", "x", "").replace("Do the task.", "Do it. {{variant}}")
        )
    };
    let ok = base("[[variants]]\nkey = \"a\"\nlabel = \"A\"\ninstruction = \"Do A.\"\n");
    let l = user(&[("p.toml", &ok)]);
    assert_eq!(l.problems, vec![]);
    let p = l.catalog.get("mine").unwrap();
    assert!(p.system(Some('a'), "en").unwrap().contains("Do it. Do A."));

    let twice = base(
        "[[variants]]\nkey = \"a\"\nlabel = \"A\"\ninstruction = \"x\"\n[[variants]]\nkey = \"a\"\nlabel = \"B\"\ninstruction = \"y\"\n",
    );
    assert!(problem_of(&twice).message.contains("two variants"));
    let missing = base("[[variants]]\nkey = \"a\"\nlabel = \"A\"\n");
    assert_eq!(
        problem_of(&missing).key.as_deref(),
        Some("variants.instruction")
    );
    let unknown =
        base("[[variants]]\nkey = \"a\"\nlabel = \"A\"\ninstruction = \"x\"\ncolour = 1\n");
    assert!(
        problem_of(&unknown)
            .message
            .contains("unknown key in a variant")
    );
    let no_marker = minimal(
        "mine",
        "x",
        "[[variants]]\nkey = \"a\"\nlabel = \"A\"\ninstruction = \"x\"\n",
    );
    assert!(problem_of(&no_marker).message.contains("{{variant}}"));
    let inline = base("variants = [{ key = \"a\", label = \"A\", instruction = \"x\" }]\n");
    assert_eq!(user(&[("p.toml", &inline)]).problems, vec![]);
}

#[test]
fn the_hash_follows_behaviour_not_form() {
    let hash = |text: &str| user(&[("p.toml", text)]).catalog.get("mine").unwrap().hash;
    let base = minimal("mine", "x", "");
    let h = hash(&base);
    // A comment, a label, or the order of keys: same prompt.
    assert_eq!(hash(&format!("# a comment\n{base}")), h);
    assert_eq!(
        hash(&base.replace(
            "label = \"Test\"",
            "label = { en = \"Other\", fr = \"Autre\" }"
        )),
        h
    );
    let reordered = base.replace("key = \"x\"\n", "") + "key = \"x\"\n";
    assert_eq!(hash(&reordered), h);
    // A word of the text, an input, the model: another prompt.
    assert_ne!(hash(&base.replace("Do the task.", "Do the task now.")), h);
    assert_ne!(
        hash(&base.replace(
            "[\"message.text\"]",
            "[\"message.text\", \"message.headers\"]"
        )),
        h
    );
    assert_ne!(
        hash(&minimal("mine", "x", "model = \"claude-haiku-5-5\"")),
        h
    );
}

#[test]
fn load_dir_reads_toml_files_in_order_and_skips_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("b.toml"), minimal("mine", "y", "")).unwrap();
    std::fs::write(dir.path().join("a.toml"), minimal("mine", "x", "")).unwrap();
    std::fs::write(dir.path().join("notes.md"), "not a prompt").unwrap();
    std::fs::write(dir.path().join(".hidden.toml"), "garbage").unwrap();
    std::fs::write(dir.path().join("latin1.toml"), [0xe9u8, 0x0a]).unwrap();
    let l = catalog::load_dir(dir.path());
    // a.toml wins over b.toml (same id), latin1.toml is refused.
    assert_eq!(l.catalog.get("mine").unwrap().key, 'x');
    assert_eq!(l.problems.len(), 2, "{:?}", l.problems);
    assert!(l.problems.iter().any(|p| p.message.contains("UTF-8")));
    assert!(l.problems.iter().any(|p| p.file.ends_with("b.toml")));
}

#[test]
fn a_missing_folder_means_builtin_prompts_only() {
    let l = catalog::load_dir(std::path::Path::new("/nonexistent/prompts"));
    assert_eq!(l.problems, vec![]);
    assert_eq!(l.catalog.prompts().len(), BUILTIN.len());
}

#[test]
fn scopes_inputs_and_outputs_round_trip() {
    for s in Scope::ALL {
        assert_eq!(Scope::parse(s.as_str()), Some(s));
    }
    for i in Input::ALL {
        assert_eq!(Input::parse(i.as_str()), Some(i));
        assert!(!i.scopes().is_empty());
    }
    for o in Output::ALL {
        assert_eq!(Output::parse(o.as_str()), Some(o));
    }
}

#[test]
fn the_example_of_the_docs_loads() {
    // `docs/ai.md`, "Writing your own": the example must stay valid.
    let doc: PathBuf = [env!("CARGO_MANIFEST_DIR"), "../../docs/ai.md"]
        .iter()
        .collect();
    let doc = std::fs::read_to_string(doc).unwrap();
    let start = doc.find("```toml\n").expect("a TOML example") + "```toml\n".len();
    let end = start + doc[start..].find("```").unwrap();
    let l = user(&[("polite-no.toml", &doc[start..end])]);
    assert_eq!(l.problems, vec![]);
    assert_eq!(l.warnings, vec![]);
    let p = l.catalog.get("polite-no").unwrap();
    assert!(
        l.catalog
            .menu(Screen::Mail, Some(FolderRole::Inbox))
            .iter()
            .any(|q| q.id == p.id)
    );
}

#[test]
fn lang_replied_needs_a_draft_that_reads_the_mail_replied_to() {
    // On a mail read: no mail replied to.
    let l = user(&[("r.toml", &minimal("mine", "x", "lang = \"replied\""))]);
    assert_eq!(l.problems.len(), 1, "{:?}", l.problems);
    assert!(l.problems[0].message.contains("draft.replied"), "{:?}", l.problems);
    // A draft that does not read it.
    let no_input = "version = 1\nid = \"mine\"\nscope = \"draft\"\nkey = \"x\"\nlabel = \"Test\"\n\
                    input = [\"draft.own_text\"]\noutput = \"replace\"\nlang = \"replied\"\nsystem = \"Do it.\"\n";
    let l = user(&[("r.toml", no_input)]);
    assert_eq!(l.problems.len(), 1, "{:?}", l.problems);
    // With it: loaded, offered on a reply only.
    let ok = no_input.replace("[\"draft.own_text\"]", "[\"draft.own_text\", \"draft.replied\"]");
    let l = user(&[("r.toml", &ok)]);
    assert_eq!(l.problems, vec![]);
    let p = l.catalog.get("mine").unwrap();
    assert!(p.replies_only());
    let system = p.system(None, "fr").unwrap();
    assert!(system.contains("language of the mail replied to"), "{system}");
    assert!(!system.contains("Answer in French"), "{system}");
}
