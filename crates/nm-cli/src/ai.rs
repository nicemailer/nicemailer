//! `nicemailer ai …`: the AI assistance from the terminal
//! (`specifications/ia.md`, section 12). The CLI is the test bench of the
//! contract: everything here goes through the same commands as the app,
//! except `prompts export` and `prompts check`, which work on the files
//! directly, like `config check`.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Subcommand;
use nm_api::method;
use serde_json::{Value, json};

use crate::Client;

/// `nicemailer ai …`.
#[derive(Subcommand)]
pub enum AiAction {
    /// Whether the AI is on, the provider and model, and whether a run can
    /// start; the problems of your prompt files.
    Status {
        /// Also ask the claude binary its version and your subscription
        /// (no call to the model).
        #[arg(long)]
        check: bool,
    },
    /// The prompts in use, built-in or yours.
    Prompts {
        #[command(subcommand)]
        action: PromptsAction,
    },
    /// Run a prompt on a draft and show what it proposes, the text as it
    /// comes. Nothing changes in the draft without --apply.
    Run {
        /// The prompt's id: proofread, rewrite, translate-draft, subject-ideas…
        prompt: String,
        /// The draft's id.
        #[arg(long)]
        draft: i64,
        /// The key of a variant, for a prompt that has some (rewrite: f, a,
        /// c, l, s; translate-draft: e, f, s, d, i).
        #[arg(long)]
        variant: Option<String>,
        /// The language of the answers that follow the app's (en, fr).
        #[arg(long, default_value = "en")]
        lang: String,
        /// Apply the proposal to the draft.
        #[arg(long)]
        apply: bool,
        /// With --apply, for a list of subjects: the number of the one to
        /// take, from 1.
        #[arg(long)]
        choice: Option<usize>,
    },
}

/// `nicemailer ai prompts …`.
#[derive(Subcommand)]
pub enum PromptsAction {
    /// The prompts of a screen's AI menu.
    List {
        /// compose (the whole text of a reply), compose-selection (a passage
        /// selected), mail (one mail), selection (several marked) or
        /// attachment.
        #[arg(long, default_value = "compose")]
        screen: String,
        /// The folder's role, for the mail and selection screens (quarantine…).
        #[arg(long)]
        folder: Option<String>,
        /// The language of the labels (en, fr).
        #[arg(long, default_value = "en")]
        lang: String,
    },
    /// Write a copy of built-in prompts to your prompts folder, to start
    /// from: a file with the same id replaces the built-in prompt. An
    /// existing file is never overwritten.
    Export {
        /// The ids to export; all of them if none.
        ids: Vec<String>,
        /// The folder to write to; by default the prompts folder next to
        /// config.toml.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Check your prompt files without the core: each problem with its file
    /// and line. Exits with status 1 if a file is refused.
    Check {
        /// The folder to check; by default the prompts folder next to
        /// config.toml.
        dir: Option<PathBuf>,
    },
}

/// The prompts folder the core reads: `prompts/` next to `config.toml`.
fn prompts_dir() -> PathBuf {
    let config = nm_engine::config::default_path();
    config.parent().unwrap_or(Path::new(".")).join("prompts")
}

pub fn run(data_dir: &Path, action: AiAction) -> anyhow::Result<()> {
    match action {
        AiAction::Status { check } => {
            let mut c = Client::connect(data_dir)?;
            let s = c.call(method::AI_STATUS, json!({ "check": check }))?;
            print!("{}", status_text(&s));
            Ok(())
        }
        AiAction::Prompts { action: PromptsAction::List { screen, folder, lang } } => {
            let mut c = Client::connect(data_dir)?;
            let list = c.call(method::AI_LIST_PROMPTS, json!({ "screen": screen, "folder": folder, "lang": lang }))?;
            print!("{}", prompts_text(&list));
            Ok(())
        }
        AiAction::Prompts { action: PromptsAction::Export { ids, dir } } => {
            let dir = dir.unwrap_or_else(prompts_dir);
            for line in export(&dir, &ids)? {
                println!("{line}");
            }
            Ok(())
        }
        AiAction::Prompts { action: PromptsAction::Check { dir } } => {
            let dir = dir.unwrap_or_else(prompts_dir);
            let loaded = nm_ai::catalog::load_dir(&dir);
            for w in &loaded.warnings {
                println!("warning: {w}");
            }
            for p in &loaded.problems {
                println!("error: {p}");
            }
            let users = loaded.catalog.prompts().iter().filter(|p| matches!(p.source, nm_ai::prompt::Source::User(_))).count();
            println!("{}: {users} prompt{} of yours in use", dir.display(), if users == 1 { "" } else { "s" });
            if !loaded.problems.is_empty() {
                std::process::exit(1);
            }
            Ok(())
        }
        AiAction::Run { prompt, draft, variant, lang, apply, choice } => {
            run_prompt(data_dir, &prompt, draft, variant.as_deref(), &lang, apply, choice)
        }
    }
}

fn status_text(s: &Value) -> String {
    let mut out = String::new();
    let on = if s["enabled"] == true { "on" } else { "off" };
    out.push_str(&format!("AI: {on}, provider {}, model {}\n", s["provider"].as_str().unwrap_or(""), s["model"].as_str().unwrap_or("")));
    if let Some(v) = s["version"].as_str() {
        let sub = s["subscription"].as_str().unwrap_or("no subscription named");
        out.push_str(&format!("claude {v}, {sub}\n"));
    }
    match (s["ready"] == true, s["reason"].as_str()) {
        (true, _) => out.push_str("Ready.\n"),
        (false, Some(r)) => out.push_str(&format!("Not ready: {r}\n")),
        (false, None) => out.push_str("Not ready.\n"),
    }
    for p in s["prompt_problems"].as_array().into_iter().flatten() {
        out.push_str(&format!("prompt refused: {}\n", p.as_str().unwrap_or("")));
    }
    for p in s["prompt_warnings"].as_array().into_iter().flatten() {
        out.push_str(&format!("prompt warning: {}\n", p.as_str().unwrap_or("")));
    }
    out
}

fn prompts_text(list: &Value) -> String {
    let mut out = String::new();
    for p in list.as_array().into_iter().flatten() {
        let variants: Vec<String> = p["variants"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| format!("{} {}", v["key"].as_str().unwrap_or(""), v["label"].as_str().unwrap_or("")))
            .collect();
        out.push_str(&format!(
            "{}  {:<22} {:<28} {:<8}{}{}\n",
            p["key"].as_str().unwrap_or(" "),
            p["id"].as_str().unwrap_or(""),
            p["label"].as_str().unwrap_or(""),
            p["source"].as_str().unwrap_or(""),
            if p["available"] == true { "" } else { "  (not available yet)" },
            if variants.is_empty() { String::new() } else { format!("  [{}]", variants.join(", ")) },
        ));
    }
    out
}

/// Writes the built-in prompts `ids` (all if empty) to `dir`; returns one
/// line per prompt saying what was done. An existing file is kept.
fn export(dir: &Path, ids: &[String]) -> anyhow::Result<Vec<String>> {
    let builtin = nm_ai::catalog::BUILTIN;
    for id in ids {
        if !builtin.iter().any(|(name, _)| name.trim_end_matches(".toml") == id) {
            let known: Vec<&str> = builtin.iter().map(|(n, _)| n.trim_end_matches(".toml")).collect();
            bail!("no built-in prompt {id}; they are: {}", known.join(", "));
        }
    }
    std::fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
    let mut out = Vec::new();
    for (name, text) in builtin {
        let id = name.trim_end_matches(".toml");
        if !ids.is_empty() && !ids.iter().any(|i| i == id) {
            continue;
        }
        let path = dir.join(name);
        // `create_new` fails if the file exists: a file of the user's is
        // never overwritten, even between the check and the write.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes()).with_context(|| format!("{}", path.display()))?;
                out.push(format!("{}: written", path.display()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                out.push(format!("{}: already there, kept", path.display()));
            }
            Err(e) => return Err(e).with_context(|| format!("{}", path.display())),
        }
    }
    Ok(out)
}

/// The proposal of a run, for the terminal: a replacement as a word diff in
/// the `wdiff` style, `[-removed-]{+added+}`, readable without colours.
fn proposal_text(p: &Value) -> String {
    match p["kind"].as_str() {
        Some("replace") if p["unchanged"] == true => "No change proposed.\n".into(),
        Some("replace") => {
            let mut out = String::new();
            for (i, s) in p["segments"].as_array().into_iter().flatten().enumerate() {
                if i > 0 {
                    out.push_str("\n---\n");
                }
                for piece in s["diff"].as_array().into_iter().flatten() {
                    let t = piece["text"].as_str().unwrap_or("");
                    match piece["kind"].as_str() {
                        Some("removed") => out.push_str(&format!("[-{t}-]")),
                        Some("added") => out.push_str(&format!("{{+{t}+}}")),
                        _ => out.push_str(t),
                    }
                }
                out.push('\n');
            }
            out
        }
        Some("subjects") => p["items"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, s)| format!("{}. {}\n", i + 1, s.as_str().unwrap_or("")))
            .collect(),
        _ => format!("{p}\n"),
    }
}

fn run_prompt(
    data_dir: &Path,
    prompt: &str,
    draft: i64,
    variant: Option<&str>,
    lang: &str,
    apply: bool,
    choice: Option<usize>,
) -> anyhow::Result<()> {
    let mut c = Client::connect(data_dir)?;
    // Subscribe before starting, so that no fragment and no end is missed.
    c.call(method::SUBSCRIBE, json!({}))?;
    // The draft's revision, as the app has it; opened and closed again as
    // the app does around an edit.
    let d = c.call(method::OPEN_DRAFT, json!({ "id": draft }))?;
    let revision = d["revision"].as_i64().unwrap_or_default();
    let result = start_and_follow(&mut c, prompt, draft, revision, variant, lang, apply, choice);
    let _ = c.call(method::CLOSE_DRAFT, json!({ "id": draft }));
    result
}

#[allow(clippy::too_many_arguments)]
fn start_and_follow(
    c: &mut Client,
    prompt: &str,
    draft: i64,
    revision: i64,
    variant: Option<&str>,
    lang: &str,
    apply: bool,
    choice: Option<usize>,
) -> anyhow::Result<()> {
    let (started, early) = c.call_keeping_events(
        method::AI_RUN,
        json!({ "prompt": prompt, "variant": variant, "draft_id": draft, "revision": revision, "lang": lang }),
    )?;
    let run_id = started["run_id"].as_i64().unwrap_or_default();
    let streamed = started["streamed"] == true;
    eprintln!("Run {run_id} (Ctrl-C stops following; `nicemailer call ai_cancel '{{\"run_id\": {run_id}}}'` stops the run)");
    // Events that came before the answer of `ai_run`, then the next ones.
    let mut queue: std::collections::VecDeque<Value> = early.into();
    loop {
        let v = match queue.pop_front() {
            Some(v) => v,
            None => serde_json::from_str(&c.read_line()?)?,
        };
        let p = &v["params"];
        if p["run_id"].as_i64() != Some(run_id) {
            continue;
        }
        match v["method"].as_str() {
            Some("AiDelta") if streamed => {
                use std::io::Write;
                eprint!("{}", p["text"].as_str().unwrap_or(""));
                let _ = std::io::stderr().flush();
            }
            Some("AiFailed") => bail!("{} ({})", p["message"].as_str().unwrap_or(""), p["code"].as_str().unwrap_or("")),
            Some("AiDone") => break,
            _ => {}
        }
    }
    if streamed {
        eprintln!();
    }
    let view = c.call(method::AI_GET_RUN, json!({ "run_id": run_id }))?;
    let proposal = &view["proposal"];
    print!("{}", proposal_text(proposal));
    if !apply {
        let _ = c.call(method::AI_DISMISS, json!({ "run_id": run_id }));
        eprintln!("Not applied (--apply to apply it).");
        return Ok(());
    }
    if proposal["unchanged"] == true {
        let _ = c.call(method::AI_DISMISS, json!({ "run_id": run_id }));
        return Ok(());
    }
    let choice = match (proposal["kind"].as_str(), choice) {
        (Some("subjects"), None) => bail!("choose a subject with --choice N"),
        (_, Some(0)) => bail!("--choice counts from 1"),
        (_, c) => c.map(|n| n - 1),
    };
    let applied = c.call(method::AI_APPLY, json!({ "run_id": run_id, "revision": revision, "choice": choice }))?;
    eprintln!("Applied: the draft is at revision {}.", applied["draft"]["revision"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_reads_as_a_word_diff() {
        let p = json!({"kind": "replace", "unchanged": false, "body": "", "segments": [
            {"start": 0, "end": 3, "old": "", "new": "", "diff": [
                {"kind": "removed", "text": "teh "}, {"kind": "added", "text": "The "}, {"kind": "same", "text": "text"}]},
            {"start": 9, "end": 10, "old": "", "new": "", "diff": [{"kind": "same", "text": "Yes."}]}]});
        assert_eq!(proposal_text(&p), "[-teh -]{+The +}text\n\n---\nYes.\n");
        assert_eq!(proposal_text(&json!({"kind": "replace", "unchanged": true})), "No change proposed.\n");
        assert_eq!(proposal_text(&json!({"kind": "subjects", "items": ["A", "B"]})), "1. A\n2. B\n");
    }

    #[test]
    fn export_writes_the_builtin_prompts_and_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("proofread.toml"), "mine").unwrap();
        let lines = export(dir.path(), &["proofread".into(), "rewrite".into()]).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("proofread.toml: already there, kept"), "{lines:?}");
        assert!(lines[1].ends_with("rewrite.toml: written"), "{lines:?}");
        assert_eq!(std::fs::read_to_string(dir.path().join("proofread.toml")).unwrap(), "mine");
        // What is written loads as the built-in prompt it replaces, unchanged.
        let loaded = nm_ai::catalog::load_dir(dir.path());
        assert!(loaded.problems.iter().all(|p| p.file.ends_with("proofread.toml")), "{:?}", loaded.problems);
        let builtin = nm_ai::catalog::load(&[]);
        let rewrite = loaded.catalog.get("rewrite").unwrap();
        assert!(matches!(rewrite.source, nm_ai::prompt::Source::User(_)));
        assert_eq!(rewrite.hash, builtin.catalog.get("rewrite").unwrap().hash);
        // All, and an unknown id refused.
        let all = tempfile::tempdir().unwrap();
        assert_eq!(export(all.path(), &[]).unwrap().len(), nm_ai::catalog::BUILTIN.len());
        assert!(export(all.path(), &["nothing".into()]).is_err());
    }

    #[test]
    fn status_and_prompts_read_well() {
        let s = json!({"enabled": true, "provider": "claude-code", "model": "claude-sonnet-5-5", "ready": true,
                       "version": "2.1.325", "subscription": "Claude Pro", "prompt_problems": ["p.toml, line 3: x"], "prompt_warnings": []});
        assert_eq!(
            status_text(&s),
            "AI: on, provider claude-code, model claude-sonnet-5-5\nclaude 2.1.325, Claude Pro\nReady.\nprompt refused: p.toml, line 3: x\n"
        );
        let l = json!([{"key": "r", "id": "rewrite", "label": "Rewrite", "source": "builtin", "available": true,
                         "variants": [{"key": "c", "label": "Shorter"}]},
                        {"key": "w", "id": "write-from-notes", "label": "Write", "source": "builtin", "available": false, "variants": []}]);
        let t = prompts_text(&l);
        assert!(t.contains("[c Shorter]") && t.contains("write-from-notes") && t.contains("(not available yet)"), "{t}");
    }
}
