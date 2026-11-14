//! The AI assistance in the core (`specifications/ia.md`, sections 10 and
//! 14): the commands of the contract, the runs in the background, the
//! history.
//!
//! # One run
//!
//! ```text
//! ai_run ─▶ checks (on, prompt, inputs, draft revision, text)  ─▶ row `running`
//!        ─▶ answers {run_id}            the client shows the panel at once
//!        ─▶ task: waits for a slot (ai.max_parallel)
//!                 provider.run ─▶ AiDelta… (transient)
//!                 answer::read ─▶ row `done` + proposal kept ─▶ AiDone
//!                              └▶ row `failed`                 ─▶ AiFailed
//! ai_apply ─▶ the draft is still at the revision of the run ─▶ body spliced,
//!             saved, decision `accepted`
//! ```
//!
//! Everything that can be checked is checked before the row is written and
//! the provider started: a run that cannot work answers an error at once,
//! rather than an `AiFailed` a moment later.
//!
//! # Cancel or end: one winner
//!
//! A run is in [`AiState::running`] from its start until it ends. Whoever
//! takes it out of that map, the task at its end or `ai_cancel`, decides
//! how it ended, under the map's lock: a cancelled run cannot then be
//! recorded as done, nor a done one as cancelled.
//!
//! # What is wired so far
//!
//! The prompts of the compose screen whose inputs are `draft.own_text` and
//! `draft.headers` (proofread, rewrite, translate, subjects): the first
//! milestone of `ia.md`, section 14. The others are listed with
//! `available: false`, and `ai_run` refuses them with a clear message.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nm_ai::answer::{self, Answer, Sent};
use nm_ai::catalog::{self, Loaded};
use nm_ai::prompt::{Input, Output, Prompt, Scope, Screen, Source as PromptSource};
use nm_ai::{AiError, AiRequest, Provider};
use nm_api::{
    AiApplyParams, AiApplyResult, AiDiffPiece, AiListPromptsParams, AiPromptView, AiProposal, AiRunIdParams, AiRunParams,
    AiRunResult, AiRunView, AiSegmentChange, AiStatusParams, AiStatusResult, AiVariantView, CommandResult, DraftView,
    Event, RpcError, error_code,
};
use nm_core::Source;
use nm_engine::config::{AiProviderKind, AiSettings};
use nm_store::ai_runs::{Decision, NewRun, RunDone, RunEnd};
use nm_store::drafts::DraftEdit;
use tokio::sync::Semaphore;

use crate::{Core, with_reader, with_store};

/// The state of the AI assistance, held by the core.
pub(crate) struct AiState {
    /// The user's prompts folder, next to `config.toml`.
    prompts_dir: PathBuf,
    /// The empty work directory of the `claude` binary.
    workdir: PathBuf,
    /// The prompts, and the state of the folder they were read from.
    catalog: Mutex<(Fingerprint, Arc<Loaded>)>,
    /// `ai.max_parallel` places, read at start.
    slots: Arc<Semaphore>,
    /// Runs started and not ended, by id.
    running: Mutex<HashMap<i64, Running>>,
    /// Runs ended with a proposal not applied nor dismissed yet, by id. In
    /// memory only: after a restart, the proposal is gone and the run is
    /// started again.
    finished: Mutex<HashMap<i64, Finished>>,
}

/// What a run was started on, to apply its answer.
#[derive(Clone)]
struct Context {
    prompt: Prompt,
    draft_id: i64,
    /// The draft's revision when the run started.
    revision: i64,
    /// The draft's body then, and the byte ranges of the segments sent.
    body: String,
    ranges: Vec<Range<usize>>,
}

struct Running {
    abort: Option<tokio::task::AbortHandle>,
    context: Context,
}

struct Finished {
    context: Context,
    answer: Answer,
}

/// The `.toml` files of the prompts folder: name, size, modification time.
/// When it differs from the one the catalog was read with, the catalog is
/// read again: a file added, removed or saved is taken at the next use,
/// without a watch thread.
type Fingerprint = Option<Vec<(std::ffi::OsString, u64, Option<SystemTime>)>>;

fn fingerprint(dir: &Path) -> Fingerprint {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut v: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .map(|e| {
            let m = e.metadata().ok();
            (e.file_name(), m.as_ref().map_or(0, |m| m.len()), m.and_then(|m| m.modified().ok()))
        })
        .collect();
    v.sort();
    Some(v)
}

impl AiState {
    pub(crate) fn new(config_path: &Path, data_dir: &Path, settings: &AiSettings) -> AiState {
        let prompts_dir = config_path.parent().unwrap_or(Path::new(".")).join("prompts");
        let loaded = catalog::load_dir(&prompts_dir);
        log_problems(&loaded);
        AiState {
            catalog: Mutex::new((fingerprint(&prompts_dir), Arc::new(loaded))),
            prompts_dir,
            workdir: data_dir.join("ai.noindex").join("work"),
            slots: Arc::new(Semaphore::new(settings.max_parallel as usize)),
            running: Mutex::new(HashMap::new()),
            finished: Mutex::new(HashMap::new()),
        }
    }

    /// The prompts, read again if the folder changed since.
    fn catalog(&self) -> Arc<Loaded> {
        let now = fingerprint(&self.prompts_dir);
        let mut c = self.catalog.lock().unwrap();
        if c.0 != now {
            let loaded = catalog::load_dir(&self.prompts_dir);
            log_problems(&loaded);
            *c = (now, Arc::new(loaded));
        }
        c.1.clone()
    }
}

fn log_problems(l: &Loaded) {
    for p in &l.problems {
        eprintln!("prompts : {p}");
    }
}

/// An AI error as a JSON-RPC error: code `AI`, the AI code in `data`.
fn ai_error(e: AiError) -> RpcError {
    let mut err = RpcError::new(error_code::AI, e.to_string());
    err.data = Some(serde_json::json!({ "code": e.code() }));
    err
}

fn settings(core: &Core) -> AiSettings {
    core.engine.lock().unwrap().config().ai.clone()
}

/// Whether this version of the core computes every input of the prompt
/// from this screen (module documentation, "What is wired so far").
fn available(p: &Prompt) -> bool {
    p.scope == Scope::Draft
        && p.inputs.iter().all(|i| matches!(i, Input::DraftOwnText | Input::DraftHeaders))
        && matches!(p.output, Output::Replace | Output::Subjects)
}

/// The provider chosen by the settings. `NICEMAILER_FAKE_AI` names a JSON
/// file `{"<task>": "<answer>"}` served by the fake provider instead: for
/// the tests of the core and the scripted tests of the app, which must not
/// depend on a network, an account or a model.
fn provider(core: &Core, s: &AiSettings) -> Result<Provider, AiError> {
    if let Some(path) = std::env::var_os("NICEMAILER_FAKE_AI") {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| AiError::Failed(format!("fake AI answers {}: {e}", Path::new(&path).display())))?;
        let table: HashMap<String, String> =
            serde_json::from_str(&text).map_err(|e| AiError::Failed(format!("fake AI answers: {e}")))?;
        let fake = table.into_iter().fold(nm_ai::fake::Fake::new(), |f, (task, answer)| f.answer(&task, answer));
        return Ok(Provider::Fake(fake));
    }
    match s.provider {
        AiProviderKind::ClaudeCode => {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            let binary = nm_ai::claude_code::find_binary(s.claude_path.as_deref(), &home).ok_or(AiError::ProviderMissing)?;
            Ok(Provider::ClaudeCode(nm_ai::claude_code::ClaudeCode {
                binary,
                workdir: core.ai.workdir.clone(),
                timeout: Duration::from_secs(s.timeout_secs),
            }))
        }
        AiProviderKind::AnthropicApi => Err(AiError::BadInput(
            "the anthropic-api provider is not available yet: set ai.provider = \"claude-code\" in config.toml".into(),
        )),
    }
}

/// `ai_status`.
pub(crate) async fn status(core: &Arc<Core>, p: AiStatusParams) -> Result<AiStatusResult, RpcError> {
    let s = settings(core);
    let loaded = core.ai.catalog();
    let mut out = AiStatusResult {
        enabled: s.enabled,
        provider: s.provider.as_str().into(),
        model: s.model.clone(),
        prompt_problems: loaded.problems.iter().map(|p| p.to_string()).collect(),
        prompt_warnings: loaded.warnings.iter().map(|p| p.to_string()).collect(),
        ..Default::default()
    };
    match provider(core, &s) {
        Err(e) => out.reason = Some(e.to_string()),
        Ok(Provider::ClaudeCode(c)) if p.check => match c.status().await {
            Ok(st) => {
                out.version = Some(st.version);
                out.subscription = st.subscription;
                out.ready = s.enabled;
            }
            Err(e) => out.reason = Some(e.to_string()),
        },
        Ok(_) => out.ready = s.enabled,
    }
    if !s.enabled && out.reason.is_none() {
        out.reason = Some("the AI assistance is off: set ai.enabled = true in config.toml".into());
    }
    Ok(out)
}

/// `ai_list_prompts`.
pub(crate) fn list_prompts(core: &Arc<Core>, p: AiListPromptsParams) -> Result<Vec<AiPromptView>, RpcError> {
    let screen = match p.screen.as_str() {
        "compose" => Screen::Compose,
        "reading" => Screen::Reading,
        "list" => Screen::List,
        "attachment" => Screen::Attachment,
        other => return Err(RpcError::new(error_code::INVALID_PARAMS, format!("unknown screen {other:?}"))),
    };
    let lang = p.lang.as_deref().unwrap_or("en");
    let loaded = core.ai.catalog();
    Ok(loaded
        .catalog
        .menu(screen, p.folder)
        .into_iter()
        .map(|q| AiPromptView {
            id: q.id.clone(),
            key: q.key.to_string(),
            label: q.label.get(lang).into(),
            scope: q.scope.as_str().into(),
            output: q.output.as_str().into(),
            variants: q.variants.iter().map(|v| AiVariantView { key: v.key.to_string(), label: v.label.get(lang).into() }).collect(),
            source: match q.source {
                PromptSource::Builtin => "builtin".into(),
                PromptSource::User(_) => "user".into(),
            },
            available: available(q),
        })
        .collect())
}

/// The byte offset of a UTF-16 offset in `s`; `None` inside a character.
fn byte_of_utf16(s: &str, units: u32) -> Option<usize> {
    let mut seen = 0u32;
    for (i, c) in s.char_indices() {
        if seen == units {
            return Some(i);
        }
        if seen > units {
            return None;
        }
        seen += c.len_utf16() as u32;
    }
    (seen == units).then_some(s.len())
}

fn utf16_of_byte(s: &str, byte: usize) -> u32 {
    s[..byte].encode_utf16().count() as u32
}

/// `ai_run`.
pub(crate) async fn run(core: &Arc<Core>, p: AiRunParams, source: Source) -> Result<AiRunResult, RpcError> {
    let s = settings(core);
    if !s.enabled {
        return Err(RpcError::new(error_code::AI_DISABLED, "the AI assistance is off: set ai.enabled = true in config.toml"));
    }
    let loaded = core.ai.catalog();
    let prompt = loaded
        .catalog
        .get(&p.prompt)
        .cloned()
        .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no prompt {}", p.prompt)))?;
    if !available(&prompt) {
        return Err(ai_error(AiError::BadInput(format!(
            "the prompt {} is not available in this version of NiceMailer yet",
            prompt.id
        ))));
    }
    let variant = match p.variant.as_deref() {
        None => None,
        Some(v) => {
            let mut chars = v.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => return Err(RpcError::new(error_code::INVALID_PARAMS, format!("a variant is one key, not {v:?}"))),
            }
        }
    };
    // The parameters first (a variant missing, unknown), before reading
    // anything: building the system prompt checks the variant.
    let system = prompt.system(variant, &p.lang).map_err(|m| RpcError::new(error_code::INVALID_PARAMS, m))?;
    let (Some(draft_id), Some(revision)) = (p.draft_id, p.revision) else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "this prompt needs draft_id and revision"));
    };
    let draft = with_reader(core, move |st| st.get_draft(draft_id)).await?;
    if draft.revision != revision {
        let mut err = RpcError::new(error_code::REVISION_CONFLICT, "the draft changed since: save it and run again");
        err.data = Some(serde_json::json!({ "revision": draft.revision }));
        return Err(err);
    }

    // The segments sent: the selection, or the user's own text.
    let body = draft.body.clone();
    let ranges: Vec<Range<usize>> = match p.selection {
        Some(sel) => {
            let (Some(a), Some(b)) = (byte_of_utf16(&body, sel.start), byte_of_utf16(&body, sel.end)) else {
                return Err(RpcError::new(error_code::INVALID_PARAMS, "the selection is outside the draft's text"));
            };
            if a >= b {
                return Err(ai_error(AiError::BadInput("the selection is empty".into())));
            }
            // One range: a selection is one segment.
            std::iter::once(a..b).collect()
        }
        None => nm_core::segments::own_segments(&body),
    };
    let texts: Vec<&str> = ranges.iter().map(|r| &body[r.clone()]).collect();
    let mut sections = Vec::new();
    for input in &prompt.inputs {
        sections.push(match input {
            Input::DraftOwnText if prompt.output == Output::Replace => nm_ai::message::segments(&texts).map_err(ai_error)?,
            Input::DraftOwnText => nm_ai::message::draft_text(&texts).map_err(ai_error)?,
            Input::DraftHeaders => draft_headers(&draft),
            _ => unreachable!("`available` lets only these inputs through"),
        });
    }
    let message = sections.join("\n\n");
    let request = AiRequest {
        system,
        message,
        model: prompt.model.clone().unwrap_or_else(|| s.model.clone()),
        schema: prompt.output.schema(&[]),
        max_output_tokens: prompt.max_output_tokens,
    };
    let provider = provider(core, &s).map_err(ai_error)?;

    let input = format!("{}\n\n{}", request.system, request.message);
    let target = serde_json::json!({ "draft": draft_id }).to_string();
    let row = (
        prompt.id.clone(),
        variant.map(|c| c.to_string()),
        match prompt.source {
            PromptSource::Builtin => "builtin",
            PromptSource::User(_) => "user",
        },
        prompt.hash,
        provider.name(),
        request.model.clone(),
        prompt.scope.as_str(),
        p.lang.clone(),
    );
    let run_id = with_store(core, move |st| {
        st.ai_run_start(&NewRun {
            created_by: source,
            prompt_id: &row.0,
            variant: row.1.as_deref(),
            prompt_source: row.2,
            prompt_hash: &row.3,
            provider: row.4,
            model: &row.5,
            scope: row.6,
            target: &target,
            lang: &row.7,
            input: &input,
        })
    })
    .await?;

    let streamed = !prompt.output.is_structured();
    let context = Context { prompt, draft_id, revision, body, ranges };
    core.ai.running.lock().unwrap().insert(run_id, Running { abort: None, context });
    let task = tokio::spawn(execute(core.clone(), run_id, request, provider));
    // The task may already have ended (a fake answer): then it is no longer
    // in the map, and there is nothing left to abort.
    if let Some(r) = core.ai.running.lock().unwrap().get_mut(&run_id) {
        r.abort = Some(task.abort_handle());
    }
    Ok(AiRunResult { run_id, streamed })
}

fn draft_headers(d: &DraftView) -> String {
    let label = |a: &nm_api::AddressView| match &a.name {
        Some(n) if !n.trim().is_empty() => format!("{} <{}>", n.trim(), a.addr),
        _ => a.addr.clone(),
    };
    let to: Vec<String> = d.to.iter().map(label).collect();
    let cc: Vec<String> = d.cc.iter().map(label).collect();
    nm_ai::message::draft_headers(&label(&d.from), &to, &cc, &d.subject)
}

/// The task of one run (module documentation, "One run").
async fn execute(core: Arc<Core>, run_id: i64, request: AiRequest, provider: Provider) {
    // Rust note: `acquire_owned` waits for a place and gives a permit that
    // frees it when dropped, at the end of this function, whatever happens.
    let Ok(_slot) = core.ai.slots.clone().acquire_owned().await else { return };
    let events = core.clone();
    // The tags of the segments are hidden from the text shown as it comes
    // (`nm_ai::message::SegmentFilter`); other outputs are shown as they are.
    let replace = core.ai.running.lock().unwrap().get(&run_id).is_some_and(|r| r.context.prompt.output == Output::Replace);
    let mut filter = replace.then(nm_ai::message::SegmentFilter::new);
    let result = provider
        .run(&request, |text| {
            let shown = match filter.as_mut() {
                Some(f) => f.push(text),
                None => text.to_string(),
            };
            if !shown.is_empty() {
                events.events.lock().unwrap().push_transient(Event::AiDelta { run_id, text: shown });
            }
        })
        .await;
    // The winner of the end (module documentation): if `ai_cancel` already
    // took the run, its end is recorded and this result is dropped.
    let Some(running) = core.ai.running.lock().unwrap().remove(&run_id) else { return };
    let context = running.context;
    let outcome = match result {
        Ok(o) => o,
        Err(e) => return fail(&core, run_id, e, None).await,
    };
    let sent = Sent { segments: context.ranges.len(), threads: &[], targets: &[] };
    match answer::read(context.prompt.output, &outcome, &sent) {
        Err(e) => fail(&core, run_id, e, Some(outcome.text.clone())).await,
        Ok(answer) => {
            // Rust note: the closure given to `with_store` runs on another
            // thread and must own what it uses (`move`): `outcome` is moved
            // in, and the `RunDone` that borrows its text is built there.
            let stored = with_store(&core, move |st| {
                st.ai_run_done(
                    run_id,
                    &RunDone {
                        output: &outcome.text,
                        input_tokens: Some(outcome.usage.input_tokens),
                        output_tokens: Some(outcome.usage.output_tokens),
                        cost_usd: outcome.cost_usd,
                        billed: outcome.billed,
                        duration_ms: outcome.duration_ms,
                    },
                )
            })
            .await;
            if let Err(e) = stored {
                eprintln!("ai : exécution {run_id} non enregistrée : {}", e.message);
            }
            core.ai.finished.lock().unwrap().insert(run_id, Finished { context, answer });
            core.events.lock().unwrap().push(Event::AiDone { run_id });
        }
    }
}

/// Records a failed run and announces it.
async fn fail(core: &Arc<Core>, run_id: i64, e: AiError, output: Option<String>) {
    let error = format!("{}: {e}", e.code());
    let stored = with_store(core, move |st| st.ai_run_end(run_id, RunEnd::Failed, &error, output.as_deref())).await;
    if let Err(err) = stored {
        eprintln!("ai : échec de l'exécution {run_id} non enregistré : {}", err.message);
    }
    core.events.lock().unwrap().push(Event::AiFailed { run_id, code: e.code().into(), message: e.to_string() });
}

/// `ai_cancel`.
pub(crate) async fn cancel(core: &Arc<Core>, p: AiRunIdParams) -> Result<CommandResult, RpcError> {
    let Some(running) = core.ai.running.lock().unwrap().remove(&p.run_id) else {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("AI run {} is not running", p.run_id)));
    };
    // Dropping the task's future kills the provider's process group
    // (`nm_ai::claude_code`, "Cancellation").
    if let Some(a) = running.abort {
        a.abort();
    }
    let run_id = p.run_id;
    with_store(core, move |st| st.ai_run_end(run_id, RunEnd::Cancelled, "ai_cancelled: cancelled", None)).await?;
    core.events.lock().unwrap().push(Event::AiFailed {
        run_id,
        code: "ai_cancelled".into(),
        message: "cancelled".into(),
    });
    Ok(CommandResult { changed: 0 })
}

/// The proposal of a finished run, as the client shows it.
fn proposal(f: &Finished) -> Option<AiProposal> {
    let c = &f.context;
    match &f.answer {
        Answer::Segments(new) => {
            let body = nm_core::segments::splice(&c.body, &c.ranges, new).ok()?;
            let segments: Vec<AiSegmentChange> = c
                .ranges
                .iter()
                .zip(new)
                .map(|(r, n)| {
                    let old = &c.body[r.clone()];
                    AiSegmentChange {
                        start: utf16_of_byte(&c.body, r.start),
                        end: utf16_of_byte(&c.body, r.end),
                        old: old.to_string(),
                        new: n.clone(),
                        diff: nm_ai::diff::words(old.trim(), n)
                            .into_iter()
                            .map(|p| match p {
                                nm_ai::diff::Piece::Same(t) => AiDiffPiece { kind: "same".into(), text: t },
                                nm_ai::diff::Piece::Removed(t) => AiDiffPiece { kind: "removed".into(), text: t },
                                nm_ai::diff::Piece::Added(t) => AiDiffPiece { kind: "added".into(), text: t },
                            })
                            .collect(),
                    }
                })
                .collect();
            let unchanged = body == c.body;
            Some(AiProposal::Replace { segments, body, unchanged })
        }
        Answer::Subjects(items) => Some(AiProposal::Subjects { items: items.clone() }),
        _ => None,
    }
}

/// `ai_get_run`.
pub(crate) async fn get_run(core: &Arc<Core>, p: AiRunIdParams) -> Result<AiRunView, RpcError> {
    let id = p.run_id;
    let row = with_reader(core, move |st| st.ai_run(id))
        .await?
        .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no AI run {id}")))?;
    let proposal = core.ai.finished.lock().unwrap().get(&id).and_then(proposal);
    let (error_code, error) = match row.error.as_deref().and_then(|e| e.split_once(": ")) {
        Some((code, message)) => (Some(code.to_string()), Some(message.to_string())),
        None => (None, row.error.clone()),
    };
    Ok(AiRunView {
        run_id: id,
        prompt: row.prompt_id,
        variant: row.variant,
        state: row.state,
        decision: row.decision,
        proposal,
        error_code,
        error,
    })
}

/// `ai_apply`.
pub(crate) async fn apply(core: &Arc<Core>, p: AiApplyParams, source: Source) -> Result<AiApplyResult, RpcError> {
    let finished = core
        .ai
        .finished
        .lock()
        .unwrap()
        .remove(&p.run_id)
        .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("AI run {} has no proposal to apply", p.run_id)))?;
    // Rust note: a closure that may fail, to put the proposal back on any
    // refusal below: a client told "the draft changed" can still dismiss
    // it, and a retry with the right revision still finds it.
    let put_back = |f: Finished| {
        core.ai.finished.lock().unwrap().insert(p.run_id, f);
    };
    let c = &finished.context;
    // The text the AI saw is the text of that revision: a later one may
    // hold words typed since, which applying would erase.
    if p.revision != c.revision {
        let current = c.revision;
        put_back(finished);
        let mut err = RpcError::new(error_code::REVISION_CONFLICT, "the draft changed since the AI read it: run it again");
        err.data = Some(serde_json::json!({ "revision": current }));
        return Err(err);
    }
    let edit = match &finished.answer {
        Answer::Segments(new) => match nm_core::segments::splice(&c.body, &c.ranges, new) {
            Ok(body) => DraftEdit { body: Some(body), ..Default::default() },
            Err(m) => {
                put_back(finished);
                return Err(RpcError::new(error_code::INTERNAL_ERROR, m));
            }
        },
        Answer::Subjects(items) => match p.choice.and_then(|i| items.get(i)) {
            Some(subject) => DraftEdit { subject: Some(subject.clone()), ..Default::default() },
            None => {
                put_back(finished);
                return Err(RpcError::new(error_code::INVALID_PARAMS, "choose one of the subjects (choice)"));
            }
        },
        _ => {
            put_back(finished);
            return Err(RpcError::new(error_code::INVALID_PARAMS, "this proposal cannot be applied yet"));
        }
    };
    let (draft_id, revision, run_id) = (c.draft_id, c.revision, p.run_id);
    let saved = with_store(core, move |st| {
        st.edit_draft(draft_id, revision, &edit, source)?;
        st.ai_run_decide(run_id, Decision::Accepted)?;
        st.get_draft(draft_id)
    })
    .await;
    match saved {
        Ok(draft) => Ok(AiApplyResult { draft }),
        Err(e) => {
            put_back(finished);
            Err(e)
        }
    }
}

/// `ai_dismiss`: the proposal is turned down; a run still going is
/// cancelled.
pub(crate) async fn dismiss(core: &Arc<Core>, p: AiRunIdParams) -> Result<CommandResult, RpcError> {
    if core.ai.running.lock().unwrap().contains_key(&p.run_id) {
        return cancel(core, p).await;
    }
    if core.ai.finished.lock().unwrap().remove(&p.run_id).is_none() {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("AI run {} has no proposal", p.run_id)));
    }
    let id = p.run_id;
    with_store(core, move |st| st.ai_run_decide(id, Decision::Rejected)).await?;
    Ok(CommandResult { changed: 0 })
}

/// At start, then every six hours: the runs a stop left `running` fail,
/// and the texts older than `ai.history_days` are purged (`ia.md`,
/// section 9).
pub(crate) fn start_upkeep(core: &Arc<Core>) {
    {
        let mut st = core.lock_store("ai");
        match st.ai_runs_abandoned() {
            Ok(0) => {}
            Ok(n) => eprintln!("ai : {n} exécution(s) interrompue(s) par l'arrêt du noyau, marquées en échec"),
            Err(e) => eprintln!("ai : {e}"),
        }
    }
    let core = core.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            every.tick().await;
            let days = settings(&core).history_days as i64;
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            let before = now - days * 86_400;
            match with_store(&core, move |st| st.ai_runs_purge(before)).await {
                Ok(0) => {}
                Ok(n) => eprintln!("ai : textes de {n} exécution(s) purgés ({days} jours)"),
                Err(e) => eprintln!("ai : purge : {}", e.message),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_convert_both_ways() {
        let s = "été 🌞 ok";
        // é is 1 unit, 🌞 is 2 (a surrogate pair).
        assert_eq!(byte_of_utf16(s, 0), Some(0));
        assert_eq!(byte_of_utf16(s, 3), Some("été".len()));
        assert_eq!(byte_of_utf16(s, 4), Some("été ".len()));
        assert_eq!(byte_of_utf16(s, 5), None, "inside the surrogate pair");
        assert_eq!(byte_of_utf16(s, 6), Some("été 🌞".len()));
        assert_eq!(byte_of_utf16(s, 9), Some(s.len()));
        assert_eq!(byte_of_utf16(s, 10), None);
        for b in [0, "été".len(), "été 🌞".len(), s.len()] {
            assert_eq!(byte_of_utf16(s, utf16_of_byte(s, b)), Some(b));
        }
    }
}
